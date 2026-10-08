//! `g r`: the references to the word under the cursor, as a list to choose
//! from.
//!
//! It used to walk them one `g r` at a time, which answers "show me the next
//! one" and never "show me where they all are". A reviewer asking about a
//! symbol is deciding which occurrence matters, so the occurrences are a list
//! with a cursor on it: the same panel the symbol picker uses, opened on the
//! reference after the one being read, `Enter` to jump and `Esc` to leave the
//! code where it was.

use anyhow::Result;

use super::Action;
use super::App;
use super::Focus;
use super::Mode;

/// One occurrence: where it is, the line it is on so the list can be read
/// without jumping to every row in it, and what the grammar called it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reference {
    pub file: usize,
    pub line: u32,
    pub text: String,
    /// What the grammar says this use is, a call or a type mention, or `None`
    /// where the list fell back to matching the name as text.
    pub kind: Option<rv_core::symbols::SymbolKind>,
}

impl App {
    /// `g r`: open the list, with the cursor on the first reference past the
    /// one under the review cursor, which is the one the old walk would have
    /// taken.
    pub(super) fn begin_references(&mut self) -> Result<()> {
        let Some(word) = self.symbol_under_cursor() else {
            self.status = "no symbol under the cursor".to_owned();
            return Ok(());
        };
        let (references, syntactic) = self.references_to(&word);
        if references.is_empty() {
            self.status = format!("no reference to {word} in this review");
            return Ok(());
        }
        self.references_syntactic = syntactic;
        let here = (self.file_index, self.selected_line_number());
        self.reference_index = references
            .iter()
            .position(|r| (r.file, r.line) > here)
            .unwrap_or(0);
        self.references = references;
        self.reference_word = word;
        self.mode = Mode::References;
        self.status = "choose a reference: ↑/↓, Enter to jump, Esc to cancel".to_owned();
        Ok(())
    }

    /// The list as drawn, and which row the cursor is on.
    #[must_use]
    pub fn references(&self) -> &[Reference] {
        &self.references
    }

    #[must_use]
    pub fn reference_index(&self) -> usize {
        self.reference_index
            .min(self.references.len().saturating_sub(1))
    }

    #[must_use]
    pub fn reference_word(&self) -> &str {
        &self.reference_word
    }

    pub(super) fn on_key_references(&mut self, key: crossterm::event::KeyCode) -> Result<Action> {
        use crossterm::event::KeyCode;
        match key {
            KeyCode::Esc => self.close_references("search cancelled"),
            KeyCode::Up => self.step_reference(-1),
            KeyCode::Down => self.step_reference(1),
            KeyCode::Home => self.reference_index = 0,
            KeyCode::End => self.reference_index = self.references.len().saturating_sub(1),
            KeyCode::Enter => {
                let chosen = self.references.get(self.reference_index()).cloned();
                self.close_references(super::status::HELP);
                if let Some(reference) = chosen {
                    self.jump_to_reference(&reference)?;
                }
            }
            _ => {}
        }
        Ok(Action::Continue)
    }

    fn step_reference(&mut self, delta: isize) {
        let last = self.references.len().saturating_sub(1);
        let at = self.reference_index();
        // Wrapping, because the list is a ring of the same word's occurrences
        // and either end is a place a reviewer walks off by accident.
        self.reference_index = match delta {
            d if d < 0 && at == 0 => last,
            d if d < 0 => at - 1,
            _ if at >= last => 0,
            _ => at + 1,
        };
    }

    fn close_references(&mut self, status: &str) {
        self.mode = Mode::Browse;
        self.references.clear();
        self.reference_index = 0;
        self.status = status.to_owned();
    }

    /// Lands on `reference` and puts the column cursor back on the word, so a
    /// jump out of the list leaves the review pointing at the same symbol.
    fn jump_to_reference(&mut self, reference: &Reference) -> Result<()> {
        self.select_file(reference.file)?;
        let found = self.displayed_lines().iter().position(|shown| {
            shown.right == Some(reference.line) || shown.left == Some(reference.line)
        });
        match found {
            Some(index) => {
                let row = self.plan().row_of_line(index).unwrap_or(0);
                self.set_cursor_row(row);
            }
            None => self.set_cursor_row(0),
        }
        self.focus = Focus::Diff;
        let word = std::mem::take(&mut self.reference_word);
        self.set_column_to(&word);
        self.reference_word = word;
        self.status = format!(
            "{}: {}:{}",
            self.reference_word, self.review.files[reference.file].path, reference.line
        );
        Ok(())
    }

    /// Whether the list came from the grammar, which is the difference between
    /// "every use of this symbol" and "every line that spells this word".
    #[must_use]
    pub fn references_are_syntactic(&self) -> bool {
        self.references_syntactic
    }

    /// The uses of `name`, and whether the grammar found them.
    ///
    /// The index answers first, and those are real uses: a word in a comment,
    /// in a string, or in a language with no grammar is not one. Only when the
    /// grammar found nothing at all does the text scan answer, because a
    /// reviewer reading a shell script still wants to know where else a name
    /// appears, and an empty list would read as "nowhere" rather than as "rv
    /// cannot parse this".
    fn references_to(&mut self, name: &str) -> (Vec<Reference>, bool) {
        let places: Vec<(usize, u32, rv_core::symbols::SymbolKind)> = self
            .index()
            .uses_named(name)
            .into_iter()
            .map(|use_| (use_.file, use_.reference.line, use_.reference.kind))
            .collect();
        let mut found: Vec<Reference> = places
            .into_iter()
            .map(|(file, line, kind)| Reference {
                file,
                line,
                text: self.source_line(file, line),
                kind: Some(kind),
            })
            .collect();
        if !found.is_empty() {
            found.sort_by_key(|reference| (reference.file, reference.line));
            found.dedup_by_key(|reference| (reference.file, reference.line));
            return (found, true);
        }
        (self.lines_naming(name), false)
    }

    /// The text of one line of a file's indexed blob, for a list row to show.
    fn source_line(&self, file: usize, line: u32) -> String {
        let scope = self.scope();
        let Some(scoped) = self
            .scoped_files(&scope)
            .into_iter()
            .find(|scoped| scoped.file == file)
        else {
            return String::new();
        };
        let Some(blob) = self.read_indexable(scoped).blob else {
            return String::new();
        };
        String::from_utf8_lossy(&blob)
            .lines()
            .nth(line.saturating_sub(1) as usize)
            .unwrap_or_default()
            .trim()
            .to_owned()
    }

    /// Every whole-word occurrence of `word` in the scope's files, by file then
    /// line. The fallback for a file no grammar claims.
    fn lines_naming(&self, word: &str) -> Vec<Reference> {
        let scope = self.scope();
        let mut found = Vec::new();
        for file in self.scoped_files(&scope) {
            let index = file.file;
            let Some(blob) = self.read_indexable(file).blob else {
                continue;
            };
            let text = String::from_utf8_lossy(&blob);
            for (number, line) in text.lines().enumerate() {
                let hit = super::column::words(line).into_iter().any(|range| {
                    line.chars()
                        .skip(range.start)
                        .take(range.len())
                        .eq(word.chars())
                });
                if hit {
                    found.push(Reference {
                        file: index,
                        line: u32::try_from(number + 1).unwrap_or(u32::MAX),
                        text: line.trim().to_owned(),
                        kind: None,
                    });
                }
            }
        }
        found.sort_unstable_by_key(|r| (r.file, r.line));
        found
    }
}
