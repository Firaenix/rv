//! The comment stack: stepping into it, out of it, and jumping from a comment
//! to the code it is about.

use anyhow::Result;
use rv_core::model::Anchor;

use super::App;
use super::Focus;
use super::SidebarTab;
use super::sidebar::BrowserRow;
use super::status::NO_COMMENTS;

impl App {
    /// `Enter`: into the selected line's comment stack, from the comment
    /// browser to the code the browsed comment is about, and in the file list
    /// **into** the row under the cursor — a zoom, not a fold, which is
    /// `Space`'s verb.
    pub(super) fn on_enter(&mut self) -> Result<()> {
        if self.focus == Focus::Sidebar {
            if self.sidebar_tab == SidebarTab::Comments {
                return self.enter_browser_row();
            }
            if self.zoom_under_cursor() {
                return Ok(());
            }
        }
        self.enter_stack();
        Ok(())
    }

    /// `Enter` in the comment browser: to the browsed comment's code, or — on
    /// a file heading — to the top of that file's diff.
    ///
    /// A heading names a file and nothing else, so opening that file is the one
    /// thing it can defensibly mean. Jumping to some comment under it would be
    /// picking one the reviewer did not point at; refusing outright would make
    /// a visible row inert.
    fn enter_browser_row(&mut self) -> Result<()> {
        match self.browser_rows().get(self.browser_index) {
            Some(BrowserRow::Comment(index)) => self.jump_to_comment(*index),
            Some(BrowserRow::File(path)) => self.open_file_named(&path.clone()),
            None => Ok(()),
        }
    }

    /// Opens the review's file at `path`, at its top, with the focus on the
    /// diff — the heading row's half of a jump.
    ///
    /// Either side's path, like [`App::jump_to_comment`]: a comment on a
    /// removed line is filed under the base-side path, which for a rename is
    /// not the path the file is listed under.
    fn open_file_named(&mut self, path: &str) -> Result<()> {
        let found = self
            .review
            .files
            .iter()
            .position(|file| file.path == path || file.source_path.as_deref() == Some(path));
        let Some(index) = found else {
            let message = format!("{path} is not in this review's range any more");
            self.status = message.clone();
            self.raise(message);
            return Ok(());
        };
        self.file_index = index;
        self.load_selected()?;
        self.set_cursor_row(0);
        self.focus = Focus::Diff;
        self.status = format!("opened {path}");
        self.resettle_sidebar();
        Ok(())
    }

    /// `Space`: folds the row under the cursor where it holds things, and is
    /// [`App::on_enter`] everywhere else — the two keys were one verb until the
    /// zoom gave `Enter` a meaning of its own on a directory.
    pub(super) fn fold_row(&mut self) -> Result<()> {
        if let Some(key) = self.sidebar_fold_key() {
            self.toggle_dir_fold(key);
            return Ok(());
        }
        self.on_enter()
    }

    /// Steps the cursor into the selected line's comment stack.
    ///
    /// From [`Focus::Diff`] only. From the Files tab `Enter` is unbound, and
    /// from inside the stack it is inert rather than a jump back to the first
    /// comment — a key that quietly moved the cursor mid-`j` would be a key the
    /// reviewer had to be careful of.
    ///
    /// A line with nothing on it is refused with a sentence rather than
    /// entered: an empty stack is a focus containing nothing.
    fn enter_stack(&mut self) {
        if self.focus != Focus::Diff {
            return;
        }
        if self.comments_for_line(self.line_index()).is_empty() {
            self.status = NO_COMMENTS.to_owned();
            return;
        }
        self.focus = Focus::Stack;
        self.comment_index = 0;
    }

    /// `Esc`: out of the stack, or one zoom level back out of the file list —
    /// wherever the reviewer has stepped *into* something, this is the step
    /// back, so nowhere is somewhere they can get stuck.
    pub(super) fn escape(&mut self) {
        if self.focus == Focus::Stack {
            self.focus = Focus::Diff;
            return;
        }
        if self.focus == Focus::Sidebar && self.zoomed() {
            self.zoom_out();
        }
    }

    /// Takes the cursor out of the comment stack and puts the stack index back
    /// at the top, because the *selection* moved out from under both.
    ///
    /// The focus leaves **unconditionally**. Entering a stack is a deliberate
    /// act, so navigation may never hand it on: `]` off a stack onto a file
    /// whose current line also carries comments would otherwise land the cursor
    /// inside that line's stack, having never entered it, with `d` and `s`
    /// aimed at a comment nobody selected. A conditional version of this
    /// shipped once and its test passed vacuously.
    pub(super) fn reset_stack(&mut self) {
        self.comment_index = 0;
        if self.focus == Focus::Stack {
            self.focus = Focus::Diff;
        }
    }

    /// Puts the stack cursor back inside the stack after the stack has changed
    /// under it.
    ///
    /// The sibling of [`App::reset_stack`], which is for when the *selection*
    /// moves: there the cursor goes back to the top, here it stays as close as
    /// it can to the comment it was on, because a delete is something the
    /// reviewer does *inside* a stack they are working through. An emptied
    /// stack hands the focus back to the diff.
    pub(super) fn sync_stack(&mut self) {
        match self.stack_len() {
            0 => {
                self.comment_index = 0;
                if self.focus == Focus::Stack {
                    self.focus = Focus::Diff;
                }
            }
            total => self.comment_index = self.comment_index.min(total - 1),
        }
    }

    /// Selects the file and line a comment is anchored to and hands the focus
    /// to the diff, so that reading a comment and looking at the code it is
    /// about are one keystroke apart.
    ///
    /// Two honest failure cases, both reported rather than papered over: the
    /// anchored **file** may have left the review's range, in which case
    /// nothing moves because there is nowhere to move to; or the anchored
    /// **line** may not be in the current diff, in which case the file opens
    /// anyway, at its top, with the line named. Being in the right file with a
    /// warning beats staying put and saying nothing.
    fn jump_to_comment(&mut self, index: usize) -> Result<()> {
        let Some(comment) = self.comments.get(index) else {
            return Ok(());
        };
        let anchor = comment.anchor.clone();

        // Either side's path: a comment on a removed line is filed under the
        // base-side path, which for a rename is not the path it is listed under.
        let found = self.review.files.iter().position(|file| {
            file.path == anchor.file || file.source_path.as_deref() == Some(anchor.file.as_str())
        });
        let Some(file_index) = found else {
            // A status *and* an alert: the status says where the reviewer is,
            // which is where they were, and a line in the bar is the easiest
            // thing on screen to miss.
            let message = format!("{} is not in this review's range any more", anchor.file);
            self.status = message.clone();
            self.raise(message);
            return Ok(());
        };

        self.file_index = file_index;
        self.load_selected()?;
        match self.line_of_anchor(&anchor) {
            Some(line) => {
                // Onto the line's own diff row rather than into its stack, so
                // `c` and `d` mean what the reviewer just clicked on.
                let row = self.plan().row_of_line(line).unwrap_or(0);
                self.set_cursor_row(row);
                self.status = format!("jumped to {}:{}", anchor.file, anchor.line);
            }
            None => {
                self.set_cursor_row(0);
                let message = format!(
                    "{}: line {} is not in this diff any more",
                    anchor.file, anchor.line
                );
                self.status = message.clone();
                self.raise(message);
            }
        }
        self.focus = Focus::Diff;
        // The sidebar's cursor follows the jump: left as it was, the next walk
        // in the Files tab would re-select the pre-jump file from a row that no
        // longer names the selection.
        self.resettle_sidebar();
        Ok(())
    }

    /// The diff line whose anchor key matches `anchor`, using the same
    /// [`App::anchor_target`] the save path goes through — so the line a jump
    /// lands on is by construction the line the comment was stored against,
    /// rename, side rule and all.
    fn line_of_anchor(&self, anchor: &Anchor) -> Option<usize> {
        self.selected_diff()?;
        let lines = self.displayed();
        (0..lines.len()).find(|index| {
            self.anchor_target(&lines[*index]).is_some_and(|target| {
                target.path == anchor.file
                    && target.side == anchor.side
                    && target.number == anchor.line
            })
        })
    }
}
