//! `v b`: which side of the change the diff pane shows.
//!
//! A reviewer sometimes wants the file as it *was* — context and the lines the
//! change removed, without the additions crowding them — or as it *will be*,
//! and sometimes both at once. The filter drops whole line kinds, so a comment
//! written on a removed line simply does not appear while the head side is
//! shown; the comment is still in the store, and the base side brings it back.

use rv_core::diff::DiffLine;
use rv_core::diff::LineKind;

/// Which side of the change the diff pane shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ViewSide {
    /// Context, removals and additions all shown — the diff proper.
    #[default]
    Diffed,
    /// Context and removals: the file as it was before the change.
    Before,
    /// Context and additions: the file as it will be after it.
    After,
}

impl ViewSide {
    /// The order `v b` cycles through: the whole diff, then the base side, then
    /// the head side, then back.
    #[must_use]
    pub fn next(self) -> Self {
        match self {
            ViewSide::Diffed => ViewSide::Before,
            ViewSide::Before => ViewSide::After,
            ViewSide::After => ViewSide::Diffed,
        }
    }

    /// The word the status line names this side by.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            ViewSide::Diffed => "before + after",
            ViewSide::Before => "before only",
            ViewSide::After => "after only",
        }
    }

    /// Whether a line of this kind is shown on this side.
    #[must_use]
    fn shows(self, kind: LineKind) -> bool {
        match self {
            ViewSide::Diffed => true,
            ViewSide::Before => kind != LineKind::Added,
            ViewSide::After => kind != LineKind::Removed,
        }
    }

    /// `lines` with the kinds this side hides dropped. `Diffed` is the identity
    /// and returns the input untouched.
    #[must_use]
    pub fn filter(self, lines: Vec<DiffLine>) -> Vec<DiffLine> {
        if self == ViewSide::Diffed {
            return lines;
        }
        lines
            .into_iter()
            .filter(|line| self.shows(line.kind))
            .collect()
    }
}
