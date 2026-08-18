//! Moving the cursor, and opening the file it lands on.

use anyhow::Context as _;
use anyhow::Result;
use rv_core::diff;

use super::App;
use super::Focus;
use super::SidebarTab;
use crate::tree::NodeKind;

impl App {
    pub(super) fn focus_left(&mut self) {
        self.focus = match self.focus {
            Focus::Stack => Focus::Diff,
            Focus::Diff | Focus::Sidebar => Focus::Sidebar,
        };
    }

    /// `Right` from the comment stack does nothing: the stack is drawn inside
    /// the diff pane, so there is no pane to its right. `Left` leads out of
    /// every focus, which is what keeps none of them a trap.
    pub(super) fn focus_right(&mut self) {
        self.focus = match self.focus {
            Focus::Sidebar => Focus::Diff,
            Focus::Diff | Focus::Stack => self.focus,
        };
    }

    /// `j` / `Down` in the focused pane — and, in the sidebar, in whichever
    /// list that pane is showing.
    pub(super) fn move_forward(&mut self) -> Result<()> {
        match self.focus {
            Focus::Sidebar => match self.sidebar_tab {
                SidebarTab::Files | SidebarTab::Commits => self.move_sidebar(true)?,
                SidebarTab::Comments => {
                    let last = self.comments.len().saturating_sub(1);
                    self.browser_index = self.browser_index.saturating_add(1).min(last);
                }
            },
            // By **row**, not by diff line: a comment box is rows, so this is
            // what lets the cursor walk into one instead of over it.
            Focus::Diff => self.set_cursor_row(self.cursor_row().saturating_add(1)),
            Focus::Stack => {
                let last = self.stack_len().saturating_sub(1);
                self.comment_index = self.comment_index.saturating_add(1).min(last);
            }
        }
        Ok(())
    }

    /// `k` / `Up` in the focused pane. Row 0 stays put rather than wrapping,
    /// which is the clamp `j` has at the other end.
    pub(super) fn move_back(&mut self) -> Result<()> {
        match self.focus {
            Focus::Sidebar => match self.sidebar_tab {
                SidebarTab::Files | SidebarTab::Commits => self.move_sidebar(false)?,
                SidebarTab::Comments => {
                    self.browser_index = self.browser_index.saturating_sub(1);
                }
            },
            Focus::Diff => self.set_cursor_row(self.cursor_row().saturating_sub(1)),
            Focus::Stack => self.comment_index = self.comment_index.saturating_sub(1),
        }
        Ok(())
    }

    /// `j`/`k` inside the file list: one row, clamped at both ends, selecting
    /// whatever file the new row holds.
    ///
    /// A row that holds no file — a directory, a change — moves the cursor and
    /// leaves the selection alone: a directory row is a thing to fold, not a
    /// file to open.
    fn move_sidebar(&mut self, forward: bool) -> Result<()> {
        // The keyboard takes the view back from the wheel: a selection the
        // reviewer is moving has to be one they can see.
        self.sidebar_scroll = None;
        let nodes = self.nodes();
        let Some(last) = nodes.len().checked_sub(1) else {
            return Ok(());
        };
        self.sidebar_row = if forward {
            self.sidebar_row.saturating_add(1).min(last)
        } else {
            self.sidebar_row.saturating_sub(1)
        };
        if let NodeKind::File { index } = nodes[self.sidebar_row].kind {
            self.select_node_file(index)?;
        }
        Ok(())
    }

    /// Moves the cursor to row `row` of the selected file's plan, clamped to
    /// that plan's last row.
    ///
    /// The one place the cursor is written, so the clamp cannot be forgotten on
    /// some path — and the one place the stack and the parked view are reset
    /// with it.
    pub(super) fn set_cursor_row(&mut self, row: usize) {
        let clamped = row.min(self.row_count().saturating_sub(1));
        if let Some(position) = self.cursor_rows.get_mut(self.file_index) {
            *position = clamped;
        }
        self.reset_stack();
        // The wheel parks the view away from the cursor deliberately —
        // scrolling is looking — but the moment the selection moves, the pane
        // the reviewer is steering has to be the pane they can see.
        self.diff_scroll = None;
    }

    /// Puts the cursor back on the row that owns `line`, after something
    /// rebuilt the plan under it — a fold, a save, a delete.
    ///
    /// The *line* is what survives such a change; a row index is an address in
    /// a list that just changed length.
    ///
    /// Deliberately does **not** reset the stack: nothing here is the reviewer
    /// moving the selection, and a delete from inside a stack is a stack they
    /// are still working through — [`App::sync_stack`] keeps the cursor in it.
    pub(super) fn resettle_cursor(&mut self, line: usize) {
        let plan = self.plan();
        let row = plan
            .row_of_line(line)
            .unwrap_or(0)
            .min(plan.rows.len().saturating_sub(1));
        if let Some(position) = self.cursor_rows.get_mut(self.file_index) {
            *position = row;
        }
        self.diff_scroll = None;
    }

    /// Moves the sidebar selection to `index` and loads that file's diff.
    ///
    /// Out-of-range indices are ignored, which is what makes `[` at the top and
    /// `]` at the bottom no-ops rather than errors. The file reopens where it
    /// was left, re-clamped on the way in because it was clamped against
    /// whatever the diff was when it was written.
    pub(super) fn select_file(&mut self, index: usize) -> Result<()> {
        if index >= self.review.files.len() || index == self.file_index {
            return Ok(());
        }
        self.file_index = index;
        self.load_selected()?;
        self.set_cursor_row(self.cursor_row());
        // `[` and `]` consult no focus, so a file can be selected from anywhere;
        // the file list's cursor follows it.
        self.resettle_sidebar();
        Ok(())
    }

    /// Computes the selected file's diff if it has not been computed yet.
    ///
    /// Both sides are read at their own path and their own commit, so a rename
    /// diffs its base-side source against its head-side target rather than
    /// against a file that does not exist.
    pub(super) fn load_selected(&mut self) -> Result<()> {
        let Some(file) = self.review.files.get(self.file_index) else {
            return Ok(());
        };
        if self.diffs[self.file_index].is_some() {
            return Ok(());
        }

        let session = &self.review.session;
        let base_commit = session.base_commit.clone();
        let head_commit = session.head_commit.clone();
        let base_path = file.source_path.as_deref().unwrap_or(&file.path).to_owned();
        let head_path = file.path.clone();
        let old = self
            .review
            .repo
            .read_blob(&base_commit, &base_path)
            .with_context(|| format!("could not read {base_path} at the base of the review"))?;
        let new = self
            .review
            .repo
            .read_blob(&head_commit, &head_path)
            .with_context(|| format!("could not read {head_path} at the head of the review"))?;

        let diff = if self.force_fallback {
            diff::compute_with(old.as_deref(), new.as_deref(), &head_path, false)
        } else {
            diff::compute(old.as_deref(), new.as_deref(), &head_path)
        };
        self.diffs[self.file_index] = Some(diff);
        // Parsed from the very blobs the diff was computed from, so the spans a
        // line is painted with describe the text that line came from — and parsed
        // *off* this thread, so a large file draws now and colours in a moment.
        self.parse_highlights(base_commit, base_path, old.as_deref());
        self.parse_highlights(head_commit, head_path, new.as_deref());
        Ok(())
    }

}
