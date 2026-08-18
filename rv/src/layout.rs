//! Every rectangle the interface occupies, computed once.
//!
//! [`crate::ui`] paints from a [`Layout`] and hit-testing reads from the same
//! one, so a click can never land somewhere other than what was drawn.
//!
//! ```text
//! ┌──────────────┬─┬────────────────────────────────┐
//! │ sidebar      │ │ diff                           │
//! │              │ │                                │
//! │              ↑ │                                │
//! │           divider                               │
//! ├──────────────┴─┴────────────────────────────────┤
//! │ bar — the status line, or the comment box       │
//! └─────────────────────────────────────────────────┘
//! ```
//!
//! The bar is along the **bottom**. It was drawn above the panes until this
//! module existed; nvim, tmux and zellij all put it below, because that is
//! where a reader's eye goes for state rather than for content.
//!
//! Two rules hold this module together, and both exist because of bugs this
//! project has already paid for:
//!
//! 1. **One layout.** No other file computes a `Rect`. If painting and
//!    hit-testing each did their own arithmetic they would drift, and a click
//!    that resolves to the wrong row looks exactly like a click that resolved
//!    to the right one — there is no red test, just a reviewer whose comment
//!    landed on the wrong line.
//! 2. **Saturating arithmetic everywhere.** A `u16` subtraction that underflows
//!    is the classic ratatui panic, and every one of these numbers comes from a
//!    terminal a user can drag to three columns wide.
//!
//! [`layout`] takes a [`Chrome`] rather than an [`crate::app::App`] or a
//! [`crate::app::Mode`] so it stays a function of the *view*: the bar's height
//! is the only thing the mode decides, and passing the number of rows keeps
//! this module from having to know what a mode means.

use ratatui::layout::Rect;

/// Columns between the two panes, which are also the column the pointer grabs
/// to resize them. One: a wider handle would cost the diff columns it needs
/// more than the pointer needs the target.
const DIVIDER: u16 = 1;

/// Rows a pane spends on its top border, which is the row a click on it must
/// *not* be counted as content.
const TOP_BORDER: u16 = 1;

/// How much of the area the help popup covers, in tenths. Large enough to hold
/// the keymap, small enough that the panes stay visible around it — a reviewer
/// reading about a key wants to see what it would act on.
const POPUP_TENTHS: u16 = 7;

/// The same for a toast, which is one line of text and its border.
const TOAST_TENTHS: u16 = 6;

/// Rows a toast occupies: its two borders and the message.
const TOAST_ROWS: u16 = 3;

/// Where the toast floats, measured from the top of the area. One row down, so
/// it reads as floating *over* the panes rather than as part of their frame.
const TOAST_INSET: u16 = 1;

/// How the width is divided between the sidebar and the diff.
///
/// A percentage rather than a column count, so a resized terminal keeps the
/// proportions the reviewer chose instead of stranding a sidebar at whatever
/// width it happened to have when the window changed.
///
/// Session-only: this is a view preference, not review state, and nothing here
/// reaches `.review/`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Split {
    ratio: u16,
}

impl Split {
    /// The share of the width the sidebar starts with.
    pub const DEFAULT: u16 = 30;

    /// The fewest columns the sidebar is worth having: a path is unreadable
    /// below this and a pane that can be dragged to nothing is a pane a user
    /// can lose.
    pub const MIN_SIDEBAR: u16 = 12;

    /// The same for the diff, which needs its five-column number field, its
    /// sigil and some code besides.
    pub const MIN_DIFF: u16 = 20;

    /// The narrowest and widest the sidebar's share may be set to. The
    /// column-count floors above are about *this* terminal; these are about the
    /// preference itself, so that a split chosen on a wide screen is still a
    /// split on a narrow one.
    pub const MIN_RATIO: u16 = 5;

    /// See [`Split::MIN_RATIO`].
    pub const MAX_RATIO: u16 = 80;

    /// A split at `ratio` percent, clamped to the bounds.
    ///
    /// Clamped rather than trusted: the ratio arrives from a drag whose pointer
    /// may be anywhere, including off the side of the window.
    #[must_use]
    pub fn new(ratio: u16) -> Self {
        Self {
            ratio: ratio.clamp(Self::MIN_RATIO, Self::MAX_RATIO),
        }
    }

    /// The share of the width the sidebar is asking for, as a percentage.
    #[must_use]
    pub fn ratio(self) -> u16 {
        self.ratio
    }

    /// The same split moved by `delta` percentage points, clamped.
    #[must_use]
    pub fn nudged(self, delta: i16) -> Self {
        let moved = i32::from(self.ratio) + i32::from(delta);
        let moved = moved.clamp(i32::from(Self::MIN_RATIO), i32::from(Self::MAX_RATIO));
        Self::new(u16::try_from(moved).unwrap_or(Self::DEFAULT))
    }

    /// How many of the `total` columns the two panes share go to the sidebar.
    ///
    /// `total` is the width of the area **less the divider**, because the
    /// divider is not part of either pane; the caller subtracts it before
    /// asking.
    ///
    /// The ratio is applied first and the floors second, so a terminal wide
    /// enough for both always honours them. When it is not wide enough for
    /// both, the floors give way to an even split rather than one of them
    /// winning and starving the other pane to nothing — a 24-column terminal
    /// showing a 20-column diff and a 3-column sidebar is not a review tool.
    #[must_use]
    pub fn sidebar_width(self, total: u16) -> u16 {
        if total < Self::MIN_SIDEBAR + Self::MIN_DIFF {
            return total / 2;
        }
        let asked = u32::from(total) * u32::from(self.ratio) / 100;
        let asked = u16::try_from(asked).unwrap_or(u16::MAX);
        asked.clamp(Self::MIN_SIDEBAR, total - Self::MIN_DIFF)
    }
}

impl Default for Split {
    fn default() -> Self {
        Self::new(Self::DEFAULT)
    }
}

/// What is on screen besides the two panes, as far as the geometry cares.
///
/// Not a `Mode` and not an `App`: the only thing the mode decides about the
/// layout is how many rows the bar wants, and taking the number keeps this
/// module independent of what any mode means.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Chrome {
    /// Rows the bar wants: one for the status line, more for the comment box.
    pub bar_rows: u16,
    /// Whether the `?` popup is up.
    pub help_open: bool,
    /// Whether an alert is floating over the panes.
    pub toast: bool,
}

/// Every rectangle of one frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    /// The file list or the comment browser.
    pub sidebar: Rect,
    /// The one column between the panes, which is also the resize handle.
    pub divider: Rect,
    /// The selected file's diff.
    pub diff: Rect,
    /// The status line, or the comment box, along the bottom under both panes.
    pub bar: Rect,
    /// The `?` popup, when it is open.
    pub popup: Option<Rect>,
    /// The floating alert, when there is one. Drawn over the panes; never a
    /// click target — see [`Target`].
    pub toast: Option<Rect>,
}

/// What is under the pointer.
///
/// The two row variants are indices **within the pane's inner area**: row 0 is
/// the first row under the pane's top border. They are not diff line numbers
/// and not list indices — the caller adds its own scroll offset, because the
/// scroll offset is state and this module has none.
///
/// There is no `Toast` variant. A toast is drawn over the panes but takes no
/// key and no gesture, so a click where one floats reaches the pane beneath it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    /// A row of the sidebar's list.
    SidebarRow(usize),
    /// A row of the diff pane's row plan — see [`crate::rows`].
    DiffRow(usize),
    /// The resize handle between the panes.
    Divider,
    /// The status line or the comment box.
    Bar,
    /// Anywhere inside the `?` popup.
    Popup,
}

/// Every rectangle for `area`, given the split and what chrome is showing.
///
/// The bar is taken off the bottom first, then what is left is tiled
/// horizontally: sidebar, divider, diff, with no gaps and no overlaps. The
/// popup and the toast float over that and take nothing from it.
#[must_use]
pub fn layout(area: Rect, split: Split, chrome: Chrome) -> Layout {
    // The bar never takes more than the area has, and the panes get whatever
    // survives — including nothing, on a one-row terminal.
    let bar_rows = chrome.bar_rows.min(area.height);
    let pane_rows = area.height - bar_rows;

    // The divider is a column the panes do not get; on a zero-width area there
    // is not even one of those.
    let shared = area.width.saturating_sub(DIVIDER);
    let divider_columns = area.width - shared;
    let sidebar_columns = split.sidebar_width(shared);
    let diff_columns = shared - sidebar_columns;

    let sidebar = Rect::new(area.x, area.y, sidebar_columns, pane_rows);
    let divider = Rect::new(sidebar.right(), area.y, divider_columns, pane_rows);
    let diff = Rect::new(divider.right(), area.y, diff_columns, pane_rows);
    let bar = Rect::new(area.x, area.y + pane_rows, area.width, bar_rows);

    Layout {
        sidebar,
        divider,
        diff,
        bar,
        popup: chrome
            .help_open
            .then(|| centered(area, POPUP_TENTHS))
            .filter(non_empty),
        toast: chrome.toast.then(|| floating(area)).filter(non_empty),
    }
}

/// What is at `column`, `row` — or [`None`] where the pointer is outside
/// everything the layout drew.
///
/// Tested in painting order, top-most first: the popup covers whatever is
/// beneath it, then the divider, then the panes, then the bar. The toast is
/// deliberately absent — it is painted over the panes but is not a target, so
/// a click passes straight through it.
///
/// A click on a pane's top border is *nothing* rather than its first row: that
/// row carries the title, and rounding it down to row 0 would move a
/// reviewer's selection every time they aimed slightly high.
#[must_use]
pub fn hit(layout: &Layout, column: u16, row: u16) -> Option<Target> {
    if let Some(popup) = layout.popup
        && within(popup, column, row)
    {
        return Some(Target::Popup);
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
/// The bottom border counts as a row, one past the last one drawn. It is a
/// single cell of slop at the edge of a pane the reviewer is aiming into, and
/// the alternative — a dead row along the bottom of both panes — is the more
/// surprising of the two. The caller clamps the index to what it actually has.
fn pane_row(rect: Rect, column: u16, row: u16) -> Option<usize> {
    (column >= rect.x && column < rect.right() && row >= rect.y + TOP_BORDER && row < rect.bottom())
        .then(|| usize::from(row - rect.y - TOP_BORDER))
}

/// A rectangle `tenths` of the area's size, centred in it.
fn centered(area: Rect, tenths: u16) -> Rect {
    let width = area.width * tenths / 10;
    let height = area.height * tenths / 10;
    Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    )
}

/// Where an alert floats: top-centre, three rows tall, over whatever the panes
/// have drawn there.
///
/// At the top because that is the half of the screen the eye is not reading
/// code in, and because the bar — the other place state lives — is at the
/// bottom; an alert that appeared next to the status line would be read as part
/// of it.
fn floating(area: Rect) -> Rect {
    let width = area.width * TOAST_TENTHS / 10;
    let height = TOAST_ROWS.min(area.height);
    let inset = TOAST_INSET.min(area.height - height);
    Rect::new(
        area.x + (area.width - width) / 2,
        area.y + inset,
        width,
        height,
    )
}

/// Whether a floating rectangle has room to be drawn at all. A terminal too
/// small for a popup gets no popup rather than a zero-sized one that
/// hit-testing would have to special-case.
fn non_empty(rect: &Rect) -> bool {
    rect.width > 0 && rect.height > 0
}
