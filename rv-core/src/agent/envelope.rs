//! The `--output-format json` wrapper Claude Code and maki print around a
//! reply. Its absence is not an error: plain stdout is the reply.

use serde_json::Value;

pub(super) struct Envelope {
    pub result: String,
    pub session_id: Option<String>,
    pub is_error: bool,
    pub duration_ms: Option<u64>,
    pub cost_usd: Option<f64>,
}

const MAX_SESSION_ID_CHARS: usize = 128;

/// The whole stdout as one object, else the last line that is one — the
/// final `result` event of `stream-json`.
pub(super) fn read(stdout: &str) -> Option<Envelope> {
    let trimmed = stdout.trim();
    std::iter::once(trimmed)
        .chain(trimmed.lines().rev())
        .find_map(|candidate| envelope(candidate.trim()))
}

fn envelope(candidate: &str) -> Option<Envelope> {
    if !candidate.starts_with('{') {
        return None;
    }
    let value: Value = serde_json::from_str(candidate).ok()?;
    let object = value.as_object()?;
    let is_result = object.get("type").and_then(Value::as_str) == Some("result")
        || object.contains_key("result")
        || object.contains_key("is_error");
    if !is_result {
        return None;
    }
    Some(Envelope {
        result: object
            .get("result")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        session_id: object
            .get("session_id")
            .and_then(Value::as_str)
            .filter(|id| is_safe_session_id(id))
            .map(str::to_owned),
        is_error: object
            .get("is_error")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        duration_ms: object.get("duration_ms").and_then(Value::as_u64),
        cost_usd: object
            .get("total_cost_usd")
            .and_then(Value::as_f64)
            .filter(|cost| cost.is_finite() && *cost >= 0.0),
    })
}

/// The id goes back on a command line after the resume flag, so one that
/// could read as another flag, or carries anything but an id's alphabet, is
/// dropped rather than passed on.
pub(crate) fn is_safe_session_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_SESSION_ID_CHARS
        && !id.starts_with('-')
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ':'))
}
