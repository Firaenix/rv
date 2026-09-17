//! A flag under its line: one amber row per wrapped line of the reason, no
//! border — a box means *comment*, and a flag must not read as one.

use ratatui::style::Color;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::text::Line;
use rv_core::store::Flag;

use super::GUTTER;
use super::text::clip;
use crate::app::App;

/// The glyph an open flag carries on its first row.
pub(super) const FLAG: &str = "⚑ ";
/// The hollow glyph of a flag that has been looked at.
const FLAG_SEEN: &str = "⚐ ";

pub(super) fn flag_row(
    app: &App,
    flag: &Flag,
    text: &str,
    first: bool,
    width: usize,
) -> Line<'static> {
    let lead = if first { FLAG } else { "  " };
    let row = format!("{}{lead}{text}", " ".repeat(GUTTER.min(width)));
    Line::styled(clip(&row, width), row_style(app, flag))
}

pub(super) fn flag_collapsed(app: &App, flag: &Flag, width: usize) -> Line<'static> {
    let glyph = if flag.acknowledged { FLAG_SEEN } else { FLAG };
    let first = flag.reason.lines().next().unwrap_or_default();
    let row = format!("{}{glyph}{first}", " ".repeat(GUTTER.min(width)));
    Line::styled(clip(&row, width), row_style(app, flag))
}

/// Amber while a flag waits to be looked at, dim once it has been.
pub(super) fn flag_style(flag: &Flag) -> Style {
    if flag.acknowledged {
        Style::default().fg(Color::Gray).add_modifier(Modifier::DIM)
    } else {
        Style::default().fg(Color::Yellow)
    }
}

/// The same, plus the selection: the flag the cursor is on is brighter and
/// bold, so `D` and `A` visibly have a target — the stack's rule for a box.
fn row_style(app: &App, flag: &Flag) -> Style {
    let selected = app
        .selected_flag()
        .is_some_and(|cursor| cursor.id == flag.id);
    if selected {
        Style::default()
            .fg(Color::LightYellow)
            .add_modifier(Modifier::BOLD)
    } else {
        flag_style(flag)
    }
}
