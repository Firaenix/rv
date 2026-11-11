//! A file shown against itself: what a tree review draws instead of a diff.

use super::binary;
use super::decode;
use super::fallback::split_lines;
use super::index_to_line;
use super::is_binary;
use super::model::DiffLine;
use super::model::DiffSource;
use super::model::FileDiff;
use super::model::LineKind;

/// Every line of `text` as context, numbered the same on both sides, so an
/// anchor on any line is a head-side anchor like one on a range review's
/// context line. No diff engine runs.
pub fn whole_file(path: &str, text: &[u8]) -> FileDiff {
    if is_binary(Some(text)) {
        return binary(path);
    }
    let decoded = decode(text);
    let lines = split_lines(&decoded)
        .into_iter()
        .enumerate()
        .map(|(index, line)| {
            let number = index_to_line(Some(index));
            DiffLine {
                kind: LineKind::Context,
                left: number,
                right: number,
                text: line.to_owned(),
            }
        })
        .collect();
    FileDiff {
        path: path.to_owned(),
        lines,
        source: DiffSource::WholeFile,
        suppressed: false,
    }
}
