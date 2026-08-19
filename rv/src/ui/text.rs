//! Fitting text to a column count, and the one place a colour crosses over
//! from [`crate::gradient`].

use ratatui::style::Color;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;

use crate::gradient::Rgb;

/// The marker a clipped row ends with.
///
/// A review tool that silently hides the code being judged is failing at its
/// one job: this repository contains 154-character lines, and the first real
/// session on `rv` read them in a 75-column pane with no sign anything had been
/// cut. Diff lines are **not** wrapped instead — the row model is one row per
/// diff line, and a reviewer counting lines against a file needs that
/// correspondence — so they are marked.
pub(super) const CLIPPED: char = '…';

/// One of [`crate::gradient`]'s colours, as ratatui sends it.
pub(super) fn colour(Rgb(red, green, blue): Rgb) -> Color {
    Color::Rgb(red, green, blue)
}

/// `text`, clipped to `width` columns with [`CLIPPED`] in place of the last one
/// when there was more of it.
///
/// By characters rather than by bytes: a clip that split a multi-byte character
/// would panic on the very comments this reviewer is meant to survive.
pub(super) fn clip(text: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    if text.chars().count() <= width {
        return text.to_owned();
    }
    let mut clipped: String = text.chars().take(width - 1).collect();
    clipped.push(CLIPPED);
    clipped
}

/// A styled row, clipped to `width` columns across all of its spans.
///
/// Plain truncation, with no marker: this is for the rows a box draws around
/// its own content, where a marker would be claiming a *border* had been cut
/// short. What gets marked is content — see [`clip`].
pub(super) fn clip_spans(spans: Vec<Span<'static>>, width: usize) -> Line<'static> {
    let mut kept = Vec::with_capacity(spans.len());
    let mut room = width;
    for span in spans {
        if room == 0 {
            break;
        }
        let length = span.content.chars().count();
        if length <= room {
            room -= length;
            kept.push(span);
        } else {
            let head: String = span.content.chars().take(room).collect();
            room = 0;
            kept.push(Span::styled(head, span.style));
        }
    }
    Line::from(kept)
}

/// A diff row, fitted to exactly `width` columns: padded with `ground` where
/// there was room to spare, and cut with a [`CLIPPED`] marker where there was
/// not.
///
/// Both halves matter. The padding is what makes a tinted line read as a band
/// across the pane instead of stopping wherever its text does. The marker is
/// [`clip`]'s promise kept for a row that is now several spans rather than one
/// string.
pub(super) fn clip_row(spans: Vec<Span<'static>>, width: usize, ground: Style) -> Line<'static> {
    if width == 0 {
        return Line::default();
    }
    let total: usize = spans.iter().map(|span| span.content.chars().count()).sum();
    if total <= width {
        let mut spans = spans;
        spans.push(Span::styled(" ".repeat(width - total), ground));
        return Line::from(spans);
    }

    // One column is kept back for the marker, which inherits the style of
    // whichever span it cut into so that it reads as part of the text rather
    // than as a glyph the pane added on its own.
    let mut kept: Vec<Span<'static>> = Vec::with_capacity(spans.len() + 1);
    let mut room = width - 1;
    let mut marker = ground;
    for span in spans {
        let length = span.content.chars().count();
        if length <= room {
            room -= length;
            marker = span.style;
            kept.push(span);
            continue;
        }
        let head: String = span.content.chars().take(room).collect();
        marker = span.style;
        if !head.is_empty() {
            kept.push(Span::styled(head, span.style));
        }
        break;
    }
    kept.push(Span::styled(CLIPPED.to_string(), marker));
    Line::from(kept)
}

/// `text` with its first `columns` characters scrolled off to the left,
/// leading with [`CLIPPED`] so a row that starts mid-word says so.
///
/// The marker appears even when nothing of the text survives: a blank row and
/// a row whose content is entirely off-screen are different facts, and `H` is
/// how the reviewer gets back to it.
pub(super) fn shift(text: &str, columns: usize) -> String {
    if columns == 0 {
        return text.to_owned();
    }
    if text.is_empty() {
        return String::new();
    }
    let tail: String = text.chars().skip(columns).collect();
    format!("{CLIPPED}{tail}")
}

/// Styled spans with their first `columns` characters removed, styles kept.
///
/// The diff pane's version of [`shift`]: a syntax-coloured line is many spans,
/// and scrolling it must cut through them without disturbing what each
/// surviving character was painted with.
pub(super) fn shift_spans(spans: Vec<Span<'static>>, columns: usize) -> Vec<Span<'static>> {
    let mut remaining = columns;
    let mut kept = Vec::with_capacity(spans.len());
    for span in spans {
        if remaining == 0 {
            kept.push(span);
            continue;
        }
        let length = span.content.chars().count();
        if length <= remaining {
            remaining -= length;
            continue;
        }
        let tail: String = span.content.chars().skip(remaining).collect();
        remaining = 0;
        kept.push(Span::styled(tail, span.style));
    }
    kept
}

/// The last `width` characters of `text`.
///
/// The comment bar follows what is being typed rather than showing where the
/// comment started: a `Paragraph` does not scroll, and the head of a long body
/// is the half the reviewer has already read.
pub(super) fn tail(text: &str, width: usize) -> String {
    let length = text.chars().count();
    text.chars().skip(length.saturating_sub(width)).collect()
}
