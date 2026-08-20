//! Which comments no longer describe the code that is there.
//!
//! `outdated` is **derived on every load and never stored** (storage spec §3).
//! A stored flag would need invalidating by something, and nothing is watching:
//! if a rebase puts the line back, or the reviewer undoes the edit, a comment
//! that had gone stale is simply about live code again. Deriving it means that
//! happens by itself.
//!
//! `rv_core::anchor::resolve` has been written and tested since milestone 1 and
//! was called by nothing, so every comment read as `open` however far the code
//! had moved under it — including comments about files that no longer exist.
//!
//! # Only an unsettled comment goes stale
//!
//! A resolved comment whose code has since changed is still resolved: it was
//! addressed, and that is a fact about what happened rather than about the
//! current text. Same for an abandoned one. So the derivation only ever moves a
//! comment *out* of `Open` or `AwaitingVerification`, which is what keeps it from
//! overwriting the two states a person deliberately set.

use std::collections::HashMap;

use rv_core::anchor;
use rv_core::diff;
use rv_core::diff::FileDiff;
use rv_core::model::Confidence;
use rv_core::model::Side;
use rv_core::store::Comment;
use rv_core::store::CommentState;

use crate::session::Review;

/// What the code under one comment's anchor has done since the comment was
/// written.
///
/// Derived once per load and handed to the renderer, never asked for per frame:
/// every field below costs a blob read, and a comment box is drawn on the paint
/// path.
#[derive(Debug)]
pub struct Drift {
    pub confidence: Confidence,
    /// The stored context against whatever stands in its place now, for a
    /// comment whose state came out [`CommentState::Outdated`] — the before/after
    /// block of storage spec §4. `None` for every other comment, which has
    /// nothing to show: the code it describes is still there.
    pub before_after: Option<FileDiff>,
    /// Whether there was any text at the anchor's place to compare with. When
    /// there was not, `before_after` is the stored lines against nothing and the
    /// block says the anchor could not be located.
    pub located: bool,
}

/// Marks every comment whose anchor no longer resolves as [`CommentState::Outdated`].
///
/// One blob read per distinct `(commit, path)`, which is the same order of work
/// the sidebar's counts already do before the first frame.
pub fn mark_outdated(review: &Review, comments: &mut [Comment]) {
    survey(review, comments);
}

/// The same derivation, keeping what it learned: `outdated` is marked and every
/// comment's [`Drift`] comes back, keyed by id.
///
/// One pass rather than a mark followed by a second look, because after marking
/// [`resolution`] short-circuits on the very comments whose drift is worth
/// showing — a settled state is not re-resolved, and `Outdated` is a settled
/// state by then.
pub fn survey(review: &Review, comments: &mut [Comment]) -> HashMap<String, Drift> {
    let mut drifts = HashMap::with_capacity(comments.len());
    for comment in comments {
        let (line, confidence) = resolution(review, comment);
        if confidence == Confidence::Outdated {
            comment.state = CommentState::Outdated;
        }
        // Keyed off the state rather than off `confidence` so that a `.review/`
        // that arrived already saying `outdated` — an agent's, or milestone 1's
        // — opens the same block as a derivation does.
        let now = (comment.state == CommentState::Outdated).then(|| current(review, comment, line));
        drifts.insert(
            comment.id.clone(),
            Drift {
                confidence,
                before_after: now
                    .as_ref()
                    .map(|now| before_after(comment, now.as_deref())),
                located: now.is_some_and(|now| now.is_some()),
            },
        );
    }
    drifts
}

/// The stored excerpt diffed against `now`, or against nothing where the anchor
/// could not be placed at all.
///
/// [`diff::compute_with`] with difftastic **off**, per storage spec §4: the
/// sibling that spawns it writes two temp files and runs a child process, and
/// this is drawn inside a frame. A slice of stored context is also not a
/// parseable file, so the language difftastic would infer from the path would be
/// right about the file and wrong about the fragment.
fn before_after(comment: &Comment, now: Option<&str>) -> FileDiff {
    let stored = comment.anchor.context.join("\n");
    diff::compute_with(
        Some(stored.as_bytes()),
        now.map(str::as_bytes),
        &comment.anchor.file,
        false,
    )
}

/// The lines standing where the anchor's excerpt used to be: as many as were
/// stored, from the file as it is now, starting where the excerpt started —
/// shifted to `line` where the cascade still managed to place the anchor.
///
/// `None` where there is no such text: the file is gone from that side, or has
/// grown too short to reach the excerpt at all.
fn current(review: &Review, comment: &Comment, line: Option<u32>) -> Option<String> {
    let anchor = &comment.anchor;
    let text = read(
        review,
        commit_of(review, anchor.side),
        &anchor.file,
        anchor.side,
    )?;
    // Where in the excerpt the anchored line sits. `snapshot_of` clamps at the
    // top of a file, so it is not always the middle one.
    let offset = anchor.line.saturating_sub(anchor.context_start);
    let start = line.unwrap_or(anchor.line).saturating_sub(offset).max(1);
    let lines: Vec<&str> = text
        .lines()
        .skip(start as usize - 1)
        .take(anchor.context.len())
        .collect();
    (!lines.is_empty()).then(|| lines.join("\n"))
}

/// Where `comment`'s anchor lands in the code as it now stands, and how
/// confidently — the same cascade [`mark_outdated`] derives `outdated` from,
/// exposed so `rv comments` can say it out loud (spec §9: confidence is
/// surfaced, never silently discarded).
///
/// A settled comment reports [`Confidence::Exact`] at its stored line without
/// a read: it is a fact about what happened, not about the current text, and
/// re-resolving it would be the "only an unsettled comment goes stale" rule
/// broken in a second place.
pub fn resolution(review: &Review, comment: &Comment) -> (Option<u32>, Confidence) {
    if !matches!(
        comment.state,
        CommentState::Open | CommentState::AwaitingVerification
    ) {
        return (Some(comment.anchor.line), Confidence::Exact);
    }
    let commit = commit_of(review, comment.anchor.side);
    let Some(text) = read(review, commit, &comment.anchor.file, comment.anchor.side) else {
        // No blob at all — the file is gone from that side. That is the most
        // outdated a comment can be.
        return (None, Confidence::Outdated);
    };
    anchor::resolve(&comment.anchor, &text)
}

/// Which revision counts as "there now" for a comment on `side`: a comment on a
/// removed line describes the base, and the base of a review does not move under
/// it.
fn commit_of(review: &Review, side: Side) -> &str {
    match side {
        Side::Left => &review.session.base_commit,
        Side::Right => &review.session.head_commit,
    }
}

/// The file's text at `commit` — followed through the review's rename records
/// where the path itself has moved — or `None` where it is absent or not UTF-8.
///
/// The rename step is what keeps a moved file from mass-outdating every
/// comment on it (spec §9): a head-side comment filed under the old name reads
/// the file at its new one.
fn read(review: &Review, commit: &str, path: &str, side: Side) -> Option<String> {
    let direct = review.repo.read_blob(commit, path).ok().flatten();
    let bytes = match (direct, side) {
        (Some(bytes), _) => Some(bytes),
        (None, Side::Right) => {
            let renamed = review
                .files
                .iter()
                .find(|file| file.source_path.as_deref() == Some(path))?;
            review.repo.read_blob(commit, &renamed.path).ok().flatten()
        }
        (None, Side::Left) => None,
    }?;
    String::from_utf8(bytes).ok()
}
