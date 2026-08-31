//! `v g`: regrouping difftastic's diff the way a unified diff prints it.
//!
//! difftastic aligns related lines across a change and lays the two sides down
//! interwoven — a removal, its replacement, the next removal — which reads
//! badly when a reviewer wants to see the old block whole and then the new one
//! whole. Grouping puts every removal in a run of changes before every addition
//! in it, with the context lines that bound the run left where they are, so the
//! result is the familiar `-` block then `+` block per hunk. Line kinds and
//! numbers are untouched, so the gutter and the highlighting still hold.

use rv_core::diff::DiffLine;
use rv_core::diff::LineKind;

/// `lines` with each hunk's removals moved ahead of its additions.
///
/// A hunk here is a maximal run of changed lines; the context lines between
/// runs are the boundaries and never move. Within a run the removals keep their
/// order and the additions keep theirs — only the two are separated.
#[must_use]
pub fn group(lines: Vec<DiffLine>) -> Vec<DiffLine> {
    let mut out: Vec<DiffLine> = Vec::with_capacity(lines.len());
    let mut removed: Vec<DiffLine> = Vec::new();
    let mut added: Vec<DiffLine> = Vec::new();
    for line in lines {
        match line.kind {
            LineKind::Removed => removed.push(line),
            LineKind::Added => added.push(line),
            LineKind::Context => {
                out.append(&mut removed);
                out.append(&mut added);
                out.push(line);
            }
        }
    }
    out.append(&mut removed);
    out.append(&mut added);
    out
}
