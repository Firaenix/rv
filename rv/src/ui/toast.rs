//! The floating panel: what has gone wrong, in the theme's yellow, over the
//! panes.

use std::time::Instant;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::widgets::Block;
use ratatui::widgets::BorderType;
use ratatui::widgets::Clear;
use ratatui::widgets::Paragraph;

use super::BORDER_ROWS;
use super::text::clip;
use crate::app::Alert;
use crate::theme;

/// The mark an alert leads with, so the panel says what it is before it is
/// read: a warning, not a status.
const ALERT_MARK: char = '⚠';

/// What sits between two alerts sharing the panel.
const ALERT_SEPARATOR: &str = " · ";

/// **One panel, however many alerts.** [`crate::layout::layout`] gives the
/// toast three rows and no rectangle in this reviewer is computed anywhere but
/// there, so several alerts share the row rather than stacking down the screen.
/// What matters is that none is lost.
///
/// It is **not** a click target: [`crate::layout`] has no `Target` for it on
/// purpose, because a toast that could be clicked would be a dialog, and a
/// dialog is something a reviewer has to answer.
///
/// The fade is one step: the panel dims for the back half of its life, which is
/// what a fade can be in an indexed colour — the yellow itself belongs to the
/// theme, and spec §9's "disappear without fading" is now simply what every
/// terminal gets.
pub(super) fn draw_toast(frame: &mut Frame, alerts: &[&Alert], area: Rect, now: Instant) {
    if alerts.is_empty() {
        return;
    }
    // The freshest alert decides the fade: they share one border, and dimming
    // it because an older message is nearly done would fade out a warning that
    // has just arrived.
    let fade = alerts
        .iter()
        .map(|alert| alert.fade(now))
        .fold(1.0_f32, f32::min);
    let mut style = Style::default().fg(theme::ALERT);
    if fade >= 0.5 {
        style = style.add_modifier(Modifier::DIM);
    }

    let width = usize::from(area.width.saturating_sub(BORDER_ROWS));
    let messages: Vec<&str> = alerts.iter().map(|alert| alert.message.as_str()).collect();
    let text = format!("{ALERT_MARK} {}", messages.join(ALERT_SEPARATOR));

    // Over whatever the panes drew there, rather than blended with it: a
    // warning read through a diff is a warning read twice.
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(Line::styled(clip(&text, width), style)).block(
            Block::bordered()
                .border_type(BorderType::Rounded)
                .border_style(style),
        ),
        area,
    );
}
