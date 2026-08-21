//! Filling the gaps between difftastic's changed-only lines with the
//! untouched majority of the file, so a reviewer sees the whole thing.
//!
//! difftastic reports only what changed; this walks `changed` (already in
//! file order — see `ordering::order`) and the full old/new text side by
//! side, synthesizing a [`LineKind::Context`] line for every stretch neither
//! side mentioned. See the design spec (`docs/superpowers/specs/
//! 2026-08-21-rv-full-file-context-design.md`) §3 for why this can fail
//! honestly rather than always succeed: difftastic can report a region as
//! unremarkable (zero chunks) while the two sides disagree on how many lines
//! it spans — a whitespace-driven reformat is the observed case — and there
//! is then no line-for-line correspondence to print without inventing one.

use super::model::DiffLine;
use super::model::LineKind;

/// Interleaves `changed` with synthesized `Context` lines for every gap
/// between two points both sides agree on — an aligned pair (a `Removed`
/// immediately followed by the `Added` it pairs with, both carrying the same
/// numbers), or either end of the file — where the old and new side of that
/// gap hold the same number of lines.
///
/// `changed` must already be in the order [`super::ordering::order`]
/// produces: file order, no repeats, every entry's own side's number
/// pointing at real text. This is [`super::difftastic::parse`]'s contract,
/// which is the only producer this is meant to run against — the `similar`
/// fallback already emits full context and must not be passed through here.
///
/// Returns `None` the moment one gap does not zip 1:1: rather than fabricate
/// a pairing across a difftastic-elided reformat, the whole file falls back
/// to the changed-only view the caller already had.
pub fn merge(changed: &[DiffLine], old_text: &str, new_text: &str) -> Option<Vec<DiffLine>> {
    let old_lines: Vec<&str> = old_text.lines().collect();
    let new_lines: Vec<&str> = new_text.lines().collect();

    let mut result = Vec::with_capacity(changed.len() + old_lines.len());
    // 0-based: how much of each file has been accounted for so far.
    let mut base_cursor = 0usize;
    let mut head_cursor = 0usize;
    let mut index = 0usize;

    while let Some(line) = changed.get(index) {
        let aligned_partner = changed.get(index + 1).is_some_and(|next| {
            line.kind == LineKind::Removed
                && next.kind == LineKind::Added
                && next.left == line.left
                && next.right == line.right
        });
        let base_at = index_of(line.left);
        let head_at = index_of(line.right);
        let step = if aligned_partner { 2 } else { 1 };

        // The gap to fill before this step, and where each cursor lands
        // after it. A step names a real number on at least one side (its
        // own); the other side, when not also named, is inferred by
        // stepping the same distance the elapsed shared lines imply — and
        // that inferred position is where the *gap* ends, not where the
        // cursor lands, because a one-sided step consumes no line on the
        // side it does not name.
        let (gap_base_end, gap_head_end, next_base, next_head) = match (base_at, head_at) {
            (Some(base), Some(head)) => (base, head, base + 1, head + 1),
            (Some(base), None) => {
                let run = base.checked_sub(base_cursor)?;
                (base, head_cursor + run, base + 1, head_cursor + run)
            }
            (None, Some(head)) => {
                let run = head.checked_sub(head_cursor)?;
                (base_cursor + run, head, base_cursor + run, head + 1)
            }
            (None, None) => return None,
        };

        fill_gap(
            &mut result,
            base_cursor,
            head_cursor,
            gap_base_end,
            gap_head_end,
            &new_lines,
        )?;
        base_cursor = next_base;
        head_cursor = next_head;

        result.push(line.clone());
        if step == 2 {
            let partner = changed.get(index + 1)?;
            result.push(partner.clone());
        }
        index += step;
    }

    fill_gap(
        &mut result,
        base_cursor,
        head_cursor,
        old_lines.len(),
        new_lines.len(),
        &new_lines,
    )?;
    Some(result)
}

/// A 1-based `DiffLine` number to the 0-based index it names.
fn index_of(number: Option<u32>) -> Option<usize> {
    Some(usize::try_from(number?).ok()?).and_then(|one_based: usize| one_based.checked_sub(1))
}

/// Pushes `Context` lines for the stretch `[base_start, base_end)` /
/// `[head_start, head_end)` — a half-open range in each file's own 0-based
/// indices — or fails the whole merge if the two stretches are not the same
/// length, which is the honesty rule the module doc comment describes.
///
/// Text is read from the **new** side throughout: the two stretches are the
/// same length by the check just performed, but not necessarily
/// byte-identical (difftastic elides whitespace-only differences too), and
/// the pane shows the file as it stands now.
fn fill_gap(
    result: &mut Vec<DiffLine>,
    base_start: usize,
    head_start: usize,
    base_end: usize,
    head_end: usize,
    new_lines: &[&str],
) -> Option<()> {
    let base_len = base_end.checked_sub(base_start)?;
    let head_len = head_end.checked_sub(head_start)?;
    if base_len != head_len {
        return None;
    }
    for offset in 0..base_len {
        result.push(DiffLine {
            kind: LineKind::Context,
            left: u32::try_from(base_start + offset + 1).ok(),
            right: u32::try_from(head_start + offset + 1).ok(),
            text: (*new_lines.get(head_start + offset)?).to_owned(),
        });
    }
    Some(())
}
