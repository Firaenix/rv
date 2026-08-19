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
/// Which line of `text` [`snapshot_of`]'s first entry is, 1-based, or `0` where
/// the snapshot is empty.
///
/// The same clamp `snapshot_of` applies, stated once: near the top of a file the
/// window cannot open five lines above the target, so the target is not in the
/// middle and its position has to be recorded rather than assumed.
#[must_use]
pub fn snapshot_start(text: &str, line: u32) -> u32 {
    let lines = text.lines().count();
    let Some(index) = line.checked_sub(1).map(|zero_based| zero_based as usize) else {
        return 0;
    };
    if index >= lines {
        return 0;
    }
    u32::try_from(index.saturating_sub(5))
        .unwrap_or(0)
        .saturating_add(1)
}

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

/// Sentinel `content_hash` for an out-of-range `line` passed to [`create`].
/// Not valid hex, so it can never equal a real blake3 hex digest — [`resolve`]
/// therefore never matches it against any line, needing no special case of
/// its own to fail such an anchor safely to `Outdated` rather than fabricating
/// a `Moved` match against an unrelated line (see [`create`]).
const OUT_OF_RANGE_HASH: &str = "<rv:out-of-range>";

/// Builds an anchor for 1-based `line` of `text` on the given `side`.
///
/// If `line` is `0` or past the end of `text` — out of range for content
/// that does not exist — the anchor is still built rather than panicking,
/// but `content_hash` is set to the [`OUT_OF_RANGE_HASH`] sentinel rather
/// than the hash of an empty string: an empty-string hash is indistinguishable
/// from any real blank line's hash, which would let such an anchor
/// "resolve" `Moved` to some unrelated blank line instead of failing safely.
/// The sentinel matches nothing, so [`resolve`] always lands `(None,
/// Outdated)` for it. `context` is empty (`snapshot_of` returns `Vec::new()`
/// for the same out-of-range condition).
pub fn create(file: &str, side: Side, line: u32, text: &str) -> Anchor {
    let lines: Vec<&str> = text.lines().collect();
    let target = line
        .checked_sub(1)
        .and_then(|zero_based| lines.get(zero_based as usize))
        .copied();
    let content_hash = match target {
        Some(target) => content_hash(target),
        None => OUT_OF_RANGE_HASH.to_owned(),
    };
    Anchor {
        file: file.to_owned(),
        side,
        line,
        content_hash,
        context: snapshot_of(text, line),
        context_start: snapshot_start(text, line),
    }
}

/// Re-locates `anchor` in `text`, a new version of its file.
///
/// The cascade:
/// 1. If the line still at `anchor.line` hashes the same, nothing moved:
///    `(Some(anchor.line), Exact)`. This applies even when that line is
///    blank — an unmoved blank line still resolves `Exact`.
/// 2. Otherwise every *non-blank* line of `text` (normalized content is not
///    empty) is scanned for a hash match; the one nearest `anchor.line` (by
///    absolute line-number distance) is taken as where the content moved
///    to: `(Some(new_line), Moved)`. Ties — content duplicated equally far
///    before and after the original line — favor the earlier line, since
///    lines are scanned in increasing order and the first minimum found
///    wins.
///
///    Blank lines are excluded from this step because every blank or
///    whitespace-only line normalizes to the same `""` and therefore hashes
///    identically: without this exclusion, a comment anchored on one blank
///    line that moved would "resolve" `Moved` to some *other*, unrelated
///    blank line in the file rather than failing safely. Excluding them
///    means a moved blank-line anchor falls through to step 3 instead.
/// 3. If no (non-blank) line matches but the file still *has* a line at
///    `anchor.line`, the raw number is the fallback: `(Some(anchor.line),
///    Weak)`. The commented-on content is gone, but "line 48 of this file" is
///    still a place a reviewer can be taken to — which beats declaring the
///    comment unplaceable while its line visibly exists (spec §9's third
///    tier).
/// 4. Otherwise the anchor cannot be placed: `(None, Outdated)`.
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
        .filter(|(_, candidate)| !normalize(candidate).is_empty())
        .filter(|(_, candidate)| content_hash(candidate) == anchor.content_hash)
        .min_by_key(|(index, _)| {
            let candidate_line = *index as u32 + 1;
            candidate_line.abs_diff(anchor.line)
        });

    match nearest {
        Some((index, _)) => (Some(index as u32 + 1), Confidence::Moved),
        // An out-of-range anchor never takes the fallback: it named no line
        // when it was created, so there is no line to fall back to.
        None if same_line.is_some() && anchor.content_hash != OUT_OF_RANGE_HASH => {
            (Some(anchor.line), Confidence::Weak)
        }
        None => (None, Confidence::Outdated),
    }
}
