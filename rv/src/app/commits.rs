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
    /// Each change's files, parallel to `session.changes`.
    files: Vec<Vec<FileChange>>,
    /// Each change's endpoints: the commit before it, and its own.
    endpoints: Vec<(String, String)>,
    /// One entry per file row, in the order [`tree::build_grouped`] numbers
    /// them: which change it belongs to and which of that change's files it is.
    pairs: Vec<(usize, usize)>,
    /// The size of each pair's change, parallel to `pairs`.
    stats: Vec<Stat>,
}

impl CommitIndex {
    /// One change's files, or nothing for a change that is not in the stack.
    pub(super) fn files_of(&self, change: usize) -> &[FileChange] {
        self.files.get(change).map_or(&[], Vec::as_slice)
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
            let files = self
                .review
                .repo
                .files(base, &change.commit_id)
                .unwrap_or_default();
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

        tree::build_grouped(
            &groups,
            &self.collapsed_dirs,
            self.tree,
            self.sort,
            &|file| index.stats.get(file).copied().unwrap_or_default(),
        )
    }

    /// The path the commits view's `file`th row names, if there is one.
    #[must_use]
    pub fn commit_path(&self, file: usize) -> Option<&str> {
        let index = self.commit_index();
        let &(change, position) = index.pairs.get(file)?;
        Some(index.files[change][position].path.as_str())
    }

    /// The rows the sidebar is showing: the bookmark's files, or the stack's
    /// changes with their files beneath.
    ///
    /// One list and one cursor, because the two tabs are never on screen at
    /// once. The Comments tab has neither — it lists comments, not nodes — and
    /// answers with an empty list.
    #[must_use]
    pub fn nodes(&self) -> Vec<tree::Node> {
        match self.sidebar_tab() {
            SidebarTab::Files => self.sidebar_nodes(),
            SidebarTab::Commits => self.commit_nodes(),
            SidebarTab::Comments => Vec::new(),
        }
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

    /// Selects the bookmark file a commits-view row names.
    ///
    /// The diff shown is still the bookmark's — what the branch did to the file
    /// — rather than what this one change did to it. The row says which change
    /// touched it; narrowing the diff to that change is the next step, and
    /// saying so is better than quietly showing one and labelling it the other.
    pub(super) fn select_commit_file(&mut self, pair: usize) -> Result<()> {
        let Some(path) = self.commit_path(pair).map(str::to_owned) else {
            return Ok(());
        };
        if let Some(index) = self
            .review
            .files
            .iter()
            .position(|file| file.path == path)
        {
            self.select_file(index)?;
        }
        Ok(())
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

    /// The change the sidebar cursor is on or inside, as
    /// `(short change id, short commit id, subject)`.
    ///
    /// `None` anywhere but the Commits tab. A file row answers with the change
    /// that holds it, which is what "or inside" means: a reviewer reading a file
    /// under a change is reading that change, and the row above them has scrolled
    /// off as often as not.
    #[must_use]
    pub fn change_under_cursor(&self) -> Option<(String, String, String)> {
        if self.sidebar_tab() != SidebarTab::Commits {
            return None;
        }
        let nodes = self.nodes();
        let row = self.sidebar_row().min(nodes.len().saturating_sub(1));
        nodes[..=row].iter().rev().find_map(|node| match &node.kind {
            tree::NodeKind::Commit {
                short_change,
                short_commit,
                subject,
                ..
            } => Some((
                short_change.clone(),
                short_commit.clone(),
                subject.clone(),
            )),
            _ => None,
        })
    }

    /// Which change the commits view's `file`th row belongs to.
    #[must_use]
    pub fn commit_change(&self, file: usize) -> Option<usize> {
        self.commit_index().pairs.get(file).map(|&(change, _)| change)
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
