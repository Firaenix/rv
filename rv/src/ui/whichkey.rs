//! The which-key popup: after a leader is pressed, the keys its submenu answers.
//!
//! Drawn from [`BINDINGS`] like the `?` keymap, so a child of a leader cannot be
//! offered here without being dispatchable, nor dispatched without appearing.
//! It lists only the children live from where the cursor is; a dimmed row would
//! teach that a key is broken rather than that the menu moved on.

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
use crate::app::App;
use crate::app::Leader;
use crate::app::keymap::RuntimeBinding;

const GAP: usize = 2;

/// Draws the submenu for whichever leader is pending, in the bottom-right corner
/// above the bar — where the `?` tip sits, for the same reason: it is the answer
/// to "what next", and the bar's `? help` hint is right beneath it.
pub(super) fn draw(frame: &mut Frame, app: &App, area: Rect, bar: Rect) {
    let Some(leader) = app.pending_leader() else {
        return;
    };
    let entries = live_children(app, leader);
    if entries.is_empty() {
        return;
    }

    let keys = entries
        .iter()
        .map(|binding| binding.keys_label.chars().count())
        .max()
        .unwrap_or(0);
    let title = format!("{} — {}", leader.label(), leader.title());
    let inner = entries
        .iter()
        .map(|binding| keys + GAP + binding.what.chars().count())
        .max()
        .unwrap_or(0)
        .max(title.chars().count());

    let width = u16::try_from(inner)
        .unwrap_or(u16::MAX)
        .saturating_add(BORDER_ROWS + 2);
    let rows = u16::try_from(entries.len())
        .unwrap_or(u16::MAX)
        .saturating_add(BORDER_ROWS);
    let rect = corner(area, bar, rows, width);

    let lines: Vec<Line<'static>> = entries
        .iter()
        .map(|binding| {
            Line::from(vec![
                Span::raw(" "),
                Span::styled(
                    format!("{:<keys$}", binding.keys_label),
                    Style::default().add_modifier(Modifier::BOLD),
                ),
                Span::raw(" ".repeat(GAP)),
                Span::raw(binding.what.to_owned()),
            ])
        })
        .collect();

    frame.render_widget(Clear, rect);
    frame.render_widget(
        Paragraph::new(lines).block(
            Block::bordered()
                .border_type(BorderType::Rounded)
                .title(title),
        ),
        rect,
    );
}

/// The leader's children that would do something from where the cursor is now.
/// The contextual (`Space`) menu also drops children that do not belong to the
/// current mode, so it shows only that mode's own actions.
fn live_children(app: &App, leader: Leader) -> Vec<&RuntimeBinding> {
    let context = app.context();
    app.keymap()
        .bindings()
        .iter()
        .filter(|binding| {
            binding.leader == Some(leader)
                && (binding.contexts.is_empty() || binding.contexts.contains(&context))
                && app.rt_binding_enabled(binding)
        })
        .collect()
}

/// The bottom-right corner rect, its lower edge on the bar, clamped to the area.
fn corner(area: Rect, bar: Rect, rows: u16, columns: u16) -> Rect {
    let width = columns.min(area.width);
    let height = rows.min(bar.y.saturating_sub(area.y));
    Rect::new(
        area.right().saturating_sub(width),
        bar.y.saturating_sub(height),
        width,
        height,
    )
}
