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

/// How many columns [`counts`]'s answer takes, the space between the two
/// numbers included.
pub(super) fn counts_columns((added, removed): &(String, String)) -> usize {
    if added.is_empty() {
        return 0;
    }
    added.chars().count() + 1 + removed.chars().count()
}
