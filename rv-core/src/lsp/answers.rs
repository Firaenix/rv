use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::protocol::{self, WireLocation};
use super::uri::uri_to_path;
use super::{Answer, Location, LspError, RequestKind};

/// Turns a server's result into rv positions, reading each target file once
/// to convert its UTF-16 columns. An unreadable target keeps the raw column.
pub(super) fn convert(kind: RequestKind, result: Value, root: &Path) -> Result<Answer, LspError> {
    if kind == RequestKind::Hover {
        return Ok(Answer::Hover(protocol::parse_hover(result)));
    }
    let wire_locations = protocol::parse_locations(result)
        .map_err(|error| LspError::Malformed(error.to_string()))?;
    let mut file_texts = HashMap::new();
    let locations = wire_locations
        .into_iter()
        .filter_map(|wire| to_location(wire, root, &mut file_texts))
        .collect();
    Ok(Answer::Locations(locations))
}

fn to_location(
    wire: WireLocation,
    root: &Path,
    file_texts: &mut HashMap<PathBuf, Option<String>>,
) -> Option<Location> {
    let absolute = uri_to_path(&wire.uri)?;
    let text = file_texts
        .entry(absolute.clone())
        .or_insert_with(|| std::fs::read_to_string(&absolute).ok());
    let column = text
        .as_deref()
        .and_then(|text| protocol::line_of(text, wire.line))
        .map_or(wire.character, |line| {
            protocol::utf16_to_char_column(line, wire.character)
        });
    let path = absolute
        .strip_prefix(root)
        .map_or_else(|_| absolute.clone(), Path::to_path_buf);
    Some(Location {
        path,
        line: wire.line + 1,
        column,
    })
}
