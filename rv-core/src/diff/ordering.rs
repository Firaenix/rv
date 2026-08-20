//! Putting difftastic's chunk entries back into the order a reviewer reads
//! the file in.

use super::model::DiffLine;
use super::model::LineKind;

/// One chunk entry, resolved against the two files: the base-side line it
/// drops, the head-side line it introduces, or the two lines difftastic
/// aligned with each other.
pub enum Entry {
    Removed {
        left: u32,
        text: String,
    },
    Added {
        right: u32,
        text: String,
    },
    Aligned {
        left: u32,
        left_text: String,
        right: u32,
        right_text: String,
    },
}

impl Entry {
    /// The 1-based line numbers this entry carries, which identify it: two
    /// entries with the same pair say the same thing about the same lines.
    pub fn numbers(&self) -> (Option<u32>, Option<u32>) {
        match self {
            Entry::Removed { left, .. } => (Some(*left), None),
            Entry::Added { right, .. } => (None, Some(*right)),
            Entry::Aligned { left, right, .. } => (Some(*left), Some(*right)),
        }
    }

    /// The diff lines this entry becomes. An aligned pair becomes exactly two
    /// adjacent lines carrying both numbers.
    pub fn lines(self) -> Vec<DiffLine> {
        match self {
            Entry::Removed { left, text } => vec![DiffLine {
                kind: LineKind::Removed,
                left: Some(left),
                right: None,
                text,
            }],
            Entry::Added { right, text } => vec![DiffLine {
                kind: LineKind::Added,
                left: None,
                right: Some(right),
                text,
            }],
            Entry::Aligned {
                left,
                left_text,
                right,
                right_text,
            } => vec![
                DiffLine {
                    kind: LineKind::Removed,
                    left: Some(left),
                    right: Some(right),
                    text: left_text,
                },
                DiffLine {
                    kind: LineKind::Added,
                    left: Some(left),
                    right: Some(right),
                    text: right_text,
                },
            ],
        }
    }
}

/// Sorts `entries` into the order a reviewer reads the file in: base-side
/// numbers ascending and head-side numbers ascending at the same time.
///
/// # The rule
///
/// The two sides number their lines differently — a block deleted early in the
/// base file pushes the two sides' numbering apart — so neither number is a
/// position the other side can be compared against, and sorting by either one
/// alone would put one of the two sequences out of order.
///
/// The aligned pairs are the fixed points: they are the only entries that name
/// a line on *both* sides, so each one cuts both files at the same place. They
/// are laid out in order first, and every one-sided entry falls into the gap
/// between the two pairs it belongs between — a removal by its base-side
/// number, an insertion by its head-side one.
///
/// What decides the order *within* a gap is how many lines the two files have
/// in common between the pair that opens the gap and the entry itself. Those
/// shared lines are the one coordinate both sides agree on: they are the lines
/// difftastic did not report at all, and they appear in both files in the same
/// order. Counting them from the gap's own anchor rather than from the top of
/// the file is what makes a removal and an insertion comparable:
///
/// - a removal at base line `L` in a gap opened by base line `A`: the `L-A-1`
///   base lines between them are each either dropped by another removal in the
///   gap or shared, so its position is `(L - A - 1) - (removals between A and
///   L)`;
/// - an insertion at head line `R` in a gap opened by head line `B`:
///   symmetrically, `(R - B - 1) - (insertions between B and R)`;
/// - the pair that closes the gap comes after everything in it.
///
/// Ordering a gap's contents by their raw numbers instead — the previous rule —
/// interleaved the two sides only where an aligned pair happened to separate
/// them, so a file whose hunks are all pure insertions and pure deletions had
/// its whole contents in one gap and rendered as every removal followed by
/// every insertion (see `one_sided_difftastic_hunks_interleave_with_each_other`
/// in `tests/diff.rs`).
///
/// Re-anchoring at every pair also keeps the rule honest when difftastic
/// silently drops a line — which it does. For a six-line base against a
/// six-line head, difft 0.70 has been observed to report one insertion and two
/// aligned pairs while its own `aligned_lines` shows a third base line with no
/// counterpart, named in no chunk at all. Counting shared lines from the top of
/// the file would then have the two sides disagree by exactly that line, and
/// the second pair would sort after an insertion that belongs below it.
/// Counting from the nearest pair cannot drift that way: a pair pins both
/// files at once, so every gap starts over from a position both sides agree
/// on.
///
/// # At a hunk boundary
///
/// A removal and an insertion tie exactly when no shared line separates them —
/// that is, when they are in the same hunk — and then the file itself does not
/// say which comes first. The tie is broken the way a unified diff prints a
/// hunk: removals (in base order), then insertions (in head order), then the
/// shared line that ends the hunk, which is the aligned pair if there is one.
/// A shared line between them puts them in different hunks, and then their
/// positions differ and the tie-break never applies. (difft 0.70 does not
/// appear to produce the tie at all: it reports a removal and an insertion at
/// the same position as one aligned pair instead. The rule still has to be
/// total and deterministic.)
pub fn order(entries: &mut [Entry]) {
    let mut pair_lefts: Vec<u32> = Vec::new();
    let mut pair_rights: Vec<u32> = Vec::new();
    let mut removed_lefts: Vec<u32> = Vec::new();
    let mut added_rights: Vec<u32> = Vec::new();
    for entry in entries.iter() {
        match entry {
            Entry::Removed { left, .. } => removed_lefts.push(*left),
            Entry::Added { right, .. } => added_rights.push(*right),
            Entry::Aligned { left, right, .. } => {
                pair_lefts.push(*left);
                pair_rights.push(*right);
            }
        }
    }
    for numbers in [
        &mut pair_lefts,
        &mut pair_rights,
        &mut removed_lefts,
        &mut added_rights,
    ] {
        numbers.sort_unstable();
    }

    // (which gap, the pair closing it goes last, shared lines before it in the
    // gap, what goes first at a tie, its own number) — a stable sort, so
    // entries that tie completely keep difftastic's own order.
    entries.sort_by_key(|entry| match entry {
        Entry::Removed { left, .. } => {
            let (index, anchor) = gap(*left, &pair_lefts);
            (
                index,
                0u8,
                shared_between(anchor, *left, &removed_lefts),
                0u8,
                *left,
            )
        }
        Entry::Added { right, .. } => {
            let (index, anchor) = gap(*right, &pair_rights);
            (
                index,
                0u8,
                shared_between(anchor, *right, &added_rights),
                1u8,
                *right,
            )
        }
        Entry::Aligned { left, right, .. } => (gap(*left, &pair_lefts).0, 1u8, 0, 0u8, *right),
    });
}

/// Which gap `number` falls in — the count of pairs before it on this side —
/// and the pair line that opens that gap (0 for the first gap, which starts at
/// the top of the file).
fn gap(number: u32, pairs: &[u32]) -> (usize, u32) {
    let index = pairs.partition_point(|at| *at < number);
    (index, index.checked_sub(1).map_or(0, |at| pairs[at]))
}

/// How many lines the two files share strictly between `anchor` and `number`
/// on the side whose one-sided entries are `dropped`: every line in between is
/// either one of those or shared with the other side.
fn shared_between(anchor: u32, number: u32, dropped: &[u32]) -> usize {
    let between = usize::try_from(number.saturating_sub(anchor)).unwrap_or(usize::MAX);
    let gone =
        dropped.partition_point(|at| *at < number) - dropped.partition_point(|at| *at <= anchor);
    between.saturating_sub(1).saturating_sub(gone)
}
