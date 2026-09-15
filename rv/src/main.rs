//! The `rv` command line.
//!
//! One invariant decides who writes what: **only the bare TUI opens a review**
//! — [`session::build`], which records the session — and every subcommand
//! resolves the same range through [`session::read`] and writes no session
//! record. The subcommands fall into two families: queries (`status`, `render`)
//! and the agent's comment operations (`comment`, `resolve`, `abandon`), which
//! write comments and the export but never the session.
//!
//! # Naming the range
//!
//! The head can be given two ways: as the positional `TARGET` (`rv my-feature`)
//! or as `--to`. `--to` wins when both appear, and it is also the escape hatch
//! for the one collision the positional form has: because subcommands share the
//! first positional slot, a bookmark literally named `render` or `status` has to
//! be passed as `rv --to status`. The base is always `--from`, defaulting to
//! `trunk()`.
//!
//! # Failure
//!
//! Nothing here panics on a foreseeable problem. `run` returns
//! [`anyhow::Result`] and `main` prints the whole error chain to stderr and
//! exits non-zero, so an unreadable workspace or an empty range reads as a
//! sentence rather than a backtrace.

use std::path::PathBuf;
use std::process::ExitCode;

mod cli;

use anyhow::Context as _;
use anyhow::Result;
use clap::Parser;
use cli::Command;
use rv::app::App;
use rv::app::DiffEngine;
use rv::config;
use rv::session;
use rv_core::model::Side;
use rv_core::store::CommentState;

/// What `jj` shows for a change nobody has described yet; reused here so the
/// text output of `status` does not print a blank column instead.
pub(crate) const NO_DESCRIPTION: &str = "(no description set)";

#[derive(Debug, Parser)]
#[command(name = "rv", version, about = "Review a jj stack in the terminal")]
struct Cli {
    /// Revision to review up to: a bookmark, change id or commit id. `--to`
    /// takes precedence when both are given.
    #[arg(value_name = "TARGET")]
    target: Option<String>,

    /// Revision the review starts from [default: trunk()].
    #[arg(long, value_name = "REV")]
    from: Option<String>,

    /// Revision the review ends at, overriding TARGET [default: @].
    #[arg(long, value_name = "REV")]
    to: Option<String>,

    /// Workspace root to review [default: the current directory].
    #[arg(long, value_name = "PATH")]
    repo: Option<PathBuf>,

    /// Diff with the in-process engine instead of difftastic.
    ///
    /// What a reviewer with no `difft` on `PATH` sees: line-based rather than
    /// structural, with context lines around each change. Useful when difftastic
    /// is slow on a large file, or to check what the fallback looks like.
    #[arg(long)]
    no_difft: bool,

    #[command(subcommand)]
    command: Option<Command>,
}

/// `message`, with `-` meaning "read stdin" — the `git commit -F -` convention,
/// so a multi-line body full of shell-significant characters arrives byte-exact
/// instead of one quoting mistake from mangled.
fn body_from(message: String) -> Result<String> {
    if message != "-" {
        return Ok(message);
    }
    let mut body = String::new();
    std::io::Read::read_to_string(&mut std::io::stdin(), &mut body)
        .context("could not read the message from stdin")?;
    Ok(body)
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(error) => {
            // `{:#}` is anyhow's single-line chain: every `context` layer and
            // the underlying `rv-core` message, separated by ": ".
            eprintln!("rv: {error:#}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<ExitCode> {
    let cli = Cli::parse();

    let repo_root = match cli.repo {
        Some(repo) => repo,
        None => std::env::current_dir().context("could not read the current directory")?,
    };
    // `--to` wins over the positional target; see the module docs.
    let head = cli.to.or(cli.target);

    // A bare `rv` is the reviewer, which is where a review *begins* and so the
    // one command that writes the session record. The subcommands are queries
    // over the same range: `status` reports it and `render` writes only the
    // export, and neither re-points `session.toml`.
    let read = || session::read(&repo_root, cli.from.as_deref(), head.as_deref());
    match cli.command {
        None => {
            let config = config::load()?;
            let settings = config::load_settings()?;
            // The flag outranks the settings file: `--no-difft` on the command
            // line is an answer for this run, whatever the default is.
            let engine = if cli.no_difft {
                DiffEngine::Fallback
            } else {
                match settings.engine {
                    Some(config::EngineName::Fallback) => DiffEngine::Fallback,
                    Some(config::EngineName::Auto) | None => DiffEngine::Auto,
                }
            };
            App::run_with_config(
                session::build(&repo_root, cli.from.as_deref(), head.as_deref())?,
                engine,
                &config,
                &settings,
            )?;
            Ok(ExitCode::SUCCESS)
        }
        Some(Command::Comment {
            file,
            line,
            side,
            message,
        }) => {
            let review = read()?;
            let body = body_from(message)?;
            let comment = session::add_comment(&review, &file, side.into(), line, &body)?;
            println!(
                "saved {} at {}:{} ({})",
                comment.id,
                comment.anchor.file,
                comment.anchor.line,
                match comment.anchor.side {
                    Side::Left => "left",
                    Side::Right => "right",
                }
            );
            Ok(ExitCode::SUCCESS)
        }
        Some(Command::Comments { json, state }) => {
            comments(&read()?, json, state.map(CommentState::from))?;
            Ok(ExitCode::SUCCESS)
        }
        Some(Command::Reply { id, message }) => {
            let review = read()?;
            let body = body_from(message)?;
            reply(&review, &id, &body)?;
            Ok(ExitCode::SUCCESS)
        }
        Some(Command::Resolve { id, by }) => {
            settle(&read()?, &id, CommentState::Resolved, by.into())?;
            Ok(ExitCode::SUCCESS)
        }
        Some(Command::Abandon { id, by }) => {
            settle(&read()?, &id, CommentState::Abandoned, by.into())?;
            Ok(ExitCode::SUCCESS)
        }
        Some(Command::Diff { file, json }) => {
            diff(&read()?, file.as_deref(), json, cli.no_difft)?;
            Ok(ExitCode::SUCCESS)
        }
        Some(Command::Render { out }) => {
            render(&read()?, out.as_deref())?;
            Ok(ExitCode::SUCCESS)
        }
        Some(Command::Config) => {
            commands::edit_config()?;
            Ok(ExitCode::SUCCESS)
        }
        Some(Command::Reviews) => {
            commands::reviews(&repo_root)?;
            Ok(ExitCode::SUCCESS)
        }
        Some(Command::Keymap { show }) => {
            if show {
                commands::print_keymap()?;
            } else {
                commands::edit_keymap()?;
            }
            Ok(ExitCode::SUCCESS)
        }
        Some(Command::Flag {
            file,
            line,
            side,
            message,
        }) => {
            let review = read()?;
            let reason = body_from(message)?;
            commands::attention::flag(&review, &file, side.into(), line, &reason)?;
            Ok(ExitCode::SUCCESS)
        }
        Some(Command::Flags { json, open }) => {
            commands::attention::flags(&read()?, json, open)?;
            Ok(ExitCode::SUCCESS)
        }
        Some(Command::Ack { id }) => {
            commands::attention::ack(&read()?, &id)?;
            Ok(ExitCode::SUCCESS)
        }
        Some(Command::Unflag { id }) => {
            commands::attention::unflag(&read()?, &id)?;
            Ok(ExitCode::SUCCESS)
        }
        Some(Command::Review { file, change }) => {
            commands::attention::mark_reviewed(&read()?, &file, change.as_deref())?;
            Ok(ExitCode::SUCCESS)
        }
        Some(Command::Unreview { file, change }) => {
            commands::attention::unmark_reviewed(&read()?, &file, change.as_deref())?;
            Ok(ExitCode::SUCCESS)
        }
        Some(Command::Reviewed { json }) => {
            commands::attention::reviewed(&read()?, json)?;
            Ok(ExitCode::SUCCESS)
        }
        Some(Command::Status { json, check }) => {
            // `--check` composes with `--json` — print the report *and* set
            // the code — and prints nothing on its own: the worker's poll and
            // a CI gate are exit-code questions.
            let open = if json {
                status(&read()?, true)?
            } else if check {
                commands::check(&read()?)?
            } else {
                status(&read()?, false)?
            };
            Ok(if check && open {
                ExitCode::FAILURE
            } else {
                ExitCode::SUCCESS
            })
        }
    }
}

mod commands;
use commands::comments;
use commands::diff;
use commands::render;
use commands::reply;
use commands::settle;
use commands::status;
