//! The commits view: which change touched which file.
//!
//! The bookmark view answers "what did this branch do to this file"; this one
//! answers "what did *this change* do", which is the question a reviewer
//! walking a stack is actually asking.
//!
//! Built once, on the first frame that needs it, and kept. A stack of thirty
//! changes is thirty diff enumerations plus a blob pair per file, and doing
//! that at startup would make every review that never opens this tab pay for
//! it.

use std::cell::OnceCell;

use rv_core::diff;
use rv_core::diff::LineKind;
use rv_core::model::FileChange;

use anyhow::Result;

use super::App;
use super::SidebarTab;
use crate::gradient::Stat;
use crate::tree;

/// Everything the commits tab draws from, computed together because the file
/// lists and the sizes come from the same pair of commits.
#[derive(Default)]
pub(super) struct CommitIndex {
    files: Vec<Vec<FileChange>>,
    endpoints: Vec<(String, String)>,
    /// One entry per file row, in the order [`tree::build_grouped`] numbers
    /// them: which change it belongs to and which of that change's files it is.
    pairs: Vec<(usize, usize)>,
    /// What went wrong enumerating, one sentence per failed change.
    ///
    /// Kept rather than swallowed: an enumeration error shown as an empty change
    /// gives the summary a `+0 -0` that reads as "this change touched nothing",
    /// which is a claim about the change when the truth is a claim about the
    /// repository. [`App::goto_tab`] raises these as alerts the first time the
    /// list is shown.
    errors: Vec<String>,
    /// The size of each pair's change, parallel to `pairs`.
    stats: Vec<Stat>,
}

impl CommitIndex {
    /// What went wrong enumerating, if anything did.
    pub(super) fn errors(&self) -> &[String] {
        &self.errors
    }

    /// One change's files, or nothing for a change that is not in the stack.
    pub(super) fn files_of(&self, change: usize) -> &[FileChange] {
        self.files.get(change).map_or(&[], Vec::as_slice)
    }

    /// The two commits one change is between: its parent, and itself.
    pub(super) fn endpoints_of(&self, change: usize) -> Option<(&str, &str)> {
        self.endpoints
            .get(change)
            .map(|(from, to)| (from.as_str(), to.as_str()))
    }

    /// The row number for one change's `position`th file.
    pub(super) fn pair_of(&self, change: usize, position: usize) -> Option<usize> {
        self.pairs
            .iter()
            .position(|pair| *pair == (change, position))
    }

    /// What one file row's change cost.
    pub(super) fn stat_of(&self, pair: usize) -> Stat {
        self.stats.get(pair).copied().unwrap_or_default()
    }

    /// Which change and which of its files the `pair`th file row addresses.
    pub(super) fn pair(&self, pair: usize) -> Option<(usize, &FileChange)> {
        let &(change, position) = self.pairs.get(pair)?;
        Some((change, self.files[change].get(position)?))
    }
}

impl App {
    /// The commits index, built on first use.
    pub(super) fn commit_index(&self) -> &CommitIndex {
        self.commits.get_or_init(|| self.build_commit_index())
    }

    /// Enumerates every change's files and measures each one against that
    /// change's own base.
    ///
    /// A change whose files cannot be enumerated touches nothing rather than
    /// failing the review: the rest of the stack is still worth reading, and an
    /// empty group is visibly empty. A file whose blobs cannot be read measures
    /// zero, exactly as it does in the bookmark view.
    fn build_commit_index(&self) -> CommitIndex {
        let mut index = CommitIndex::default();
        let changes = &self.review.session.changes;
        // `Repository::stack` lists the **newest change first**, so a change's
        // own base is the next entry along, and the review's base commit stands
        // in for the one after the oldest. Walking this the other way is not a
        // subtle error: it attributes every file to the wrong change and gives
        // the oldest one a diff full of removals.
        for (position, change) in changes.iter().enumerate() {
            let base = changes
                .get(position + 1)
                .map_or(self.review.session.base_commit.as_str(), |older| {
                    older.commit_id.as_str()
                });
            let files = match self.review.repo.files(base, &change.commit_id) {
                Ok(files) => files,
                // The rest of the stack is still worth reading, but the failure
                // is said out loud rather than drawn as a change that touched
                // nothing: `+0 -0` under a change reads as a claim about the
                // change when the truth is a claim about the repository.
                Err(error) => {
                    index.errors.push(format!(
                        "could not list {}'s files: {error}",
                        &change.change_id[..8.min(change.change_id.len())]
                    ));
                    Vec::new()
                }
            };
            index
                .endpoints
                .push((base.to_owned(), change.commit_id.clone()));
            index.files.push(files);
        }

        for (change, files) in index.files.iter().enumerate() {
            let (from, to) = &index.endpoints[change];
            for (position, file) in files.iter().enumerate() {
                index.pairs.push((change, position));
                index.stats.push(measure(self, from, to, file));
            }
        }
        index
    }

    /// The rows the commits tab draws: one per change, with its files beneath.
    ///
    /// File indices run *across* the groups, so the nth file row of the whole
    /// list addresses `pairs[n]`. That is [`tree::build_grouped`]'s contract,
    /// and the pairing is what lets a row know whose diff of the file it names.
    #[must_use]
    pub fn commit_nodes(&self) -> Vec<tree::Node> {
        let index = self.commit_index();
        let paths: Vec<Vec<&str>> = index
            .files
            .iter()
            .map(|files| files.iter().map(|file| file.path.as_str()).collect())
            .collect();
        let groups: Vec<tree::Group<'_>> = self
            .review
            .session
            .changes
            .iter()
            .zip(&paths)
            .map(|(change, paths)| tree::Group {
                change_id: &change.change_id,
                commit_id: &change.commit_id,
                description: &change.description,
                paths,
            })
            .collect();

        self.zoom_view(tree::build_grouped(
            &groups,
            &self.collapsed_dirs,
            self.tree,
            self.sort,
            &|file| index.stats.get(file).copied().unwrap_or_default(),
        ))
    }

    /// The path the commits view's `file`th row names, if there is one.
    #[must_use]
    pub fn commit_path(&self, file: usize) -> Option<&str> {
        let index = self.commit_index();
        let &(change, position) = index.pairs.get(file)?;
        Some(index.files[change][position].path.as_str())
    }

    /// The rows the sidebar is showing: the bookmark's files, or the stack's
    /// changes with their files beneath. Memoized — see [`App::nodes_cache`].
    #[must_use]
    pub fn nodes(&self) -> Vec<tree::Node> {
        let key = self.nodes_fingerprint();
        if let Some((cached_key, nodes)) = &*self.nodes_cache.borrow()
            && *cached_key == key
        {
            return nodes.clone();
        }
        let nodes = match self.sidebar_tab() {
            SidebarTab::Files => self.sidebar_nodes(),
            SidebarTab::Commits => self.commit_nodes(),
            SidebarTab::Comments => Vec::new(),
        };
        *self.nodes_cache.borrow_mut() = Some((key, nodes.clone()));
        nodes
    }

    /// A cheap hash of everything that shapes [`App::nodes`]: the tab, the
    /// list's shape and order, the folded rows, and the zoom. The files and
    /// changes only move on a refresh, which builds a fresh app.
    fn nodes_fingerprint(&self) -> u64 {
        use std::hash::Hash;
        use std::hash::Hasher;
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        (self.sidebar_tab() as u8).hash(&mut hasher);
        self.tree.hash(&mut hasher);
        (self.sort as u8).hash(&mut hasher);
        // Sorted so a fold then unfold hashes the same set, order aside.
        let mut folded: Vec<&String> = self.collapsed_dirs.iter().collect();
        folded.sort();
        folded.hash(&mut hasher);
        self.zoom.iter().for_each(|z| z.key.hash(&mut hasher));
        hasher.finish()
    }

    /// Selects whatever file the row under the cursor names, in the vocabulary
    /// of the tab that drew it: a bookmark file index in the Files tab, a
    /// (change, file) pair in the Commits tab.
    pub(super) fn select_node_file(&mut self, index: usize) -> Result<()> {
        match self.sidebar_tab() {
            SidebarTab::Files => self.select_file(index),
            SidebarTab::Commits => self.select_commit_file(index),
            SidebarTab::Comments => Ok(()),
        }
    }

    /// The paths one change touched, in the order the repository lists them.
    #[must_use]
    pub fn commit_change_paths(&self, change: usize) -> Vec<String> {
        self.commit_index()
            .files_of(change)
            .iter()
            .map(|file| file.path.clone())
            .collect()
    }

    /// Which change the commits view's `file`th row belongs to.
    #[must_use]
    pub fn commit_change(&self, file: usize) -> Option<usize> {
        self.commit_index()
            .pairs
            .get(file)
            .map(|&(change, _)| change)
    }
}

/// One file's size in one change, measured through the in-process engine.
///
/// `similar` rather than difftastic for the same reason the bookmark view
/// measures that way: this runs over every file of every change before the tab
/// draws, and a subprocess per file would be a visible pause.
fn measure(app: &App, from: &str, to: &str, file: &FileChange) -> Stat {
    let base_path = file.source_path.as_deref().unwrap_or(&file.path);
    let old = app.review.repo.read_blob(from, base_path).ok().flatten();
    let new = app.review.repo.read_blob(to, &file.path).ok().flatten();

    diff::compute_with(old.as_deref(), new.as_deref(), &file.path, false)
        .lines
        .iter()
        .fold(Stat::default(), |stat, line| match line.kind {
            LineKind::Added => Stat {
                added: stat.added.saturating_add(1),
                ..stat
            },
            LineKind::Removed => Stat {
                removed: stat.removed.saturating_add(1),
                ..stat
            },
            LineKind::Context => stat,
        })
}

/// The cell an [`App`] holds the index in.
pub(super) type Commits = OnceCell<CommitIndex>;
