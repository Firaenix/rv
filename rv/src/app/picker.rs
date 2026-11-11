//! `g o` and `g s`: the picker opened onto the review's files, and a text
//! search over every one of them.
//!
//! Both share [`Mode::Pick`] and its panel with the symbol picker; what the
//! query is matched against is [`Picking`]. The search runs on `Enter` rather
//! than per keystroke, because it reads every file's blob, and its hits open in
//! the same list `g r` uses.

use anyhow::Result;
use crossterm::event::KeyCode;

use super::Action;
use super::App;
use super::Focus;
use super::JumpList;
use super::Mode;
use super::Reference;
use super::search::matches_of;
use super::symbols::Scope;

/// The most hits a search lists; past this the query is too broad to choose
/// from anyway.
const MAX_HITS: usize = 500;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Picking {
    #[default]
    Symbol,
    File,
    Text,
}

impl App {
    #[must_use]
    pub fn picking(&self) -> Picking {
        self.picking
    }

    pub(super) fn begin_open_file(&mut self) {
        if self.review.files.is_empty() {
            self.status = "no files in this review".to_owned();
            return;
        }
        self.begin_query(
            Picking::File,
            "open a file: type part of its path, Enter to open, Esc to cancel",
        );
    }

    pub(super) fn begin_repo_search(&mut self) {
        if self.review.files.is_empty() {
            self.status = "no files in this review".to_owned();
            return;
        }
        self.begin_query(
            Picking::Text,
            "search every file: type, Enter to list the hits, Esc to cancel",
        );
    }

    fn begin_query(&mut self, picking: Picking, status: &str) {
        self.picking = picking;
        self.mode = Mode::Pick;
        self.buffer.clear();
        self.status = status.to_owned();
    }

    pub(super) fn on_key_picker(&mut self, key: KeyCode) -> Result<Action> {
        if self.picking == Picking::Symbol {
            return self.on_key_pick(key);
        }
        match key {
            KeyCode::Esc => {
                self.close_query();
                self.status = "search cancelled".to_owned();
            }
            KeyCode::Backspace => {
                self.buffer.pop();
            }
            KeyCode::Enter => {
                let picking = self.picking;
                let chosen = self.file_matches().first().copied();
                let typed = std::mem::take(&mut self.buffer);
                self.close_query();
                match picking {
                    Picking::File => self.open_picked(chosen)?,
                    Picking::Text | Picking::Symbol => self.search_repo(&typed),
                }
            }
            KeyCode::Char(character) => self.buffer.push(character),
            _ => {}
        }
        Ok(Action::Continue)
    }

    fn close_query(&mut self) {
        self.mode = Mode::Browse;
        self.buffer.clear();
        self.picking = Picking::Symbol;
    }

    fn open_picked(&mut self, chosen: Option<usize>) -> Result<()> {
        let Some(file) = chosen else {
            self.status = "no file matches".to_owned();
            return Ok(());
        };
        self.record_jump();
        self.select_file(file)?;
        self.focus = Focus::Diff;
        self.status = format!("opened {}", self.review.files[file].path);
        Ok(())
    }

    /// The files whose path holds the query, best first: a hit in the file's
    /// own name ahead of one in its directories, a substring ahead of letters
    /// merely in order, and shorter paths first within each.
    #[must_use]
    pub fn file_matches(&self) -> Vec<usize> {
        let query = self.buffer.to_lowercase();
        let mut ranked: Vec<(u8, usize, usize)> = self
            .review
            .files
            .iter()
            .enumerate()
            .filter_map(|(index, file)| {
                let path = file.path.to_lowercase();
                let name = path.rsplit('/').next().unwrap_or(&path);
                let tier = if name.contains(&query) {
                    0
                } else if path.contains(&query) {
                    1
                } else if in_order(&query, &path) {
                    2
                } else {
                    return None;
                };
                Some((tier, path.len(), index))
            })
            .collect();
        ranked.sort_unstable();
        ranked.into_iter().map(|(_, _, index)| index).collect()
    }

    /// Every line of every file in the review that holds `query`, as the list
    /// `g r` shows.
    fn search_repo(&mut self, query: &str) {
        if query.trim().is_empty() {
            self.status = super::status::HELP.to_owned();
            return;
        }
        let mut hits = Vec::new();
        for scoped in self.scoped_files(&self.whole_review()) {
            let file = scoped.file;
            let Some(blob) = self.read_indexable(scoped).blob else {
                continue;
            };
            if !crate::index::indexable_text(&blob) {
                continue;
            }
            for (number, line) in String::from_utf8_lossy(&blob).lines().enumerate() {
                if !matches_of(query, line).is_empty() {
                    hits.push(Reference {
                        file,
                        line: u32::try_from(number + 1).unwrap_or(u32::MAX),
                        text: line.trim().to_owned(),
                        kind: None,
                        elsewhere: None,
                    });
                }
            }
        }
        if hits.is_empty() {
            self.status = format!("no match for \"{query}\" in any file");
            return;
        }
        hits.truncate(MAX_HITS);
        self.references = hits;
        self.reference_index = 0;
        self.jump_list = JumpList::Matches;
        self.reference_word = query.to_owned();
        self.mode = Mode::References;
        self.status = "choose a hit: ↑/↓, Enter to jump, Esc to cancel".to_owned();
    }

    /// Every file the review lists, whichever tab the sidebar is on.
    pub(super) fn whole_review(&self) -> Scope {
        if self.review.is_tree() {
            Scope::Tree
        } else {
            Scope::Bookmark
        }
    }

    /// Where a jump out of the list leaves the column cursor: on the symbol,
    /// or on the first hit of a search's query.
    pub(super) fn put_column_on(&mut self, word: &str) {
        if self.jump_list != JumpList::Matches {
            self.set_column_to(word);
            return;
        }
        let Some(line) = self.selected_line() else {
            return;
        };
        if let Some(hit) = matches_of(word, &line.text).first() {
            self.set_column(hit.start);
        }
    }
}

fn in_order(query: &str, path: &str) -> bool {
    let mut rest = path.chars();
    query
        .chars()
        .all(|wanted| rest.by_ref().any(|found| found == wanted))
}
