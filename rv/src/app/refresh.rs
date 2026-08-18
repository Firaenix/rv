//! `R`: re-resolve the range and pick up what the repository now says.
//!
//! jj-lib loads a repo at an operation, so the `Review` opened at launch is a
//! snapshot: move a bookmark, absorb a change, or let an agent push while the
//! reviewer is open, and the pane keeps showing the world as it was. This
//! re-asks the *original question* — the `--from`/`--to` the reviewer typed,
//! with `@` resolving to wherever `@` is now — rather than re-using the resolved
//! commits, which would pin the review to the moment it was opened.

use anyhow::Result;

use super::App;

impl App {
    /// Rebuilds the review against the repository as it stands, keeping every
    /// preference and, where it survives, the reviewer's place.
    ///
    /// Built as a fresh `App` and moved into place rather than patched field by
    /// field: everything derived — diffs, stats, highlights, the symbol index,
    /// the commits view — must be re-derived from the new snapshot, and a list
    /// of fields to clear is a list that rots. What is *kept* is the short
    /// list: view preferences, and the fold state, which describes the
    /// reviewer's screen rather than the repository.
    pub(super) fn refresh(&mut self) -> Result<()> {
        let (from, to) = self.review.asked.clone();
        let root = self.review.store.root().to_owned();
        let review = crate::session::build(&root, from.as_deref(), to.as_deref())?;
        let selected = self.selected_file().map(|file| file.path.clone());

        let mut fresh = Self::build(review, self.engine)?;
        fresh.split = self.split;
        fresh.tree = self.tree;
        fresh.sort = self.sort;
        fresh.sidebar_tab = self.sidebar_tab;
        fresh.sidebar_hidden = self.sidebar_hidden;
        fresh.info_dismissed = self.info_dismissed;
        fresh.collapsed = std::mem::take(&mut self.collapsed);
        fresh.collapsed_dirs = std::mem::take(&mut self.collapsed_dirs);

        // The file, not the index: a rebased stack lists files in a new order,
        // and index 3 of the new list is not what the reviewer was reading.
        if let Some(path) = selected
            && let Some(index) = fresh
                .review
                .files
                .iter()
                .position(|file| file.path == path)
        {
            fresh.select_file(index)?;
        }
        fresh.resettle_sidebar();
        fresh.status = format!(
            "refreshed — {} files, {} changes, {} comments",
            fresh.review.files.len(),
            fresh.review.session.changes.len(),
            fresh.comments.len()
        );
        *self = fresh;
        Ok(())
    }
}
