//! The agent's corner: one line on the diff pane's bottom border — the shown
//! file's purpose, its open agent flags, and the agent keys. The purpose is
//! what gives way when the border is short; the count and the keys stay.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;

use super::flag_row::AGENT_FLAG;
use super::text::clip;
use crate::app::CornerTip;

const SEPARATOR: &str = " · ";
/// Narrower than this, a purpose says nothing and is dropped whole.
const SHORTEST_PURPOSE: usize = 8;

/// How many columns `tip` wants, padding included.
#[must_use]
pub fn corner_columns(tip: &CornerTip) -> u16 {
    u16::try_from(corner_text(tip, usize::MAX).chars().count()).unwrap_or(u16::MAX)
}

/// The corner's text in at most `width` columns.
#[must_use]
pub fn corner_text(tip: &CornerTip, width: usize) -> String {
    let mut fixed = vec![format!("{AGENT_FLAG}{} flagged", tip.flagged)];
    fixed.extend(tip.hints.iter().cloned());
    let fixed = fixed.join(SEPARATOR);
    let padding = 2;
    let fixed_width = fixed.chars().count() + padding;
    let room = width
        .saturating_sub(fixed_width)
        .saturating_sub(SEPARATOR.chars().count());
    let text = if tip.purpose.is_empty() || room < SHORTEST_PURPOSE {
        fixed
    } else {
        format!("{}{SEPARATOR}{fixed}", clip(&tip.purpose, room))
    };
    clip(&format!(" {text} "), width)
}

pub(super) fn draw_corner(frame: &mut Frame, tip: &CornerTip, area: Rect) {
    let text = corner_text(tip, usize::from(area.width));
    frame.render_widget(
        Line::from(Span::styled(
            text,
            Style::default().add_modifier(Modifier::DIM),
        )),
        area,
    );
}
