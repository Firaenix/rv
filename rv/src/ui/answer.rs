//! The agent's last answer in full, over the diff: what it was asked, the
//! summary, the prose and the lines it pointed at, with what it cost along
//! the bottom border. Every string was cleaned when the reply landed.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::widgets::Block;
use ratatui::widgets::BorderType;
use ratatui::widgets::Clear;
use ratatui::widgets::Padding;
use ratatui::widgets::Paragraph;
use ratatui::widgets::Wrap;

use crate::app::App;
use crate::theme;

pub(super) fn draw_answer(frame: &mut Frame, app: &App, area: Rect) {
    let (Some(answer), Some(scroll)) = (app.answer(), app.answer_scroll()) else {
        return;
    };
    let mut lines: Vec<Line<'static>> = answer.lines().into_iter().map(Line::raw).collect();
    if let Some(summary) = lines.first_mut() {
        *summary = summary
            .clone()
            .style(Style::default().add_modifier(Modifier::BOLD));
    }
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme::FOCUS))
        .padding(Padding::horizontal(1))
        .title(format!("▸ Agent on {} — Esc closes", answer.about))
        .title_bottom(format!(" {} ", answer.footer));
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(lines)
            .block(block)
            .wrap(Wrap { trim: false })
            .scroll((u16::try_from(scroll).unwrap_or(u16::MAX), 0)),
        area,
    );
}
