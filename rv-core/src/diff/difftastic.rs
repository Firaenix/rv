//! Running difftastic and reading what it says.
//!
//! difftastic is invoked per the contract in the project's global constraints:
//! `DFT_UNSTABLE=yes difft --display json <old> <new>`, reading the JSON
//! `status` field rather than trusting the exit code. It takes file paths, not
//! stdin, so the two sides are written to temporary files first.

use std::collections::HashSet;
use std::io::Write as _;
use std::path::Path;

use serde_json::Value;

use super::decode;
use super::index_to_line;
use super::model::DiffLine;
use super::model::DiffSource;
use super::model::LineKind;
use super::ordering::Entry;
use super::ordering::order;
use super::probe;

/// What difftastic said about one file: its lines, the label naming it, and
/// whether the difference is one worth showing.
pub type Answer = (Vec<DiffLine>, DiffSource, bool);

/// Runs difftastic over `old`/`new` and parses its JSON, returning `None` when
/// this run produced nothing usable — a spawn that failed, or output that is
/// not the shape [`parse`] reads.
///
/// The caller has already established that difftastic is *installed and new
/// enough* through [`probe::verdict`]; a `None` from here is therefore about
/// this run, not about the binary.
pub fn run(old: Option<&[u8]>, new: Option<&[u8]>, path: &str) -> Option<Answer> {
    run_with(old, new, path, false)
}

/// As [`run`], but with `--byte-limit 0` — difftastic's switch to its
/// line-oriented engine, which reports whitespace-only edits (§3's
/// reformatted-region case) as chunks the merger can walk. See the design
/// spec §4.6: the caller — [`super::merge`]'s worker in rv — only asks for
/// this after the syntax-aware answer's merge returned `None`.
///
/// The parser is the same one [`run`] uses; the only difference is which
/// engine inside difftastic produced the chunks it returns.
pub fn run_line_oriented(old: Option<&[u8]>, new: Option<&[u8]>, path: &str) -> Option<Answer> {
    run_with(old, new, path, true)
}

fn run_with(
    old: Option<&[u8]>,
    new: Option<&[u8]>,
    path: &str,
    line_oriented: bool,
) -> Option<Answer> {
    let suffix = extension_suffix(path);
    let mut old_file = tempfile::Builder::new().suffix(&suffix).tempfile().ok()?;
    let mut new_file = tempfile::Builder::new().suffix(&suffix).tempfile().ok()?;
    old_file.write_all(old.unwrap_or(&[])).ok()?;
    new_file.write_all(new.unwrap_or(&[])).ok()?;

    let mut command = probe::command();
    command.env("DFT_UNSTABLE", "yes");
    if line_oriented {
        // `--byte-limit 0` selects difftastic's line-oriented engine (see
        // difftastic's `DFT_BYTE_LIMIT` docs): every file is over the limit,
        // so no file gets the syntax-aware parser. Available since 0.51.0
        // (rv's minimum, `probe::MINIMUM_DIFFT`), so no version bump.
        command.arg("--byte-limit").arg("0");
    }
    let output = command
        .arg("--display")
        .arg("json")
        .arg(old_file.path())
        .arg(new_file.path())
        .output()
        .ok()?;
    // Per the difft invocation contract: never trust the exit code, always
    // try to parse stdout as the documented JSON.
    let json: Value = serde_json::from_slice(&output.stdout).ok()?;
    parse(&json, old, new)
}

/// The suffix to give difft's temp files so it detects the same language it
/// would for the real path — difftastic picks a language from the file
/// extension, not the content.
fn extension_suffix(path: &str) -> String {
    match Path::new(path).extension().and_then(|ext| ext.to_str()) {
        Some(ext) => format!(".{ext}"),
        None => String::new(),
    }
}

/// Parses difftastic's `--display json` output. `None` on anything that does
/// not match the documented shape (missing/mistyped fields, an out-of-range
/// line number): schema-tolerant by construction, since every step uses `?`
/// rather than indexing or unwrapping.
///
/// This is the parser [`probe::MINIMUM_DIFFT`] is pinned to: the field set read
/// below is what defines which difftastic releases are usable.
pub fn parse(json: &Value, old: Option<&[u8]>, new: Option<&[u8]>) -> Option<Answer> {
    let status = json.get("status")?.as_str()?;
    let language = json
        .get("language")
        .and_then(Value::as_str)
        .unwrap_or("Text")
        .to_owned();
    // `line_oriented` is `false` here regardless of which engine actually
    // ran: the parser describes what difftastic returned, and the caller
    // (`super::merge`'s worker) is the only thing that knows whether this
    // parse resulted from the retry — it flips the flag on the way back.
    let source = DiffSource::Difftastic {
        language,
        line_oriented: false,
    };

    // "unchanged" means no *semantic* difference (e.g. pure reindentation).
    // difftastic emits no chunks in this case, so there is nothing further to
    // parse; the diff is suppressed rather than empty-by-accident.
    if status == "unchanged" {
        return Some((Vec::new(), source, true));
    }

    // A whole-file add/remove: difftastic reports these as "created"/
    // "deleted" rather than "changed", and — unlike "changed" — emits no
    // `chunks` at all, even though it ran successfully and correctly
    // identified the language. Building the all-Added/all-Removed lines
    // directly from the text difftastic was given (rather than falling back
    // to `similar`) keeps `source` truthful: difftastic did answer, so the
    // diff it labels should say so.
    if status == "created" {
        return Some((all_added(&decode(new.unwrap_or(&[]))), source, false));
    }
    if status == "deleted" {
        return Some((all_removed(&decode(old.unwrap_or(&[]))), source, false));
    }

    // Any other status (in practice, "changed") carries `chunks`; requiring
    // the key here, rather than defaulting to an empty Vec, is what sends a
    // genuinely unexpected shape to the `similar` fallback instead of
    // silently returning an empty diff for a file that did change.
    let chunks = json.get("chunks")?.as_array()?;

    let old_text = decode(old.unwrap_or(&[]));
    let new_text = decode(new.unwrap_or(&[]));
    let old_lines: Vec<&str> = old_text.lines().collect();
    let new_lines: Vec<&str> = new_text.lines().collect();

    // Resolve every entry first, then order and emit. Two things make that
    // necessary rather than fussy, both of them observed from difft 0.70 and
    // both of them things a reviewer would see, since the TUI renders
    // `lines` in order:
    //
    // - The same entry can appear in two chunks (old `"b\n\n"` vs new
    //   `"a\n\n"` yields `[[{lhs:0,rhs:0}],[{lhs:0,rhs:0}]]`), so appending
    //   chunk after chunk showed a one-line change in a two-line file as four
    //   lines, the same pair drawn twice.
    // - Chunks are not ordered by line number (old `"a\nb\nc\nd\n"` vs new
    //   `"a\nB\nc\nd\ne\n"` reports new line 5 before line 2), so appending
    //   put the file's last line above its second.
    let mut seen = HashSet::new();
    let mut entries = Vec::new();
    for chunk in chunks {
        for entry in chunk.as_array()? {
            let lhs = entry.get("lhs").filter(|value| !value.is_null());
            let rhs = entry.get("rhs").filter(|value| !value.is_null());
            let entry = match (lhs, rhs) {
                // difftastic aligned these two lines together (they may be
                // identical apart from an inline change); both sides' line
                // numbers apply to each of the lines this produces.
                (Some(lhs), Some(rhs)) => {
                    let (left, left_text) = line_ref(lhs, &old_lines)?;
                    let (right, right_text) = line_ref(rhs, &new_lines)?;
                    Entry::Aligned {
                        left,
                        left_text,
                        right,
                        right_text,
                    }
                }
                (Some(lhs), None) => {
                    let (left, text) = line_ref(lhs, &old_lines)?;
                    Entry::Removed { left, text }
                }
                (None, Some(rhs)) => {
                    let (right, text) = line_ref(rhs, &new_lines)?;
                    Entry::Added { right, text }
                }
                (None, None) => return None,
            };
            // An entry is fully described by the line numbers it carries —
            // the text comes from those numbers — so a repeat carries no
            // information and dropping it loses none.
            if seen.insert(entry.numbers()) {
                entries.push(entry);
            }
        }
    }

    order(&mut entries);
    Some((
        entries.into_iter().flat_map(Entry::lines).collect(),
        source,
        false,
    ))
}

/// Resolves one side of a chunk entry (`difftastic`'s 0-based `line_number`)
/// to a 1-based line number and that line's full text, taken from the
/// original content rather than difftastic's own `changes`, which only
/// carries the sub-line span that differs.
fn line_ref(side: &Value, lines: &[&str]) -> Option<(u32, String)> {
    let line_number = side.get("line_number")?.as_u64()?;
    let index = usize::try_from(line_number).ok()?;
    let text = (*lines.get(index)?).to_owned();
    let one_based = u32::try_from(index).ok()?.checked_add(1)?;
    Some((one_based, text))
}

/// Every line of `text` as an `Added` line, `right`-numbered from 1 — the
/// shape difftastic's "created" status implies but does not spell out itself.
fn all_added(text: &str) -> Vec<DiffLine> {
    text.lines()
        .enumerate()
        .map(|(index, line)| DiffLine {
            kind: LineKind::Added,
            left: None,
            right: index_to_line(Some(index)),
            text: line.to_owned(),
        })
        .collect()
}

/// Every line of `text` as a `Removed` line, `left`-numbered from 1 — the
/// shape difftastic's "deleted" status implies but does not spell out itself.
fn all_removed(text: &str) -> Vec<DiffLine> {
    text.lines()
        .enumerate()
        .map(|(index, line)| DiffLine {
            kind: LineKind::Removed,
            left: index_to_line(Some(index)),
            right: None,
            text: line.to_owned(),
        })
        .collect()
}
