//! The row model: a diff and its comments, flattened into the rows a terminal
//! actually draws.
//!
//! A diff line is one row, but a comment is a box of several — a border, one
//! row per wrapped line of its body, another border — so "the third diff line"
//! and "the third row on screen" stop being the same thing the moment a
//! comment exists. Everything that has to reconcile the two lives here:
//! [`plan`] builds the flat list, [`Plan::row_of_line`] and
//! [`Plan::row_of_comment`] map a cursor onto it, and [`window`] decides which
//! slice of it fits the pane.
//!
//! Nothing in this module knows about ratatui. It borrows the diff and the
//! comments rather than copying them, so a plan is cheap enough to rebuild
//! every frame — which is what lets the drawing code stay a pure function of
//! the app's state.

use std::collections::HashSet;
use std::ops::Range;

use rv_core::diff::DiffLine;
use rv_core::diff::FileDiff;
use rv_core::store::Comment;

/// What a reply is labelled with inside its comment's box. A reply is part of
/// the same conversation as the body it answers, so it shares the box rather
/// than opening a second one.
const REPLY_PREFIX: &str = "reply: ";

/// One drawable row.
///
/// `line` on the box variants is the index of the diff line the box hangs
/// under, not a file line number: it is what [`Plan::row_of_comment`] is asked
/// about and what the cursor is expressed in.
#[derive(Clone, Debug, PartialEq)]
pub enum Row<'a> {
    /// A line of the diff. `index` is its position in [`FileDiff::lines`].
    Diff { index: usize, line: &'a DiffLine },
    /// The top border of an expanded comment box.
    BoxTop { line: usize, comment: &'a Comment },
    /// One wrapped line of a comment's body or reply. `text` is the row's
    /// content with no border or indent: the drawing code owns the frame it
    /// sits in.
    BoxBody {
        line: usize,
        comment: &'a Comment,
        text: String,
    },
    /// The bottom border of an expanded comment box.
    BoxBottom { line: usize, comment: &'a Comment },
    /// A collapsed comment box, which occupies exactly one row. It is still a
    /// row of its own so that a collapsed comment can be selected, deleted and
    /// expanded again like any other.
    BoxCollapsed { line: usize, comment: &'a Comment },
}

/// Every row of a file's diff, in the order they are drawn.
#[derive(Clone, Debug, PartialEq)]
pub struct Plan<'a> {
    pub rows: Vec<Row<'a>>,
}

/// Flattens `diff` and its comments into rows.
///
/// `comments_for` is asked for each diff line's comments in the order they
/// should stack, oldest first — a closure rather than a borrowed map so the
/// caller keeps ownership of how a comment is matched to a line, which is the
/// one rule this module must not get a second opinion on.
///
/// `collapsed` holds the ids of comments the reviewer has folded away; each of
/// those costs one row instead of a box. `width` is how many columns a body
/// row may occupy — the caller subtracts whatever border and indent it draws
/// around the text before passing it in.
pub fn plan<'a>(
    diff: &'a FileDiff,
    comments_for: &dyn Fn(usize) -> Vec<&'a Comment>,
    collapsed: &HashSet<String>,
    width: usize,
) -> Plan<'a> {
    let mut rows = Vec::with_capacity(diff.lines.len());
    for (index, line) in diff.lines.iter().enumerate() {
        rows.push(Row::Diff { index, line });
        for comment in comments_for(index) {
            if collapsed.contains(&comment.id) {
                rows.push(Row::BoxCollapsed {
                    line: index,
                    comment,
                });
                continue;
            }
            rows.push(Row::BoxTop {
                line: index,
                comment,
            });
            for text in wrap(&comment.body, width) {
                rows.push(Row::BoxBody {
                    line: index,
                    comment,
                    text,
                });
            }
            if let Some(reply) = comment.reply.as_deref() {
                for text in wrap(&format!("{REPLY_PREFIX}{reply}"), width) {
                    rows.push(Row::BoxBody {
                        line: index,
                        comment,
                        text,
                    });
                }
            }
            rows.push(Row::BoxBottom {
                line: index,
                comment,
            });
        }
    }
    Plan { rows }
}

impl Plan<'_> {
    /// The row holding diff line `line`, or `None` when the diff has no such
    /// line.
    ///
    /// Linear, like [`Plan::row_of_comment`]: a plan is rebuilt every frame and
    /// a reviewed file is thousands of rows at the very outside, so an index
    /// would cost more to keep correct than the scan costs to run.
    pub fn row_of_line(&self, line: usize) -> Option<usize> {
        self.rows
            .iter()
            .position(|row| matches!(row, Row::Diff { index, .. } if *index == line))
    }

    /// The row where the `comment_index`-th box under diff line `line` starts
    /// — its top border, or the single row of a collapsed box — or `None` when
    /// that line has no such box.
    pub fn row_of_comment(&self, line: usize, comment_index: usize) -> Option<usize> {
        self.rows
            .iter()
            .enumerate()
            .filter(|(_, row)| match row {
                Row::BoxTop { line: at, .. } | Row::BoxCollapsed { line: at, .. } => *at == line,
                _ => false,
            })
            .nth(comment_index)
            .map(|(row, _)| row)
    }
}

/// The half-open range of rows to draw: `height` of them where there are that
/// many, centered on `anchor` as far as the ends of the list allow.
///
/// The anchor is always inside the returned range when there is anything to
/// return, so the cursor can never scroll off the pane it is meant to be
/// steering.
pub fn window(rows: usize, anchor: usize, height: usize) -> Range<usize> {
    if rows == 0 || height == 0 {
        return 0..0;
    }
    if rows <= height {
        return 0..rows;
    }
    let start = anchor.saturating_sub(height / 2).min(rows - height);
    start..start + height
}

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
fn wrap(text: &str, width: usize) -> Vec<String> {
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
