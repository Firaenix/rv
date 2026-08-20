//! The `similar` line diff: what a reviewer sees when difftastic's structural
//! diff is unavailable or not believed.

use super::decode;
use super::index_to_line;
use super::model::DiffLine;
use super::model::LineKind;

/// The fallback line diff, used when difftastic is skipped, refused by the
/// version probe, fails to run, or returns something the JSON parser cannot
/// make sense of. Returns the lines and whether the diff should be suppressed.
///
/// The diff runs over the lines *as this module renders them* — terminators
/// stripped — not over `similar`'s own tokens, which keep the terminator
/// attached. Diffing the tokens marked a line as changed when only its
/// terminator changed, and since `DiffLine::text` is the whole of what a
/// reviewer sees, that showed up as a `Removed`/`Added` pair whose displayed
/// text was character-for-character identical: a change with nothing changed
/// in it. Comparing what is displayed keeps "what changed" and "what is shown"
/// the same question.
///
/// A terminator-only difference is still a difference, though, and the diff
/// must not go silent about it — it says so through the suppression flag, the
/// same way difftastic reports the same inputs (`status: "unchanged"`), rather
/// than by inventing lines. The two sides differ only in their terminators
/// exactly when they render to the same lines but their decoded text does not
/// match.
pub fn diff(old: Option<&[u8]>, new: Option<&[u8]>) -> (Vec<DiffLine>, bool) {
    let old_text = decode(old.unwrap_or(&[]));
    let new_text = decode(new.unwrap_or(&[]));
    let old_lines = split_lines(&old_text);
    let new_lines = split_lines(&new_text);
    let diff = similar::TextDiff::from_slices(&old_lines, &new_lines);

    let lines = diff
        .iter_all_changes()
        .map(|change| {
            let kind = match change.tag() {
                similar::ChangeTag::Equal => LineKind::Context,
                similar::ChangeTag::Delete => LineKind::Removed,
                similar::ChangeTag::Insert => LineKind::Added,
            };
            DiffLine {
                kind,
                left: index_to_line(change.old_index()),
                right: index_to_line(change.new_index()),
                text: change.value().to_owned(),
            }
        })
        .collect();

    let suppressed = old_lines == new_lines && old_text != new_text;
    (lines, suppressed)
}

/// The lines of `text` as this module renders them: `\r\n`, `\n` and a bare
/// `\r` all terminate a line — matching what `similar`'s own line tokenizer
/// recognizes, which is what the fallback used to inherit — the terminator is
/// not part of the line, and a file ending in a terminator does not gain an
/// empty last line.
///
/// Byte indexing is safe here: `\r` and `\n` are ASCII, so they never occur
/// inside a multi-byte UTF-8 sequence and every split lands on a character
/// boundary.
fn split_lines(text: &str) -> Vec<&str> {
    let bytes = text.as_bytes();
    let mut lines = Vec::new();
    let mut start = 0;
    let mut at = 0;
    while at < bytes.len() {
        match bytes[at] {
            b'\n' => {
                lines.push(&text[start..at]);
                at += 1;
            }
            b'\r' => {
                lines.push(&text[start..at]);
                at += if bytes.get(at + 1) == Some(&b'\n') {
                    2
                } else {
                    1
                };
            }
            _ => {
                at += 1;
                continue;
            }
        }
        start = at;
    }
    if start < bytes.len() {
        lines.push(&text[start..]);
    }
    lines
}
