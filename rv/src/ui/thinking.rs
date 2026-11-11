//! The tooltip in the diff pane's top-right corner while an ask runs: a
//! spinner, the agent, what it was asked about, how long it has taken, and
//! the key that stops it.

use std::time::Duration;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::widgets::Block;
use ratatui::widgets::BorderType;
use ratatui::widgets::Clear;
use ratatui::widgets::Padding;
use ratatui::widgets::Paragraph;

use super::text::clip;
use crate::app::Thinking;
use crate::theme;

const SPINNER: [char; 10] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
const SPINNER_STEP: Duration = Duration::from_millis(100);
const SEPARATOR: &str = " · ";
/// The border and one column of padding on each side.
const FRAME_COLUMNS: u16 = 4;
/// Past this the subject is clipped, so the tooltip stays a tooltip.
const LONGEST_SUBJECT: usize = 40;

/// How many columns the tooltip wants, frame included.
#[must_use]
pub fn thinking_columns(thinking: &Thinking) -> u16 {
    let text: usize = parts(thinking)
        .iter()
        .map(|(text, _)| text.chars().count())
        .sum();
    u16::try_from(text)
        .unwrap_or(u16::MAX)
        .saturating_add(FRAME_COLUMNS)
}

pub(super) fn draw_thinking(frame: &mut Frame, thinking: &Thinking, area: Rect) {
    let room = usize::from(area.width.saturating_sub(FRAME_COLUMNS));
    let mut used = 0;
    let spans: Vec<Span<'static>> = parts(thinking)
        .into_iter()
        .map(|(text, style)| {
            let text = clip(&text, room.saturating_sub(used));
            used += text.chars().count();
            Span::styled(text, style)
        })
        .collect();
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(Line::from(spans)).block(
            Block::bordered()
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(theme::FOCUS))
                .padding(Padding::horizontal(1)),
        ),
        area,
    );
}

fn parts(thinking: &Thinking) -> Vec<(String, Style)> {
    let dim = Style::default().add_modifier(Modifier::DIM);
    let frame_index = (thinking.elapsed.as_millis() / SPINNER_STEP.as_millis()) as usize;
    let mut parts = vec![
        (
            format!("{} ", SPINNER[frame_index % SPINNER.len()]),
            Style::default().fg(theme::FOCUS),
        ),
        (
            thinking.agent.clone(),
            Style::default().add_modifier(Modifier::BOLD),
        ),
        (
            format!(" thinking about {}", clip(&thinking.about, LONGEST_SUBJECT)),
            Style::default(),
        ),
        (format!("{SEPARATOR}{}", elapsed(thinking.elapsed)), dim),
    ];
    if let Some(hint) = &thinking.stop_hint {
        parts.push((format!("{SEPARATOR}{hint}"), dim));
    }
    parts
}

fn elapsed(duration: Duration) -> String {
    let seconds = duration.as_secs();
    if seconds < 60 {
        format!("{seconds}s")
    } else {
        format!("{}m {:02}s", seconds / 60, seconds % 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn elapsed_reads_in_seconds_then_minutes() {
        assert_eq!(elapsed(Duration::from_millis(12_900)), "12s");
        assert_eq!(elapsed(Duration::from_secs(65)), "1m 05s");
    }
}
