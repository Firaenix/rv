//! Moving the cursor, and opening the file it lands on.

use anyhow::Context as _;
use anyhow::Result;
use rv_core::diff;

use super::App;
use super::DiffEngine;
use super::Focus;
use super::SidebarTab;
use super::hunks;
use super::sidebar::BrowserRow;
use crate::tree::NodeKind;

impl App {
    /// `j` / `Down` in the focused pane — and, in the sidebar, in whichever
    /// list that pane is showing.
    pub(super) fn move_forward(&mut self) -> Result<()> {
        match self.focus {
            Focus::Sidebar => match self.sidebar_tab {
                SidebarTab::Files | SidebarTab::Commits => self.move_sidebar(true)?,
                SidebarTab::Comments => self.move_browser(true),
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
                SidebarTab::Comments => self.move_browser(false),
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
        self.step_sidebar(forward, 1)
    }

    /// The file list moved `count` rows in one direction, clamped, selecting the
    /// file the landing row holds. One is `j`/`k`; a page is many.
    pub(super) fn step_sidebar(&mut self, forward: bool, count: usize) -> Result<()> {
        // The keyboard takes the view back from the wheel: a selection the
        // reviewer is moving has to be one they can see.
        self.sidebar_scroll = None;
        let nodes = self.nodes();
        let Some(last) = nodes.len().checked_sub(1) else {
            return Ok(());
        };
        self.sidebar_row = if forward {
            self.sidebar_row.saturating_add(count).min(last)
        } else {
            self.sidebar_row.saturating_sub(count)
        };
        if let NodeKind::File { index } = nodes[self.sidebar_row].kind {
            self.select_node_file(index)?;
        }
        Ok(())
    }

    /// The file list jumped to its first (`forward` false) or last row, opening
    /// the file there if it holds one.
    pub(super) fn jump_sidebar(&mut self, forward: bool) -> Result<()> {
        self.sidebar_scroll = None;
        let nodes = self.nodes();
        let Some(last) = nodes.len().checked_sub(1) else {
            return Ok(());
        };
        self.sidebar_row = if forward { last } else { 0 };
        if let NodeKind::File { index } = nodes[self.sidebar_row].kind {
            self.select_node_file(index)?;
        }
        Ok(())
    }

    /// `j`/`k` inside the comment browser: to the next comment row in that
    /// direction, clamped at both ends.
    ///
    /// The cursor is a **row** — a heading is a real row the pointer can land
    /// on — but the keyboard walks *comments*. A step that parked on a heading
    /// would cost the reviewer a keystroke per file to say nothing, and `k` off
    /// the first comment would leave `d` and `s` with no target at the top of
    /// the list they had just walked to. So the walk skips headings and stops
    /// on the first and last comments, exactly as `k` stops at diff row 0.
    fn move_browser(&mut self, forward: bool) {
        self.step_browser(forward, 1);
    }

    /// The browser walked `count` comments in one direction, skipping headings
    /// and stopping on the first and last comment.
    pub(super) fn step_browser(&mut self, forward: bool, count: usize) {
        self.sidebar_scroll = None;
        let rows = self.browser_rows();
        let is_comment = |row: &usize| matches!(rows[*row], BrowserRow::Comment(_));
        for _ in 0..count {
            let found = if forward {
                (self.browser_index.saturating_add(1)..rows.len()).find(is_comment)
            } else {
                (0..self.browser_index).rev().find(is_comment)
            };
            match found {
                Some(row) => self.browser_index = row,
                None => break,
            }
        }
    }

    /// The browser jumped to its first or last comment, skipping headings.
    pub(super) fn jump_browser(&mut self, forward: bool) {
        self.sidebar_scroll = None;
        let rows = self.browser_rows();
        let mut comments =
            (0..rows.len()).filter(|row| matches!(rows[*row], BrowserRow::Comment(_)));
        let landing = if forward {
            comments.next_back()
        } else {
            comments.next()
        };
        if let Some(row) = landing {
            self.browser_index = row;
        }
    }

    /// `H`/`L`: scrolls the focused pane's text sideways by `delta` columns.
    ///
    /// The stack scrolls the diff it is drawn inside. Unclamped on the right on
    /// purpose — only the renderer knows how long the longest visible line is,
    /// and a scroll past the end shows the marker column, which is its own
    /// answer.
    pub(super) fn hscroll_focused(&mut self, delta: isize) {
        match self.focus {
            Focus::Sidebar => {
                self.sidebar_hscroll = self.sidebar_hscroll.saturating_add_signed(delta);
            }
            Focus::Diff | Focus::Stack => {
                self.diff_hscroll = self.diff_hscroll.saturating_add_signed(delta);
            }
        }
    }

    /// `J`: to the first line of the next hunk below the cursor.
    pub(super) fn next_hunk(&mut self) {
        self.step_hunk(true);
    }

    /// `K`: to the first line of the previous one.
    pub(super) fn previous_hunk(&mut self) {
        self.step_hunk(false);
    }

    /// Moves to the nearest hunk start on one side of the cursor.
    ///
    /// No wrap at either end, as with `n`/`N`: a jump from the last hunk back to
    /// the first looks exactly like a jump that failed, so the last hunk says so
    /// in the bar instead. A file whose diff is pure context — a rename that
    /// changed nothing — has no hunk at all, and says that rather than the same
    /// end-of-list sentence, which would be a lie about where the cursor is.
    ///
    /// Through [`App::set_cursor_row`] like every other cursor write, so the
    /// clamp, the stack reset and the parked view are not forgotten here.
    fn step_hunk(&mut self, forward: bool) {
        let line = self.line_index();
        // The whole answer computed against the borrowed diff and the borrow
        // released before anything is written: `(where to go, is there a hunk
        // at all)`, which is the pair the two failure sentences need to tell
        let (found, any) = if self.selected_diff().is_some() {
            let lines = self.displayed_lines();
            let mut starts = hunks::hunk_starts(&lines);
            let found = if forward {
                starts.find(|start| *start > line)
            } else {
                starts.take_while(|start| *start < line).last()
            };
            (found, hunks::hunk_starts(&lines).next().is_some())
        } else {
            (None, false)
        };

        let Some(start) = found else {
            self.status = if !any {
                "no hunks in this file".to_owned()
            } else if forward {
                "the last hunk in this file".to_owned()
            } else {
                "the first hunk in this file".to_owned()
            };
            return;
        };

        let row = self.plan().row_of_line(start).unwrap_or(0);
        self.set_cursor_row(row);
        self.focus = Focus::Diff;
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
        // A scroll chosen for one file's long lines is noise on the next one's.
        self.diff_hscroll = 0;
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
            // A fast diff whose refinement was dropped — its request replaced in
            // the slot while the reviewer scrolled past — is re-asked on return.
            // Without this, one pass through a long list left every intermediate
            // file pinned to the fast diff for the rest of the session.
            let file = self.file_index;
            if self.engine == DiffEngine::Auto
                && !self.refining.contains(&file)
                && !self.refined.contains(&file)
            {
                self.request_refinement(file)?;
            }
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

        self.diffs[self.file_index] = Some(match self.engine {
            // The in-process engine first: 0.2 ms against difftastic's flat 26 ms
            // spawn, so the keystroke never waits. difftastic is asked for in the
            // background and the pane swaps when it lands — see
            // [`crate::app::diffs`].
            DiffEngine::Auto | DiffEngine::Fallback => {
                diff::compute_with(old.as_deref(), new.as_deref(), &head_path, false)
            }
            DiffEngine::Structural => diff::compute(old.as_deref(), new.as_deref(), &head_path),
        });
        // Cloned rather than moved: `refine` below takes the originals for the
        // background structural pass, and the bytes stored here are what
        // `crate::app::merges` reads to synthesize full-file context
        // — the same blobs regardless of which engine ends up answering, so
        // there is nothing to update when the structural diff later replaces
        // the fast one.
        self.blobs[self.file_index] = Some((
            old.clone().unwrap_or_default(),
            new.clone().unwrap_or_default(),
        ));
        // Parsed from the very blobs the diff was computed from, so the spans a
        // line is painted with describe the text that line came from — and parsed
        // *off* this thread, so a large file draws now and colours in a moment.
        self.parse_highlights(base_commit, base_path, old.as_deref());
        self.parse_highlights(head_commit, head_path, new.as_deref());
        // Kick the full-file merge in the background, off the very blobs the
        // diff was computed from — the pane draws the fallback view (the
        // diff's own changed-only lines) until it lands. When difftastic
        // later replaces the fast diff, `apply_refined` requeues the merge.
        self.start_merge(self.file_index);
        if self.engine == DiffEngine::Auto {
            self.refine(self.file_index, old, new);
        }
        Ok(())
    }

    /// Re-reads `file`'s blobs and asks the refiner for its structural diff.
    ///
    /// For a file whose first request was dropped by slot replacement. The blobs
    /// are re-read rather than kept from the first load: keeping every
    /// scrolled-past file's bytes alive for a maybe-return would trade a bounded
    /// re-read on selection for unbounded memory on a large review.
    fn request_refinement(&mut self, file: usize) -> Result<()> {
        let Some(entry) = self.review.files.get(file) else {
            return Ok(());
        };
        let base_path = entry
            .source_path
            .as_deref()
            .unwrap_or(&entry.path)
            .to_owned();
        let head_path = entry.path.clone();
        let base_commit = self.review.session.base_commit.clone();
        let head_commit = self.review.session.head_commit.clone();
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
        self.refine(file, old, new);
        Ok(())
    }
}
