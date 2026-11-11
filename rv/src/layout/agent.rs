//! Where the agent's corner and answer popup go.
//!
//! The corner sits **on the diff pane's bottom border**, right-aligned, just
//! above the bar and under where the `?` tip opens. A border row is never a
//! click target ([`super::hit`] counts only the rows between the borders), so
//! the corner covers nothing a click can select and needs no target of its
//! own. The answer popup is modal and is hit-tested as the popup.
//!
//! The thinking tooltip floats in the diff pane's top-right corner while an
//! ask runs. Like the toast it is not a click target.

use ratatui::layout::Rect;

use super::Layout;

/// The border's corner glyph and one cell of rule kept on each side.
const BORDER_KEPT: u16 = 2;

/// Columns and rows left round the answer popup, so it reads as floating.
const ANSWER_MARGIN: u16 = 2;

impl Layout {
    /// `corner_columns` and `thinking_columns` are how wide those want to be,
    /// `None` when absent; `answer` whether the popup is up.
    #[must_use]
    pub fn with_agent(
        mut self,
        corner_columns: Option<u16>,
        thinking_columns: Option<u16>,
        answer: bool,
    ) -> Self {
        self.corner = corner_columns.and_then(|columns| corner(self.diff, columns));
        self.thinking = thinking_columns.and_then(|columns| thinking(self.diff, columns));
        self.answer = answer.then(|| popup(self.diff, self.bar)).flatten();
        self
    }
}

fn corner(diff: Rect, columns: u16) -> Option<Rect> {
    if diff.height < 2 {
        return None;
    }
    let room = diff.width.saturating_sub(2 * BORDER_KEPT);
    let width = columns.min(room);
    if width == 0 {
        return None;
    }
    Some(Rect::new(
        diff.right().saturating_sub(BORDER_KEPT + width),
        diff.bottom() - 1,
        width,
        1,
    ))
}

/// Three rows inside the pane's top-right corner, clear of its border.
fn thinking(diff: Rect, columns: u16) -> Option<Rect> {
    const ROWS: u16 = 3;
    const KEPT: u16 = 2;
    if diff.height < ROWS + 2 {
        return None;
    }
    let width = columns.min(diff.width.saturating_sub(2 * KEPT));
    (width > 4).then(|| {
        Rect::new(
            diff.right().saturating_sub(KEPT + width),
            diff.y + 1,
            width,
            ROWS,
        )
    })
}

/// Over the diff pane, inset by the margin where the pane has room for it.
fn popup(diff: Rect, bar: Rect) -> Option<Rect> {
    let inset = |length: u16| {
        if length > 4 * ANSWER_MARGIN {
            ANSWER_MARGIN
        } else {
            0
        }
    };
    let (x, y) = (inset(diff.width), inset(diff.height) / 2);
    let width = diff.width.saturating_sub(2 * x);
    let height = bar.y.saturating_sub(diff.y).saturating_sub(2 * y);
    (width > 2 && height > 2).then(|| Rect::new(diff.x + x, diff.y + y, width, height))
}
