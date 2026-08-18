//! One gesture in, resolved against the rectangles the last frame was painted
//! with.
//!
//! **One layout, two consumers.** [`crate::ui::draw`] hands the [`Layout`] it
//! painted to [`App::note_layout`], and every hit test reads that same value —
//! so a click cannot land somewhere other than what the reviewer can see. A
//! second copy of the arithmetic would drift, and a click that resolves to the
//! wrong row looks exactly like one that resolved to the right row: there is no
//! red test, just a comment on the wrong line.

use anyhow::Result;
use crossterm::event::MouseButton;
use crossterm::event::MouseEvent;
use crossterm::event::MouseEventKind;

use super::Action;
use super::App;
use super::Focus;
use super::Mode;
use super::SidebarTab;
use crate::layout::Layout;
use crate::layout::Split;
use crate::layout::Target;
use crate::layout::hit;
use crate::rows::Row;
use crate::tree::NodeKind;
use crate::ui;

/// How many rows one notch of the wheel moves a pane's view.
///
/// Three, which is what every terminal application scrolls by and what the
/// terminals themselves send for a trackpad's flick.
const WHEEL: isize = 3;

impl App {
    /// Handles one gesture.
    ///
    /// A click in the sidebar focuses it and selects that row (a directory row
    /// folds); a click on a diff line focuses the diff and selects that row; a
    /// click on a comment box focuses the stack and selects that comment; a
    /// drag on the divider resizes until the button comes up; and the wheel
    /// scrolls the pane under the pointer **without moving the selection**.
    ///
    /// **Scrolling is looking; clicking is choosing.** Conflating them means a
    /// stray wheel nudge silently re-aims the next `c` or `d` at another line.
    ///
    /// **No gesture deletes anything.** The confirmation exists because
    /// deletion is unrecoverable, and a mis-click is the accident it guards
    /// against.
    ///
    /// Anything modal answers no gesture: the `?` popup takes only the wheel,
    /// and a half-typed comment takes nothing, because a click that moved the
    /// selection under it would save that comment against a line nobody chose.
    ///
    /// It returns an [`Action`] for symmetry with [`App::on_key`] and always
    /// returns [`Action::Continue`]: no gesture ends a review.
    pub fn on_mouse(&mut self, event: MouseEvent) -> Result<Action> {
        if self.help_open {
            match event.kind {
                MouseEventKind::ScrollDown => self.scroll_help(1),
                MouseEventKind::ScrollUp => self.scroll_help(-1),
                _ => {}
            }
            return Ok(Action::Continue);
        }
        if self.mode != Mode::Browse {
            return Ok(Action::Continue);
        }

        let painted = self.painted.get();
        match event.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                self.on_press(&painted, event.column, event.row)?;
            }
            MouseEventKind::Drag(MouseButton::Left) => self.drag_divider(&painted, event.column),
            MouseEventKind::Up(MouseButton::Left) => self.dragging = false,
            MouseEventKind::ScrollDown => self.wheel(&painted, event.column, event.row, WHEEL),
            MouseEventKind::ScrollUp => self.wheel(&painted, event.column, event.row, -WHEEL),
            // Every other button, and the pointer merely moving: `rv` binds
            // nothing to them, and a right-click menu is a second keymap.
            _ => {}
        }
        Ok(Action::Continue)
    }

    /// Records the rectangles the last frame was painted with. Called by
    /// [`crate::ui::draw`] and nowhere else.
    pub fn note_layout(&self, painted: Layout) {
        self.painted.set(painted);
    }

    /// The button going down: on the divider it takes hold of it, and anywhere
    /// else it is a choice.
    fn on_press(&mut self, painted: &Layout, column: u16, row: u16) -> Result<()> {
        // Cleared first, so a press in a pane ends whatever the last one began:
        // a drag only ever resizes when it *started* on the handle.
        self.dragging = false;
        match hit(painted, column, row) {
            Some(Target::Divider) => self.dragging = true,
            Some(Target::SidebarRow(row)) => self.click_sidebar(painted, row)?,
            Some(Target::DiffRow(row)) => self.click_diff(painted, row),
            // The bar reports state and the popup is dismissed by key; neither
            // answers a click. `None` is the pointer outside everything drawn.
            Some(Target::Bar | Target::Popup) | None => {}
        }
        Ok(())
    }

    /// A click in the left column: it takes the focus, and the row under the
    /// pointer becomes the selection — or folds, where it is a row that holds
    /// others.
    fn click_sidebar(&mut self, painted: &Layout, row: usize) -> Result<()> {
        let Some(index) = ui::sidebar_index_at(self, painted.sidebar, row) else {
            return Ok(());
        };
        self.focus = Focus::Sidebar;
        match self.sidebar_tab {
            SidebarTab::Comments => self.browser_index = index,
            SidebarTab::Files => {
                self.sidebar_row = index;
                // `get` rather than `[index]`: a panic in a mouse handler is a
                // review lost to a mis-click.
                let file = match self.sidebar_nodes().get(index).map(|node| &node.kind) {
                    Some(NodeKind::File { index }) => Some(*index),
                    // The same verb `s` has on the same row.
                    Some(NodeKind::Dir { .. } | NodeKind::Commit { .. }) => None,
                    None => return Ok(()),
                };
                match file {
                    Some(index) => self.select_file(index)?,
                    None => self.toggle_collapse(),
                }
            }
        }
        Ok(())
    }

    /// A click in the diff pane: the row under the pointer becomes the cursor,
    /// and a box row takes the focus into that comment's stack.
    ///
    /// Which comment is read off the plan *before* the cursor moves, because
    /// the click was resolved against that plan.
    fn click_diff(&mut self, painted: &Layout, row: usize) {
        let Some(index) = ui::diff_row_at(self, painted.diff, row) else {
            return;
        };
        let clicked = self.plan().rows.get(index).and_then(comment_of_row);
        self.set_cursor_row(index);
        self.focus = Focus::Diff;
        // `set_cursor_row` has just put the stack cursor back at the top, so
        // this is the whole of the stack's state and cannot be stale.
        if let Some(id) = clicked
            && let Some(position) = self
                .comments_for_line(self.line_index())
                .iter()
                .position(|comment| comment.id == id)
        {
            self.focus = Focus::Stack;
            self.comment_index = position;
        }
    }

    /// The pointer moving with the button down: resize, if it took hold of the
    /// divider.
    ///
    /// The ratio comes from where the pointer is over the columns the two panes
    /// share, which does not change as the split moves — so a drag follows the
    /// pointer instead of accelerating away from it.
    fn drag_divider(&mut self, painted: &Layout, column: u16) {
        if !self.dragging {
            return;
        }
        let shared = u32::from(painted.sidebar.width) + u32::from(painted.diff.width);
        if shared == 0 {
            return;
        }
        let asked = u32::from(column.saturating_sub(painted.sidebar.x)) * 100 / shared;
        self.split = Split::new(u16::try_from(asked).unwrap_or(Split::MAX_RATIO));
    }

    /// The wheel: park the view of whichever pane the pointer is over, `delta`
    /// rows from where it is now, and leave every selection alone.
    fn wheel(&mut self, painted: &Layout, column: u16, row: u16, delta: isize) {
        match hit(painted, column, row) {
            Some(Target::SidebarRow(_)) => {
                self.sidebar_scroll = Some(ui::sidebar_scrolled(self, painted.sidebar, delta));
            }
            Some(Target::DiffRow(_)) => {
                self.diff_scroll = Some(ui::diff_scrolled(self, painted.diff, delta));
            }
            _ => {}
        }
    }
}

/// Which comment a row of the plan belongs to, or `None` for a row of the diff
/// itself.
///
/// The mouse's question and nobody else's: the keyboard reaches a box by
/// walking into it, so only a caller handed a row by a pointer has to turn one
/// back into a comment.
fn comment_of_row(row: &Row<'_>) -> Option<String> {
    match row {
        Row::Diff { .. } => None,
        Row::BoxTop { comment, .. }
        | Row::BoxBody { comment, .. }
        | Row::BoxBottom { comment, .. }
        | Row::BoxCollapsed { comment, .. } => Some(comment.id.clone()),
    }
}
