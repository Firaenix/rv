//! `g b`: back along the jumps that brought the cursor here.
//!
//! Every jump records where the cursor stood before it moved — `g d`, a
//! chosen reference or search hit, a picked symbol or file, a comment or flag
//! jump. A place is kept as code (path, side, line) as well as a row, so going
//! back lands on the same line even after the plan under it was rebuilt.

use anyhow::Result;

use super::App;
use super::Focus;
use super::anchor::SourcePosition;

/// How many places the stack holds before the oldest is forgotten.
const DEPTH: usize = 100;

#[derive(Clone, Debug, PartialEq)]
pub(in crate::app) struct Place {
    file: usize,
    position: Option<SourcePosition>,
    row: usize,
    column: usize,
}

#[derive(Default)]
pub(in crate::app) struct Jumps {
    places: Vec<Place>,
}

impl Jumps {
    fn push(&mut self, place: Place) {
        if self.places.last() == Some(&place) {
            return;
        }
        self.places.push(place);
        if self.places.len() > DEPTH {
            self.places.remove(0);
        }
    }

    pub(in crate::app) fn drop_newest(&mut self) {
        self.places.pop();
    }

    pub(in crate::app) fn is_empty(&self) -> bool {
        self.places.is_empty()
    }
}

impl App {
    /// How many places `g b` can go back through.
    #[must_use]
    pub fn jump_depth(&self) -> usize {
        self.jumps.places.len()
    }

    /// Called by every jump, before it moves the cursor.
    pub(super) fn record_jump(&mut self) {
        let place = Place {
            file: self.file_index,
            position: self.cursor_position(),
            row: self.cursor_row(),
            column: self.column,
        };
        self.jumps.push(place);
    }

    pub(super) fn jump_back(&mut self) -> Result<()> {
        let Some(place) = self.jumps.places.pop() else {
            self.status = "no jump to go back to".to_owned();
            return Ok(());
        };
        self.select_file(place.file)?;
        let line = place
            .position
            .as_ref()
            .and_then(|(path, side, number)| self.line_index_at_anchor(path, *side, *number));
        let row = line
            .and_then(|line| self.plan().row_of_line(line))
            .unwrap_or(place.row);
        self.set_cursor_row(row);
        self.set_column(place.column);
        self.focus = Focus::Diff;
        self.status = match place.position {
            Some((path, _, number)) => format!("back to {path}:{number}"),
            None => "back".to_owned(),
        };
        Ok(())
    }
}
