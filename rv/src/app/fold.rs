//! `s`: folding away whatever the cursor is on.
//!
//! Two fold sets under one key, kept apart because a comment id and a
//! directory path could otherwise collide: `collapsed` holds comment boxes,
//! `collapsed_dirs` holds file-list rows. Neither reaches `.review/` — see
//! [`super::sidebar`] for why a view preference stays in the session.

use super::App;
use super::Focus;
use super::SidebarTab;
use super::status::NO_COMMENTS;
use super::status::NO_COMMENTS_IN_REVIEW;

impl App {
    /// Folds comment boxes away, or unfolds them.
    ///
    /// What it acts on follows the cursor, exactly as `d` does: the one box in
    /// the stack, the browsed comment in the Comments tab, the directory row in
    /// the file list, and otherwise the whole of the selected line's stack.
    ///
    /// A line whose boxes are in *mixed* states collapses rather than expands.
    /// The reason to press `s` on a line is to get it out of the way, and a
    /// toggle that flipped each box independently would need a second press to
    /// finish a job the reviewer asked for once.
    pub(super) fn toggle_collapse(&mut self) {
        if let Some(key) = self.sidebar_fold_key() {
            self.toggle_dir_fold(key);
            return;
        }

        let ids = self.fold_targets();
        if ids.is_empty() {
            // About the review from the browser, which is not showing a line,
            // and about the line everywhere else — the same split `d` makes,
            // because it is the same question about the same two cursors.
            self.status = match (self.focus, self.sidebar_tab) {
                (Focus::Sidebar, SidebarTab::Comments) => NO_COMMENTS_IN_REVIEW,
                _ => NO_COMMENTS,
            }
            .to_owned();
            return;
        }

        // Read before the fold: folding rebuilds the plan the cursor indexes,
        // since a box the cursor was inside becomes one row.
        let line = self.line_index();
        let folded = ids.iter().all(|id| self.collapsed.contains(id));
        for id in ids {
            if folded {
                self.collapsed.remove(&id);
            } else {
                self.collapsed.insert(id);
            }
        }
        self.resettle_cursor(line);
    }

    /// Which comments `s` would fold, as ids. Empty where it would fold
    /// nothing, which is also how [`App::binding_enabled`] knows to dim it —
    /// one rule asked twice rather than a copy in the renderer.
    pub(super) fn fold_targets(&self) -> Vec<String> {
        match (self.focus, self.sidebar_tab) {
            (Focus::Stack, _) => self
                .selected_comment()
                .map(|comment| comment.id.clone())
                .into_iter()
                .collect(),
            (Focus::Sidebar, SidebarTab::Comments) => self
                .browsed_comment()
                .map(|comment| comment.id.clone())
                .into_iter()
                .collect(),
            (Focus::Diff | Focus::Sidebar, _) => self
                .comments_for_line(self.line_index())
                .iter()
                .map(|comment| comment.id.clone())
                .collect(),
        }
    }
}
