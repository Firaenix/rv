//! The column cursor: one word of the selected line, so a line with several
//! symbols on it can say which one `g d` and `g r` are about.
//!
//! A character offset into the selected line's text, clamped when read
//! rather than when the line changes: the row cursor moves far more often
//! than the column matters, and a stale offset costs one clamp.

use std::ops::Range;

use anyhow::Result;

use super::App;
use super::Focus;

fn is_word(character: char) -> bool {
    character.is_alphanumeric() || character == '_'
}

/// The words of `text`, as character ranges.
fn words(text: &str) -> Vec<Range<usize>> {
    let mut ranges = Vec::new();
    let mut start = None;
    for (at, character) in text.chars().enumerate() {
        match (is_word(character), start) {
            (true, None) => start = Some(at),
            (false, Some(from)) => {
                ranges.push(from..at);
                start = None;
            }
            _ => {}
        }
    }
    if let Some(from) = start {
        ranges.push(from..text.chars().count());
    }
    ranges
}

impl App {
    /// The column cursor, clamped to the selected line.
    pub fn column(&self) -> usize {
        let length = self
            .selected_line()
            .map_or(0, |line| line.text.chars().count());
        self.column.min(length.saturating_sub(1))
    }

    pub(super) fn set_column(&mut self, column: usize) {
        self.column = column;
    }

    /// The word the column cursor is in, or the next one along the line.
    pub fn word_range(&self) -> Option<Range<usize>> {
        let line = self.selected_line()?;
        let column = self.column();
        words(&line.text)
            .into_iter()
            .find(|range| range.end > column)
    }

    pub fn word_under_cursor(&self) -> Option<String> {
        let line = self.selected_line()?;
        let range = self.word_range()?;
        Some(
            line.text
                .chars()
                .skip(range.start)
                .take(range.len())
                .collect(),
        )
    }

    /// `h`/`l`: to the start of the previous or next word — relative to the
    /// word the cursor reads as being on, not the raw column: a cursor before
    /// the first word already *shows* that word, so a step right must reach
    /// the one after it.
    pub(super) fn move_word(&mut self, forward: bool) {
        let Some(line) = self.selected_line() else {
            return;
        };
        let current = self.word_range();
        let column = self.column();
        let words = words(&line.text);
        let target = if forward {
            let after = current.map_or(column + 1, |range| range.end);
            words.iter().find(|range| range.start >= after)
        } else {
            let before = current
                .filter(|range| range.start <= column)
                .map_or(column, |range| range.start);
            words.iter().rev().find(|range| range.start < before)
        };
        if let Some(range) = target {
            self.column = range.start;
        }
    }

    /// `g d`: to the definition of the word under the cursor — the next one
    /// on from here when the review defines it more than once.
    pub(super) fn goto_definition(&mut self) -> Result<()> {
        let Some(word) = self.word_under_cursor() else {
            self.status = "no word under the cursor".to_owned();
            return Ok(());
        };
        let here = (self.file_index, self.selected_line_number());
        let entries: Vec<crate::index::Entry> = self
            .index()
            .entries()
            .iter()
            .filter(|entry| entry.symbol.name == word)
            .cloned()
            .collect();
        if entries.is_empty() {
            self.status = format!("no definition of {word} in this review");
            return Ok(());
        }
        let next = entries
            .iter()
            .find(|entry| (entry.file, entry.symbol.line) > here)
            .unwrap_or(&entries[0]);
        self.jump_to_symbol(next)?;
        self.set_column_to(&word);
        Ok(())
    }

    /// `g r`: to the next place the word under the cursor appears, across
    /// every file in scope, wrapping.
    pub(super) fn goto_reference(&mut self) -> Result<()> {
        let Some(word) = self.word_under_cursor() else {
            self.status = "no word under the cursor".to_owned();
            return Ok(());
        };
        let references = self.references_of(&word);
        if references.is_empty() {
            self.status = format!("no other reference to {word}");
            return Ok(());
        }
        let here = (self.file_index, self.selected_line_number());
        let position = references
            .iter()
            .position(|&(file, line)| (file, line) > here)
            .unwrap_or(0);
        let (file, line) = references[position];
        self.select_file(file)?;
        let found = self
            .displayed_lines()
            .iter()
            .position(|shown| shown.right == Some(line) || shown.left == Some(line));
        match found {
            Some(index) => {
                let row = self.plan().row_of_line(index).unwrap_or(0);
                self.set_cursor_row(row);
            }
            None => self.set_cursor_row(0),
        }
        self.focus = Focus::Diff;
        self.set_column_to(&word);
        self.status = format!(
            "{word}: reference {} of {} — {}:{line}",
            position + 1,
            references.len(),
            self.review.files[file].path
        );
        Ok(())
    }

    /// Every whole-word occurrence of `word` in the scope's files, by file
    /// then line — read from the blobs, so a reference in a line this change
    /// did not touch still counts.
    fn references_of(&self, word: &str) -> Vec<(usize, u32)> {
        let scope = self.scope();
        let mut found = Vec::new();
        for file in self.scoped_files(&scope) {
            let index = file.file;
            let Some(blob) = self.read_indexable(file).blob else {
                continue;
            };
            let text = String::from_utf8_lossy(&blob);
            for (number, line) in text.lines().enumerate() {
                let hit = words(line).into_iter().any(|range| {
                    line.chars()
                        .skip(range.start)
                        .take(range.len())
                        .eq(word.chars())
                });
                if hit {
                    found.push((index, u32::try_from(number + 1).unwrap_or(u32::MAX)));
                }
            }
        }
        found.sort_unstable();
        found
    }

    fn selected_line_number(&self) -> u32 {
        self.selected_line()
            .and_then(|line| line.right.or(line.left))
            .unwrap_or(0)
    }

    /// Puts the column on `word` in the line just landed on, so a chain of
    /// jumps keeps pointing at the same symbol.
    fn set_column_to(&mut self, word: &str) {
        let Some(line) = self.selected_line() else {
            return;
        };
        if let Some(range) = words(&line.text).into_iter().find(|range| {
            line.text
                .chars()
                .skip(range.start)
                .take(range.len())
                .eq(word.chars())
        }) {
            self.column = range.start;
        }
    }
}
