//! The panel along the bottom while a picker or a jump list is open: the query
//! and what it matches, or the list a jump chooses from.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::widgets::Block;
use ratatui::widgets::BorderType;
use ratatui::widgets::Paragraph;

use super::BORDER_ROWS;
use super::text::clip;
use super::text::tail;
use crate::app::App;
use crate::app::JumpList;
use crate::app::Picking;

/// The query on the first row and the matches under it, best first, so the
/// one `Enter` would take is the one nearest what was typed.
pub(super) fn draw_picker(frame: &mut Frame, app: &App, area: Rect) {
    let width = usize::from(area.width.saturating_sub(BORDER_ROWS));
    let rows = usize::from(area.height.saturating_sub(BORDER_ROWS)).saturating_sub(1);
    let mut lines = vec![Line::from(format!("/{}", tail(app.buffer(), width)))];
    let (title, choices, none) = match app.picking() {
        Picking::Symbol => (
            format!("Find a symbol ({} in scope)", app.symbols_in_scope()),
            symbol_choices(app),
            "no symbol matches",
        ),
        Picking::File => (
            format!("Open a file ({} in the review)", app.files().len()),
            app.file_matches()
                .into_iter()
                .map(|file| app.files()[file].path.clone())
                .collect(),
            "no file matches",
        ),
        // Nothing is listed until Enter: the search reads every file.
        Picking::Text => (
            "Search every file — Enter lists the hits".to_owned(),
            Vec::new(),
            "",
        ),
    };
    lines.extend(choices.iter().take(rows).enumerate().map(|(rank, text)| {
        // The one `Enter` takes, marked: a list whose first row is the choice
        // has to say which row that is.
        let (mark, style) = if rank == 0 {
            ("▸", Style::default().add_modifier(Modifier::BOLD))
        } else {
            (" ", Style::default().add_modifier(Modifier::DIM))
        };
        Line::styled(clip(&format!("{mark} {text}"), width), style)
    }));
    if choices.is_empty() && !none.is_empty() {
        lines.push(Line::styled(
            none,
            Style::default().add_modifier(Modifier::DIM),
        ));
    }
    frame.render_widget(
        Paragraph::new(lines).block(
            Block::bordered()
                .border_type(BorderType::Rounded)
                .title(title),
        ),
        area,
    );
}

/// The kind in its language's own keyword, so two symbols sharing a name — a
/// struct and its constructor fn — are told apart without jumping to both.
fn symbol_choices(app: &App) -> Vec<String> {
    app.matches()
        .iter()
        .map(|entry| {
            format!(
                "{} {}  {}:{}",
                entry.symbol.kind.label(),
                entry.symbol.name,
                entry.path,
                entry.symbol.line
            )
        })
        .collect()
}

/// The list a jump chooses from, with a cursor on it. The window follows the
/// cursor rather than the list's top, so a choice far down a long list is still
/// on screen when it is made.
pub(super) fn draw_jump_list(frame: &mut Frame, app: &App, area: Rect) {
    let width = usize::from(area.width.saturating_sub(BORDER_ROWS));
    let rows = usize::from(area.height.saturating_sub(BORDER_ROWS));
    let references = app.references();
    let at = app.reference_index();
    let first = at.saturating_sub(rows.saturating_sub(1));
    let lines: Vec<Line> = references
        .iter()
        .enumerate()
        .skip(first)
        .take(rows)
        .map(|(rank, reference)| {
            let text = format!(
                "{} {} {}:{}  {}",
                if rank == at { "▸" } else { " " },
                // The grammar's word for the use, so a call is told from a
                // type mention without jumping to both.
                if reference.elsewhere.is_some() {
                    "outside"
                } else {
                    reference.kind.map_or("", |kind| kind.label())
                },
                app.reference_path(reference),
                reference.line,
                reference.text
            );
            let style = if rank == at {
                Style::default().add_modifier(Modifier::BOLD)
            } else {
                Style::default().add_modifier(Modifier::DIM)
            };
            Line::styled(clip(&text, width), style)
        })
        .collect();
    let asked = match app.jump_list() {
        JumpList::Definitions => "Definitions of",
        JumpList::References => "References to",
        // No grammar for these files, so this is every line that spells the
        // word. Saying so is the difference between a list a reviewer can
        // trust and one they cannot.
        JumpList::Lines => "Lines naming",
        JumpList::Matches => "Lines holding",
    };
    frame.render_widget(
        Paragraph::new(lines).block(Block::bordered().border_type(BorderType::Rounded).title(
            format!(
                "{asked} {} ({} of {}){}",
                app.reference_word(),
                at + 1,
                references.len(),
                app.jump_list_note()
                    .map(|note| format!(" · {note}"))
                    .unwrap_or_default()
            ),
        )),
        area,
    );
}
