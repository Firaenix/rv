//! The row under both panes: the status bar, the confirmation being answered,
//! or the comment being typed.

use ratatui::Frame;
use ratatui::layout::Rect;
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
pub(super) fn draw_bar(frame: &mut Frame, app: &App, area: Rect) {
    match app.mode() {
        Mode::Browse => {
            let view = status_view(app);
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
    }
}

/// What the status bar needs to know about the review, read off the app in one
/// place.
///
/// The bar takes plain data rather than an `&App` — see [`statusbar::View`] —
/// so this is the whole of the coupling between the two, and the bar stays
/// testable without a workspace.
///
/// `mode` is `BROWSE` and nothing else, because this is the only mode that
/// draws the bar. Naming the *context* the cursor is in is a later wave's; the
/// segment is here so that what a reviewer reads is a fact about the keyboard
/// rather than about which pane happened to draw last.
fn status_view(app: &App) -> statusbar::View<'_> {
    statusbar::View {
        mode: "BROWSE",
        file: app.selected_file().map(|file| file.path.as_str()),
        file_index: app.file_index(),
        file_count: app.files().len(),
        stat: app.selected_file().map(|_| app.stat(app.file_index())),
        scope: &app.session().revset,
        open_comments: app
            .comments()
            .iter()
            .filter(|comment| comment.state == CommentState::Open)
            .count(),
        status: app.status(),
    }
}
