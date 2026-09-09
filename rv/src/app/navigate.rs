//! Moving the cursor.
//!
//! What happens where it lands — the file selected, and the diff and blobs
//! that file is drawn from — is [`opening`].

use anyhow::Result;

use super::App;
use super::Focus;
use super::SidebarTab;
use super::hunks;
use super::sidebar::BrowserRow;
use crate::tree::NodeKind;

mod opening;

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
        let is_comment = |row: &usize| matches!(rows[*row], BrowserRow::Comment { .. });
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
            (0..rows.len()).filter(|row| matches!(rows[*row], BrowserRow::Comment { .. }));
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
        self.remember_cursor_anchor();
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
    ///
    /// Nor does it touch the recorded cursor anchor: the cursor moved to the row
    /// that *owns* `line`, so the place in the code it stands on is the one
    /// already recorded.
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

    /// Keeps the cursor a row of the plan it indexes after a view toggle
    /// rebuilt that plan shorter under it.
    ///
    /// Not [`App::resettle_cursor`]: `f` and `v b` change *which* lines exist, so
    /// the line the cursor was on is not a fact that survives the change the way
    /// it does across a fold or a delete. The row is all there is to hold on to,
    /// so the cursor keeps its position and is clamped to the last row the
    /// shorter plan has — the same clamp the diff pane applies to what it draws.
    ///
    /// A row the reviewer never moved to names no place in the code, so when
    /// this actually moves the cursor it drops the recorded anchor: a later
    /// background result must not drag the cursor back to a place the reviewer
    /// has already been moved off.
    pub(super) fn clamp_cursor_to_plan(&mut self) {
        let last = self.row_count().saturating_sub(1);
        let moved = match self.cursor_rows.get_mut(self.file_index) {
            Some(position) => {
                let clamped = (*position).min(last);
                let moved = clamped != *position;
                *position = clamped;
                moved
            }
            None => false,
        };
        if moved {
            self.cursor_anchor = None;
        }
    }
}
