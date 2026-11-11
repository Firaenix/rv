use serde::Deserialize;
use serde_json::{Value, json};

use super::{Hover, HoverFormat};

pub const METHOD_NOT_FOUND: i64 = -32601;

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct ResponseError {
    pub code: i64,
    pub message: String,
}

#[derive(Debug, PartialEq)]
pub enum Incoming {
    Response {
        id: Value,
        outcome: Result<Value, ResponseError>,
    },
    Request {
        id: Value,
        method: String,
        params: Value,
    },
    Notification {
        method: String,
        params: Value,
    },
}

#[derive(Deserialize)]
struct RawMessage {
    id: Option<Value>,
    method: Option<String>,
    #[serde(default)]
    params: Value,
    result: Option<Value>,
    error: Option<ResponseError>,
}

pub fn parse_incoming(body: &[u8]) -> Option<Incoming> {
    let raw: RawMessage = serde_json::from_slice(body).ok()?;
    match (raw.id, raw.method) {
        (Some(id), Some(method)) => Some(Incoming::Request {
            id,
            method,
            params: raw.params,
        }),
        (None, Some(method)) => Some(Incoming::Notification {
            method,
            params: raw.params,
        }),
        (Some(id), None) => {
            let outcome = match raw.error {
                Some(error) => Err(error),
                None => Ok(raw.result.unwrap_or(Value::Null)),
            };
            Some(Incoming::Response { id, outcome })
        }
        (None, None) => None,
    }
}

pub fn request(id: i64, method: &str, params: Value) -> Vec<u8> {
    json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })
        .to_string()
        .into_bytes()
}

pub fn notification(method: &str, params: Value) -> Vec<u8> {
    json!({ "jsonrpc": "2.0", "method": method, "params": params })
        .to_string()
        .into_bytes()
}

pub fn response(id: &Value, result: Value) -> Vec<u8> {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
        .to_string()
        .into_bytes()
}

pub fn error_response(id: &Value, code: i64, message: &str) -> Vec<u8> {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
        .to_string()
        .into_bytes()
}

pub fn initialize_params(root_uri: &str, root_name: &str) -> Value {
    json!({
        "processId": std::process::id(),
        "clientInfo": { "name": "rv" },
        "rootUri": root_uri,
        "workspaceFolders": [{ "uri": root_uri, "name": root_name }],
        "capabilities": {
            "general": { "positionEncodings": ["utf-16"] },
            "window": { "workDoneProgress": true },
            "textDocument": {
                "synchronization": { "dynamicRegistration": false },
                "definition": { "linkSupport": true },
                "references": {},
                "hover": { "contentFormat": ["markdown", "plaintext"] }
            }
        }
    })
}

pub fn position_params(uri: &str, line: u32, character: u32) -> Value {
    json!({ "textDocument": { "uri": uri }, "position": { "line": line, "character": character } })
}

pub fn references_params(uri: &str, line: u32, character: u32) -> Value {
    let mut params = position_params(uri, line, character);
    params["context"] = json!({ "includeDeclaration": false });
    params
}

pub fn did_open_params(uri: &str, language_id: &str, version: i32, text: &str) -> Value {
    json!({ "textDocument": { "uri": uri, "languageId": language_id, "version": version, "text": text } })
}

pub fn did_change_params(uri: &str, version: i32, text: &str) -> Value {
    json!({ "textDocument": { "uri": uri, "version": version }, "contentChanges": [{ "text": text }] })
}

/// A position exactly as the server sent it: 0-based line, UTF-16 character.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WireLocation {
    pub uri: String,
    pub line: u32,
    pub character: u32,
}

#[derive(Deserialize)]
struct WirePosition {
    line: u32,
    character: u32,
}

#[derive(Deserialize)]
struct WireRange {
    start: WirePosition,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum LocationOrLink {
    #[serde(rename_all = "camelCase")]
    Link {
        target_uri: String,
        target_selection_range: WireRange,
    },
    Location {
        uri: String,
        range: WireRange,
    },
}

impl From<LocationOrLink> for WireLocation {
    fn from(value: LocationOrLink) -> Self {
        let (uri, range) = match value {
            LocationOrLink::Link {
                target_uri,
                target_selection_range,
            } => (target_uri, target_selection_range),
            LocationOrLink::Location { uri, range } => (uri, range),
        };
        WireLocation {
            uri,
            line: range.start.line,
            character: range.start.character,
        }
    }
}

/// Accepts `null`, one `Location`, or an array of `Location`/`LocationLink`.
pub fn parse_locations(result: Value) -> Result<Vec<WireLocation>, serde_json::Error> {
    match result {
        Value::Null => Ok(Vec::new()),
        Value::Array(items) => items
            .into_iter()
            .map(|item| serde_json::from_value::<LocationOrLink>(item).map(WireLocation::from))
            .collect(),
        single => Ok(vec![
            serde_json::from_value::<LocationOrLink>(single)?.into(),
        ]),
    }
}

/// Accepts `MarkupContent`, a `MarkedString`, or an array of `MarkedString`.
pub fn parse_hover(result: Value) -> Option<Hover> {
    let contents = result.get("contents")?;
    let hover = match contents {
        Value::Object(map) if map.contains_key("kind") => Hover {
            format: if map.get("kind")?.as_str()? == "markdown" {
                HoverFormat::Markdown
            } else {
                HoverFormat::PlainText
            },
            text: map.get("value")?.as_str()?.to_owned(),
        },
        Value::Array(items) => Hover {
            format: HoverFormat::Markdown,
            text: items
                .iter()
                .filter_map(marked_string)
                .collect::<Vec<_>>()
                .join("\n\n"),
        },
        other => Hover {
            format: HoverFormat::Markdown,
            text: marked_string(other)?,
        },
    };
    (!hover.text.trim().is_empty()).then_some(hover)
}

fn marked_string(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Object(map) => {
            let language = map.get("language")?.as_str()?;
            let code = map.get("value")?.as_str()?;
            Some(format!("```{language}\n{code}\n```"))
        }
        _ => None,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProgressUpdate {
    Begin {
        title: String,
        percentage: Option<u32>,
    },
    Report {
        percentage: Option<u32>,
    },
    End,
}

/// Returns the progress token (as its JSON text, since tokens may be numbers
/// or strings) and what happened to it.
pub fn parse_progress(params: &Value) -> Option<(String, ProgressUpdate)> {
    let token = params.get("token")?.to_string();
    let value = params.get("value")?;
    let percentage = value
        .get("percentage")
        .and_then(Value::as_u64)
        .map(|percent| percent.min(100) as u32);
    let update = match value.get("kind")?.as_str()? {
        "begin" => ProgressUpdate::Begin {
            title: value
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            percentage,
        },
        "report" => ProgressUpdate::Report { percentage },
        "end" => ProgressUpdate::End,
        _ => return None,
    };
    Some((token, update))
}

/// Columns past the end of the line count one unit per missing character, so
/// the conversion stays invertible for a cursor beyond the text.
pub fn char_column_to_utf16(line_text: &str, char_column: u32) -> u32 {
    let mut units = 0u32;
    let mut chars = line_text.chars();
    for _ in 0..char_column {
        units += chars.next().map_or(1, |c| c.len_utf16() as u32);
    }
    units
}

/// A UTF-16 offset that lands inside a surrogate pair maps to that character.
pub fn utf16_to_char_column(line_text: &str, utf16_column: u32) -> u32 {
    let mut units = 0u32;
    let mut column = 0u32;
    for c in line_text.chars() {
        if units >= utf16_column {
            return column;
        }
        units += c.len_utf16() as u32;
        if units > utf16_column {
            return column;
        }
        column += 1;
    }
    column + utf16_column.saturating_sub(units)
}

/// Line `index` (0-based) of `text` without its line terminator.
pub fn line_of(text: &str, index: u32) -> Option<&str> {
    text.split('\n')
        .nth(index as usize)
        .map(|line| line.strip_suffix('\r').unwrap_or(line))
}
