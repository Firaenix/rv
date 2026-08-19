//! Where each pane is, and what it drew there.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;
use rv::layout::Chrome;
use rv::layout::HelpChrome;
use rv::layout::Split;
use rv::layout::layout;

/// Where each pane is at `width` x `height`, asked of the very function
/// [`rv::ui::draw`] paints from — so a test reading a column out of the buffer
/// reads the column the renderer wrote to.
///
/// The one place these tests are allowed to talk about geometry; see
/// `rv/src/layout.rs`.
pub fn areas(width: u16, height: u16, split: Split) -> rv::layout::Layout {
    layout(
        Rect::new(0, 0, width, height),
        split,
        Chrome {
            bar_rows: 1,
            help: HelpChrome::Closed,
            tooltip: None,
            toast: false,
            sidebar_hidden: false,
        },
    )
}

/// A rectangle's interior: the pane inside its own borders.
///
/// The panes' corners are rounded, so a `╭` at the edge of the frame is a
/// *pane* and a `╭` inside one is a comment box.
pub fn inner(area: Rect) -> Rect {
    Rect::new(
        area.x + 1,
        area.y + 1,
        area.width.saturating_sub(2),
        area.height.saturating_sub(2),
    )
}

/// One frame row, cut to the columns of `area`.
pub fn row_in(buffer: &Buffer, area: Rect, y: u16) -> String {
    (area.x..area.right())
        .map(|x| buffer[(x, y)].symbol())
        .collect()
}

/// The text inside `area`, one row per line.
pub fn text_in(buffer: &Buffer, area: Rect) -> String {
    (area.y..area.bottom())
        .map(|y| row_in(buffer, area, y))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Where `needle` first appears inside `area`, scanning rows top to bottom.
pub fn find_char_in(buffer: &Buffer, area: Rect, needle: char) -> Option<(u16, u16)> {
    let wanted = needle.to_string();
    (area.y..area.bottom())
        .flat_map(|y| (area.x..area.right()).map(move |x| (x, y)))
        .find(|(x, y)| buffer[(*x, *y)].symbol() == wanted)
}

/// Whether the first cell of `needle` inside `area` is drawn in blue — the
/// colour this reviewer reserves for comments.
pub fn styled_blue_in(buffer: &Buffer, area: Rect, needle: char) -> bool {
    find_char_in(buffer, area, needle)
        .is_some_and(|(x, y)| buffer[(x, y)].style().fg == Some(Color::Blue))
}

/// The diff pane's interior at a 100x24 terminal.
pub fn box_area() -> Rect {
    inner(areas(100, 24, Split::default()).diff)
}

/// The file list's own rows, with the pane's borders taken off.
pub fn sidebar_rows(buffer: &Buffer, width: u16, height: u16, split: Split) -> Vec<String> {
    let area = inner(areas(width, height, split).sidebar);
    (area.y..area.bottom())
        .map(|y| {
            (area.x..area.right())
                .map(|x| buffer[(x, y)].symbol())
                .collect()
        })
        .collect()
}

/// The same, as one string.
pub fn sidebar_text(buffer: &Buffer, width: u16, height: u16, split: Split) -> String {
    sidebar_rows(buffer, width, height, split).join("\n")
}

/// The rows of the file list that have anything on them, in order.
pub fn sidebar_filled(buffer: &Buffer, width: u16, height: u16, split: Split) -> Vec<String> {
    sidebar_rows(buffer, width, height, split)
        .into_iter()
        .filter(|row| !row.trim().is_empty())
        .collect()
}

/// The frame row the file list draws `needle` on, at a 100x24 terminal.
pub fn sidebar_row_for(buffer: &Buffer, needle: &str) -> u16 {
    sidebar_row_for_in(
        buffer,
        inner(areas(100, 24, Split::default()).sidebar),
        needle,
    )
}

/// The same, in a file list drawn at some other size.
pub fn sidebar_row_for_in(buffer: &Buffer, area: Rect, needle: &str) -> u16 {
    (area.y..area.bottom())
        .find(|y| row_in(buffer, area, *y).contains(needle))
        .unwrap_or_else(|| {
            panic!(
                "{needle:?} is not in the file list:\n{}",
                text_in(buffer, area)
            )
        })
}

/// What the file list says along its bottom border: its shape and its order.
pub fn sidebar_shape(buffer: &Buffer) -> String {
    let area = areas(100, 24, Split::default()).sidebar;
    (area.x..area.right())
        .map(|x| buffer[(x, area.bottom() - 1)].symbol())
        .collect()
}

/// The background of one cell, or `None` where it is left on the terminal's own
/// ground.
pub fn bg_of(buffer: &Buffer, x: u16, y: u16) -> Option<Color> {
    match buffer[(x, y)].style().bg {
        None | Some(Color::Reset) => None,
        colour => colour,
    }
}

/// Every foreground the file list drew, row by row and cell by cell.
///
/// Foregrounds rather than backgrounds because no row of this pane carries a
/// background at all — see `no_row_of_the_file_list_is_painted_over`.
pub fn sidebar_inks(buffer: &Buffer) -> Vec<Option<Color>> {
    let area = inner(areas(100, 24, Split::default()).sidebar);
    (area.y..area.bottom())
        .flat_map(|y| (area.x..area.right()).map(move |x| (x, y)))
        .map(|(x, y)| match buffer[(x, y)].style().fg {
            None | Some(Color::Reset) => None,
            colour => colour,
        })
        .collect()
}
