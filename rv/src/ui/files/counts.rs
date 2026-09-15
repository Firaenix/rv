//! What a row costs to review: the two numbers beside its name.

use crate::gradient::Stat;
use crate::tree;

/// What a row costs to review, as the two numbers the pane prints — or two
/// empty strings where it cost no lines, because zero is not a measurement.
///
/// Two strings rather than one because they are drawn in two colours, which is
/// where the sidebar's colour lives now that no row is washed. Abbreviated by
/// [`tree::abbreviate`], which is never wider than four characters, so the
/// counts cannot push the path out of a narrow column by being long.
pub(super) fn counts(stat: Stat) -> (String, String) {
    if stat.total() == 0 {
        return (String::new(), String::new());
    }
    (
        format!("+{}", tree::abbreviate(stat.added)),
        format!("-{}", tree::abbreviate(stat.removed)),
    )
}

/// The widths of the list's two counts columns: additions and removals are
/// each right-aligned in their own, so `+1` sits under `+302` and `-2` under
/// `-195` rather than the pairs drifting with the length of their neighbour.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct CountsColumns {
    pub(super) added: usize,
    pub(super) removed: usize,
}

impl CountsColumns {
    pub(super) fn fitting(counted: &[(String, String)]) -> Self {
        counted
            .iter()
            .fold(Self::default(), |columns, (added, removed)| Self {
                added: columns.added.max(added.chars().count()),
                removed: columns.removed.max(removed.chars().count()),
            })
    }

    /// Both columns and the space between them; zero when there is nothing
    /// to show, which is when no column is reserved at all.
    pub(super) fn width(self) -> usize {
        if self.added == 0 {
            return 0;
        }
        self.added + 1 + self.removed
    }
}
