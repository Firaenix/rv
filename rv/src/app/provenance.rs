//! `v p` and `i`: which change last touched each head-side line.
//!
//! Annotating walks the file's history, so it runs on [`worker`]'s thread and
//! lands through [`App::collect_provenance`] on the event loop. Until it does,
//! the gutter says so and `i` waits for it rather than for the reviewer.

use std::collections::HashMap;
use std::collections::HashSet;

use rv_core::diff::DiffLine;
use rv_core::diff::LineKind;
use rv_core::vcs::Annotation;

use super::App;
use super::bindings::ProvenanceCommand;
use super::hover::Hover;
use crate::ui;

mod card;
mod worker;

/// A file at a commit: `(head commit, path)`.
type BlobKey = (String, String);

/// The fixed prefix of a change id the gutter and the card show.
pub const SHORT_ID_CHARS: usize = 8;

struct Annotated {
    annotation: Annotation,
    as_of_secs: i64,
}

/// Carried whole across a refresh: a file at a commit never changes, so
/// neither does who wrote it.
#[derive(Default)]
pub(in crate::app) struct Provenance {
    /// `None` where there is nothing to annotate, or the walk failed.
    annotations: HashMap<BlobKey, Option<Annotated>>,
    asked: HashSet<BlobKey>,
    /// The line `i` asked about before its file's annotation had landed.
    awaited: Option<(BlobKey, u32)>,
    worker: worker::Annotator,
}

/// What the gutter has to draw from for the file on screen.
pub enum Gutter<'a> {
    Pending,
    Unavailable,
    Ready(&'a Annotation),
}

/// The head-side line number a diff line's provenance is looked up at —
/// `None` for a removed line, which no longer exists at the head.
#[must_use]
pub fn head_line(line: &DiffLine) -> Option<u32> {
    match line.kind {
        LineKind::Removed => None,
        LineKind::Added | LineKind::Context => line.right,
    }
}

#[must_use]
pub fn short_change_id(change_id: &str) -> &str {
    change_id
        .char_indices()
        .nth(SHORT_ID_CHARS)
        .map_or(change_id, |(end, _)| &change_id[..end])
}

impl App {
    pub(in crate::app) fn run_provenance(&mut self, command: ProvenanceCommand) {
        match command {
            ProvenanceCommand::ToggleGutter => self.toggle_provenance(),
            ProvenanceCommand::ShowOrigin => self.show_origin(),
        }
    }

    #[must_use]
    pub fn provenance_on(&self) -> bool {
        self.view.provenance
    }

    /// `None` with the gutter off. Looked up once per frame, never per row.
    #[must_use]
    pub fn provenance_gutter(&self) -> Option<Gutter<'_>> {
        if !self.view.provenance {
            return None;
        }
        let Some(key) = self.shown_head() else {
            return Some(Gutter::Unavailable);
        };
        Some(match self.provenance.annotations.get(&key) {
            Some(Some(annotated)) => Gutter::Ready(&annotated.annotation),
            Some(None) => Gutter::Unavailable,
            None => Gutter::Pending,
        })
    }

    /// Whether a change is one of the changes this review covers.
    #[must_use]
    pub fn change_in_review(&self, change_id: &str) -> bool {
        self.review
            .session
            .changes
            .iter()
            .any(|change| change.change_id == change_id)
    }

    /// Takes every annotation that has landed, first asking for the shown
    /// file's where the gutter wants it. Never blocks.
    pub fn collect_provenance(&mut self) -> bool {
        self.ask_for_wanted();
        let mut arrived = false;
        while let Ok(landed) = self.provenance.worker.results.try_recv() {
            self.land_annotation(landed);
            arrived = true;
        }
        arrived
    }

    /// Blocks until every asked-for annotation has landed — for a caller that
    /// wants to look at the finished frame, never the event loop.
    pub fn finish_provenance(&mut self) {
        self.ask_for_wanted();
        while self.provenance_busy() {
            match self.provenance.worker.results.recv() {
                Ok(landed) => self.land_annotation(landed),
                Err(_) => return,
            }
        }
    }

    #[must_use]
    pub fn provenance_busy(&self) -> bool {
        !self.provenance.asked.is_empty()
    }

    fn toggle_provenance(&mut self) {
        self.view.provenance = !self.view.provenance;
        self.status = if self.view.provenance {
            "provenance gutter on".to_owned()
        } else {
            "provenance gutter off".to_owned()
        };
        self.ask_for_wanted();
    }

    fn show_origin(&mut self) {
        let Some(line) = self.selected_line() else {
            self.status = "no line under the cursor".to_owned();
            return;
        };
        let Some(number) = head_line(&line) else {
            self.status = "a removed line has no origin at the head".to_owned();
            return;
        };
        let Some(key) = self.shown_head() else {
            return;
        };
        match self.provenance.annotations.get(&key) {
            Some(Some(_)) => self.open_origin_card(&key, number),
            Some(None) => self.status = no_provenance(&key),
            None => {
                self.status = format!("tracing who wrote {}…", key.1);
                self.provenance.awaited = Some((key, number));
                self.ask_for_wanted();
            }
        }
    }

    /// The head commit and path the diff on screen was read from.
    fn shown_head(&self) -> Option<BlobKey> {
        let path = self.selected_file()?.path.clone();
        Some((self.shown_endpoints().1, path))
    }

    fn ask_for_wanted(&mut self) {
        if let Some((key, _)) = self.provenance.awaited.clone() {
            self.request_annotation(key);
        }
        if self.view.provenance
            && let Some(key) = self.shown_head()
        {
            self.request_annotation(key);
        }
    }

    fn request_annotation(&mut self, key: BlobKey) {
        let provenance = &mut self.provenance;
        if provenance.annotations.contains_key(&key) || provenance.asked.contains(&key) {
            return;
        }
        provenance.asked.insert(key.clone());
        let root = self.review.store.root().to_owned();
        if let Some(dropped) = self.provenance.worker.submit(key, root) {
            self.provenance.asked.remove(&dropped);
        }
    }

    fn land_annotation(&mut self, landed: worker::Landed) {
        let key = landed.key;
        self.provenance.asked.remove(&key);
        let annotated = match landed.outcome {
            Ok(annotation) => annotation.map(|annotation| Annotated {
                annotation,
                as_of_secs: landed.as_of_secs,
            }),
            Err(error) => {
                tracing::warn!(path = %key.1, %error, "could not annotate");
                None
            }
        };
        let found = annotated.is_some();
        self.provenance.annotations.insert(key.clone(), annotated);

        let Some((awaited, number)) = self
            .provenance
            .awaited
            .take_if(|(awaited, _)| *awaited == key)
        else {
            return;
        };
        let still_there = self.shown_head().as_ref() == Some(&awaited)
            && self.selected_line().as_ref().and_then(head_line) == Some(number);
        if !still_there {
            return;
        }
        if found {
            self.open_origin_card(&awaited, number);
        } else {
            self.status = no_provenance(&awaited);
        }
    }

    fn open_origin_card(&mut self, key: &BlobKey, number: u32) {
        let Some(Some(annotated)) = self.provenance.annotations.get(key) else {
            return;
        };
        let Some(origin) = annotated.annotation.origin_of_line(number as usize) else {
            self.status = format!("line {number} is past the end of {}", key.1);
            return;
        };
        let card = card::origin_card(origin, number, annotated.as_of_secs);
        let pane = self.painted.get().diff;
        let row = ui::diff_screen_row(self, pane, self.cursor_row()).unwrap_or(0);
        self.hover = Some(Hover {
            card,
            row: u16::try_from(row).unwrap_or(u16::MAX),
            by_pointer: false,
        });
    }
}

fn no_provenance(key: &BlobKey) -> String {
    format!(
        "no provenance for {} — binary, or not a file at the head",
        key.1
    )
}
