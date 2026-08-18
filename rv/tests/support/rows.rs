//! Which rows of a file's plan a pane can show.

use std::ops::Range;

use ratatui::layout::Rect;
use rv::app::App;
use rv::layout::Split;
use rv::ui;
use super::areas;


/// The diff pane's rectangle on a terminal `width` columns wide, sized so the
/// pane itself is `height` rows tall.
///
/// The bar takes the row under both panes, so the terminal has to be a row
/// taller than the pane asked for. The assertion is what keeps that arithmetic
/// honest rather than silently off by one if the chrome ever changes.
pub fn diff_pane(width: u16, height: u16) -> Rect {
    let area = areas(width, height + 1, Split::default()).diff;
    assert_eq!(
        area.height, height,
        "the diff pane is not {height} rows tall"
    );
    area
}

/// The rows of the diff pane's plan that are on screen at that size, as
/// indices into the plan.
///
/// Asked of [`rv::ui::visible`] — the very function [`rv::ui::draw`] windows
/// with — rather than recomputed here. That matters more in this section than
/// anywhere else in the file: the defect below *was* a window and a cursor
/// disagreeing, and a test with its own copy of the arithmetic would be
/// asserting about a third thing that neither of them uses.
pub fn visible_row_indices(app: &App, width: u16, height: u16) -> Range<usize> {
    ui::visible(app, diff_pane(width, height)).1
}

/// How many rows that plan holds in total.
pub fn row_count(app: &App, width: u16, height: u16) -> usize {
    ui::visible(app, diff_pane(width, height)).0.rows.len()
}
