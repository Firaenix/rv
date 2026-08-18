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

mod hit;

pub use hit::hit;

use ratatui::layout::Rect;

/// Columns between the two panes, which are also the column the pointer grabs
/// to resize them. One: a wider handle would cost the diff columns it needs
/// more than the pointer needs the target.
const DIVIDER: u16 = 1;

/// Rows a pane spends on its top border, which is the row a click on it must
/// *not* be counted as content.
pub(super) const TOP_BORDER: u16 = 1;

/// The same for the bottom border. Kept separate from [`TOP_BORDER`] rather
/// than folded into one "borders: 2", because the two are subtracted at
/// different ends of the pane and a single constant used twice is how an
/// off-by-one hides: this module shipped with the bottom border reporting a
/// content row one past the last row [`crate::ui`] paints.
pub(super) const BOTTOM_BORDER: u16 = 1;

/// How much of the area the help popup covers, in tenths. Large enough to hold
/// the keymap, small enough that the panes stay visible around it — a reviewer
/// reading about a key wants to see what it would act on.
///
/// Eight rather than seven since the keymap grew past twenty rows: at seven a
/// 24-row terminal gave the popup fourteen content rows, which the groups could
/// not be dealt into two columns without splitting one — so the popup fell back
/// to a single scrolling column and hid the last three keys. A keymap you must
/// scroll to read is a keymap you will not read.
const POPUP_TENTHS: u16 = 8;

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
    /// Whether the reviewer has put the sidebar away with `z`.
    ///
    /// A narrow enough terminal puts it away regardless — see
    /// [`NARROW_COLUMNS`] — so this is what the reviewer asked for, not what
    /// they get.
    pub sidebar_hidden: bool,
}

/// Every rectangle of one frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    /// The file list or the comment browser.
    pub sidebar: Rect,
    /// The one column between the panes, which is also the resize handle.
    pub divider: Rect,
    pub diff: Rect,
    /// The status line, or the comment box, along the bottom under both panes.
    pub bar: Rect,
    pub popup: Option<Rect>,
    /// The floating alert, when there is one. Drawn over the panes; never a
    /// click target — see [`Target`].
    pub toast: Option<Rect>,
    /// The one cell that opens and closes the sidebar by pointer.
    ///
    /// A key is not enough on a phone over ssh, which is the whole reason the
    /// sidebar folds away at all. It is the bar's first cell — the bottom-left
    /// of the screen, where every editor puts the same control — so it is in
    /// the same place whether the sidebar is showing or not.
    pub chevron: Rect,
}

/// What is under the pointer.
///
/// The two row variants are indices **within the pane's inner area**: row 0 is
/// the first row under the pane's top border and the last is the row above its
/// bottom border, so a pane of `height` rows answers for `height - 2` of them
/// and for no others. They are not diff line numbers and not list indices — the
/// caller adds its own scroll offset, because the scroll offset is state and
/// this module has none.
///
/// There is no `Toast` variant. A toast is drawn over the panes but takes no
/// key and no gesture, so a click where one floats reaches the pane beneath it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    SidebarRow(usize),
    DiffRow(usize),
    /// The resize handle between the panes.
    Divider,
    /// The status line or the comment box.
    Bar,
    Popup,
    /// The one cell that opens and closes the sidebar.
    Chevron,
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
    let hidden = chrome.sidebar_hidden;
    // With the sidebar away there is nothing to divide and nothing to drag, so
    // the handle gives its column to the diff. On the narrow screens this fold
    // exists for, a column of nothing is a column of code.
    let divider_columns = if hidden { 0 } else { DIVIDER.min(area.width) };
    let shared = area.width - divider_columns;
    // Put away only on request. A narrow screen does *not* hide it by itself:
    // the file list already degrades deliberately as columns run out — the
    // change bar goes, then the counts, then the path is clipped — and a rule
    // that overrode all of that would take the choice away on exactly the
    // screens where the reviewer most needs it. `z` and the chevron are the
    // choice; the degradation is the default.
    let sidebar_columns = if hidden { 0 } else { split.sidebar_width(shared) };
    let diff_columns = shared - sidebar_columns;

    let sidebar = Rect::new(area.x, area.y, sidebar_columns, pane_rows);
    let divider = Rect::new(sidebar.right(), area.y, divider_columns, pane_rows);
    let diff = Rect::new(divider.right(), area.y, diff_columns, pane_rows);
    let bar = Rect::new(area.x, area.y + pane_rows, area.width, bar_rows);

    // The bar's first cell. Not a pane's corner: a control drawn over `╭`
    // destroys the frame it sits on, and the bar is the one row that is always
    // there whatever the panes are doing.
    let chevron = Rect::new(bar.x, bar.y, DIVIDER.min(bar.width), bar.height.min(1));

    Layout {
        sidebar,
        divider,
        diff,
        chevron,
        bar,
        popup: chrome
            .help_open
            .then(|| centered(area, POPUP_TENTHS))
            .filter(non_empty),
        toast: chrome.toast.then(|| floating(area)).filter(non_empty),
    }
}

/// Whether a floating rectangle has room to be drawn at all. A terminal too
/// small for a popup gets no popup rather than a zero-sized one that
/// hit-testing would have to special-case.
fn non_empty(rect: &Rect) -> bool {
    rect.width > 0 && rect.height > 0
}

/// A rectangle `tenths` of the area's size, centred in it.
fn centered(area: Rect, tenths: u16) -> Rect {
    // Trimmed to the area's own parity, so the margin left over is even and can
    // be split in half exactly. Without it a popup one row off the area's parity
    // sits a row higher than it sits low, which reads as a mis-drawn frame
    // rather than as a rounding decision nobody made.
    let width = even_margin(area.width, area.width * tenths / 10);
    let height = even_margin(area.height, area.height * tenths / 10);
    Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    )
}

/// `size`, less one where that is what makes `whole - size` even.
fn even_margin(whole: u16, size: u16) -> u16 {
    if (whole - size).is_multiple_of(2) {
        size
    } else {
        size.saturating_sub(1)
    }
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

