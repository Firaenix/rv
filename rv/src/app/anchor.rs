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
    pub(super) commit: String,
}

/// A place in the **code**: which file, which side of the diff, and which line
/// number on that side.
pub(super) type SourcePosition = (String, Side, u32);

impl App {
    /// The place in the code the cursor is standing on, if it stands on one at
    /// all.
    pub(super) fn cursor_position(&self) -> Option<SourcePosition> {
        let line = self.selected_line()?;
        let target = self.anchor_target(&line)?;
        Some((target.path.to_owned(), target.side, target.number))
    }

    /// Records where in the code the cursor stands, so a background result can
    /// find that place again after it replaces the diff the place was read from.
    ///
    /// Called from [`App::set_cursor_row`] — the one place a reviewer moves the
    /// cursor — and from nowhere in the resettle paths. A fallback line says
    /// where the cursor *landed*, not what the reviewer was reading; stamping
    /// the anchor from it turns drift into truth, which is how a refinement and
    /// a merge that landed one after the other ended a jump two files deep in
    /// the wrong function, at a row that depended on which answer arrived first.
    pub(super) fn remember_cursor_anchor(&mut self) {
        self.cursor_anchor = self
            .cursor_position()
            .map(|position| (self.shown_target(), position));
    }

    /// Puts the cursor back on the place in the code it was standing on when a
    /// background result replaced the diff that place was read from: a
    /// structural diff answering after the fast one had already drawn, or a
    /// whole-file merge landing after the changed-only view.
    ///
    /// Deliberately not [`App::set_cursor_row`]: nothing here moved the
    /// reviewer's selection, so the comment stack they were working through
    /// stays open and the view they parked with the wheel stays parked.
    ///
    /// An anchor recorded against another pair of commits is not this view's to
    /// resolve, so it is left where it is.
    pub(super) fn reanchor_cursor(&mut self) {
        let Some((target, position)) = self.cursor_anchor.clone() else {
            return;
        };
        if target != self.shown_target() {
            return;
        }
        let (path, side, number) = &position;
        let parked = self.diff_scroll;
        match self.line_index_at_anchor(path, *side, *number) {
            Some(line) => {
                self.resettle_cursor(line);
                self.diff_scroll = parked;
            }
            None => self.clamp_cursor_to_plan(),
        }
    }

    /// Which index into [`App::displayed_lines`] is anchored at `path`:`number`
    /// on `side`.
    ///
    /// The exact line where a diff still carries it, and otherwise the nearest
    /// line it has on the same side and path: a structural changed-only diff
    /// holds none of the context a whole-file diff or a merge shows, so most of
    /// what the fast answer carried is simply absent from the answer that
    /// replaces it, and the closest surviving line is the same code seen from
    /// further away. Holding on to the old row index instead would name
    /// unrelated code.
    pub(super) fn line_index_at_anchor(
        &self,
        path: &str,
        side: Side,
        number: u32,
    ) -> Option<usize> {
        let lines = self.displayed_lines();
        let mut nearest: Option<(u32, usize)> = None;
        for (index, line) in lines.iter().enumerate() {
            let Some(target) = self.anchor_target(line) else {
                continue;
            };
            if target.path != path || target.side != side {
                continue;
            }
            let distance = target.number.abs_diff(number);
            if distance == 0 {
                return Some(index);
            }
            if nearest.is_none_or(|(best, _)| distance < best) {
                nearest = Some((distance, index));
            }
        }
        nearest.map(|(_, index)| index)
    }

    /// Where a comment on `line` of the selected file belongs.
    ///
    /// `None` when the line carries no number on the side it belongs to, which
    /// is the same condition the save path refuses under — so a line that
    /// cannot be commented on shows no comments either, rather than borrowing
    /// some other line's.
    pub(super) fn anchor_target(&self, line: &DiffLine) -> Option<AnchorTarget<'_>> {
        let file = self.selected_file()?;
        // The pair the text on screen was read from, which in the commits view is
        // the selected change's rather than the review's.
        let (base, head) = self.shown_endpoints();
        let side = anchored_side(line.kind);
        let (path, number, commit) = match side {
            Side::Left => (
                file.source_path.as_deref().unwrap_or(&file.path),
                line.left,
                base,
            ),
            Side::Right => (file.path.as_str(), line.right, head),
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
pub fn comment_id(change_id: &str, path: &str, side: Side, line: u32, body: &str) -> String {
    let side = match side {
        Side::Left => "left",
        Side::Right => "right",
    };
    let seed = format!("{change_id}:{path}:{side}:{line}:{body}");
    let digest = blake3::hash(seed.as_bytes()).to_hex();
    digest[..ID_CHARS].to_owned()
}
