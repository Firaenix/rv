//! The orders a list of rows can be in, and the sizes they are compared by.

use std::cmp::Reverse;

use crate::gradient::Stat;

/// The order the sidebar's rows are in.
///
/// One mode rather than one per view, which is why one key serves both:
/// [`Sort::Natural`] means "the order the thing already has" — path order for
/// files, stack order for commits — and the other two weigh a row by one hand
/// of its [`Stat`], heaviest first.
///
/// An order applies *within* the grouping and never across it: siblings sort
/// against each other, a directory sorts among its own siblings by its
/// aggregate, and its children stay under it. A reviewer asked for a tree and
/// for sorting; they compose, and neither disables the other.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Sort {
    /// The order the thing already has: path order in the bookmark view,
    /// stack order in the commits view.
    #[default]
    Natural,
    /// Most lines added first.
    Added,
    /// Most lines removed first.
    Removed,
}

impl Sort {
    /// The next order, cycling — what the one key that switches them does.
    #[must_use]
    pub const fn next(self) -> Self {
        match self {
            Self::Natural => Self::Added,
            Self::Added => Self::Removed,
            Self::Removed => Self::Natural,
        }
    }

    /// The one word the sidebar's title says, so that the name of a mode is
    /// declared beside the mode rather than invented by a renderer.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Natural => "natural",
            Self::Added => "added",
            Self::Removed => "removed",
        }
    }

    /// What this order weighs a row by, or `None` when it weighs nothing and
    /// the rows keep the order they arrived in.
    const fn weigh(self, stat: Stat) -> Option<u32> {
        match self {
            Self::Natural => None,
            Self::Added => Some(stat.added),
            Self::Removed => Some(stat.removed),
        }
    }
}

/// Puts `items` in `sort`'s order, heaviest first, and leaves them exactly as
/// they were under [`Sort::Natural`].
///
/// The sort is stable, so rows of equal weight keep the order they already had
/// rather than swapping for nothing — a sidebar that reshuffles equal rows
/// moves under the cursor and buys nothing for it. One helper and three
/// callers, so a directory's children, a flat list and a stack of changes
/// cannot come to disagree about what "sorted" means.
pub(super) fn order<T>(items: &mut [T], sort: Sort, stat: impl Fn(&T) -> Stat) {
    if matches!(sort, Sort::Natural) {
        return;
    }
    items.sort_by_key(|item| Reverse(sort.weigh(stat(item)).unwrap_or_default()));
}

/// A count as a narrow sidebar can afford to print it: `42` stays `42`, `1234`
/// becomes `1.2k` and `45678` becomes `46k`.
///
/// Never more than four characters wide, for any `u32`. The counts are the
/// first thing dropped when the sidebar is squeezed, and a number that
/// overflowed its column would push the path out instead — which is the wrong
/// thing to lose, since the gradient still carries the ratio but nothing else
/// carries the name.
///
/// A value under ten in its unit keeps one decimal, because `1.2k` and `9.8k`
/// are four times apart and `1k` against `10k` would be the only alternative;
/// above ten the decimal is noise and is dropped. Rounding that would carry a
/// value up to the next unit moves it there rather than printing `1000k`.
#[must_use]
pub fn abbreviate(n: u32) -> String {
    if n < 1_000 {
        return n.to_string();
    }

    let mut scale = 1_000u64;
    for suffix in ["k", "M", "G"] {
        // The value in this unit, rounded half up: first to a tenth, then —
        // if that has grown past ten — to the unit itself.
        let tenths = (u64::from(n) * 10 + scale / 2) / scale;
        if tenths < 100 {
            let (whole, tenth) = (tenths / 10, tenths % 10);
            return if tenth == 0 {
                format!("{whole}{suffix}")
            } else {
                format!("{whole}.{tenth}{suffix}")
            };
        }
        let units = (u64::from(n) + scale / 2) / scale;
        if units < 1_000 {
            return format!("{units}{suffix}");
        }
        scale *= 1_000;
    }

    // Unreachable for a `u32`, whose largest value is 4.3G and so returns at
    // "G" or sooner. Total rather than panicking: a count is not worth a
    // crash.
    n.to_string()
}

