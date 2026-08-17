//! Diff production: difftastic's structural, syntax-aware diff with a plain
//! line-based fallback from the `similar` crate.
//!
//! This module takes plain byte slices in and produces a [`FileDiff`] out — no
//! `jj_lib` type crosses this boundary, so it can be exercised without a
//! repository at all. difftastic is an external process, invoked per the
//! contract in the project's global constraints:
//! `DFT_UNSTABLE=yes difft --display json <old> <new>`, reading the JSON
//! `status` field rather than trusting the exit code. difftastic takes file
//! paths, not stdin, so the two sides are written to temporary files first.
//!
//! [`compute`] and [`compute_with`] never panic and never return an error:
//! anything that goes wrong — no `difft` on `PATH`, output that is not the
//! JSON shape this module expects, non-UTF-8 content — degrades to the
//! `similar` fallback rather than failing the caller.

use std::io::Write as _;
use std::path::Path;
use std::process::Command;

use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;

/// Set to force the `similar` fallback, bypassing difftastic even when it is
/// on `PATH`. Read only by `compute`; [`compute_with`] takes the choice as an
/// explicit argument so it never touches the environment or a process.
const RV_NO_DIFFT: &str = "RV_NO_DIFFT";

/// How a file differs between the two endpoints of a review.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FileDiff {
    pub path: String,
    pub lines: Vec<DiffLine>,
    pub source: DiffSource,
    /// Set when difftastic reports the change as syntactically unchanged
    /// (e.g. pure reindentation): the lines above are not worth showing a
    /// reviewer by default.
    pub suppressed: bool,
}

/// One line of a [`FileDiff`]. `left` and `right` are 1-based line numbers on
/// the base and head side respectively; either may be absent, but both are
/// populated when difftastic aligns a changed line with its counterpart on
/// the other side.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DiffLine {
    pub kind: LineKind,
    pub left: Option<u32>,
    pub right: Option<u32>,
    pub text: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum LineKind {
    Context,
    Added,
    Removed,
}

/// Where the lines of a [`FileDiff`] came from.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum DiffSource {
    /// Produced from difftastic's JSON output; `language` is whatever
    /// difftastic detected (it always reports one, defaulting to `"Text"`).
    Difftastic { language: String },
    /// Produced by the `similar` crate's line diff, either because
    /// difftastic was skipped or because it failed or returned something
    /// this module could not parse.
    Similar,
    /// Neither side was diffed: a NUL byte was found on at least one side.
    Binary,
}

/// Computes the diff between `old` and `new`, trying difftastic unless
/// `RV_NO_DIFFT` is set in the environment, and falling back to `similar`
/// otherwise. `old`/`new` of `None` mean the file does not exist on that
/// side (a whole-file add or remove).
pub fn compute(old: Option<&[u8]>, new: Option<&[u8]>, path: &str) -> FileDiff {
    let use_difft = std::env::var_os(RV_NO_DIFFT).is_none();
    compute_with(old, new, path, use_difft)
}

/// As [`compute`], but with explicit control over whether difftastic is
/// attempted. Passing `false` never spawns a process or reads the
/// environment, which is what keeps fallback-focused tests hermetic.
pub fn compute_with(
    old: Option<&[u8]>,
    new: Option<&[u8]>,
    path: &str,
    use_difft: bool,
) -> FileDiff {
    if is_binary(old) || is_binary(new) {
        return FileDiff {
            path: path.to_owned(),
            lines: Vec::new(),
            source: DiffSource::Binary,
            suppressed: false,
        };
    }

    if use_difft && let Some((lines, source, suppressed)) = try_difft(old, new, path) {
        return FileDiff {
            path: path.to_owned(),
            lines,
            source,
            suppressed,
        };
    }

    FileDiff {
        path: path.to_owned(),
        lines: similar_diff(old, new),
        source: DiffSource::Similar,
        suppressed: false,
    }
}

fn is_binary(side: Option<&[u8]>) -> bool {
    side.is_some_and(|bytes| bytes.contains(&0))
}

/// Lossy UTF-8 decode: good enough to diff, per the module's contract that it
/// never fails on non-UTF-8 input.
fn decode(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// Runs difftastic over `old`/`new` and parses its JSON, returning `None` on
/// any failure so the caller falls back to `similar` — a missing binary, a
/// non-zero exit some difft version might use, or JSON that is not the shape
/// this module expects all take the same path.
fn try_difft(
    old: Option<&[u8]>,
    new: Option<&[u8]>,
    path: &str,
) -> Option<(Vec<DiffLine>, DiffSource, bool)> {
    let suffix = extension_suffix(path);
    let mut old_file = tempfile::Builder::new().suffix(&suffix).tempfile().ok()?;
    let mut new_file = tempfile::Builder::new().suffix(&suffix).tempfile().ok()?;
    old_file.write_all(old.unwrap_or(&[])).ok()?;
    new_file.write_all(new.unwrap_or(&[])).ok()?;

    let output = Command::new("difft")
        .env("DFT_UNSTABLE", "yes")
        .arg("--display")
        .arg("json")
        .arg(old_file.path())
        .arg(new_file.path())
        .output()
        .ok()?;
    // Per the difft invocation contract: never trust the exit code, always
    // try to parse stdout as the documented JSON.
    let json: Value = serde_json::from_slice(&output.stdout).ok()?;
    parse_difft_json(&json, old, new)
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
fn parse_difft_json(
    json: &Value,
    old: Option<&[u8]>,
    new: Option<&[u8]>,
) -> Option<(Vec<DiffLine>, DiffSource, bool)> {
    let status = json.get("status")?.as_str()?;
    let language = json
        .get("language")
        .and_then(Value::as_str)
        .unwrap_or("Text")
        .to_owned();
    let source = DiffSource::Difftastic { language };

    // "unchanged" means no *semantic* difference (e.g. pure reindentation).
    // difftastic emits no chunks in this case, so there is nothing further to
    // parse; the diff is suppressed rather than empty-by-accident.
    if status == "unchanged" {
        return Some((Vec::new(), source, true));
    }

    // Statuses other than "changed"/"unchanged" (difftastic also has
    // "created"/"deleted" for a whole-file add/remove) carry no chunks at
    // all, so there is nothing to build a structural diff from. Requiring
    // the key here, rather than defaulting to an empty Vec, is what sends
    // those cases to the `similar` fallback instead of silently returning an
    // empty diff for a file that did change.
    let chunks = json.get("chunks")?.as_array()?;

    let old_text = decode(old.unwrap_or(&[]));
    let new_text = decode(new.unwrap_or(&[]));
    let old_lines: Vec<&str> = old_text.lines().collect();
    let new_lines: Vec<&str> = new_text.lines().collect();

    let mut lines = Vec::new();
    for chunk in chunks {
        for entry in chunk.as_array()? {
            let lhs = entry.get("lhs").filter(|value| !value.is_null());
            let rhs = entry.get("rhs").filter(|value| !value.is_null());
            match (lhs, rhs) {
                (Some(lhs), Some(rhs)) => {
                    // difftastic aligned these two lines together (they may
                    // be identical apart from an inline change); both sides'
                    // line numbers apply to each of the lines this produces.
                    let (left, left_text) = line_ref(lhs, &old_lines)?;
                    let (right, right_text) = line_ref(rhs, &new_lines)?;
                    lines.push(DiffLine {
                        kind: LineKind::Removed,
                        left: Some(left),
                        right: Some(right),
                        text: left_text,
                    });
                    lines.push(DiffLine {
                        kind: LineKind::Added,
                        left: Some(left),
                        right: Some(right),
                        text: right_text,
                    });
                }
                (Some(lhs), None) => {
                    let (left, text) = line_ref(lhs, &old_lines)?;
                    lines.push(DiffLine {
                        kind: LineKind::Removed,
                        left: Some(left),
                        right: None,
                        text,
                    });
                }
                (None, Some(rhs)) => {
                    let (right, text) = line_ref(rhs, &new_lines)?;
                    lines.push(DiffLine {
                        kind: LineKind::Added,
                        left: None,
                        right: Some(right),
                        text,
                    });
                }
                (None, None) => return None,
            }
        }
    }
    Some((lines, source, false))
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

/// The fallback line diff, used when difftastic is skipped, fails to run, or
/// returns something [`parse_difft_json`] cannot make sense of.
fn similar_diff(old: Option<&[u8]>, new: Option<&[u8]>) -> Vec<DiffLine> {
    let old_text = decode(old.unwrap_or(&[]));
    let new_text = decode(new.unwrap_or(&[]));
    let diff = similar::TextDiff::from_lines(old_text.as_str(), new_text.as_str());

    diff.iter_all_changes()
        .map(|change| {
            let kind = match change.tag() {
                similar::ChangeTag::Equal => LineKind::Context,
                similar::ChangeTag::Delete => LineKind::Removed,
                similar::ChangeTag::Insert => LineKind::Added,
            };
            let left = index_to_line(change.old_index());
            let right = index_to_line(change.new_index());
            let text = change.value().trim_end_matches(['\n', '\r']).to_owned();
            DiffLine {
                kind,
                left,
                right,
                text,
            }
        })
        .collect()
}

/// `similar`'s 0-based index to this module's 1-based line number.
fn index_to_line(index: Option<usize>) -> Option<u32> {
    let index = u32::try_from(index?).ok()?;
    index.checked_add(1)
}
