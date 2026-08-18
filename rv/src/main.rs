//! The `rv` command line.
//!
//! Every invocation assembles the same thing first — a [`Review`] over one
//! revision range, via [`session::build`] — and only then branches on what the
//! user asked for. With no subcommand that is the interactive reviewer. Two
//! subcommands exist beside it, for the same review without a terminal:
//! `render` writes `.review/REVIEW-FEEDBACK.md`, and `status` reports the
//! range, its stack, its files and its comment counts as text or as JSON.
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
    /// For reviewer agents: the anchor, the id and the export are all handled,
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
        /// The comment itself.
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
        /// The comment's id, from the export's `<!-- rv:anchor id=… -->` marker.
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
    /// Write `.review/REVIEW-FEEDBACK.md` for the current review.
    Render,
    /// Report the range, its changes, its files and its comment counts.
    Status {
        /// Emit JSON instead of text.
        #[arg(long)]
        json: bool,
    },
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            // `{:#}` is anyhow's single-line chain: every `context` layer and
            // the underlying `rv-core` message, separated by ": ".
            eprintln!("rv: {error:#}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<()> {
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
    match cli.command {
        None => App::run(
            session::build(&repo_root, cli.from.as_deref(), head.as_deref())?,
            if cli.no_difft {
                DiffEngine::Fallback
            } else {
                DiffEngine::Auto
            },
        ),
        Some(Command::Comment {
            file,
            line,
            side,
            message,
        }) => {
            let review = session::read(&repo_root, cli.from.as_deref(), head.as_deref())?;
            let comment = session::add_comment(&review, &file, side.into(), line, &message)?;
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
            Ok(())
        }
        Some(Command::Resolve { id, by }) => settle(
            &session::read(&repo_root, cli.from.as_deref(), head.as_deref())?,
            &id,
            CommentState::Resolved,
            by.into(),
        ),
        Some(Command::Abandon { id, by }) => settle(
            &session::read(&repo_root, cli.from.as_deref(), head.as_deref())?,
            &id,
            CommentState::Abandoned,
            by.into(),
        ),
        Some(Command::Render) => render(&session::read(
            &repo_root,
            cli.from.as_deref(),
            head.as_deref(),
        )?),
        Some(Command::Status { json }) => status(
            &session::read(&repo_root, cli.from.as_deref(), head.as_deref())?,
            json,
        ),
    }
}


mod commands;
use commands::ByArg;
use commands::SideArg;
use commands::render;
use commands::settle;
use commands::status;
