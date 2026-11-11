//! The provenance gutter: the change that last touched each head-side line,
//! named on the first line of each run and left blank down the rest of it.
//!
//! Drawn ahead of the line number, after the code row is finished and
//! emphasized, so the row's own column math is untouched; the pointer's is
//! shifted by [`columns`].

use ratatui::style::Color;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
use rv_core::diff::DiffLine;

use crate::app::App;
use crate::app::provenance::Gutter;
use crate::app::provenance::SHORT_ID_CHARS;
use crate::app::provenance::head_line;
use crate::app::provenance::short_change_id;
use crate::theme;

const COLUMNS: usize = SHORT_ID_CHARS + 1;

/// How many columns the gutter takes ahead of every code row.
pub(super) fn columns(app: &App) -> usize {
    if app.provenance_on() { COLUMNS } else { 0 }
}

/// One frame's gutter, walked down the rows on screen in order.
pub(super) struct Rows<'a> {
    gutter: Gutter<'a>,
    /// Per origin, whether its change is one this review covers — those are
    /// drawn bright, everything older dim.
    in_review: Vec<bool>,
    /// The previous code row's head line and origin, which is what decides
    /// whether this row continues a run.
    previous: Option<(u32, usize)>,
}

impl<'a> Rows<'a> {
    pub(super) fn of(app: &'a App) -> Option<Self> {
        let gutter = app.provenance_gutter()?;
        let in_review = match &gutter {
            Gutter::Ready(annotation) => annotation
                .origins
                .iter()
                .map(|origin| app.change_in_review(&origin.change_id))
                .collect(),
            Gutter::Pending | Gutter::Unavailable => Vec::new(),
        };
        Some(Self {
            gutter,
            in_review,
            previous: None,
        })
    }

    /// `row`, the code row drawn for `line`, with its gutter cell in front.
    pub(super) fn prefixed(&mut self, line: &DiffLine, mut row: Line<'static>) -> Line<'static> {
        row.spans.insert(0, self.cell(line));
        row
    }

    fn cell(&mut self, line: &DiffLine) -> Span<'static> {
        let blank = || Span::raw(" ".repeat(COLUMNS));
        let Some(number) = head_line(line) else {
            self.previous = None;
            return blank();
        };
        let annotation = match &self.gutter {
            Gutter::Ready(annotation) => annotation,
            Gutter::Pending => {
                return Span::styled(
                    format!("{:<COLUMNS$}", "…"),
                    Style::default().add_modifier(Modifier::DIM),
                );
            }
            Gutter::Unavailable => return blank(),
        };
        let found = (number as usize)
            .checked_sub(1)
            .and_then(|index| annotation.line_origins.get(index))
            .and_then(|&origin| Some((origin, annotation.origins.get(origin)?)));
        let Some((origin, change)) = found else {
            self.previous = None;
            return blank();
        };
        let continues = self.previous == Some((number.wrapping_sub(1), origin));
        self.previous = Some((number, origin));
        if continues {
            return blank();
        }
        let id = short_change_id(&change.change_id);
        let colour = if self.in_review.get(origin).copied().unwrap_or(false) {
            theme::HASH
        } else {
            Color::DarkGray
        };
        Span::styled(format!("{id:<COLUMNS$}"), Style::default().fg(colour))
    }
}
