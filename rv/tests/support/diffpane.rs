//! Reading the diff pane's rows, washes and foregrounds out of a frame.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;
use rv::gradient;

use super::buffer_text;

/// The diff pane's rows at `width` x `height`, as `(frame row, text)` pairs
/// with the pane's own borders taken off.
pub fn diff_rows(buffer: &Buffer, area: Rect) -> Vec<(u16, String)> {
    ((area.y + 1)..area.bottom().saturating_sub(1))
        .map(|y| {
            let text = ((area.x + 1)..area.right().saturating_sub(1))
                .map(|x| buffer[(x, y)].symbol())
                .collect();
            (y, text)
        })
        .collect()
}

/// The frame row the diff pane draws its first line carrying `sigil` on.
///
/// The sigil is column 6 of the pane's inner area — a five-wide number field
/// and a space — so this cannot be fooled by a `+` inside a line's text.
pub fn row_of_sigil(buffer: &Buffer, area: Rect, sigil: char) -> u16 {
    diff_rows(buffer, area)
        .into_iter()
        .find(|(_, text)| text.chars().nth(6) == Some(sigil))
        .map(|(y, _)| y)
        .unwrap_or_else(|| {
            panic!(
                "no diff line carries the sigil {sigil:?}:\n{}",
                buffer_text(buffer)
            )
        })
}

/// The background the diff pane painted row `y` with, or `None` where the row
/// is left on the terminal's own ground.
pub fn diff_bg(buffer: &Buffer, area: Rect, y: u16) -> Option<Color> {
    match buffer[(area.x + 1, y)].style().bg {
        None | Some(Color::Reset) => None,
        colour => colour,
    }
}

/// Every distinct foreground the diff pane used on row `y`, ignoring the cells
/// that hold nothing.
pub fn distinct_foregrounds(buffer: &Buffer, area: Rect, y: u16) -> Vec<Color> {
    let mut seen: Vec<Color> = Vec::new();
    for x in (area.x + 1)..area.right().saturating_sub(1) {
        let cell = &buffer[(x, y)];
        if cell.symbol().trim().is_empty() {
            continue;
        }
        let fg = cell.style().fg.unwrap_or(Color::Reset);
        if !seen.contains(&fg) {
            seen.push(fg);
        }
    }
    seen
}

/// Every cell of the diff pane's interior that carries a glyph, as
/// `(column, row)`.
///
/// Blank cells are skipped because a blank cell has no foreground to judge:
/// what is being asked here is what colour the *code a reviewer is reading* was
/// sent in.
pub fn diff_pane_cells(buffer: &Buffer, area: Rect) -> Vec<(u16, u16)> {
    let mut cells = Vec::new();
    for y in (area.y + 1)..area.bottom().saturating_sub(1) {
        for x in (area.x + 1)..area.right().saturating_sub(1) {
            if !buffer[(x, y)].symbol().trim().is_empty() {
                cells.push((x, y));
            }
        }
    }
    cells
}

/// The foreground the diff pane drew the first `//` comment in.
pub fn colour_of_first_comment(buffer: &Buffer, area: Rect) -> Option<Color> {
    let (y, text) = diff_rows(buffer, area)
        .into_iter()
        .find(|(_, text)| text.contains("//"))
        .unwrap_or_else(|| panic!("no `//` comment is on screen:\n{}", buffer_text(buffer)));
    let at = text.find("//").expect("the row holds it");
    let column = area.x + 1 + u16::try_from(text[..at].chars().count()).expect("a small column");
    buffer[(column, y)].style().fg
}

/// Whether `target` sits on the ramp from `from` to the ink the diff washes
/// with — which is what "the diff and the sidebar share one green" means in
/// cells rather than in prose.
pub fn on_the_ramp(target: Color, from: gradient::Rgb) -> bool {
    (0..=1000).any(|step| {
        let gradient::Rgb(r, g, b) =
            gradient::oklab_mix(from, gradient::INK_DARK, step as f32 / 1000.0);
        Color::Rgb(r, g, b) == target
    })
}
