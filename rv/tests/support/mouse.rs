//! Synthesising mouse events, and the columns and rows they land on.

use super::areas;
use super::frame_at;
use super::inner;
use crossterm::event::KeyModifiers;
use crossterm::event::MouseButton;
use crossterm::event::MouseEvent;
use crossterm::event::MouseEventKind;
use rv::app::App;

/// A left-button press at `(column, row)`, which is what a click sends first
/// and the only half of one `rv` acts on: a click is a *choice*, and the choice
/// is made where the button went down.
pub fn click(column: u16, row: u16) -> MouseEvent {
    mouse(MouseEventKind::Down(MouseButton::Left), column, row)
}

/// The same event, under the name a drag starts with. Spelled twice on purpose:
/// a press on the divider begins a resize and a press anywhere else is a click,
/// and reading `press(divider, 6)` beside `click(60, 6)` is what says so.
pub fn press(column: u16, row: u16) -> MouseEvent {
    click(column, row)
}

pub fn drag(column: u16, row: u16) -> MouseEvent {
    mouse(MouseEventKind::Drag(MouseButton::Left), column, row)
}

pub fn release(column: u16, row: u16) -> MouseEvent {
    mouse(MouseEventKind::Up(MouseButton::Left), column, row)
}

pub fn scroll_down(column: u16, row: u16) -> MouseEvent {
    mouse(MouseEventKind::ScrollDown, column, row)
}

pub fn scroll_up(column: u16, row: u16) -> MouseEvent {
    mouse(MouseEventKind::ScrollUp, column, row)
}

pub fn mouse(kind: MouseEventKind, column: u16, row: u16) -> MouseEvent {
    MouseEvent {
        kind,
        column,
        row,
        modifiers: KeyModifiers::NONE,
    }
}

/// Paints a frame at `width` x `height` — which is how the app comes to know
/// the geometry the pointer is about to be over — and answers the frame row of
/// the diff pane's `row`-th content row.
///
/// The frame is not incidental. A click resolves against the layout that was
/// *painted*, so a test that clicked without drawing would be asking about a
/// screen the reviewer never saw. Assumes [`Mode::Browse`], which is the only
/// mode with a one-row bar.
pub fn diff_pane_row(app: &App, width: u16, height: u16, row: u16) -> u16 {
    let _ = frame_at(app, width, height);
    inner(areas(width, height, app.split()).diff).y + row
}

/// The same for the sidebar.
pub fn sidebar_pane_row(app: &App, width: u16, height: u16, row: u16) -> u16 {
    let _ = frame_at(app, width, height);
    inner(areas(width, height, app.split()).sidebar).y + row
}

/// The same for the one column between the panes, which is the resize handle.
pub fn divider_column(app: &App, width: u16, height: u16) -> u16 {
    let _ = frame_at(app, width, height);
    areas(width, height, app.split()).divider.x
}
