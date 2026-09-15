//! Marks laid over a code row after it is drawn: the `/` query's matches, and
//! the column cursor's word on the selected line.
//!
//! Applied by column on the finished row rather than woven into the syntax
//! spans, so the code row stays one function of the line and neither mark
//! has to know how a row is scrolled or clipped — only where its columns
//! landed.

use std::ops::Range;

use ratatui::style::Color;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
use rv_core::diff::DiffLine;

use crate::app::App;
use crate::app::Focus;

/// The columns of the gutter — the line number and the sigil — that a code
/// row's text starts after.
const GUTTER_COLUMNS: usize = 7;

pub(super) struct Mark {
    pub(super) chars: Range<usize>,
    pub(super) style: Style,
}

/// A query match: underlined, so it reads through any wash and any syntax
/// colour without claiming either channel.
fn match_style() -> Style {
    Style::default().add_modifier(Modifier::UNDERLINED)
}

/// The column cursor's word: a grey ground under it, the one background in
/// the pane that is not a line wash, so it reads as a cursor and not as a
/// second selected line.
fn cursor_style() -> Style {
    Style::default()
        .bg(Color::DarkGray)
        .add_modifier(Modifier::BOLD)
}

/// What to lay over diff line `index`: every query match, and — on the
/// selected line while the diff pane has the focus — the word under the
/// column cursor.
pub(super) fn marks(app: &App, index: usize, line: &DiffLine) -> Vec<Mark> {
    let mut marks: Vec<Mark> = app
        .matches_in(&line.text)
        .into_iter()
        .map(|chars| Mark {
            chars,
            style: match_style(),
        })
        .collect();
    if index == app.line_index()
        && app.focus() == Focus::Diff
        && let Some(chars) = app.word_range()
    {
        marks.push(Mark {
            chars,
            style: cursor_style(),
        });
    }
    marks
}

/// `row` with each mark's modifier added over the columns its characters
/// were drawn in, scrolled `hscroll` characters leftwards like the text was.
pub(super) fn emphasized(row: Line<'static>, marks: &[Mark], hscroll: usize) -> Line<'static> {
    if marks.is_empty() {
        return row;
    }
    // A scrolled row leads its text with the one-column `CLIPPED` marker.
    let text_start = if hscroll > 0 {
        GUTTER_COLUMNS + 1
    } else {
        GUTTER_COLUMNS
    };
    let mut line = row;
    for mark in marks {
        let start = mark.chars.start.saturating_sub(hscroll) + text_start;
        let end = mark.chars.end.saturating_sub(hscroll) + text_start;
        if mark.chars.end <= hscroll || end <= start {
            continue;
        }
        line = apply(line, start..end, mark.style);
    }
    line
}

/// The line with `patch` laid over every character in `columns`.
fn apply(line: Line<'static>, columns: Range<usize>, patch: Style) -> Line<'static> {
    let mut out: Vec<Span<'static>> = Vec::with_capacity(line.spans.len() + 2);
    let style = line.style;
    let alignment = line.alignment;
    let mut at = 0;
    for span in line.spans {
        let length = span.content.chars().count();
        let span_range = at..at + length;
        at += length;
        if span_range.end <= columns.start || span_range.start >= columns.end {
            out.push(span);
            continue;
        }
        let marked_from = columns.start.max(span_range.start) - span_range.start;
        let marked_to = columns.end.min(span_range.end) - span_range.start;
        let chars: Vec<char> = span.content.chars().collect();
        let piece = |from: usize, to: usize| chars[from..to].iter().collect::<String>();
        if marked_from > 0 {
            out.push(Span::styled(piece(0, marked_from), span.style));
        }
        out.push(Span::styled(
            piece(marked_from, marked_to),
            span.style.patch(patch),
        ));
        if marked_to < length {
            out.push(Span::styled(piece(marked_to, length), span.style));
        }
    }
    let mut line = Line::from(out).style(style);
    line.alignment = alignment;
    line
}
