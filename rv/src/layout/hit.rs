//! What is under the pointer.
//!
//! The other half of "one layout, two consumers": this reads back exactly the
//! rectangles [`layout`](super::layout) produced, so a click cannot land
//! somewhere other than what was drawn.

use ratatui::layout::Rect;

use super::BOTTOM_BORDER;
use super::Layout;
use super::TOP_BORDER;
use super::Target;

/// What is at `column`, `row` — or [`None`] where the pointer is outside
/// everything the layout drew.
///
/// Tested in painting order, top-most first: the popup covers whatever is
/// beneath it, then the divider, then the panes, then the bar. The toast is
/// deliberately absent — it is painted over the panes but is not a target, so
/// a click passes straight through it.
///
/// A click on either of a pane's horizontal borders is *nothing* rather than a
/// row: the top row carries the title, the bottom row carries nothing at all,
/// and a pane of `height` rows paints only the `height - 2` between them. See
/// [`pane_row`].
#[must_use]
pub fn hit(layout: &Layout, column: u16, row: u16) -> Option<Target> {
    if let Some(popup) = layout.popup
        && within(popup, column, row)
    {
        return Some(Target::Popup);
    }
    // Before the bar, which it is one cell of. Everything else is tested in
    // painting order; this is the one target that sits inside another.
    if within(layout.chevron, column, row) {
        return Some(Target::Chevron);
    }
    if within(layout.divider, column, row) {
        return Some(Target::Divider);
    }
    if let Some(index) = pane_row(layout.sidebar, column, row) {
        return Some(Target::SidebarRow(index));
    }
    if let Some(index) = pane_row(layout.diff, column, row) {
        return Some(Target::DiffRow(index));
    }
    if within(layout.bar, column, row) {
        return Some(Target::Bar);
    }
    None
}

/// Whether a point is inside a rectangle. An empty rectangle contains nothing,
/// which is what makes the degenerate terminals fall out for free.
fn within(rect: Rect, column: u16, row: u16) -> bool {
    column >= rect.x && column < rect.right() && row >= rect.y && row < rect.bottom()
}

/// Which row of a pane's content a point is on, counting from zero under the
/// top border.
///
/// Both borders are excluded, so the rows this answers for are exactly the
/// `height - 2` rows a bordered pane paints into and nothing else. It used to
/// count the bottom border as one more row — sold as a cell of slop for a
/// reviewer aiming at the edge — but a pane of `h` rows draws its last one at
/// `bottom() - 2`, so that row was an index past the end of every list on
/// screen: a click there selects nothing, or whatever the caller's clamp
/// happens to land on. Slop that points at a row that was never drawn is not
/// slop.
///
/// The columns are the pane's full width, borders included: which pane a click
/// is in is the only thing they decide, and a vertical border does not make the
/// row under the pointer any less clear.
fn pane_row(rect: Rect, column: u16, row: u16) -> Option<usize> {
    let first = rect.y.saturating_add(TOP_BORDER);
    let past_last = rect.bottom().saturating_sub(BOTTOM_BORDER);
    (column >= rect.x && column < rect.right() && row >= first && row < past_last)
        .then(|| usize::from(row - first))
}
