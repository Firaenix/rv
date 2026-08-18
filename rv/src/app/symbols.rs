//! Stepping between the symbols in scope, and picking one by name.
//!
//! Scope is whatever the sidebar is listing. On the Files tab that is every
//! changed file in the range; on the Commits tab it is the files of the change
//! the cursor is in. `rv/src/index.rs` has no opinion about which — the caller
//! owns the scope, and this is the caller.
//!
//! Built on first use and cached per scope, because reading a blob and parsing a
//! grammar for every file in a review is not something to do at startup for a
//! key the reviewer may never press.

use anyhow::Result;
use rv_core::model::ChangeKind;

use super::App;
use super::SidebarTab;
use crate::index::Index;
use crate::index::Scoped;
use crate::index::indexed_side;
use rv_core::model::Side;

/// Which scope an index was built for, so a cached one is only reused where it
/// still describes what the sidebar is showing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Scope {
    /// Every changed file in the range.
    Bookmark,
    /// One change's files, by its position in the stack.
    Change(usize),
}

/// One file in scope, before its bytes have been read.
struct ScopedFile {
    file: usize,
    path: String,
    kind: ChangeKind,
    change_id: Option<String>,
}

/// The same, with the blob its symbols come from.
struct Indexable {
    file: usize,
    path: String,
    kind: ChangeKind,
    change_id: Option<String>,
    blob: Option<Vec<u8>>,
}

impl App {
    /// The symbols in scope, indexed on first use.
    ///
    /// Re-indexed when the scope changes — switching tabs, or moving to another
    /// change — and otherwise returned from the cache. `&mut self` because
    /// building it fills that cache; every read below goes through here so a
    /// stale index cannot be handed out.
    pub fn index(&mut self) -> &Index {
        let scope = self.scope();
        if self.indexed_scope.as_ref() != Some(&scope) {
            let index = self.build_index(&scope);
            self.symbol_index = index;
            self.indexed_scope = Some(scope);
        }
        &self.symbol_index
    }

    /// What the sidebar is listing, which is what `n` walks.
    fn scope(&self) -> Scope {
        match self.sidebar_tab {
            SidebarTab::Commits => self
                .commit_change(self.commit_file_under_cursor().unwrap_or(0))
                .map_or(Scope::Bookmark, Scope::Change),
            SidebarTab::Files | SidebarTab::Comments => Scope::Bookmark,
        }
    }

    /// Which (change, file) pair the commits cursor is on or inside, so a change
    /// row and the files under it share one scope.
    fn commit_file_under_cursor(&self) -> Option<usize> {
        let nodes = self.nodes();
        // Walk back to the nearest file row: a change row is not a pair, but the
        // pairs beneath it belong to the change it names.
        nodes[..=self.sidebar_row.min(nodes.len().saturating_sub(1))]
            .iter()
            .rev()
            .find_map(|node| match node.kind {
                crate::tree::NodeKind::File { index } => Some(index),
                _ => None,
            })
            .or_else(|| {
                nodes
                    .get(self.sidebar_row)
                    .and_then(|_| nodes.iter().find_map(|node| match node.kind {
                        crate::tree::NodeKind::File { index } => Some(index),
                        _ => None,
                    }))
            })
    }

    /// Reads the blobs every file in `scope` needs and hands them to
    /// [`Index::of`].
    ///
    /// Only the side that file's symbols come from is read — [`indexed_side`]'s
    /// decision — because reading both would double the I/O for a blob nothing
    /// looks at.
    fn build_index(&self, scope: &Scope) -> Index {
        // Read into owned values first, so the borrows outlive the `Scoped`
        // values that point at them.
        let read: Vec<Indexable> = self
            .scoped_files(scope)
            .into_iter()
            .map(|file| self.read_indexable(file))
            .collect();

        let scoped: Vec<Scoped<'_>> = read
            .iter()
            .map(|file| Scoped {
                file: file.file,
                path: &file.path,
                kind: file.kind,
                change_id: file.change_id.as_deref(),
                // Only the indexed side is ever read, so the other is `None`
                // rather than a blob nothing looks at.
                base: match indexed_side(file.kind) {
                    Side::Left => file.blob.as_deref(),
                    Side::Right => None,
                },
                head: match indexed_side(file.kind) {
                    Side::Right => file.blob.as_deref(),
                    Side::Left => None,
                },
            })
            .collect();
        Index::of(&scoped)
    }

    /// One in-scope file with the bytes its symbols come from.
    fn read_indexable(&self, file: ScopedFile) -> Indexable {
        let (commit, read_path) = match indexed_side(file.kind) {
            Side::Left => (
                &self.review.session.base_commit,
                // A removed file's only path is its base-side one, and a
                // rename's base path is not the path it is listed under.
                self.base_path_of(&file.path),
            ),
            Side::Right => (&self.review.session.head_commit, file.path.clone()),
        };
        let blob = self
            .review
            .repo
            .read_blob(commit, &read_path)
            .ok()
            .flatten();
        Indexable {
            file: file.file,
            path: file.path,
            kind: file.kind,
            change_id: file.change_id,
            blob,
        }
    }

    /// The files `scope` covers, as `(file index, path, kind, change)`.
    ///
    /// The file index is the number the *caller* addresses a file by, which is a
    /// position in `App::files()` either way: a commits-view scope is narrower
    /// but still numbered in the bookmark's terms, so a jump the index hands
    /// back is a jump `select_file` can perform.
    fn scoped_files(&self, scope: &Scope) -> Vec<ScopedFile> {
        match scope {
            Scope::Bookmark => self
                .review
                .files
                .iter()
                .enumerate()
                .map(|(index, file)| ScopedFile {
                    file: index,
                    path: file.path.clone(),
                    kind: file.kind,
                    change_id: None,
                })
                .collect(),
            Scope::Change(change) => {
                let Some(entry) = self.review.session.changes.get(*change) else {
                    return Vec::new();
                };
                let paths = self.commit_change_paths(*change);
                paths
                    .into_iter()
                    .filter_map(|path| {
                        let index = self.review.files.iter().position(|f| f.path == path)?;
                        Some(ScopedFile {
                            file: index,
                            kind: self.review.files[index].kind,
                            path,
                            change_id: Some(entry.change_id.clone()),
                        })
                    })
                    .collect()
            }
        }
    }

    /// The base-side path of a file listed under `path`, which differs from it
    /// only for a rename.
    fn base_path_of(&self, path: &str) -> String {
        self.review
            .files
            .iter()
            .find(|file| file.path == path)
            .and_then(|file| file.source_path.clone())
            .unwrap_or_else(|| path.to_owned())
    }

    /// `n`: to the next symbol after the cursor, wrapping nowhere.
    pub(super) fn next_symbol(&mut self) -> Result<()> {
        self.step_symbol(true)
    }

    /// `N`: to the previous one.
    pub(super) fn previous_symbol(&mut self) -> Result<()> {
        self.step_symbol(false)
    }

    /// Moves to the next or previous symbol in scope.
    ///
    /// No wrap at either end: a reviewer who has walked to the last symbol has
    /// finished, and a jump back to the first would look exactly like a jump
    /// that failed to move.
    fn step_symbol(&mut self, forward: bool) -> Result<()> {
        let file = self.file_index;
        // The cursor's own source line, on whichever side it is on: a symbol is
        // a place in a file, and the walk is over places rather than over rows.
        let line = self
            .selected_line()
            .and_then(|line| line.right.or(line.left))
            .unwrap_or(0);
        let found = {
            let index = self.index();
            if forward {
                index.next_after(file, line)
            } else {
                index.previous_before(file, line)
            }
            .cloned()
        };

        let Some(entry) = found else {
            self.status = if self.index().is_empty() {
                "no symbols in this scope — rv ships no grammar for these files".to_owned()
            } else if forward {
                "the last symbol in scope".to_owned()
            } else {
                "the first symbol in scope".to_owned()
            };
            return Ok(());
        };

        self.jump_to_symbol(&entry)
    }

    /// Puts the cursor on `entry`'s definition and says where it went.
    pub(super) fn jump_to_symbol(&mut self, entry: &crate::index::Entry) -> Result<()> {
        self.select_file(entry.file)?;
        // The definition's source line, found in the diff by number rather than
        // by position: a diff holds only the lines that changed, so the nth
        // source line is rarely the nth diff line.
        let found = self
            .selected_diff()
            .and_then(|diff| {
                diff.lines
                    .iter()
                    .position(|line| line.right == Some(entry.symbol.line) || line.left == Some(entry.symbol.line))
            });
        match found {
            Some(line) => {
                let row = self.plan().row_of_line(line).unwrap_or(0);
                self.set_cursor_row(row);
            }
            // The symbol is in the file but not in its diff — it is a definition
            // this change did not touch. Its file is still the right place to
            // land, and the status says which line was wanted.
            None => self.set_cursor_row(0),
        }
        self.focus = super::Focus::Diff;
        self.status = format!(
            "{} — {}:{}",
            entry.symbol.name, entry.path, entry.symbol.line
        );
        Ok(())
    }
}

impl App {
    /// `/`: starts a symbol query on an empty buffer.
    ///
    /// Refused where there is nothing to find, with the reason: a picker that
    /// opened onto an empty list would make a review with no indexable files
    /// look like a broken key.
    pub(super) fn begin_pick(&mut self) {
        if self.index().is_empty() {
            self.status =
                "no symbols in this scope — rv ships no grammar for these files".to_owned();
            return;
        }
        self.mode = super::Mode::Pick;
        self.buffer.clear();
        self.status = "find a symbol: type a name, Enter to jump, Esc to cancel".to_owned();
    }

    /// The keys the picker answers.
    ///
    /// The same four the comment buffer answers, for the same reason: text
    /// arrives in one place in this reviewer, so the escape and the backspace
    /// cannot behave differently depending on why you are typing.
    pub(super) fn on_key_pick(&mut self, key: crossterm::event::KeyCode) -> Result<super::Action> {
        use crossterm::event::KeyCode;
        match key {
            KeyCode::Esc => {
                self.mode = super::Mode::Browse;
                self.buffer.clear();
                self.status = "search cancelled".to_owned();
            }
            KeyCode::Backspace => {
                self.buffer.pop();
            }
            KeyCode::Enter => {
                let chosen = self.matches().first().cloned();
                self.mode = super::Mode::Browse;
                self.buffer.clear();
                match chosen {
                    Some(entry) => self.jump_to_symbol(&entry)?,
                    None => self.status = "no symbol matches".to_owned(),
                }
            }
            KeyCode::Char(character) => self.buffer.push(character),
            _ => {}
        }
        Ok(super::Action::Continue)
    }

    /// The entries the query matches, best first.
    ///
    /// A case-insensitive substring of the name, ranked by where the match
    /// starts: a name that *begins* with what was typed is what the reviewer
    /// meant, and one that merely contains it is a fallback. Ties keep index
    /// order, so the list never reorders itself under a keystroke that did not
    /// change the ranking.
    #[must_use]
    pub fn matches(&self) -> Vec<crate::index::Entry> {
        let query = self.buffer.to_lowercase();
        if query.is_empty() {
            return self.symbol_index.entries().to_vec();
        }
        let mut found: Vec<(usize, usize, crate::index::Entry)> = self
            .symbol_index
            .entries()
            .iter()
            .enumerate()
            .filter_map(|(position, entry)| {
                let at = entry.symbol.name.to_lowercase().find(&query)?;
                Some((at, position, entry.clone()))
            })
            .collect();
        found.sort_by_key(|(at, position, _)| (*at, *position));
        found.into_iter().map(|(_, _, entry)| entry).collect()
    }
}
