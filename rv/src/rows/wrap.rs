//! Wrapping a comment body into rows — split from [`super`] for length.

/// Breaks `text` into rows of at most `width` columns.
///
/// Wrapping is on whitespace, with a word longer than a whole row broken
/// mid-word rather than truncated: a reviewer must be able to read every
/// character of a comment, including a pasted path or identifier that fits
/// nowhere. The reviewer's own line breaks are kept, so a body written as two
/// paragraphs stays two paragraphs.
///
/// A `width` of 0 is treated as 1. A row must always take at least one
/// character or wrapping would make no progress and loop forever, which is a
/// hang rather than a visual glitch — and panes really do get squeezed to
/// nothing.
pub(super) fn wrap(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut rows = Vec::new();
    for paragraph in text.split('\n') {
        wrap_paragraph(paragraph, width, &mut rows);
    }
    rows
}

/// Wraps one newline-free paragraph onto the end of `rows`, always adding at
/// least one row so that an empty line in a body stays an empty row.
fn wrap_paragraph(paragraph: &str, width: usize, rows: &mut Vec<String>) {
    let mut row = String::new();
    let mut row_width = 0;

    for word in paragraph.split_whitespace() {
        let mut rest = word;
        loop {
            let separator = usize::from(row_width > 0);
            let rest_width = rest.chars().count();
            if row_width + separator + rest_width <= width {
                if separator == 1 {
                    row.push(' ');
                }
                row.push_str(rest);
                row_width += separator + rest_width;
                break;
            }
            if row_width > 0 {
                // Try again at the start of the next row, where the word may
                // well fit whole.
                rows.push(std::mem::take(&mut row));
                row_width = 0;
                continue;
            }
            // A row of its own is not enough for this word: take what fits and
            // carry the remainder. `width` is at least 1, so this always
            // consumes something.
            let (head, tail) = split_at_chars(rest, width);
            rows.push(head.to_owned());
            rest = tail;
        }
    }

    rows.push(row);
}

/// Splits `text` after `count` characters, or returns the whole of it and an
/// empty remainder when it is shorter. Character-wise rather than byte-wise so
/// that a multi-byte character is never cut in half.
fn split_at_chars(text: &str, count: usize) -> (&str, &str) {
    match text.char_indices().nth(count) {
        Some((offset, _)) => text.split_at(offset),
        None => (text, ""),
    }
}
