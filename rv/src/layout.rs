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
mod split;

pub use hit::hit;
pub use split::Split;

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
/// Raised twice as the keymap grew, and by the same argument each time: **a
/// keymap you must scroll to read is a keymap you will not read**, which outranks
/// keeping the panes visible around it.
///
/// At seven tenths a 24-row terminal gave the popup fourteen content rows and the
/// groups could not be dealt into two columns without splitting one; at eight,
/// sixteen rows, and twenty-two bindings in five groups need seventeen. Nine
/// leaves a two-row, four-column frame of the panes showing, which is enough to
/// see what a key would act on.
///
/// The alternative was abbreviating the manual — `previous symbol` to
/// `prev symbol` — to make three narrower columns fit. A keymap that has to be
/// decoded is worth less than two rows of visible diff.
const POPUP_TENTHS: u16 = 9;

/// The same for a toast, which is one line of text and its border.
const TOAST_TENTHS: u16 = 6;

/// Rows a toast occupies: its two borders and the message.
const TOAST_ROWS: u16 = 3;

/// Where the toast floats, measured from the top of the area. One row down, so
/// it reads as floating *over* the panes rather than as part of their frame.
const TOAST_INSET: u16 = 1;

/// How much of the keymap is up, as far as the geometry cares: nothing, a tip
/// of a known size in the corner, or the whole keymap centred over the panes.
///
/// The tip carries its size because only [`crate::ui`] knows how many bindings
/// the current context lists; where that box *goes* is decided here, like every
/// other rectangle.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum HelpChrome {
    #[default]
    Closed,
    Tip {
        rows: u16,
        columns: u16,
    },
    Full,
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
    /// Whether the `?` keymap is up, and at which size.
    pub help: HelpChrome,
    /// Which sidebar row the change tooltip hangs off, and how tall it wants to
    /// be — `None` when there is no change under the cursor.
    ///
    /// A row and a height rather than a rectangle: where a tooltip *goes* is
    /// geometry, and this module is the only thing that computes that.
    pub tooltip: Option<(u16, u16)>,
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
    /// The change tooltip, hanging off the highlighted row.
    pub tooltip: Option<Rect>,
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
    let sidebar_columns = if hidden {
        0
    } else {
        split.sidebar_width(shared)
    };
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
        popup: match chrome.help {
            HelpChrome::Closed => None,
            HelpChrome::Full => Some(centered(area, POPUP_TENTHS)),
            // In the corner, sitting on the bar so it points at the `? help`
            // hint underneath it — the key that grew it is the key it names.
            HelpChrome::Tip { rows, columns } => Some(corner(area, bar, rows, columns)),
        }
        .filter(non_empty),
        tooltip: chrome
            .tooltip
            .and_then(|(row, rows)| beside(sidebar, diff, row, rows))
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

/// Where the `?` tip goes: the bottom-right corner of the panes, its lower edge
/// on the bar — directly above the `? help` hint at the bar's right-hand end.
///
/// Clamped to the area rather than trusted: the tip asks for the size its rows
/// want, and a terminal shorter than the list gets a shorter tip rather than
/// one drawn off the top of the screen.
fn corner(area: Rect, bar: Rect, rows: u16, columns: u16) -> Rect {
    let width = columns.min(area.width);
    let height = rows.min(bar.y.saturating_sub(area.y));
    Rect::new(
        area.right().saturating_sub(width),
        bar.y.saturating_sub(height),
        width,
        height,
    )
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

/// Where a tooltip hanging off sidebar `row` goes: in the diff pane, top-aligned
/// with the row, `rows` tall and as wide as the pane allows.
///
/// In the diff pane rather than over the sidebar, because the sidebar is the thing
/// it describes and covering it would hide the row the reviewer is on. It slides up
/// where it would run off the bottom, so the whole tooltip is always on screen —
/// a tooltip clipped by the frame is the problem it was built to fix.
///
/// On a narrow terminal it is most of the diff pane, which is the honest answer:
/// there is nowhere else for it, and the reviewer moved the cursor onto a change
/// to read about it.
fn beside(sidebar: Rect, diff: Rect, row: u16, rows: u16) -> Option<Rect> {
    if diff.width <= TOP_BORDER + BOTTOM_BORDER || diff.height == 0 {
        return None;
    }
    let height = rows.min(diff.height);
    let top = sidebar
        .y
        .saturating_add(TOP_BORDER)
        .saturating_add(row)
        .min(diff.bottom().saturating_sub(height));
    Some(Rect::new(diff.x, top.max(diff.y), diff.width, height))
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
