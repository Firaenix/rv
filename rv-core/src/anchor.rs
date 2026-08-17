//! Comment anchors: locating a commented-on line in a new version of a
//! file's text, even after the file has been edited or its history rewritten
//! (spec §9). Content wins over line number: an [`Anchor`] carries a hash of
//! its target line, normalized so pure reindentation cannot invalidate it,
//! plus a snapshot of the surrounding lines for a reviewer to orient by.
//! [`resolve`] re-locates the anchor in new text by that hash rather than by
//! trusting the line number to have stayed put.
//!
//! Pure data in, pure data out: no jj-lib, no filesystem, no process.

use crate::model::Anchor;
use crate::model::Confidence;
use crate::model::Side;

/// Collapses insignificant whitespace so two lines that differ only in
/// indentation or run-length of internal whitespace hash the same: splits on
/// any run of whitespace and rejoins with a single space, which also strips
/// leading and trailing whitespace as a side effect of `split_whitespace`
/// never yielding empty leading/trailing pieces.
pub fn normalize(line: &str) -> String {
    line.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The content hash [`resolve`] matches on: the blake3 hex digest of `line`
/// after [`normalize`].
pub fn content_hash(line: &str) -> String {
    blake3::hash(normalize(line).as_bytes())
        .to_hex()
        .to_string()
}

/// Up to 5 lines of context before and after 1-based `line` in `text`, plus
/// `line` itself, clamped at the file's edges. Returns an empty `Vec` if
/// `line` is `0` or past the end of `text` — there is no line to center on.
///
/// Purely descriptive: [`resolve`] does not consult this, so a stale
/// snapshot never changes where a comment re-anchors, only what a reviewer
/// sees while it dangles.
pub fn snapshot_of(text: &str, line: u32) -> Vec<String> {
    let lines: Vec<&str> = text.lines().collect();
    let Some(index) = line.checked_sub(1).map(|zero_based| zero_based as usize) else {
        return Vec::new();
    };
    if index >= lines.len() {
        return Vec::new();
    }
    let start = index.saturating_sub(5);
    let end = (index + 5).min(lines.len() - 1);
    lines[start..=end]
        .iter()
        .map(|line| line.to_string())
        .collect()
}

/// Builds an anchor for 1-based `line` of `text` on the given `side`.
///
/// If `line` is `0` or past the end of `text` — out of range for content
/// that does not exist — the anchor is still built rather than panicking:
/// `content_hash` becomes the hash of an empty line and `context` is empty.
/// Such an anchor cannot land `Exact` or `Moved` against any real text (an
/// empty line's hash only matches other empty lines), so it degrades to
/// `Outdated` on the first [`resolve`] rather than the caller needing to
/// handle a `Result` for a case that should not arise in practice — callers
/// are expected to only anchor lines that exist.
pub fn create(file: &str, side: Side, line: u32, text: &str) -> Anchor {
    let lines: Vec<&str> = text.lines().collect();
    let target = line
        .checked_sub(1)
        .and_then(|zero_based| lines.get(zero_based as usize))
        .copied()
        .unwrap_or("");
    Anchor {
        file: file.to_owned(),
        side,
        line,
        content_hash: content_hash(target),
        context: snapshot_of(text, line),
    }
}

/// Re-locates `anchor` in `text`, a new version of its file.
///
/// The cascade:
/// 1. If the line still at `anchor.line` hashes the same, nothing moved:
///    `(Some(anchor.line), Exact)`.
/// 2. Otherwise every line of `text` is scanned for a hash match; the one
///    nearest `anchor.line` (by absolute line-number distance) is taken as
///    where the content moved to: `(Some(new_line), Moved)`. Ties — content
///    duplicated equally far before and after the original line — favor the
///    earlier line, since lines are scanned in increasing order and the
///    first minimum found wins.
/// 3. If no line matches at all, the anchor cannot be placed:
///    `(None, Outdated)`.
///
/// `Weak` (a line-number-only fallback) is never produced here; it is later
/// milestone work.
pub fn resolve(anchor: &Anchor, text: &str) -> (Option<u32>, Confidence) {
    let lines: Vec<&str> = text.lines().collect();

    let same_line = anchor
        .line
        .checked_sub(1)
        .and_then(|zero_based| lines.get(zero_based as usize));
    if let Some(same_line) = same_line
        && content_hash(same_line) == anchor.content_hash
    {
        return (Some(anchor.line), Confidence::Exact);
    }

    let nearest = lines
        .iter()
        .enumerate()
        .filter(|(_, candidate)| content_hash(candidate) == anchor.content_hash)
        .min_by_key(|(index, _)| {
            let candidate_line = *index as u32 + 1;
            candidate_line.abs_diff(anchor.line)
        });

    match nearest {
        Some((index, _)) => (Some(index as u32 + 1), Confidence::Moved),
        None => (None, Confidence::Outdated),
    }
}
