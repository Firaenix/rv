//! What each subcommand does, once the arguments are parsed.

use anyhow::Context as _;
use anyhow::Result;
use rv::session;
use rv::session::Review;
use rv::stale;
use rv_core::markdown;
use rv_core::store::Comment;
use rv_core::store::CommentState;
use rv_core::store::SettledBy;
use rv_core::model::ChangeKind;
use rv_core::model::Side;
use serde_json::json;

use crate::NO_DESCRIPTION;

/// Settles `id` into `state` — or back to open where it is already there, which
/// is what makes the command its own undo — and refreshes the export so a
/// polling worker sees the change.
pub fn settle(review: &Review, id: &str, state: CommentState, by: SettledBy) -> Result<()> {
    let comments = read_comments(review)?;
    let comment = comments
        .iter()
        .find(|comment| comment.id == id)
        .with_context(|| format!("no comment {id} in this review — ids are in the export's markers"))?;
    let (target, said) = if comment.state == state {
        (CommentState::Open, "reopened")
    } else {
        match state {
            CommentState::Resolved => (state, "resolved"),
            _ => (state, "abandoned"),
        }
    };
    review.store.settle_comment(id, target, by)?;
    session::write_markdown(review)?;
    println!(
        "{said} {id} at {}:{}",
        comment.anchor.file, comment.anchor.line
    );
    Ok(())
}

/// `--by` as clap sees it.
#[derive(Clone, Copy, Debug, clap::ValueEnum)]
pub enum ByArg {
    Agent,
    User,
}

impl From<ByArg> for SettledBy {
    fn from(by: ByArg) -> Self {
        match by {
            ByArg::Agent => SettledBy::Agent,
            ByArg::User => SettledBy::User,
        }
    }
}

/// `--side` as clap sees it.
#[derive(Clone, Copy, Debug, clap::ValueEnum)]
pub enum SideArg {
    Left,
    Right,
}

impl From<SideArg> for Side {
    fn from(side: SideArg) -> Self {
        match side {
            SideArg::Left => Side::Left,
            SideArg::Right => Side::Right,
        }
    }
}

/// Writes `.review/REVIEW-FEEDBACK.md` from the session and its stored
/// comments, folding any reply already in the document back into the store
/// first — see [`session::write_markdown`], which the TUI shares.
pub fn render(review: &Review) -> Result<()> {
    session::write_markdown(review)?;
    println!("wrote {}", review.store.markdown_path().display());
    Ok(())
}

/// Reports the review as text, or as the JSON the `--json` flag asks for.
pub fn status(review: &Review, json: bool) -> Result<()> {
    let comments = read_comments(review)?;
    let counts = Counts::of(&comments);
    let session = &review.session;

    if json {
        let report = json!({
            "revset": session.revset,
            "base": session.base_commit,
            "head": session.head_commit,
            "degraded_base": markdown::degraded_base(session).is_some(),
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
    // The revset records what was typed; this says what it resolved to, which is
    // the difference between a branch review and a whole-history dump.
    if markdown::degraded_base(session).is_some() {
        println!("\nnote    {}", markdown::DEGRADED);
    }

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
    let mut comments = review
        .store
        .comments()
        .context("could not read the review's comments")?;
    // `status` is a load, and `outdated` is derived on every load — see
    // [`stale::mark_outdated`]. Reporting the stored state here would have the
    // command and the TUI disagree about the same review, and the command is the
    // half a script reads.
    stale::mark_outdated(review, &mut comments);
    Ok(comments)
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
