//! Where a hover card floats over the diff pane.

use ratatui::layout::Rect;

use super::BOTTOM_BORDER;
use super::Layout;
use super::TOP_BORDER;

/// Columns a card is indented by from the diff pane's left edge: the border and
/// the gutter, so it hangs under the code rather than over the line numbers.
const CARD_INDENT: u16 = 8;

/// What the geometry needs to know about an open hover card: which content
/// row of the diff pane it describes, and how big it wants to be.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CardChrome {
    pub row: u16,
    pub rows: u16,
    pub columns: u16,
}

impl Layout {
    /// This layout with the hover card placed: under the row it describes, or
    /// above it where there is no room below, and never outside the diff pane.
    #[must_use]
    pub fn with_card(mut self, card: Option<CardChrome>) -> Self {
        self.hover = card.and_then(|card| place(self.diff, card));
        self
    }
}

fn place(diff: Rect, card: CardChrome) -> Option<Rect> {
    let first = diff.y.saturating_add(TOP_BORDER);
    let past_last = diff.bottom().saturating_sub(BOTTOM_BORDER);
    let room = past_last.saturating_sub(first);
    let height = card.rows.min(room);
    let indent = CARD_INDENT.min(diff.width.saturating_sub(card.columns));
    let width = card.columns.min(diff.width.saturating_sub(indent));
    if height == 0 || width == 0 {
        return None;
    }
    let anchor = first
        .saturating_add(card.row)
        .min(past_last.saturating_sub(1));
    let below = anchor.saturating_add(1);
    let top = if below.saturating_add(height) <= past_last {
        below
    } else if anchor.saturating_sub(first) >= height {
        anchor - height
    } else {
        past_last.saturating_sub(height)
    };
    Some(Rect::new(diff.x.saturating_add(indent), top, width, height))
}
