//! Putting a landed answer in place of the grammar's, or not.

use std::collections::HashMap;
use std::path::Path;
use std::path::PathBuf;

use rv_core::lsp::Answer;
use rv_core::lsp::Hover;
use rv_core::lsp::Location;
use rv_core::lsp::RequestKind;

use super::Question;
use super::Spot;
use super::hovertext::card_lines;
use crate::app::App;
use crate::app::JumpList;
use crate::app::JumpSource;
use crate::app::Mode;
use crate::app::Reference;

/// Target files bigger than this are listed without their line's text.
const MAX_QUOTED_FILE_BYTES: u64 = 4 * 1024 * 1024;

impl App {
    pub(in crate::app) fn apply_answer(
        &mut self,
        question: &Question,
        landing: &Spot,
        answer: Answer,
    ) {
        match (question.kind, answer) {
            (RequestKind::Hover, Answer::Hover(Some(hover))) => self.apply_hover(question, &hover),
            (RequestKind::Definition, Answer::Locations(found)) if !found.is_empty() => {
                self.apply_definition(question, landing, &found);
            }
            (RequestKind::References, Answer::Locations(found)) if !found.is_empty() => {
                self.apply_references(question, landing, &found);
            }
            _ => {}
        }
    }

    fn apply_hover(&mut self, question: &Question, hover: &Hover) {
        let lines = card_lines(&hover.text);
        let Some(open) = self.hover.as_mut() else {
            return;
        };
        if open.by_pointer || open.card.name != question.word || lines.is_empty() {
            return;
        }
        open.card.server_text = lines;
        open.card.source = JumpSource::LanguageServer;
    }

    fn apply_definition(&mut self, question: &Question, landing: &Spot, found: &[Location]) {
        let targets = self.located(found);
        let server = self.selected_server_name();
        let list_still_open = self.mode == Mode::References
            && self.jump_list == JumpList::Definitions
            && self.reference_word == question.word;
        if list_still_open || (targets.len() > 1 && self.unmoved_since(landing)) {
            self.close_hover();
            self.show_server_list(question, targets, JumpList::Definitions);
            return;
        }
        let target = &targets[0];
        let place = format!("{}:{}", self.reference_path(target), target.line);
        if !self.unmoved_since(landing) {
            self.status = format!("{server}: {} — {place}", question.word);
            return;
        }
        if target.elsewhere.is_some() {
            self.status = format!(
                "{} — {place} · {server}, outside this review",
                question.word
            );
            return;
        }
        let already_there =
            self.file_index == target.file && self.selected_line_number() == target.line;
        if !already_there {
            self.close_hover();
            self.reference_word.clone_from(&question.word);
            if let Err(error) = self.jump_to_reference(target) {
                self.raise(format!("could not open {place}: {error:#}"));
                return;
            }
            if *landing != question.origin {
                self.jumps.drop_newest();
            }
            self.set_column(found[0].column as usize);
        }
        self.navigation.list_source = JumpSource::LanguageServer;
        self.status = format!("{} — {place} · {server}", question.word);
    }

    fn apply_references(&mut self, question: &Question, landing: &Spot, found: &[Location]) {
        let list_still_open = self.mode == Mode::References
            && matches!(self.jump_list, JumpList::References | JumpList::Lines)
            && self.reference_word == question.word;
        let grammar_missed = self.mode == Mode::Browse && self.unmoved_since(landing);
        if !list_still_open && !grammar_missed {
            return;
        }
        self.close_hover();
        let targets = self.located(found);
        self.show_server_list(question, targets, JumpList::References);
    }

    fn unmoved_since(&self, landing: &Spot) -> bool {
        self.spot() == *landing
    }

    /// Replaces (or opens) the jump list with the server's answer, keeping the
    /// list cursor on the same place where the new list still has it.
    fn show_server_list(&mut self, question: &Question, targets: Vec<Reference>, list: JumpList) {
        let selected = (self.mode == Mode::References)
            .then(|| self.references.get(self.reference_index()))
            .flatten()
            .map(|reference| (reference.file, reference.line));
        let here = (question.file, question.line);
        self.reference_index = selected
            .and_then(|place| targets.iter().position(|r| (r.file, r.line) == place))
            .or_else(|| {
                targets
                    .iter()
                    .position(|r| r.elsewhere.is_none() && (r.file, r.line) > here)
            })
            .unwrap_or(0);
        self.references = targets;
        self.reference_word.clone_from(&question.word);
        self.jump_list = list;
        self.mode = Mode::References;
        self.navigation.list_source = JumpSource::LanguageServer;
        let what = if list == JumpList::Definitions {
            "definition"
        } else {
            "reference"
        };
        self.status = format!(
            "choose a {what}: ↑/↓, Enter to jump, Esc to cancel · {}",
            self.selected_server_name()
        );
    }

    /// The server's locations as list rows, each line quoted from disk (the
    /// server answered about the disk). Not openable unless the review lists
    /// the path.
    fn located(&self, found: &[Location]) -> Vec<Reference> {
        let root = self.review.store.root();
        let mut texts: HashMap<PathBuf, Option<String>> = HashMap::new();
        let mut targets: Vec<Reference> = Vec::new();
        for location in found {
            let file = self
                .review
                .files
                .iter()
                .position(|file| Path::new(&file.path) == location.path);
            let text = texts
                .entry(location.path.clone())
                .or_insert_with(|| read_small(&root.join(&location.path)))
                .as_deref()
                .and_then(|text| text.lines().nth(location.line.saturating_sub(1) as usize))
                .map(|line| line.trim().chars().filter(|c| !c.is_control()).collect())
                .unwrap_or_default();
            let target = Reference {
                file: file.unwrap_or(usize::MAX),
                line: location.line,
                text,
                kind: None,
                elsewhere: file.is_none().then(|| location.path.display().to_string()),
            };
            if !targets.iter().any(|seen| {
                seen.file == target.file
                    && seen.line == target.line
                    && seen.elsewhere == target.elsewhere
            }) {
                targets.push(target);
            }
        }
        targets
    }
}

fn read_small(path: &Path) -> Option<String> {
    let size = std::fs::metadata(path).ok()?.len();
    if size > MAX_QUOTED_FILE_BYTES {
        return None;
    }
    std::fs::read(path)
        .ok()
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
}
