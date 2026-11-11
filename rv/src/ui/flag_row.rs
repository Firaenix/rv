//! A flag under its line: one amber row per wrapped line of the reason, no
//! border — a box means *comment*, and a flag must not read as one.

use ratatui::style::Color;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::text::Line;
use rv_core::store::Flag;
use rv_core::store::FlaggedBy;

use super::GUTTER;
use super::text::clip;
use crate::app::App;

/// The glyph an open flag carries on its first row.
pub(super) const FLAG: &str = "⚑ ";
/// The hollow glyph of a flag that has been looked at.
const FLAG_SEEN: &str = "⚐ ";
/// The same pair for a flag the agent filed, so its suggestions never read as
/// the reviewer's own.
pub(super) const AGENT_FLAG: &str = "✦ ";
const AGENT_FLAG_SEEN: &str = "✧ ";

pub(super) fn glyph(flag: &Flag) -> &'static str {
    match (flag.by, flag.acknowledged) {
        (FlaggedBy::User, false) => FLAG,
        (FlaggedBy::User, true) => FLAG_SEEN,
        (FlaggedBy::Agent, false) => AGENT_FLAG,
        (FlaggedBy::Agent, true) => AGENT_FLAG_SEEN,
    }
}

pub(super) fn flag_row(
    app: &App,
    flag: &Flag,
    text: &str,
    first: bool,
    width: usize,
) -> Line<'static> {
    let lead = if first { glyph_open(flag) } else { "  " };
    let row = format!("{}{lead}{text}", " ".repeat(GUTTER.min(width)));
    Line::styled(clip(&row, width), row_style(app, flag))
}

pub(super) fn flag_collapsed(app: &App, flag: &Flag, width: usize) -> Line<'static> {
    let first = flag.reason.lines().next().unwrap_or_default();
    let row = format!("{}{}{first}", " ".repeat(GUTTER.min(width)), glyph(flag));
    Line::styled(clip(&row, width), row_style(app, flag))
}

fn glyph_open(flag: &Flag) -> &'static str {
    match flag.by {
        FlaggedBy::User => FLAG,
        FlaggedBy::Agent => AGENT_FLAG,
    }
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
