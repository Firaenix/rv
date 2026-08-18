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

use rv_core::anchor;
use rv_core::model::Confidence;
use rv_core::model::Side;
use rv_core::store::Comment;
use rv_core::store::CommentState;

use crate::session::Review;

/// Marks every comment whose anchor no longer resolves as [`CommentState::Outdated`].
///
/// One blob read per distinct `(commit, path)`, which is the same order of work
/// the sidebar's counts already do before the first frame.
pub fn mark_outdated(review: &Review, comments: &mut [Comment]) {
    for comment in comments {
        if !matches!(
            comment.state,
            CommentState::Open | CommentState::AwaitingVerification
        ) {
            continue;
        }
        // The side the comment is anchored to decides which revision counts as
        // "there now": a comment on a removed line describes the base, and the
        // base of a review does not move under it.
        let commit = match comment.anchor.side {
            Side::Left => &review.session.base_commit,
            Side::Right => &review.session.head_commit,
        };
        let Some(text) = read(review, commit, &comment.anchor.file) else {
            // No blob at all — the file is gone from that side. That is the most
            // outdated a comment can be.
            comment.state = CommentState::Outdated;
            continue;
        };
        if anchor::resolve(&comment.anchor, &text).1 == Confidence::Outdated {
            comment.state = CommentState::Outdated;
        }
    }
}

/// The file's text at `commit`, or `None` where it is absent or not UTF-8.
fn read(review: &Review, commit: &str, path: &str) -> Option<String> {
    let bytes = review.repo.read_blob(commit, path).ok().flatten()?;
    String::from_utf8(bytes).ok()
}
