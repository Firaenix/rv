//! `/`: plain-text search in the shown diff, and `n`/`N` to walk the matches.
//!
//! Dumb on purpose — a substring, no regex — so that a reviewer who has lost
//! their place can type three letters they remember and land on them. Case
//! follows the query: all lower-case matches any case, a capital anywhere
//! makes it exact. The query outlives the mode, so `n` works after `Esc`.

use std::ops::Range;

use anyhow::Result;
use crossterm::event::KeyCode;

use super::Action;
use super::App;
use super::Focus;
use super::Mode;

impl App {
    pub fn query(&self) -> &str {
        &self.query
    }

    pub(super) fn has_query(&self) -> bool {
        !self.query.is_empty()
    }

    pub(super) fn begin_search(&mut self) {
        if self.selected_diff().is_none() {
            self.status = "no diff to search".to_owned();
            return;
        }
        self.mode = Mode::Search;
        self.buffer.clear();
    }

    pub(super) fn on_key_search(&mut self, key: KeyCode) -> Result<Action> {
        match key {
            KeyCode::Esc => {
                self.mode = Mode::Browse;
                self.buffer.clear();
                self.status = super::status::HELP.to_owned();
            }
            KeyCode::Backspace => {
                self.buffer.pop();
            }
            KeyCode::Enter => {
                let typed = std::mem::take(&mut self.buffer);
                self.mode = Mode::Browse;
                if !typed.trim().is_empty() {
                    self.query = typed;
                    self.focus = Focus::Diff;
                    self.jump_match_from(true, true)?;
                }
            }
            KeyCode::Char(character) => self.buffer.push(character),
            _ => {}
        }
        Ok(Action::Continue)
    }

    /// Where `query` occurs in `text`, as character ranges.
    pub fn matches_in(&self, text: &str) -> Vec<Range<usize>> {
        matches_of(&self.query, text)
    }

    pub(super) fn jump_match(&mut self, forward: bool) -> Result<()> {
        self.jump_match_from(forward, false)
    }

    /// To the next match after the column cursor — or, `here` set, at it —
    /// wrapping round the file.
    fn jump_match_from(&mut self, forward: bool, here: bool) -> Result<()> {
        if self.query.is_empty() {
            self.status = "no search yet — / to start one".to_owned();
            return Ok(());
        }
        let lines = self.displayed_lines();
        let mut found: Vec<(usize, usize)> = Vec::new();
        for (index, line) in lines.iter().enumerate() {
            for range in matches_of(&self.query, &line.text) {
                found.push((index, range.start));
            }
        }
        if found.is_empty() {
            self.status = format!("no match for \"{}\" in this file", self.query);
            return Ok(());
        }
        let line = self.line_index();
        let column = self.column();
        let position = if forward {
            found
                .iter()
                .position(|&(at, start)| {
                    at > line || (at == line && (start > column || (here && start >= column)))
                })
                .unwrap_or(0)
        } else {
            found
                .iter()
                .rposition(|&(at, start)| at < line || (at == line && start < column))
                .unwrap_or(found.len() - 1)
        };
        let (target_line, start) = found[position];
        let row = self.plan().row_of_line(target_line).unwrap_or(0);
        self.set_cursor_row(row);
        self.set_column(start);
        self.status = format!(
            "match {} of {} for \"{}\"",
            position + 1,
            found.len(),
            self.query
        );
        Ok(())
    }
}

/// Every occurrence of `query` in `text` as character ranges; case-blind
/// unless the query carries a capital.
pub fn matches_of(query: &str, text: &str) -> Vec<Range<usize>> {
    if query.is_empty() {
        return Vec::new();
    }
    let exact = query.chars().any(char::is_uppercase);
    let haystack: Vec<char> = if exact {
        text.chars().collect()
    } else {
        text.chars().flat_map(char::to_lowercase).collect()
    };
    let needle: Vec<char> = if exact {
        query.chars().collect()
    } else {
        query.chars().flat_map(char::to_lowercase).collect()
    };
    if needle.len() > haystack.len() {
        return Vec::new();
    }
    let mut found = Vec::new();
    let mut at = 0;
    while at + needle.len() <= haystack.len() {
        if haystack[at..at + needle.len()] == needle[..] {
            found.push(at..at + needle.len());
            at += needle.len();
        } else {
            at += 1;
        }
    }
    found
}
