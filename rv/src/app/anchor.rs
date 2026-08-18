//! Where a comment on a diff line belongs, and what it is filed under.

use rv_core::diff::DiffLine;
use rv_core::diff::LineKind;
use rv_core::model::Side;

use super::App;

/// How many hex characters of the digest make up a comment id.
///
/// Eight, not the four spec §10 writes.
/// [`rv_core::store::Store::append_comment`] upserts by id, so two *different*
/// comments sharing a prefix mean the second save silently replaces the first,
/// snapshot and all, under a "comment saved" status line. Four hex characters
/// is a 65,536-value space: by the birthday bound a ~2% chance of losing a
/// comment at 50 of them, ~7% at 100 — reachable on one real review.
const ID_CHARS: usize = 8;

/// Where a comment on one diff line belongs: which side it is anchored to, and
/// the path, line number and commit **on that side**.
///
/// Four values from one function because they have to agree: the pane labels a
/// line with `number`, the store anchors it at `path`:`number` on `side`, and
/// `commit` is the revision whose blob that text is read and hashed from. A
/// comment on a removed line whose `commit` names the head points at a revision
/// the quoted text cannot be read back from, which is `commit`'s only job.
pub(super) struct AnchorTarget<'a> {
    pub(super) side: Side,
    pub(super) path: &'a str,
    pub(super) number: u32,
    pub(super) commit: &'a str,
}

impl App {
    /// Where a comment on `line` of the selected file belongs.
    ///
    /// `None` when the line carries no number on the side it belongs to, which
    /// is the same condition the save path refuses under — so a line that
    /// cannot be commented on shows no comments either, rather than borrowing
    /// some other line's.
    pub(super) fn anchor_target(&self, line: &DiffLine) -> Option<AnchorTarget<'_>> {
        let file = self.selected_file()?;
        let session = &self.review.session;
        let side = anchored_side(line.kind);
        let (path, number, commit) = match side {
            Side::Left => (
                file.source_path.as_deref().unwrap_or(&file.path),
                line.left,
                session.base_commit.as_str(),
            ),
            Side::Right => (file.path.as_str(), line.right, session.head_commit.as_str()),
        };
        Some(AnchorTarget {
            side,
            path,
            number: number?,
            commit,
        })
    }
}

/// Which side of the diff a comment on a line of this kind belongs to: a
/// removed line only exists on the base side, and everything else — added and
/// context alike — is commented against the head.
///
/// Public because [`crate::ui`] labels each line with the number on the side
/// this returns. A pane that showed one number while the anchor stored another
/// would be lying about what the reviewer just commented on, which Milestone 1
/// shipped once.
pub fn anchored_side(kind: LineKind) -> Side {
    match kind {
        LineKind::Removed => Side::Left,
        LineKind::Added | LineKind::Context => Side::Right,
    }
}

/// A comment's id: the first [`ID_CHARS`] hex characters of the blake3 digest
/// of the change, location and body it covers.
///
/// Derived rather than random so that re-typing the same comment on the same
/// line of the same change upserts the entry it already made.
///
/// `side` is part of the seed because the *whole* location has to be, and a
/// location is a side as well as a path and a number: difftastic gives both
/// halves of a rewritten pair both numbers, so one sentence typed on each half
/// of a rewrite that did not move would otherwise seed two identical ids and
/// the second save would silently replace the first. Unlike a digest collision
/// that happens with probability 1. The path alone is not enough — the two
/// paths differ only for a rename.
///
/// `change_id` is the same string for every comment in a review (see the save
/// path), so within one review the location and the body carry the whole of the
/// seed's discriminating power. It stays in because ids outlive the review that
/// made them.
pub(super) fn comment_id(
    change_id: &str,
    path: &str,
    side: Side,
    line: u32,
    body: &str,
) -> String {
    let side = match side {
        Side::Left => "left",
        Side::Right => "right",
    };
    let seed = format!("{change_id}:{path}:{side}:{line}:{body}");
    let digest = blake3::hash(seed.as_bytes()).to_hex();
    digest[..ID_CHARS].to_owned()
}
