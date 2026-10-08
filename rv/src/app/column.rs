//! The column cursor: one word of the selected line, so a line with several
//! symbols on it can say which one `g d` and `g r` are about.
//!
//! A character offset into the selected line's text, clamped when read
//! rather than when the line changes: the row cursor moves far more often
//! than the column matters, and a stale offset costs one clamp.

use std::ops::Range;

use anyhow::Result;
use rv_core::diff::DiffLine;

use super::App;

fn is_word(character: char) -> bool {
    character.is_alphanumeric() || character == '_'
}

/// The words of `text`, as character ranges.
pub(super) fn words(text: &str) -> Vec<Range<usize>> {
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
        self.selected_line().map_or(0, |line| self.column_in(&line))
    }

    /// The column cursor clamped to `line` — the selected line, handed in by
    /// a caller that already has it, so the plan is not rebuilt to find it.
    pub fn column_in(&self, line: &DiffLine) -> usize {
        let length = line.text.chars().count();
        self.column.min(length.saturating_sub(1))
    }

    pub(super) fn set_column(&mut self, column: usize) {
        self.column = column;
    }

    /// The word the column cursor is in, or the next one along the line.
    pub fn word_range(&self) -> Option<Range<usize>> {
        let line = self.selected_line()?;
        self.word_range_in(&line)
    }

    /// The same, on a `line` the caller already holds.
    pub fn word_range_in(&self, line: &DiffLine) -> Option<Range<usize>> {
        let column = self.column_in(line);
        words(&line.text)
            .into_iter()
            .find(|range| range.end > column)
    }

    /// The symbol a jump should follow: the name of the use the cursor sits in,
    /// as the grammar sees it, and the bare word only where the grammar has
    /// nothing to say about this file.
    ///
    /// The difference shows on `self.foo.bar()` and on a word in a comment: the
    /// first resolves to the name the parse found at that column, and the
    /// second to nothing at all rather than to every line that spells it.
    #[must_use]
    pub fn symbol_under_cursor(&mut self) -> Option<String> {
        let word = self.word_under_cursor()?;
        let line = self.selected_line_number();
        // The word's own column, not the cursor's: the cursor reads as being on
        // the next word along when it sits in the whitespace before it, and the
        // grammar records where the name is.
        let column = self
            .word_range()
            .map_or(0, |range| u32::try_from(range.start).unwrap_or(u32::MAX));
        let file = self.file_index;
        // The use the grammar put there wins over the characters under the
        // cursor: on `self.write()` the name to follow is `write`. Where the
        // grammar found nothing, the word stands, which is what keeps a file no
        // grammar claims navigable.
        self.index()
            .use_at(file, line, column)
            .map(|use_| use_.reference.name.clone())
            .or(Some(word))
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

    /// `←`/`→` inside the diff: one character along the selected line.
    ///
    /// Answers whether it moved, so `←` on the first character can mean "leave
    /// the pane" instead — the column cursor and the pane cursor share one key
    /// without either stealing it.
    pub(super) fn step_column(&mut self, forward: bool) -> bool {
        let Some(line) = self.selected_line() else {
            return false;
        };
        let column = self.column_in(&line);
        let last = line.text.chars().count().saturating_sub(1);
        let target = if forward {
            column.saturating_add(1).min(last)
        } else {
            column.saturating_sub(1)
        };
        if target == column {
            return false;
        }
        self.set_column(target);
        true
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
        let Some(word) = self.symbol_under_cursor() else {
            self.status = "no symbol under the cursor".to_owned();
            return Ok(());
        };
        let wanted = self.use_kind_under_cursor();
        let entries = self.definitions_of(&word, wanted);
        match entries.as_slice() {
            [] => {
                self.status = format!("no definition of {word} in this review");
                Ok(())
            }
            // One answer is a jump. Asking would be a keystroke spent on a
            // question with one option.
            [only] => {
                let only = only.clone();
                self.jump_to_symbol(&only)?;
                self.set_column_to(&word);
                Ok(())
            }
            // Several names match and nothing here can tell them apart: tags is
            // an index, not a type checker. Walking them one `g d` at a time
            // hid that, so the list says it instead and the reviewer decides.
            _ => {
                self.choose_definition(&word, &entries);
                Ok(())
            }
        }
    }

    /// The definitions of `name` worth offering, narrowed by what the cursor is
    /// sitting on.
    ///
    /// Two narrowings, both about not offering an answer that cannot be the one
    /// meant. A call reaches a function, so a call with any function definition
    /// in scope is not also asking about a type of the same name. And an `impl`
    /// block is a definition *about* a type rather than of it, so it only ever
    /// answers when nothing else does — which is what used to make `g d` on a
    /// type cycle through every `impl` it had.
    fn definitions_of(
        &mut self,
        name: &str,
        wanted: Option<rv_core::symbols::SymbolKind>,
    ) -> Vec<crate::index::Entry> {
        use rv_core::symbols::SymbolKind;
        let all: Vec<crate::index::Entry> = self
            .index()
            .definitions_named(name)
            .into_iter()
            .cloned()
            .collect();
        let named = |kinds: &[SymbolKind], entries: &[crate::index::Entry]| -> Vec<_> {
            entries
                .iter()
                .filter(|entry| kinds.contains(&entry.symbol.kind))
                .cloned()
                .collect::<Vec<_>>()
        };
        let by_kind = match wanted {
            Some(SymbolKind::Function) => named(&[SymbolKind::Function], &all),
            Some(SymbolKind::Type | SymbolKind::Struct | SymbolKind::Enum | SymbolKind::Trait) => {
                named(
                    &[
                        SymbolKind::Struct,
                        SymbolKind::Enum,
                        SymbolKind::Trait,
                        SymbolKind::Type,
                    ],
                    &all,
                )
            }
            _ => Vec::new(),
        };
        let candidates = if by_kind.is_empty() { all } else { by_kind };
        let without_impls = candidates
            .iter()
            .filter(|entry| entry.symbol.kind != SymbolKind::Impl)
            .cloned()
            .collect::<Vec<_>>();
        if without_impls.is_empty() {
            candidates
        } else {
            without_impls
        }
    }

    /// What the grammar calls the use under the cursor, where there is one: a
    /// call, a type mention. `None` in a file no grammar claims.
    fn use_kind_under_cursor(&mut self) -> Option<rv_core::symbols::SymbolKind> {
        let line = self.selected_line_number();
        let column = self
            .word_range()
            .map_or(0, |range| u32::try_from(range.start).unwrap_or(u32::MAX));
        let file = self.file_index;
        self.index()
            .use_at(file, line, column)
            .map(|use_| use_.reference.kind)
    }

    pub(super) fn selected_line_number(&self) -> u32 {
        self.selected_line()
            .and_then(|line| line.right.or(line.left))
            .unwrap_or(0)
    }

    /// Puts the column on `word` in the line just landed on, so a chain of
    /// jumps keeps pointing at the same symbol.
    pub(super) fn set_column_to(&mut self, word: &str) {
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
