//! The read half of the agent loop: `rv status` and `rv comments`.
//!
//! Both are loads over the same review the TUI shows — the same `in_range`
//! filter, the same derived `outdated` — so one review has one answer whoever
//! asks. The JSON is the contract; the text is for humans.

use anyhow::Context as _;
use anyhow::Result;
use rv::session;
use rv::session::Review;
use rv::stale;
use rv_core::markdown;
use rv_core::model::ChangeKind;
use rv_core::model::Confidence;
use rv_core::store::Comment;
use rv_core::store::CommentState;
use rv_core::store::SettledBy;
use serde_json::json;

use crate::NO_DESCRIPTION;

/// Reports the review as text, or as the JSON the `--json` flag asks for.
///
/// Returns whether any comment is still open, for `--check`: the worker's poll
/// and a CI gate are both exit-code questions, and neither should need `jq` to
/// ask one.
pub fn status(review: &Review, json: bool) -> Result<bool> {
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
        return Ok(counts.open > 0);
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
    Ok(counts.open > 0)
}

/// `rv status --check` with neither `--json` nor text: the same load, nothing
/// printed, and the answer carried entirely in the exit code.
pub fn check(review: &Review) -> Result<bool> {
    Ok(Counts::of(&read_comments(review)?).open > 0)
}

/// `rv comments`: the read channel — everything the store and a load can say
/// about each comment, so an agent never has to parse the markdown for it.
pub fn comments(review: &Review, json: bool, state: Option<CommentState>) -> Result<()> {
    let comments = read_comments(review)?;
    let comments: Vec<&Comment> = comments
        .iter()
        .filter(|comment| state.is_none_or(|state| comment.state == state))
        .collect();

    if json {
        let listed: Vec<_> = comments
            .iter()
            .map(|comment| comment_json(review, comment))
            .collect();
        let serialized =
            serde_json::to_string_pretty(&listed).context("could not serialize the comments")?;
        println!("{serialized}");
        return Ok(());
    }

    for comment in comments {
        let first = comment.body.lines().next().unwrap_or_default();
        println!(
            "{} {:<22} {}:{} {first}",
            comment.id,
            state_name(comment.state),
            comment.anchor.file,
            comment.anchor.line,
        );
        if let Some(reply) = &comment.reply {
            let answer = reply.lines().next().unwrap_or_default();
            println!("{:>8}   reply: {answer}", "");
        }
    }
    Ok(())
}

fn comment_json(review: &Review, comment: &Comment) -> serde_json::Value {
    // Where the anchor lands in the code as it now stands: the resolved line
    // (which may differ from the stored one when the content moved) and how
    // confidently the cascade placed it — spec §9's "confidence is surfaced".
    let (resolved_line, confidence) = stale::resolution(review, comment);
    json!({
        "id": comment.id,
        "change_id": comment.change_id,
        "commit_id": comment.commit_id,
        "state": state_name(comment.state),
        "resolved_line": resolved_line,
        "confidence": confidence_name(confidence),
        "settled_by": comment.settled_by.map(|by| match by {
            SettledBy::Agent => "agent",
            SettledBy::User => "user",
        }),
        // Derived on this very load, never stored — the same rule `rv status`
        // and the TUI live by.
        "outdated": comment.state == CommentState::Outdated,
        "body": comment.body,
        "reply": comment.reply,
        "anchor": {
            "file": comment.anchor.file,
            "side": match comment.anchor.side {
                rv_core::model::Side::Left => "left",
                rv_core::model::Side::Right => "right",
            },
            "line": comment.anchor.line,
            "context_start": comment.anchor.context_start,
            "context": comment.anchor.context,
        },
    })
}

/// A [`Confidence`]'s name, in the same lowercase vocabulary as the states.
fn confidence_name(confidence: Confidence) -> &'static str {
    match confidence {
        Confidence::Exact => "exact",
        Confidence::Moved => "moved",
        Confidence::Weak => "weak",
        Confidence::Outdated => "outdated",
    }
}

/// A comment state's name, spelled the way the store serializes it.
fn state_name(state: CommentState) -> &'static str {
    match state {
        CommentState::Open => "open",
        CommentState::AwaitingVerification => "awaiting-verification",
        CommentState::Resolved => "resolved",
        CommentState::Abandoned => "abandoned",
        CommentState::Outdated => "outdated",
    }
}

fn read_comments(review: &Review) -> Result<Vec<Comment>> {
    // The same in-range filter the TUI applies: `.review/` outlives any one
    // revset, and a comment this range cannot display is a comment a script
    // acts wrongly on.
    let mut comments = session::in_range(
        review,
        review
            .store
            .comments()
            .context("could not read the review's comments")?,
    );
    // A load derives `outdated` — see [`stale::mark_outdated`]. Reporting the
    // stored state here would have the command and the TUI disagree about the
    // same review, and the command is the half a script reads.
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
/// of the vocabulary `rv` writes for other programs to read.
fn kind_name(kind: ChangeKind) -> &'static str {
    match kind {
        ChangeKind::Added => "added",
        ChangeKind::Modified => "modified",
        ChangeKind::Removed => "removed",
        ChangeKind::Renamed => "renamed",
    }
}
