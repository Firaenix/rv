//! Where one hunk ends and the next begins.
//!
//! # Why the boundaries are derived rather than carried
//!
//! Neither engine hands rv a hunk. `difftastic::parse` flattens difftastic's
//! `chunks[]` into one flat `Vec<DiffLine>` — and has to, because the same
//! entry can appear in two chunks and the chunks arrive out of reading order —
//! while `similar` produces a flat list to begin with. By the time the TUI sees
//! a [`FileDiff`] there is no chunk list left to consult, and there is no way
//! to get one back.
//!
//! It does not need to be recovered. A hunk is *a contiguous run of changed
//! lines*, and that survives flattening: every [`DiffLine`] still says whether
//! it changed and which line of which side it is. Deriving the runs gives the
//! same answer for both engines and for a diff read back out of the store,
//! which chasing difftastic's original chunking never could.
//!
//! # Why contiguity is line numbers rather than adjacency in the list
//!
//! The two engines disagree about context. `similar` emits the unchanged lines
//! between two edits as [`LineKind::Context`], so a break is visible in the
//! list itself. Difftastic emits **only** the lines that changed: three edits
//! thirty lines apart arrive as six consecutive `DiffLine`s with nothing
//! between them, and a rule that read only the kinds would call that one hunk
//! and leave `J` with nowhere to go in exactly the file it is for.
//!
//! So a run continues while the lines stay next to each other *in the file*: a
//! jump on either side's numbering ends it, as does a context line. Where
//! neither side can be compared — `similar`'s removal followed by its
//! replacement, one carrying only a left number and the other only a right —
//! the run continues, because nothing there evidences a gap.
//!
//! [`FileDiff`]: rv_core::diff::FileDiff

use rv_core::diff::DiffLine;
use rv_core::diff::LineKind;

/// The index of the first line of every hunk, in order.
///
/// An iterator rather than a `Vec`: this is walked once per key press to find
/// one neighbour, and the whole list is never wanted.
pub(super) fn hunk_starts(lines: &[DiffLine]) -> impl Iterator<Item = usize> + '_ {
    lines
        .iter()
        .enumerate()
        .filter(|(index, line)| {
            changed(line)
                && index
                    .checked_sub(1)
                    .and_then(|before| lines.get(before))
                    .is_none_or(|before| !changed(before) || !contiguous(before, line))
        })
        .map(|(index, _)| index)
}

/// Whether `line` is part of a hunk rather than of the context around one.
fn changed(line: &DiffLine) -> bool {
    match line.kind {
        LineKind::Added | LineKind::Removed => true,
        LineKind::Context => false,
    }
}

/// Whether two changed lines sit next to each other in the file, and so belong
/// to the same hunk.
///
/// A side both lines number is evidence; a side only one of them numbers is
/// not, and says nothing either way.
fn contiguous(before: &DiffLine, line: &DiffLine) -> bool {
    !apart(before.left, line.left) && !apart(before.right, line.right)
}

/// Whether a side both lines number shows a gap between them.
fn apart(before: Option<u32>, line: Option<u32>) -> bool {
    matches!((before, line), (Some(before), Some(line)) if line.abs_diff(before) > 1)
}
