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
use rv::session;
use rv::session::Review;
use rv_core::model::ChangeKind;
use rv_core::store::Comment;
use rv_core::store::CommentState;
use serde_json::json;

/// What `jj` shows for a change nobody has described yet; reused here so the
/// text output of `status` does not print a blank column instead.
const NO_DESCRIPTION: &str = "(no description set)";

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

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
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

    let review = session::build(&repo_root, cli.from.as_deref(), head.as_deref())?;

    // A bare `rv` is the reviewer; the subcommands are the same review with no
    // terminal in the way.
    match cli.command {
        None => App::run(review),
        Some(Command::Render) => render(&review),
        Some(Command::Status { json }) => status(&review, json),
    }
}

/// Writes `.review/REVIEW-FEEDBACK.md` from the session and its stored
/// comments, folding any reply already in the document back into the store
/// first — see [`session::write_markdown`], which the TUI shares.
fn render(review: &Review) -> Result<()> {
    session::write_markdown(review)?;
    println!("wrote {}", review.store.markdown_path().display());
    Ok(())
}

/// Reports the review as text, or as the JSON the `--json` flag asks for.
fn status(review: &Review, json: bool) -> Result<()> {
    let comments = read_comments(review)?;
    let counts = Counts::of(&comments);
    let session = &review.session;

    if json {
        let report = json!({
            "revset": session.revset,
            "base": session.base_commit,
            "head": session.head_commit,
            "changes": session
                .changes
                .iter()
                .map(|change| json!({
                    "change_id": change.change_id,
                    "commit_id": change.commit_id,
                    "description": change.description,
                }))
                .collect::<Vec<_>>(),
            "files": review
                .files
                .iter()
                .map(|file| json!({
                    "path": file.path,
                    "kind": kind_name(file.kind),
                    "binary": file.binary,
                }))
                .collect::<Vec<_>>(),
            "comments": {
                "open": counts.open,
                "awaiting_verification": counts.awaiting_verification,
                "resolved": counts.resolved,
                "abandoned": counts.abandoned,
                "outdated": counts.outdated,
            },
        });
        let serialized = serde_json::to_string_pretty(&report)
            .context("could not serialize the status report")?;
        println!("{serialized}");
        return Ok(());
    }

    println!("revset  {}", session.revset);
    println!("base    {}", session.base_commit);
    println!("head    {}", session.head_commit);

    println!("\nchanges ({})", session.changes.len());
    for change in &session.changes {
        let description = match change.description.lines().next() {
            Some(first) if !first.trim().is_empty() => first,
            _ => NO_DESCRIPTION,
        };
        println!("  {} {} {description}", change.change_id, change.commit_id);
    }

    println!("\nfiles ({})", review.files.len());
    for file in &review.files {
        // The same three fields `--json` reports, so the two forms cannot
        // disagree about what the review covers.
        let binary = if file.binary { " (binary)" } else { "" };
        println!("  {:<8}  {}{binary}", kind_name(file.kind), file.path);
    }

    // Resolved and abandoned are counted apart, never summed: one is work that
    // happened and the other is work that was decided against.
    println!(
        "\ncomments  {} open, {} awaiting verification, {} resolved, {} abandoned, {} outdated",
        counts.open,
        counts.awaiting_verification,
        counts.resolved,
        counts.abandoned,
        counts.outdated
    );
    Ok(())
}

fn read_comments(review: &Review) -> Result<Vec<Comment>> {
    review
        .store
        .comments()
        .context("could not read the review's comments")
}

/// How many comments sit in each state.
#[derive(Debug, Default)]
struct Counts {
    open: usize,
    awaiting_verification: usize,
    resolved: usize,
    abandoned: usize,
    outdated: usize,
}

impl Counts {
    fn of(comments: &[Comment]) -> Self {
        let mut counts = Self::default();
        for comment in comments {
            let bucket = match comment.state {
                CommentState::Open => &mut counts.open,
                CommentState::AwaitingVerification => &mut counts.awaiting_verification,
                CommentState::Resolved => &mut counts.resolved,
                CommentState::Abandoned => &mut counts.abandoned,
                CommentState::Outdated => &mut counts.outdated,
            };
            *bucket += 1;
        }
        counts
    }
}

/// A [`ChangeKind`] as it appears in `rv`'s own output.
///
/// Lowercase rather than the variant's `Serialize` spelling, matching the rest
/// of the vocabulary `rv` writes for other programs to read (`Confidence`,
/// `CommentState`).
fn kind_name(kind: ChangeKind) -> &'static str {
    match kind {
        ChangeKind::Added => "added",
        ChangeKind::Modified => "modified",
        ChangeKind::Removed => "removed",
        ChangeKind::Renamed => "renamed",
    }
}
