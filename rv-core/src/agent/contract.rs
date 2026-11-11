//! The reply contract: prose, then one fenced JSON block naming a summary and
//! the lines worth a look. Parsing never fails — a reply without the block is
//! still an answer, just one with nothing to flag.

use std::path::Component;
use std::path::Path;

use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;

use super::sanitize;

pub const MAX_SUMMARY_CHARS: usize = 400;
pub const MAX_WHY_CHARS: usize = 300;
pub const MAX_PATH_CHARS: usize = 512;
pub const MAX_TEXT_CHARS: usize = 20_000;
pub const MAX_FOCUS: usize = 50;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Importance {
    High,
    #[default]
    Medium,
    Low,
}

impl Importance {
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Importance::High => "high",
            Importance::Medium => "medium",
            Importance::Low => "low",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Focus {
    pub path: String,
    pub line: u32,
    pub why: String,
    pub importance: Importance,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Answer {
    /// One line, for a tooltip. Falls back to the prose's first line.
    pub summary: String,
    /// The reply with the contract block taken out, sanitized.
    pub text: String,
    /// Sorted high importance first; malformed entries are dropped one by one.
    pub focus: Vec<Focus>,
    /// Whether the reply carried a parseable contract block at all.
    pub structured: bool,
}

#[must_use]
pub fn parse_answer(raw: &str) -> Answer {
    let raw = &sanitize::text(raw, usize::MAX);
    let (block, prose) = match find_contract(raw) {
        Some((value, range)) => {
            let mut prose = raw.to_owned();
            prose.replace_range(range, "");
            (Some(value), prose)
        }
        None => (None, raw.to_owned()),
    };
    let text = sanitize::text(&prose, MAX_TEXT_CHARS);
    let stated_summary = block
        .as_ref()
        .and_then(|value| value.get("summary"))
        .and_then(Value::as_str)
        .map(|summary| sanitize::line(summary, MAX_SUMMARY_CHARS))
        .filter(|summary| !summary.is_empty());
    let summary = stated_summary.unwrap_or_else(|| {
        let first = text.lines().find(|line| !line.trim().is_empty());
        sanitize::line(first.unwrap_or_default(), MAX_SUMMARY_CHARS)
    });
    let mut focus: Vec<Focus> = block
        .as_ref()
        .and_then(|value| value.get("focus"))
        .and_then(Value::as_array)
        .map(|entries| entries.iter().filter_map(focus_entry).collect())
        .unwrap_or_default();
    focus.sort_by_key(|entry| entry.importance);
    focus.truncate(MAX_FOCUS);
    Answer {
        summary,
        text,
        focus,
        structured: block.is_some(),
    }
}

/// The last fenced block that parses as a contract object, else the whole
/// reply when it is one — with the byte range to cut from the prose.
fn find_contract(raw: &str) -> Option<(Value, std::ops::Range<usize>)> {
    let mut found = None;
    let mut cursor = 0;
    while let Some(open) = raw[cursor..].find("```") {
        let fence_start = cursor + open;
        let after_fence = fence_start + 3;
        let Some(newline) = raw[after_fence..].find('\n') else {
            break;
        };
        let body_start = after_fence + newline + 1;
        let Some(close) = raw[body_start..].find("```") else {
            break;
        };
        let body_end = body_start + close;
        let fence_end = body_end + 3;
        if let Some(value) = contract_object(&raw[body_start..body_end]) {
            found = Some((value, fence_start..fence_end));
        }
        cursor = fence_end;
    }
    found.or_else(|| contract_object(raw).map(|value| (value, 0..raw.len())))
}

fn contract_object(candidate: &str) -> Option<Value> {
    let value: Value = serde_json::from_str(candidate.trim()).ok()?;
    let object = value.as_object()?;
    (object.contains_key("summary") || object.contains_key("focus")).then_some(value)
}

fn focus_entry(value: &Value) -> Option<Focus> {
    let path = value.get("path")?.as_str()?;
    let path = sanitize::line(path, MAX_PATH_CHARS)
        .trim_start_matches("./")
        .to_owned();
    if !is_relative_inside(&path) {
        return None;
    }
    let line = match value.get("line")? {
        Value::Number(number) => number.as_u64()?,
        Value::String(text) => text.trim().parse().ok()?,
        _ => return None,
    };
    let line = u32::try_from(line).ok().filter(|&line| line > 0)?;
    let why = sanitize::line(value.get("why")?.as_str()?, MAX_WHY_CHARS);
    if why.is_empty() {
        return None;
    }
    let importance = value
        .get("importance")
        .and_then(Value::as_str)
        .and_then(|name| serde_json::from_value(Value::String(name.to_lowercase())).ok())
        .unwrap_or_default();
    Some(Focus {
        path,
        line,
        why,
        importance,
    })
}

fn is_relative_inside(path: &str) -> bool {
    !path.is_empty()
        && Path::new(path)
            .components()
            .all(|component| matches!(component, Component::Normal(_) | Component::CurDir))
}
