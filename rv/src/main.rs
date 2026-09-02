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

use anyhow::Context as _;
use anyhow::Result;
use clap::Parser;
use clap::Subcommand;
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

#[derive(Debug, Subcommand)]
enum Command {
    /// Add a comment to the review, exactly as the TUI would.
    ///
    /// For reviewer agents: the anchor, the id and the store are all handled,
    /// so nothing writes `.review/` files by hand.
    Comment {
        /// The file, as `rv status` lists it.
        file: String,
        /// The 1-based line the comment is about.
        #[arg(long)]
        line: u32,
        /// Which side of the diff the line is on: `right` is the code as it
        /// will exist (the default), `left` a removed line's base side.
        #[arg(long, default_value = "right")]
        side: SideArg,
        /// The comment itself; `-` reads it from stdin, so a body full of
        /// quotes and backticks never meets the shell.
        #[arg(short, long)]
        message: String,
    },
    /// List the review's comments — the agent's read channel.
    Comments {
        /// Emit JSON instead of text. The JSON is the contract.
        #[arg(long)]
        json: bool,
        /// Only comments in this state, e.g. `--state open` for "what is
        /// waiting on me".
        #[arg(long)]
        state: Option<StateArg>,
    },
    /// Store a reply on a comment — the agent's answer channel.
    ///
    /// A second reply replaces the first. Replying changes no state: resolving
    /// stays its own deliberate act.
    Reply {
        /// The comment's id, from `rv comments`.
        id: String,
        /// The reply; `-` reads it from stdin.
        #[arg(short, long)]
        message: String,
    },
    /// Mark a comment resolved: it was addressed.
    ///
    /// Records who settled it. The default is `agent`, because this command is
    /// the agent's path — a human resolves in the TUI with `r`, which records
    /// `user`. Either state re-applied is the undo: resolving a resolved
    /// comment reopens it.
    Resolve {
        /// The comment's id, from `rv comments`.
        id: String,
        /// Who is settling it.
        #[arg(long, default_value = "agent")]
        by: ByArg,
    },
    /// Mark a comment abandoned: dropped without being addressed.
    ///
    /// A separate state from resolved on purpose — *fixed* and *dropped unfixed*
    /// are different conclusions, and a count that adds them together misreports
    /// what the review decided.
    Abandon {
        /// The comment's id.
        id: String,
        /// Who is settling it.
        #[arg(long, default_value = "agent")]
        by: ByArg,
    },
    /// Print the range's diffs in rv's own side-aware coordinates.
    ///
    /// The numbers printed here are the numbers `rv comment --line` accepts:
    /// `right` is the head file's, `left` the base file's.
    Diff {
        /// One file, as `rv status` lists it [default: every file].
        file: Option<String>,
        /// Emit JSON instead of rows. The JSON is the contract.
        #[arg(long)]
        json: bool,
    },
    /// Print the review as markdown — a view, which nothing reads back.
    Render {
        /// Write to this file instead of stdout.
        #[arg(long, value_name = "PATH")]
        out: Option<PathBuf>,
    },
    /// Open the keybindings file (`~/.config/rv/keybindings.toml`) in $EDITOR.
    ///
    /// A missing file is seeded with the fully-commented defaults first, and
    /// the result is validated the moment the editor exits.
    Config,
    /// Print the effective keymap — the defaults plus the config's patch — as
    /// the same TOML the config file speaks.
    Keymap,
    /// Report the range, its changes, its files and its comment counts.
    Status {
        /// Emit JSON instead of text.
        #[arg(long)]
        json: bool,
        /// Exit 1 while any comment is open — the worker's poll and a CI
        /// gate in one flag. Prints nothing unless `--json` asks it to.
        #[arg(long)]
        check: bool,
    },
}

/// `--state` as clap sees it.
#[derive(Clone, Copy, Debug, clap::ValueEnum)]
enum StateArg {
    Open,
    AwaitingVerification,
    Resolved,
    Abandoned,
    Outdated,
}

impl From<StateArg> for CommentState {
    fn from(state: StateArg) -> Self {
        match state {
            StateArg::Open => CommentState::Open,
            StateArg::AwaitingVerification => CommentState::AwaitingVerification,
            StateArg::Resolved => CommentState::Resolved,
            StateArg::Abandoned => CommentState::Abandoned,
            StateArg::Outdated => CommentState::Outdated,
        }
    }
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
            App::run_with_config(
                session::build(&repo_root, cli.from.as_deref(), head.as_deref())?,
                if cli.no_difft {
                    DiffEngine::Fallback
                } else {
                    DiffEngine::Auto
                },
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
        Some(Command::Keymap) => {
            commands::print_keymap()?;
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
use commands::ByArg;
use commands::SideArg;
use commands::comments;
use commands::diff;
use commands::render;
use commands::reply;
use commands::settle;
use commands::status;
