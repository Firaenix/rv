//! How a review looks: one function that paints an [`App`] onto a frame.
//!
//! Nothing here holds state or decides anything the state machine could decide
//! instead — [`draw`] takes `&App` rather than `&mut App` precisely so that it
//! cannot. Every frame is painted from scratch from the app's fields, which is
//! why the tests can assert on a rendered frame with a `TestBackend` and no
//! terminal at all.
//!
//! The layout is two panes over a bar:
//!
//! ```text
//! ╭──────────────╮│╭───────────────────────────────╮
//! │ sidebar (30%)│││ diff (70%)                    │
//! ╰──────────────╯│╰───────────────────────────────╯
//!  status bar (1 row) — or the comment box (3 rows)
//! ```
//!
//! The bar carries the status bar while browsing and becomes the comment box
//! while typing rather than adding a fourth region: the two are never needed at
//! once, and a review is worth every row the diff can have.
//!
//! **This module computes no rectangle of its own.** Every `Rect` comes from
//! [`crate::layout`], which hit-testing reads from too, so a click cannot land
//! somewhere other than what was drawn. The one thing decided here is how many
//! rows the bar wants, handed over as a [`Chrome`].
//!
//! [`draw`] takes a `now` for the reason [`App`] itself takes one: the only
//! thing on screen that ages is the toast, and its fade being a function of an
//! argument is what makes "it is dim at four and a half seconds" an assertion
//! rather than a sleep.
//!
//! # Colour
//!
//! Two layers, kept apart **by channel, not by hue**. The chrome — borders, the
//! file list, comment boxes, the bar, the gutter — spends one colour per
//! meaning: blue is a *comment*, cyan a *commit hash*, yellow an *alert*,
//! magenta the *focused pane* — ANSI indices from [`crate::theme`], resolved by
//! the terminal's own scheme — and green/red the *additions and removals*,
//! which stay [`crate::gradient`]'s RGB because a proportion is a blend. The
//! code inside the diff pane carries the *terminal's* own syntax colours — see
//! [`capture_colour`]. A wash is a background and a syntax colour is a
//! foreground, so the two never contend for the same channel; spec §6 holds
//! the ruling and §14 the history.
//!
//! Focus is shown three times over — the `▸` on the title, a bold border, and
//! the magenta — because the two cheap signals survive a sixteen-colour
//! terminal and a reader who does not separate magenta from red. Colour
//! enhances the mark; it never carries it alone.
//!
//! Dim is the second axis and it means *not the thing being asked about*: the
//! comment above, a `reply:` inside a box, a key in the `?` popup that would do
//! nothing from where the cursor is. Nothing is hidden — but none of them
//! competes with what is still waiting on somebody.

mod bar;
mod code;
mod comment_box;
mod diff;
mod files;
mod help;
mod info;
mod list;
mod pane;
mod sidebar;
mod text;
mod toast;
mod whichkey;

pub use code::capture_colour;
pub use code::line_background;
pub use diff::diff_row_at;
pub use diff::diff_scrolled;
pub use diff::title;
pub use diff::visible;
pub use list::sidebar_index_at;
pub use list::sidebar_scrolled;

use std::time::Instant;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::Span;

use crate::app::Alert;
use crate::app::App;
use crate::app::Mode;
use crate::layout::Chrome;
use crate::layout::HelpChrome;
use crate::layout::Layout;
use crate::layout::Split;
use crate::layout::layout;
use crate::statusbar;
use crate::theme;

/// Rows the symbol picker takes along the bottom: its two borders, the query
/// being typed, and the matches under it.
///
/// Six rather than the comment box's three: a picker that showed one match is a
/// picker you cannot choose from, and five candidates is what fits without
/// taking a whole small terminal.
const PICKER_ROWS: u16 = 8;

/// Rows the comment box needs: its two borders and the line being typed.
const COMMENT_ROWS: u16 = 3;

/// Rows a bordered pane spends on its own borders. The same number of columns,
/// which is why it is used for both.
pub(crate) const BORDER_ROWS: u16 = 2;

/// Columns a diff line spends before its text starts: a five-wide number field,
/// a space, and the one-character sigil.
///
/// A comment box is indented by exactly this much, so it hangs off the line it
/// is about — under the code rather than under the line numbers.
const GUTTER: usize = 7;

/// Columns a comment box spends on itself per body row: `│`, a space, a space
/// and `│`.
const BOX_PADDING: usize = 4;

/// Paints the whole reviewer, as it stands at `now`.
///
/// This is where the `Layout` that was painted is handed to
/// [`App::note_layout`], so that hit-testing reads the very rectangles the
/// reviewer can see.
pub fn draw(frame: &mut Frame, app: &App, now: Instant) {
    let alerts: Vec<&Alert> = app.alerts().iter().filter(|a| a.live(now)).collect();
    let rects = layout(frame.area(), app.split(), chrome(app, !alerts.is_empty()));
    // Before anything is painted, so that a gesture arriving between this frame
    // and the next resolves against the geometry this frame had.
    app.note_layout(rects);

    // The bar starts after the chevron's column, so the control and the status
    // text never contend for the same cell.
    bar::draw_bar(frame, app, beside(rects.bar, rects.chevron.width), now);
    sidebar::draw_sidebar(frame, app, rects.sidebar);
    diff::draw_diff(frame, app, rects.diff);
    // After the bar, which it is drawn on top of.
    draw_chevron(frame, rects.chevron, rects.sidebar.width > 0);
    // Over the panes, and under the keymap: a reviewer who asked for the manual
    // is reading it, and an alert that covered it would be interrupting the one
    // thing they asked to see.
    if let Some(area) = rects.toast {
        toast::draw_toast(frame, &alerts, area, now);
    }
    // Under the keymap: a reviewer who asked for the manual is reading it.
    if let Some(area) = rects.tooltip {
        info::draw_info(frame, app, area);
    }
    if let Some(area) = rects.popup {
        if app.help_full() {
            help::draw_help(frame, app, area);
        } else {
            help::draw_tip(frame, app, area);
        }
    }
    // Above everything else: a pressed leader is waiting for its next key, and
    // the menu answering it must sit over whatever it was raised in front of.
    whichkey::draw(frame, app, frame.area(), rects.bar);
}

/// `area` with its first `columns` taken off the left.
fn beside(area: Rect, columns: u16) -> Rect {
    Rect::new(
        area.x.saturating_add(columns),
        area.y,
        area.width.saturating_sub(columns),
        area.height,
    )
}

/// The one cell that opens and closes the sidebar by pointer.
///
/// It points the way it would move the edge: `‹` closes the sidebar leftwards,
/// `›` brings it back. In the bar rather than on a pane, so it costs no row of
/// either list and no corner of either frame — which matters most on the narrow
/// screen it exists for.
fn draw_chevron(frame: &mut Frame, area: Rect, showing: bool) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let mark = if showing { "‹" } else { "›" };
    frame.render_widget(
        Span::styled(
            mark,
            // On the bar's own ground, which since the bar went indexed is the
            // terminal's: the control reads as part of the row either way.
            Style::default().fg(theme::FOCUS).bg(statusbar::fill()),
        ),
        area,
    );
}

/// What the layout needs to know about the frame being painted.
///
/// The bar's height is the only thing a [`Mode`] decides about the geometry, so
/// it is the only thing that crosses over. `toast` is a `bool` for the same
/// reason: how many alerts there are does not change where the panel goes.
fn chrome(app: &App, toast: bool) -> Chrome {
    Chrome {
        bar_rows: match app.mode() {
            // A confirmation is a question in the status line, not a box to
            // type in, so it takes the same single row browsing does.
            // The picker is a query in the status line and a list above it.
            Mode::Browse | Mode::ConfirmDelete { .. } => 1,
            Mode::Pick => PICKER_ROWS,
            Mode::Comment => COMMENT_ROWS,
        },
        help: if !app.help_open() {
            HelpChrome::Closed
        } else if app.help_full() {
            HelpChrome::Full
        } else {
            let (rows, columns) = help::tip_size(app);
            HelpChrome::Tip { rows, columns }
        },
        tooltip: app.tooltip(),
        toast,
        sidebar_hidden: app.sidebar_hidden(),
    }
}

/// The geometry of a frame nobody has painted yet: an 80x24 terminal at the
/// default split, browsing.
///
/// The narrowest terminal anyone reviews in, so what [`App`] assumes before its
/// first frame can only be *smaller* than what it gets — which is what makes a
/// click arriving before the first frame land somewhere plausible rather than
/// nowhere.
#[must_use]
pub fn default_layout() -> Layout {
    layout(
        Rect::new(0, 0, 80, 24),
        Split::default(),
        Chrome {
            bar_rows: 1,
            help: HelpChrome::Closed,
            tooltip: None,
            toast: false,
            sidebar_hidden: false,
        },
    )
}

/// The width a comment box's text is wrapped at before any frame has been
/// drawn: [`default_layout`]'s diff pane, less its borders, the gutter a box
/// hangs off and the box's own frame. The first frame can only widen a box.
#[must_use]
pub fn default_body_width() -> usize {
    usize::from(default_layout().diff.width.saturating_sub(BORDER_ROWS))
        .saturating_sub(GUTTER + BOX_PADDING)
}
