//! Which slice of a sidebar list is on screen — the answer painting and
//! hit-testing both read.
//!
//! One module for both tabs, because both are a `List` of one-row items with
//! one selection, and the mouse's question is the same for either.

use ratatui::layout::Rect;
use ratatui::widgets::ListState;

use super::BORDER_ROWS;
use crate::app::App;
use crate::app::SidebarTab;

/// Which slice of a sidebar list is drawn, and whether the selection is in it.
///
/// The offset is **handed to the widget** rather than left to it. ratatui
/// scrolls a `List` far enough to keep its selected item visible, which is
/// right while the view is following the selection and wrong once the wheel has
/// parked it elsewhere: the widget would quietly scroll back, and the row a
/// click resolved to would not be the row that was drawn. So the offset comes
/// from [`list_offset`] — the same function hit-testing reads — and the
/// selection is passed only while it is inside that window, which is what stops
/// ratatui from moving it.
pub(super) fn list_state(app: &App, area: Rect, rows: usize, selected: usize) -> ListState {
    let height = usize::from(area.height.saturating_sub(BORDER_ROWS));
    let offset = list_offset(selected, rows, height, app.sidebar_scroll());
    let shown = (offset..offset.saturating_add(height)).contains(&selected);
    ListState::default()
        .with_offset(offset)
        .with_selected((rows > 0 && shown).then_some(selected))
}

/// Which entry of the sidebar's list is under the `row`-th content row of a
/// sidebar drawn at `pane`, or `None` where the list has no such entry.
#[must_use]
pub fn sidebar_index_at(app: &App, pane: Rect, row: usize) -> Option<usize> {
    let (count, _, offset) = list_view(app, pane);
    let index = offset.checked_add(row)?;
    (index < count).then_some(index)
}

/// The same for the wheel: the first entry on screen after `delta` rows of it.
#[must_use]
pub fn sidebar_scrolled(app: &App, pane: Rect, delta: isize) -> usize {
    let (count, height, offset) = list_view(app, pane);
    let last = count.saturating_sub(height);
    offset.saturating_add_signed(delta).min(last)
}

/// What the sidebar is showing: how many rows its list has, how many of them
/// fit, and which one is on top.
fn list_view(app: &App, pane: Rect) -> (usize, usize, usize) {
    let height = usize::from(pane.height.saturating_sub(BORDER_ROWS));
    let (count, selected) = match app.sidebar_tab() {
        SidebarTab::Files | SidebarTab::Commits => (app.nodes().len(), app.sidebar_row()),
        SidebarTab::Comments => (app.comments().len(), app.browser_index()),
    };
    (
        count,
        height,
        list_offset(selected, count, height, app.sidebar_scroll()),
    )
}

/// Which entry a list `height` rows tall starts at.
///
/// Two rules, and the second is why this exists at all:
///
/// * with no parked view, the list scrolls as little as it can to keep the
///   selection on screen — spelled out here so that the offset the renderer
///   *hands* the widget is the one hit-testing reads;
/// * with one, the reviewer's own position wins and the selection is simply off
///   screen while it does. A list that snapped back to its selection on every
///   wheel notch could not be scrolled past it at all.
fn list_offset(selected: usize, rows: usize, height: usize, scroll: Option<usize>) -> usize {
    let last = rows.saturating_sub(height);
    match scroll {
        Some(offset) => offset.min(last),
        None => selected.saturating_sub(height.saturating_sub(1)).min(last),
    }
}
