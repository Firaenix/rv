//! The row under both panes: the status bar, the confirmation being answered,
//! or the comment being typed.

use std::time::Instant;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::widgets::Block;
use ratatui::widgets::BorderType;
use ratatui::widgets::Paragraph;
use rv_core::store::CommentState;

use super::BORDER_ROWS;
use super::text::clip;
use super::text::tail;
use crate::app::App;
use crate::app::Mode;
use crate::statusbar;

/// Draws whichever of the three the mode calls for.
///
/// **Browsing draws [`crate::statusbar`]'s segments**, not `app.status()`
/// across the row. That is what fixes the defect the `?` popup was a workaround
/// for: the status used to *be* the bar, so the first `d` a reviewer pressed
/// replaced the keymap with `deleted comment at a.rs:42` and it never came
/// back. As one segment among six it can displace nothing.
///
/// **A confirmation is not a status message**, so it keeps the whole row: it is
/// a modal question whose answer destroys written work, and a question that
/// could be dropped for want of room is one the reviewer answers blind. It is
/// clipped with a marker rather than dropped, for the same reason.
pub(super) fn draw_bar(frame: &mut Frame, app: &App, area: Rect, now: Instant) {
    match app.mode() {
        Mode::Browse => {
            let view = status_view(app, now);
            frame.render_widget(
                Paragraph::new(statusbar::render(
                    &statusbar::segments(&view),
                    area.width,
                    app.ascii(),
                )),
                area,
            );
        }
        Mode::ConfirmDelete { .. } => frame.render_widget(
            Paragraph::new(clip(app.status(), usize::from(area.width))),
            area,
        ),
        // The **tail** of the buffer, not its head: a `Paragraph` neither wraps
        // nor scrolls, so a comment longer than the bar used to be typed blind
        // from the character that reached the right-hand edge onwards.
        Mode::Comment => {
            let width = usize::from(area.width.saturating_sub(BORDER_ROWS));
            frame.render_widget(
                Paragraph::new(tail(app.buffer(), width)).block(
                    Block::bordered()
                        .border_type(BorderType::Rounded)
                        .title("Comment"),
                ),
                area,
            )
        }
        // The query on the first row and the matches under it, best first, so
        // the one `Enter` would take is the one nearest what was typed.
        Mode::Pick => {
            let width = usize::from(area.width.saturating_sub(BORDER_ROWS));
            let rows = usize::from(area.height.saturating_sub(BORDER_ROWS)).saturating_sub(1);
            let mut lines = vec![Line::from(format!("/{}", tail(app.buffer(), width)))];
            let matches = app.matches();
            lines.extend(matches.iter().take(rows).enumerate().map(|(rank, entry)| {
                let text = format!(
                    "{} {} {}  {}:{}",
                    // The one `Enter` takes, marked: a list whose first row is
                    // the choice has to say which row that is.
                    if rank == 0 { "▸" } else { " " },
                    // The kind, in its language's own keyword, so two symbols
                    // sharing a name — a struct and its constructor fn — are
                    // told apart without jumping to both.
                    entry.symbol.kind.label(),
                    entry.symbol.name,
                    entry.path,
                    entry.symbol.line
                );
                let style = if rank == 0 {
                    Style::default().add_modifier(Modifier::BOLD)
                } else {
                    Style::default().add_modifier(Modifier::DIM)
                };
                Line::styled(clip(&text, width), style)
            }));
            if matches.is_empty() {
                lines.push(Line::styled(
                    "no symbol matches",
                    Style::default().add_modifier(Modifier::DIM),
                ));
            }
            frame.render_widget(
                Paragraph::new(lines).block(
                    Block::bordered()
                        .border_type(BorderType::Rounded)
                        .title(format!(
                            "Find a symbol ({} in scope)",
                            app.symbols_in_scope()
                        )),
                ),
                area,
            )
        }
    }
}

/// What the status bar needs to know about the review, read off the app in one
/// place.
///
/// The bar takes plain data rather than an `&App` — see [`statusbar::View`] —
/// so this is the whole of the coupling between the two, and the bar stays
/// testable without a workspace.
///
/// `mode` names the [`crate::app::Context`] the cursor is in — `FILES`,
/// `COMMITS`, `DIFF`, `STACK` — in that context's own hue, so the bar says not
/// just that keys are being browsed but *what the next keystroke moves*.
fn status_view(app: &App, now: Instant) -> statusbar::View<'_> {
    let context = app.context();
    statusbar::View {
        mode: context.name(),
        mode_colour: Some(context.colour()),
        file: app.selected_file().map(|file| file.path.as_str()),
        line: app.cursor_line_number(),
        // Only when the index is warm — the bar never builds one.
        symbol: app.enclosing_symbol().unwrap_or_default(),
        file_index: app.file_index(),
        file_count: app.files().len(),
        stat: app.selected_file().map(|_| app.stat(app.file_index())),
        scope: &app.session().revset,
        // `id subject`, which is the same shape the row shows, so the bar and
        // the sidebar name a change the same way.
        change: app
            .change_under_cursor()
            .map(|(change, _, subject)| format!("{change} {subject}"))
            .unwrap_or_default(),
        open_comments: app
            .comments()
            .iter()
            .filter(|comment| comment.state == CommentState::Open)
            .count(),
        // The last thing that happened — empty once it has expired, which is
        // the eight-second rule the viewport spec asks for.
        status: app.status_line(now),
        busy: app.merging(),
        view_state: view_state(app),
    }
}

/// The diff-view toggles that are off their default, joined for the bar's
/// ViewState segment — empty in the standard view, so the segment costs no
/// columns until something is actually different about what the pane shows.
fn view_state(app: &App) -> String {
    let mut tokens = Vec::new();
    if !app.full_context() {
        tokens.push("changes-only");
    }
    if app.grouped() {
        tokens.push("grouped");
    }
    match app.view_side() {
        crate::app::ViewSide::Diffed => {}
        crate::app::ViewSide::Before => tokens.push("before"),
        crate::app::ViewSide::After => tokens.push("after"),
    }
    // The fallback engine's line diff looks exactly like a structural diff
    // that happens to be line-shaped, so the bar is the only place a reviewer
    // can learn which one they are reading. Shown while it is what is on
    // screen — including the moment before difftastic's answer lands.
    if let Some(diff) = app.selected_diff()
        && matches!(diff.source, rv_core::diff::DiffSource::Similar { .. })
    {
        tokens.push("line-diff");
    }
    tokens.join(" · ")
}
