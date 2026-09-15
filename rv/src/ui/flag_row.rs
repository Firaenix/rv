//! A flag under its line: one amber row per wrapped line of the reason, no
//! border — a box means *comment*, and a flag must not read as one.

use ratatui::style::Color;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::text::Line;
use rv_core::store::Flag;

use super::GUTTER;
use super::text::clip;

/// The glyph an open flag carries on its first row.
pub(super) const FLAG: &str = "⚑ ";
/// The hollow glyph of a flag that has been looked at.
const FLAG_SEEN: &str = "⚐ ";

pub(super) fn flag_row(flag: &Flag, text: &str, first: bool, width: usize) -> Line<'static> {
    let lead = if first { FLAG } else { "  " };
    let row = format!("{}{lead}{text}", " ".repeat(GUTTER.min(width)));
    Line::styled(clip(&row, width), flag_style(flag))
}

pub(super) fn flag_collapsed(flag: &Flag, width: usize) -> Line<'static> {
    let glyph = if flag.acknowledged { FLAG_SEEN } else { FLAG };
    let first = flag.reason.lines().next().unwrap_or_default();
    let row = format!("{}{glyph}{first}", " ".repeat(GUTTER.min(width)));
    Line::styled(clip(&row, width), flag_style(flag))
}

/// Amber while a flag waits to be looked at, dim once it has been.
pub(super) fn flag_style(flag: &Flag) -> Style {
    if flag.acknowledged {
        Style::default().fg(Color::Gray).add_modifier(Modifier::DIM)
    } else {
        Style::default().fg(Color::Yellow)
    }
}
