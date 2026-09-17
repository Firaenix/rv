//! Looking rows up in a [`Plan`]: which row holds a line, a box, a flag,
//! and which rows of it a pane of a given height draws.

use std::ops::Range;

use super::Plan;
use super::Row;

impl Plan<'_> {
    /// The row holding diff line `line`, or `None` when the diff has no such
    /// line.
    ///
    /// Linear, like [`Plan::row_of_comment`]: a plan is rebuilt every frame and
    /// a reviewed file is thousands of rows at the very outside, so an index
    /// would cost more to keep correct than the scan costs to run.
    pub fn row_of_line(&self, line: usize) -> Option<usize> {
        self.rows
            .iter()
            .position(|row| matches!(row, Row::Diff { index, .. } if *index == line))
    }

    /// The diff line that owns row `row`, or `None` when the plan has no such
    /// row.
    ///
    /// The inverse of [`Plan::row_of_line`] for a diff row, and the whole of
    /// what makes the row cursor usable for a box row: see [`Row::line`].
    pub fn line_of_row(&self, row: usize) -> Option<usize> {
        self.rows.get(row).map(Row::line)
    }

    /// The row where the `comment_index`-th box under diff line `line` starts
    /// — its top border, or the single row of a collapsed box — or `None` when
    /// that line has no such box.
    pub fn row_of_comment(&self, line: usize, comment_index: usize) -> Option<usize> {
        self.rows
            .iter()
            .enumerate()
            .filter(|(_, row)| match row {
                Row::BoxTop { line: at, .. } | Row::BoxCollapsed { line: at, .. } => *at == line,
                _ => false,
            })
            .nth(comment_index)
            .map(|(row, _)| row)
    }

    /// The row where the `flag_index`-th flag under diff line `line` starts.
    pub fn row_of_flag(&self, line: usize, flag_index: usize) -> Option<usize> {
        self.rows
            .iter()
            .enumerate()
            .filter(|(_, row)| match row {
                Row::Flag {
                    line: at, first, ..
                } => *at == line && *first,
                Row::FlagCollapsed { line: at, .. } => *at == line,
                _ => false,
            })
            .nth(flag_index)
            .map(|(row, _)| row)
    }
}

/// The half-open range of rows to draw: `height` of them where there are that
/// many, centered on `anchor` as far as the ends of the list allow.
///
/// The anchor is always inside the returned range when there is anything to
/// return, so the cursor can never scroll off the pane it is meant to be
/// steering.
pub fn window(rows: usize, anchor: usize, height: usize) -> Range<usize> {
    if rows == 0 || height == 0 {
        return 0..0;
    }
    if rows <= height {
        return 0..rows;
    }
    let start = anchor.saturating_sub(height / 2).min(rows - height);
    start..start + height
}
