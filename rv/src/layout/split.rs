//! How the width is divided between the two panes.

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
