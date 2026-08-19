//! `rv diff`: the range's changes in rv's own coordinates.
//!
//! The reviewer used to read hunks from `jj diff` and translate them into rv's
//! side-aware numbers by inference; a mistranslation was either a refused
//! comment or an anchor on a plausible-looking *wrong line*. The numbers this
//! prints are numbers rv itself computed — the tool that validates the anchor
//! is the tool that issues the coordinates — so `right: 238` here is a line
//! `rv comment --line 238` accepts, by construction.

use anyhow::Context as _;
use anyhow::Result;
use rv::session::Review;
use rv_core::diff;
use rv_core::diff::DiffSource;
use rv_core::diff::FileDiff;
use rv_core::diff::LineKind;
use rv_core::model::FileChange;
use serde_json::json;

/// Prints the diff of `file` — or of every file in the range — as JSON or as
/// plain rows.
///
/// Diffs are computed per file as they are printed, the same lazy discipline
/// the pane has: this is a query iterating them, not an eager whole-range load.
pub fn diff(review: &Review, file: Option<&str>, json: bool, no_difft: bool) -> Result<()> {
    let files: Vec<&FileChange> = match file {
        Some(path) => vec![
            review
                .files
                .iter()
                .find(|file| file.path == path || file.source_path.as_deref() == Some(path))
                .with_context(|| {
                    format!(
                        "{path} is not in this review's range ({})",
                        review.session.revset
                    )
                })?,
        ],
        None => review.files.iter().collect(),
    };

    let mut reports = Vec::with_capacity(files.len());
    for file in files {
        let computed = compute(review, file, no_difft)?;
        if json {
            reports.push(file_json(&computed));
        } else {
            print_plain(&computed);
        }
    }
    if json {
        let serialized =
            serde_json::to_string_pretty(&reports).context("could not serialize the diff")?;
        println!("{serialized}");
    }
    Ok(())
}

/// One file's diff, read at the same endpoints and paths the pane reads: both
/// sides at their own path, so a rename diffs its source against its target.
fn compute(review: &Review, file: &FileChange, no_difft: bool) -> Result<FileDiff> {
    let base_path = file.source_path.as_deref().unwrap_or(&file.path);
    let old = review
        .repo
        .read_blob(&review.session.base_commit, base_path)
        .with_context(|| format!("could not read {base_path} at the base of the review"))?;
    let new = review
        .repo
        .read_blob(&review.session.head_commit, &file.path)
        .with_context(|| format!("could not read {} at the head of the review", file.path))?;
    Ok(if no_difft {
        diff::compute_with(old.as_deref(), new.as_deref(), &file.path, false)
    } else {
        diff::compute(old.as_deref(), new.as_deref(), &file.path)
    })
}

/// The JSON the spec pins: engine and suppression stated — a degraded or
/// suppressed diff is never presented as a structural one — and a binary file
/// reported as such, with no lines.
fn file_json(computed: &FileDiff) -> serde_json::Value {
    let (engine, language) = match &computed.source {
        DiffSource::Difftastic { language } => ("difftastic", Some(language.as_str())),
        DiffSource::Similar => ("fallback", None),
        DiffSource::Binary => ("binary", None),
    };
    json!({
        "file": computed.path,
        "engine": engine,
        "language": language,
        "binary": computed.source == DiffSource::Binary,
        "suppressed": computed.suppressed,
        "lines": computed
            .lines
            .iter()
            .map(|line| json!({
                "kind": kind_name(line.kind),
                "left": line.left,
                "right": line.right,
                "text": line.text,
            }))
            .collect::<Vec<_>>(),
    })
}

/// The human form: one header per file, one row per line, the same numbers.
fn print_plain(computed: &FileDiff) {
    let engine = match &computed.source {
        DiffSource::Difftastic { language } => format!("difftastic ({language})"),
        DiffSource::Similar => "fallback".to_owned(),
        DiffSource::Binary => "binary".to_owned(),
    };
    let suppressed = if computed.suppressed {
        " — no semantic change"
    } else {
        ""
    };
    println!("{} — {engine}{suppressed}", computed.path);
    for line in &computed.lines {
        let number = |side: Option<u32>| {
            side.map_or_else(|| " ".repeat(5), |number| format!("{number:>5}"))
        };
        let sigil = match line.kind {
            LineKind::Added => '+',
            LineKind::Removed => '-',
            LineKind::Context => ' ',
        };
        println!(
            "  {} {} {sigil} {}",
            number(line.left),
            number(line.right),
            line.text
        );
    }
}

fn kind_name(kind: LineKind) -> &'static str {
    match kind {
        LineKind::Context => "context",
        LineKind::Added => "added",
        LineKind::Removed => "removed",
    }
}
