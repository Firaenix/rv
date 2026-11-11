//! A hover card raised by the pointer resting on a name, when `Config.toml`
//! asks for it (`hover = true`).
//!
//! Clock-free like the rest of the state machine: a motion only records where
//! the pointer is, and [`App::dwell`] — handed the time by the event loop —
//! stamps the first pass that sees it and opens the card once the pointer has
//! stayed put for [`DWELL`]. Motion reporting is terminal-dependent, so `g i`
//! stays the primary way in.

use std::time::Duration;
use std::time::Instant;

use super::App;
use super::HelpStage;
use super::Mode;
use super::column::words;
use super::hover::Hover;
use crate::layout::Target;
use crate::layout::hit;
use crate::rows::Row;
use crate::ui;

/// How long the pointer rests on a name before its card opens.
const DWELL: Duration = Duration::from_millis(400);

#[derive(Default)]
pub(in crate::app) struct Dwell {
    at: Option<(u16, u16)>,
    since: Option<Instant>,
}

impl App {
    /// The pointer moved to `column`, `row`.
    pub(super) fn pointer_moved(&mut self, column: u16, row: u16) {
        if !self.view.hover_on_pointer || !self.may_hover() {
            self.resting = Dwell::default();
            return;
        }
        if self.resting.at == Some((column, row)) {
            return;
        }
        let over = hit(&self.painted.get(), column, row);
        if over == Some(Target::Hover) {
            return;
        }
        if self.hover.as_ref().is_some_and(|hover| hover.by_pointer) {
            self.hover = None;
        }
        self.resting = Dwell {
            at: matches!(over, Some(Target::DiffRow(_))).then_some((column, row)),
            since: None,
        };
    }

    /// Opens the card the pointer has rested long enough to ask for.
    pub fn dwell(&mut self, now: Instant) {
        let Some(at) = self.resting.at else {
            return;
        };
        if !self.may_hover() {
            self.resting = Dwell::default();
            return;
        }
        match self.resting.since {
            None => self.resting.since = Some(now),
            Some(since) if now.saturating_duration_since(since) >= DWELL => {
                self.resting = Dwell::default();
                self.hover_at(at.0, at.1);
            }
            Some(_) => {}
        }
    }

    /// How long the event loop may sleep before a resting pointer is due.
    #[must_use]
    pub fn dwell_deadline(&self, now: Instant) -> Option<Duration> {
        self.resting.at?;
        Some(
            self.resting
                .since
                .map_or(DWELL, |since| DWELL.saturating_sub(now - since)),
        )
    }

    /// Never while text is being written, a menu is waiting for its key, or
    /// the keymap is up.
    fn may_hover(&self) -> bool {
        self.mode == Mode::Browse && self.help == HelpStage::Closed && self.pending_leader.is_none()
    }

    fn hover_at(&mut self, column: u16, row: u16) {
        let painted = self.painted.get();
        let Some(Target::DiffRow(screen_row)) = hit(&painted, column, row) else {
            return;
        };
        let Some(plan_row) = ui::diff_row_at(self, painted.diff, screen_row) else {
            return;
        };
        let line = match self.plan().rows.get(plan_row) {
            Some(Row::Diff { line, .. }) => line.clone(),
            _ => return,
        };
        let at = ui::diff_column_at(self, painted.diff, column);
        let Some(range) = words(&line.text)
            .into_iter()
            .find(|range| range.contains(&at))
        else {
            return;
        };
        let Some(number) = line.right.or(line.left) else {
            return;
        };
        let word: String = line
            .text
            .chars()
            .skip(range.start)
            .take(range.len())
            .collect();
        let file = self.file_index;
        let start = u32::try_from(range.start).unwrap_or(u32::MAX);
        let named = self
            .index()
            .use_at(file, number, start)
            .map(|found| (found.reference.name.clone(), found.reference.kind));
        let (name, wanted) = named.map_or((word, None), |(name, kind)| (name, Some(kind)));
        let card = self.card_for(&name, wanted);
        self.hover = Some(Hover {
            card,
            row: u16::try_from(screen_row).unwrap_or(u16::MAX),
            by_pointer: true,
        });
    }
}
