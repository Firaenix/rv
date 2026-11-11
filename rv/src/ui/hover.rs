//! The hover card: a symbol's kind and name, where it is declared, its first
//! lines, how often it is used, and the keys that go on from here.
//!
//! Drawn from the card [`App`] cached when it was asked for; nothing here
//! reads the index or the plan.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::widgets::Block;
use ratatui::widgets::BorderType;
use ratatui::widgets::Clear;
use ratatui::widgets::Paragraph;

use super::BORDER_ROWS;
use super::text::clip;
use crate::app::App;
use crate::app::HoverCard;
use crate::theme;

pub(super) fn draw_hover(frame: &mut Frame, app: &App, area: Rect) {
    let Some(card) = app.hover() else {
        return;
    };
    let width = usize::from(area.width.saturating_sub(BORDER_ROWS + 2));
    let rows = usize::from(area.height.saturating_sub(BORDER_ROWS));
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(lines(card, width, rows)).block(
            Block::bordered()
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(theme::FOCUS))
                .padding(ratatui::widgets::Padding::horizontal(1)),
        ),
        area,
    );
}

/// The heading, the declaration, the usage and the hints — the hints kept
/// when rows run out, since they are how the reviewer acts on the card.
fn lines(card: &HoverCard, width: usize, rows: usize) -> Vec<Line<'static>> {
    let dim = Style::default().add_modifier(Modifier::DIM);
    let mut out = vec![heading(card, width)];
    out.extend(
        card.body()
            .iter()
            .map(|line| Line::raw(clip(&format!("  {line}"), width))),
    );
    out.push(Line::styled(clip(&card.usage(), width), dim));
    out.truncate(rows.saturating_sub(1));
    out.push(Line::styled(clip(&card.hints, width), dim));
    out
}

/// `kind name` on the left and `path:line` flush right, the name winning the
/// row when both do not fit.
fn heading(card: &HoverCard, width: usize) -> Line<'static> {
    let name = clip(&card.heading(), width);
    let used = name.chars().count();
    let location = card
        .location()
        .filter(|location| used + 2 + location.chars().count() <= width)
        .unwrap_or_default();
    let gap = width.saturating_sub(used + location.chars().count());
    Line::from(vec![
        Span::styled(name, Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(" ".repeat(if location.is_empty() { 0 } else { gap })),
        Span::styled(location, Style::default().fg(theme::HASH)),
    ])
}
