//! The left column: the review's files, or its comments.
//!
//! One column with two tabs rather than two columns, because a reviewer moves
//! through comments the way they move through files — the same keys, in the
//! same place on screen — and because a review is worth every column the diff
//! can have.
//!
//! # Nothing here paints a background
//!
//! The same ruling the file list follows (see [`files`]): a full-row wash reads
//! as a selection and competes with the real one, so selection is the only
//! full-row background in this pane. A file heading is set apart by being bold
//! and by the indent under it, and the suppression note by sitting on the
//! bottom border — neither by a band of colour.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::style::Style;
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
use crate::app::BrowserRow;
use crate::app::Focus;
use crate::app::SidebarTab;
use crate::app::Suppression;

/// What the Comments tab says when the review has no comments in it yet.
const NO_COMMENTS_YET: &str = "no comments yet";

/// The open-directory mark a tree heading carries, matching the files pane's.
const DIR_MARK: &str = "▾  ";

pub(super) fn draw_sidebar(frame: &mut Frame, app: &App, area: Rect) {
    let focused = app.focus() == Focus::Sidebar;
    match app.sidebar_tab() {
        SidebarTab::Files => files::draw_files(frame, app, area, focused),
        SidebarTab::Commits => files::draw_commits(frame, app, area, focused),
        SidebarTab::Comments => draw_comment_browser(frame, app, area, focused),
    }
}

/// Every comment in the review, wherever it lives: grouped under the file it
/// is anchored in, in `(file, line)` order, each row showing `path:line`, its
/// state, and the first line of its body.
///
/// The reason it exists is arithmetic rather than taste. The first real session
/// on `rv` spent 2,200 of its 11,101 keystrokes on `j` and `]`, with one known
/// line costing 940 consecutive presses of `j`; `Enter` on a row here is that
/// same trip in one keystroke.
///
/// The grouping is a **heading row, not a collapsible node** (inline-comments
/// spec §3): comments are few enough at review scale that hiding them behind an
/// expansion costs more keystrokes than it saves.
fn draw_comment_browser(frame: &mut Frame, app: &App, area: Rect, focused: bool) {
    // The note is a fact about the *review*, not about its comments, so it goes
    // on the border of both shapes this pane has. A review that has been
    // reindented and not yet commented on is exactly the one whose reviewer
    // most wants to be told.
    let block = pane(format!("Comments ({})", app.comments().len()), focused)
        .title_bottom(unchanged_note(app.suppression()));
    let rows = app.browser_rows();
    if rows.is_empty() {
        frame.render_widget(Paragraph::new(NO_COMMENTS_YET).block(block), area);
        return;
    }

    let width = usize::from(area.width.saturating_sub(BORDER_ROWS));
    let items: Vec<ListItem> = rows
        .iter()
        .map(|row| browser_row(app, row, width))
        .collect();
    let list = List::new(items)
        .block(block)
        .highlight_style(selection_style(focused));

    let mut state = list_state(app, area, rows.len(), app.browser_index());
    frame.render_stateful_widget(list, area, &mut state);
}

/// One row of the browser: a directory of the tree, a file heading, or a
/// comment indented under one.
fn browser_row<'a>(app: &App, row: &BrowserRow, width: usize) -> ListItem<'a> {
    let (text, style) = match row {
        BrowserRow::Dir { label, depth } => (
            format!("{}{DIR_MARK}{label}", "  ".repeat(*depth)),
            Style::default().add_modifier(Modifier::BOLD),
        ),
        BrowserRow::File { label, depth, .. } => (
            format!("{}{label}", "  ".repeat(*depth)),
            Style::default().add_modifier(Modifier::BOLD),
        ),
        BrowserRow::Comment { index, depth } => match app.comments().get(*index) {
            Some(comment) => (
                format!("{}{}", "  ".repeat(*depth), summary(comment)),
                comment_style(comment),
            ),
            // A row addressing a comment the review does not have cannot happen
            // — the rows are built from that very list — and is drawn blank
            // rather than panicking a frame over it.
            None => (String::new(), Style::default()),
        },
    };
    ListItem::new(Line::styled(
        clip(&shift(&text, app.sidebar_hscroll()), width),
        style,
    ))
}

/// One comment on one row: the line it is on, its state, and the first line of
/// what it says. **No path** — the heading above the row already names the
/// file, and a path here is what used to eat the whole width and leave the one
/// part that says what the comment *is* clipped off the right edge.
fn summary(comment: &Comment) -> String {
    let first = comment.body.lines().next().unwrap_or_default();
    format!(
        ":{} {} · {first}",
        comment.anchor.line,
        state_name(comment.state),
    )
}

/// What the browser says along its bottom border about the files whose change
/// carries no semantic difference — reindentation, a pure move (spec §7.2).
///
/// **It states how much of the review it has actually looked at.** Blobs load
/// lazily, for the file being viewed (spec §7), so this number grows as the
/// reviewer browses; a bare "3 files with no semantic change" would be a claim
/// about the whole review made from the part of it that happens to have been
/// opened, and a note whose number silently changed under the reviewer would be
/// worse than no note.
///
/// So the partial answer carries its own denominator — `3/8 · no semantic
/// change`, three of the eight files rv has an answer for so far — and only
/// the settled one speaks plainly about the review: `3 · no semantic change`.
/// The ratio **leads**, so a border too narrow for the whole note clips a
/// partial answer into a shorter partial answer and never into the complete
/// one.
///
/// Terse because the border is: a 100-column terminal leaves the sidebar 27
/// columns, and `3 files with no semantic change` is thirty-four. The phrase
/// itself is the diff pane's, word for word, so the two surfaces name one fact
/// the same way.
///
/// Absent entirely until something has been suppressed: a review where nothing
/// was reindented has nothing to report, and a permanent `0/8` would spend the
/// border on a fact about the loader.
fn unchanged_note(counted: Suppression) -> String {
    if counted.suppressed == 0 {
        return String::new();
    }
    if counted.checked >= counted.total {
        return format!(" {} · no semantic change ", counted.suppressed);
    }
    format!(
        " {}/{} · no semantic change ",
        counted.suppressed, counted.checked
    )
}
