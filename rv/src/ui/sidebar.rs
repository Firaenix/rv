//! The left column: the review's files, or its comments.
//!
//! One column with two tabs rather than two columns, because a reviewer moves
//! through comments the way they move through files — the same keys, in the
//! same place on screen — and because a review is worth every column the diff
//! can have.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::Line;
use ratatui::widgets::List;
use ratatui::widgets::ListItem;
use ratatui::widgets::Paragraph;
use rv_core::store::Comment;

use super::BORDER_ROWS;
use super::comment_box::comment_style;
use super::comment_box::state_name;
use super::files;
use super::list::list_state;
use super::pane::pane;
use super::pane::selection_style;
use super::text::clip;
use super::text::shift;
use crate::app::App;
use crate::app::Focus;
use crate::app::SidebarTab;

/// What the Comments tab says when the review has no comments in it yet.
const NO_COMMENTS_YET: &str = "no comments yet";

pub(super) fn draw_sidebar(frame: &mut Frame, app: &App, area: Rect) {
    let focused = app.focus() == Focus::Sidebar;
    match app.sidebar_tab() {
        SidebarTab::Files => files::draw_files(frame, app, area, focused),
        SidebarTab::Commits => files::draw_commits(frame, app, area, focused),
        SidebarTab::Comments => draw_comment_browser(frame, app, area, focused),
    }
}

/// Every comment in the review, wherever it lives: `path:line`, its state, and
/// the first line of its body.
///
/// The reason it exists is arithmetic rather than taste. The first real session
/// on `rv` spent 2,200 of its 11,101 keystrokes on `j` and `]`, with one known
/// line costing 940 consecutive presses of `j`; `Enter` on a row here is that
/// same trip in one keystroke.
fn draw_comment_browser(frame: &mut Frame, app: &App, area: Rect, focused: bool) {
    let block = pane(format!("Comments ({})", app.comments().len()), focused);
    if app.comments().is_empty() {
        frame.render_widget(Paragraph::new(NO_COMMENTS_YET).block(block), area);
        return;
    }

    let width = usize::from(area.width.saturating_sub(BORDER_ROWS));
    let items: Vec<ListItem> = app
        .comments()
        .iter()
        .map(|comment| {
            ListItem::new(Line::styled(
                clip(&shift(&summary(comment), app.sidebar_hscroll()), width),
                comment_style(comment),
            ))
        })
        .collect();
    let list = List::new(items)
        .block(block)
        .highlight_style(selection_style(focused));

    let mut state = list_state(app, area, app.comments().len(), app.browser_index());
    frame.render_stateful_widget(list, area, &mut state);
}

/// One comment on one row: where it is, what state it is in, and the first line
/// of what it says.
fn summary(comment: &Comment) -> String {
    let first = comment.body.lines().next().unwrap_or_default();
    format!(
        "{}:{} {} {first}",
        comment.anchor.file,
        comment.anchor.line,
        state_name(comment.state),
    )
}
