//! What a row costs to review: the two numbers, and the bar that shows their
//! proportion.
//!
//! The colour of a sidebar row lives here and nowhere else. There is no
//! background wash — thirty files became thirty slabs of green and the tree
//! stopped looking like a tree — so the change's shape is carried by the
//! counts' own foregrounds and by six cells of bar where there is room.

use ratatui::style::Style;
use ratatui::text::Span;

use super::super::text::colour;
use super::BAR;
use crate::gradient;
use crate::gradient::Rgb;
use crate::gradient::Stat;
use crate::tree;

/// The proportion of a change, as `columns` cells of [`BAR`] running from
/// [`gradient::ADDED`] through [`gradient::pivot`]'s seam to
/// [`gradient::REMOVED`].
///
/// Consecutive cells of one colour are one span, so a flat green bar is one
/// span rather than six.
pub(super) fn change_bar(stat: Stat, columns: usize) -> Vec<Span<'static>> {
    let Some(ratio) = stat.added_ratio() else {
        return vec![Span::raw(" ".repeat(columns))];
    };
    let width = u16::try_from(columns).unwrap_or(u16::MAX);
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut run = String::new();
    let mut ink: Option<Rgb> = None;
    for column in 0..columns {
        let colour_of =
            gradient::column_colour(ratio, u16::try_from(column).unwrap_or(u16::MAX), width);
        if ink != Some(colour_of) {
            if let Some(previous) = ink {
                spans.push(Span::styled(
                    std::mem::take(&mut run),
                    Style::default().fg(colour(previous)),
                ));
            }
            ink = Some(colour_of);
        }
        run.push(BAR);
    }
    if let Some(previous) = ink {
        spans.push(Span::styled(run, Style::default().fg(colour(previous))));
    }
    spans
}

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
