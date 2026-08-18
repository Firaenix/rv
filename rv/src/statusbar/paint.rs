//! Painting the segments: their glyphs, their grounds and what is dropped when
//! the row is too narrow.
//!
//! Dropping is whole-segment and by rank — see [`Role::rank`](super::Role) —
//! because half a fact is worse than none: `deleted comment at app.r` is a
//! claim about a file that does not exist.

use std::cmp::Reverse;
use std::ffi::OsStr;

use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;

use crate::gradient;
use crate::gradient::Rgb;

use super::ARROW;
use super::ARROW_LEFT;
use super::PIPE;
use super::RV_ASCII;
use super::Segment;
use super::colour;
use super::columns;
use super::neutral;
use super::printable;

/// The bar, exactly `width` columns wide.
///
/// Segments keep their given order; [`Role::Hint`] is drawn at the right-hand
/// end and everything else in a run from the left, with the space between them
/// filled. When the segments do not fit, the least important is dropped —
/// whole, never truncated — and the question asked again, until what is left
/// fits or nothing is left. See [`Role::rank`] for the order, and the module
/// docs for why the hint outlives the mode.
///
/// `ascii` turns the powerline glyphs off; see [`ascii_from_env`].
#[must_use]
pub fn render(segments: &[Segment], width: u16, ascii: bool) -> Line<'static> {
    let width = usize::from(width);
    let mut kept: Vec<usize> = (0..segments.len())
        .filter(|index| !segments[*index].text.is_empty())
        .collect();

    // Each drop removes at least two columns, so this cannot spin: `kept`
    // shrinks every time round and an empty bar measures zero.
    while measure(segments, &kept, ascii) > width {
        let Some(position) = kept
            .iter()
            .enumerate()
            .min_by_key(|(_, index)| (segments[**index].role.rank(), Reverse(**index)))
            .map(|(position, _)| position)
        else {
            break;
        };
        kept.remove(position);
    }

    let (trailing, leading): (Vec<usize>, Vec<usize>) = kept
        .iter()
        .partition(|index| segments[**index].role.trailing());

    let mut spans = Vec::with_capacity(kept.len() * 2 + 1);
    for (position, index) in leading.iter().enumerate() {
        let segment = &segments[*index];
        spans.push(block(segment));
        let next = leading
            .get(position + 1)
            .map_or_else(fill, |index| segments[*index].role.background());
        match (ascii, position + 1 == leading.len()) {
            // The run ends: in powerline the chevron is what carries the colour
            // across into the fill, and with the glyphs off the fill's own
            // ground is the boundary.
            (false, _) => spans.push(separator(ARROW, segment.role.background(), next)),
            (true, false) => spans.push(separator(PIPE, gradient::readable_on(next), next)),
            (true, true) => {}
        }
    }

    let padding = width.saturating_sub(measure(segments, &kept, ascii));
    if padding > 0 {
        spans.push(Span::styled(
            " ".repeat(padding),
            Style::new().bg(colour(fill())),
        ));
    }

    for (position, index) in trailing.iter().enumerate() {
        let segment = &segments[*index];
        let previous = position
            .checked_sub(1)
            .map_or_else(fill, |before| segments[trailing[before]].role.background());
        let background = segment.role.background();
        if ascii {
            if position > 0 {
                spans.push(separator(
                    PIPE,
                    gradient::readable_on(background),
                    background,
                ));
            }
        } else {
            spans.push(separator(ARROW_LEFT, background, previous));
        }
        spans.push(block(segment));
    }

    Line::from(spans)
}

/// Whether the powerline glyphs are turned off, from the process environment.
///
/// Call this **once**, at startup, and carry the answer: the variable cannot
/// change while rv runs, and a lookup per frame is a syscall per keystroke.
#[must_use]
pub fn ascii_from_env() -> bool {
    ascii_from(std::env::var_os(RV_ASCII).as_deref())
}

/// Whether the value of [`RV_ASCII`] turns the glyphs off.
///
/// Presence is the switch, exactly as it is for `RV_NO_DIFFT`: `RV_ASCII=0`
/// turns the glyphs *off* like any other value, because a reviewer who has
/// learned one escape hatch in this tool has learned both, and a variable that
/// meant one thing here and another there is worse than a variable with a blunt
/// rule. Split from [`ascii_from_env`] so it can be tested without mutating the
/// environment of a threaded test binary.
#[must_use]
pub fn ascii_from(value: Option<&OsStr>) -> bool {
    value.is_some()
}

/// How many columns the kept segments need, separators and padding spaces
/// included and the fill excluded.
///
/// Measured with the same call ratatui uses when it paints, so the arithmetic
/// that decides what to drop and the arithmetic that decides where a cell goes
/// cannot disagree about a wide character.
fn measure(segments: &[Segment], kept: &[usize], ascii: bool) -> usize {
    let (trailing, leading): (Vec<usize>, Vec<usize>) = kept
        .iter()
        .partition(|index| segments[**index].role.trailing());
    [leading, trailing]
        .iter()
        .map(|run| {
            if run.is_empty() {
                return 0;
            }
            let text: usize = run
                .iter()
                .map(|index| columns(&segments[*index].text) + 2)
                .sum();
            // One separator between each pair, plus — with the glyphs on — the
            // chevron that caps the run against the fill.
            let separators = run.len() - usize::from(ascii);
            text + separators
        })
        .sum()
}

/// One segment, padded with a space on each side so the chevrons do not sit
/// against the text.
fn block(segment: &Segment) -> Span<'static> {
    let background = segment.role.background();
    Span::styled(
        format!(" {} ", printable(&segment.text)),
        Style::new()
            .fg(colour(gradient::readable_on(background)))
            .bg(colour(background))
            .add_modifier(segment.role.modifier()),
    )
}

/// A separator between two blocks: `ink` is the colour it is drawn in and
/// `ground` what it is drawn on.
///
/// A powerline chevron is the *previous* block's colour drawn on the next
/// one's, which is what makes the boundary read as one block overlapping the
/// other rather than as two blocks with a glyph between them. A pipe is
/// ordinary text and takes the ink of the ground it sits on.
fn separator(glyph: &'static str, ink: Rgb, ground: Rgb) -> Span<'static> {
    Span::styled(glyph, Style::new().fg(colour(ink)).bg(colour(ground)))
}

/// The ground the bar's empty middle is painted on: dark enough to read as the
/// bar's own background rather than as another segment, and painted rather than
/// left bare so the row is one bar instead of two blocks with the pane showing
/// through between them.
pub fn fill() -> Rgb {
    neutral(0.86)
}
