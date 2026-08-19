//! Everything about one change, on `i`.
//!
//! A sidebar row can hold two ids and as much of a subject as fits. This is where
//! the rest lives: the whole change and commit id, the description in full
//! including its body, and every file the change touched with what it cost.

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

use ratatui::style::Color;

use super::BORDER_ROWS;
use super::text::clip;
use super::text::colour;
use crate::app::App;
use crate::app::ChangeInfo;
use crate::gradient;
use crate::theme;

pub(super) fn draw_info(frame: &mut Frame, app: &App, area: Rect) {
    let Some(info) = app.change_info() else {
        return;
    };
    let width = usize::from(area.width.saturating_sub(BORDER_ROWS));
    let rows = usize::from(area.height.saturating_sub(BORDER_ROWS));

    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(lines(&info, width, app.info_scroll(), rows)).block(
            Block::bordered()
                .border_type(BorderType::Rounded)
                .title("▸ Change — i hides"),
        ),
        area,
    );
}

fn lines(info: &ChangeInfo, width: usize, scroll: usize, rows: usize) -> Vec<Line<'static>> {
    let mut out = vec![
        labelled("change", &info.change_id, theme::FOCUS, width),
        labelled("commit", &info.commit_id, theme::HASH, width),
        Line::raw(""),
    ];

    if info.description.trim().is_empty() {
        out.push(Line::styled(
            "(no description set)",
            Style::default().add_modifier(Modifier::DIM),
        ));
    } else {
        out.extend(
            info.description
                .lines()
                .map(|line| Line::raw(clip(line, width))),
        );
    }

    out.push(Line::raw(""));
    out.push(Line::styled(
        format!(
            "{} file{} · +{} -{}",
            info.files.len(),
            if info.files.len() == 1 { "" } else { "s" },
            info.stat.added,
            info.stat.removed
        ),
        Style::default().add_modifier(Modifier::BOLD),
    ));
    out.extend(info.files.iter().map(|(path, stat)| {
        Line::from(vec![
            Span::raw(clip(&format!("  {path}"), width.saturating_sub(12))),
            Span::raw(" "),
            Span::styled(
                format!("+{}", stat.added),
                Style::default().fg(colour(gradient::ADDED)),
            ),
            Span::raw(" "),
            Span::styled(
                format!("-{}", stat.removed),
                Style::default().fg(colour(gradient::REMOVED)),
            ),
        ])
    }));

    // Clamped here rather than in `App`, which does not know how tall the popup
    // is: holding `j` down cannot scroll past the end.
    let start = scroll.min(out.len().saturating_sub(rows));
    out.into_iter().skip(start).take(rows).collect()
}

/// A `label  value` row, with the prefix that selects the value picked out in the
/// same colour the sidebar row uses for it.
fn labelled(label: &str, value: &str, ink: Color, width: usize) -> Line<'static> {
    let short: String = value.chars().take(8).collect();
    let rest: String = value.chars().skip(8).collect();
    Line::from(vec![
        Span::styled(
            format!("{label:<7}"),
            Style::default().add_modifier(Modifier::DIM),
        ),
        Span::styled(short, Style::default().fg(ink).add_modifier(Modifier::BOLD)),
        Span::styled(
            clip(&rest, width.saturating_sub(15)),
            Style::default().add_modifier(Modifier::DIM),
        ),
    ])
}
