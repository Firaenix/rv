//! What the renderer, the keyboard and the tests read off an [`App`].
//!
//! Everything here is derived on demand rather than cached. A plan and a node
//! list are pure functions of the state they are built from, and a cache would
//! be one more thing to keep in step — which is the shape of the defect spec
//! §10 describes.

use std::collections::HashSet;

use rv_core::diff::DiffLine;
use rv_core::diff::FileDiff;
use rv_core::highlight::Highlights;
use rv_core::model::FileChange;
use rv_core::model::Side;
use rv_core::store::Comment;
use rv_core::store::Session;

use super::App;
use super::Focus;
use super::Mode;
use super::SidebarTab;
use crate::gradient::Stat;
use crate::layout::Split;
use crate::rows;
use crate::rows::Plan;
use crate::tree;

impl App {
    /// The file the sidebar has selected, or `None` when the range changed no
    /// files at all.
    pub fn selected_file(&self) -> Option<&FileChange> {
        self.review.files.get(self.file_index)
    }

    /// The selected file's diff, once it has been loaded.
    pub fn selected_diff(&self) -> Option<&FileDiff> {
        self.diffs.get(self.file_index).and_then(Option::as_ref)
    }

    /// Every file in the review, in sidebar order.
    pub fn files(&self) -> &[FileChange] {
        &self.review.files
    }

    /// The range under review: its two endpoint commits and the changes
    /// between them.
    pub fn session(&self) -> &Session {
        &self.review.session
    }

    /// Which file the sidebar has selected.
    pub fn file_index(&self) -> usize {
        self.file_index
    }

    /// How many lines file `index` adds and removes, or nothing at all for an
    /// index the review has no file at.
    ///
    /// Measured once when the review was opened. The sidebar tints and counts
    /// every row from this and the status bar names the selected file's, so
    /// there is one answer rather than one per renderer.
    pub fn stat(&self, index: usize) -> Stat {
        self.stats.get(index).copied().unwrap_or_default()
    }

    /// Whether the file list is drawn as a directory tree. Session-only.
    pub fn tree_view(&self) -> bool {
        self.tree
    }

    /// The order the file list's rows are in. Session-only.
    pub fn sort(&self) -> tree::Sort {
        self.sort
    }

    /// Which row of the file list the cursor is on — see the field.
    pub fn sidebar_row(&self) -> usize {
        self.sidebar_row
    }

    /// Whether the status bar draws in ASCII, decided once at startup.
    pub fn ascii(&self) -> bool {
        self.ascii
    }

    /// The file list's rows, as the sidebar draws them and as the cursor walks
    /// them.
    ///
    /// The one place the rows are made, so what the keyboard walks and what the
    /// pane shows are the same list rather than two that agree by inspection.
    pub fn sidebar_nodes(&self) -> Vec<tree::Node> {
        let paths: Vec<&str> = self
            .review
            .files
            .iter()
            .map(|file| file.path.as_str())
            .collect();
        tree::build(
            &paths,
            &self.collapsed_dirs,
            self.tree,
            self.sort,
            &|index| self.stat(index),
        )
    }

    /// Which **row** of the selected file's plan the cursor is on.
    pub fn cursor_row(&self) -> usize {
        self.cursor_rows.get(self.file_index).copied().unwrap_or(0)
    }

    /// The row plan for the selected file, at the width the pane last drew a
    /// comment box's text in.
    ///
    /// The one place a plan is *made*: [`crate::ui::visible`] draws from this,
    /// so the rows the keyboard walks and the rows the pane shows are the same
    /// list.
    pub fn plan(&self) -> Plan<'_> {
        let Some(diff) = self.selected_diff() else {
            return Plan { rows: Vec::new() };
        };
        rows::plan(
            diff,
            &|index| self.comments_for_line(index),
            &self.collapsed,
            self.body_width.get(),
        )
    }

    /// Records how many columns of body text the pane drew a comment box with.
    ///
    /// Called by [`crate::ui::visible`] and nowhere else; the renderer is the
    /// only thing that knows how wide it drew a box.
    pub fn note_body_width(&self, width: usize) {
        self.body_width.set(width);
    }

    /// How many rows the selected file's plan has.
    pub(super) fn row_count(&self) -> usize {
        self.plan().rows.len()
    }

    /// Which line of the selected diff is highlighted: the line that **owns**
    /// the row under the cursor.
    ///
    /// **Derived, never stored.** A stored copy would be a second cursor to
    /// keep in step with [`App::cursor_row`], which is the defect spec §10
    /// describes. Zero where there is nothing to point at, because a clamp
    /// belongs on the way in and a `None` here would reach every caller for a
    /// case none of them can do anything about.
    pub fn line_index(&self) -> usize {
        self.plan().line_of_row(self.cursor_row()).unwrap_or(0)
    }

    /// What the keyboard is doing right now.
    ///
    /// By value: [`Mode`] stopped being [`Copy`] when
    /// [`Mode::ConfirmDelete`] gained the id it is about, and every caller
    /// either compares against a literal or holds the answer across the next
    /// `&mut self` call.
    pub fn mode(&self) -> Mode {
        self.mode.clone()
    }

    /// Which pane the movement keys act on. The diff, on launch: that is what
    /// a reviewer came to read.
    pub fn focus(&self) -> Focus {
        self.focus
    }

    /// What the left column is listing. Files, on launch.
    pub fn sidebar_tab(&self) -> SidebarTab {
        self.sidebar_tab
    }

    /// Which row of the comment browser the cursor is on.
    pub fn browser_index(&self) -> usize {
        self.browser_index
    }

    /// The comment the browser's cursor is on, or `None` when the sidebar is
    /// not listing comments.
    ///
    /// Gated on the tab, not the focus: `d` asks this to decide what it
    /// destroys, and answering with a comment that is not on screen is how a
    /// delete hits the wrong one. The browser draws its selection whether or
    /// not the keys are pointed at it, so the selection is real either way.
    pub fn browsed_comment(&self) -> Option<&Comment> {
        if self.sidebar_tab != SidebarTab::Comments {
            return None;
        }
        self.comments.get(self.browser_index)
    }

    /// Every comment in the review, in store order (oldest first).
    pub fn comments(&self) -> &[Comment] {
        &self.comments
    }

    /// The ids of the comments the reviewer has folded away.
    pub fn collapsed(&self) -> &HashSet<String> {
        &self.collapsed
    }

    /// Which comment of the selected line's stack the cursor is on. Only
    /// meaningful while [`App::focus`] is [`Focus::Stack`].
    pub fn comment_index(&self) -> usize {
        self.comment_index
    }

    /// The comment the stack cursor is on, or `None` when the cursor is not in
    /// a stack.
    ///
    /// Deliberately `None` off [`Focus::Stack`]: `d` and `s` ask this to decide
    /// what a keystroke acts on, and answering with a comment the reviewer has
    /// not selected is how a delete hits the wrong one.
    pub fn selected_comment(&self) -> Option<&Comment> {
        if self.focus != Focus::Stack {
            return None;
        }
        self.comments_for_line(self.line_index())
            .get(self.comment_index)
            .copied()
    }

    /// The comments anchored to diff line `index` of the selected file, oldest
    /// first.
    ///
    /// Matched by the key the line would anchor *under*, never by its raw
    /// number: the side and the side's path come from the same
    /// [`App::anchor_target`] the save path goes through, so a comment can
    /// never be stored against one line and displayed against another.
    pub fn comments_for_line(&self, index: usize) -> Vec<&Comment> {
        let Some(line) = self.selected_diff().and_then(|diff| diff.lines.get(index)) else {
            return Vec::new();
        };
        let Some(target) = self.anchor_target(line) else {
            return Vec::new();
        };
        self.comments
            .iter()
            .filter(|comment| {
                comment.anchor.file == target.path
                    && comment.anchor.side == target.side
                    && comment.anchor.line == target.number
            })
            .collect()
    }

    /// How many comments the selected line carries.
    pub(super) fn stack_len(&self) -> usize {
        self.comments_for_line(self.line_index()).len()
    }

    pub(super) fn selected_line(&self) -> Option<&DiffLine> {
        self.selected_diff()
            .and_then(|diff| diff.lines.get(self.line_index()))
    }

    /// How the width is divided between the sidebar and the diff. Session-only.
    pub fn split(&self) -> Split {
        self.split
    }

    /// Whether the `?` keymap is up.
    pub fn help_open(&self) -> bool {
        self.help_open
    }

    /// How far the keymap has been scrolled, in rows. Clamped by the renderer
    /// against the popup it actually has.
    pub fn help_scroll(&self) -> usize {
        self.help_scroll
    }

    /// The highlight spans for the selected file's blob **on `side`**.
    ///
    /// `None` for a side the commit has no plain file at, and for a file whose
    /// extension names no grammar rv ships.
    ///
    /// Callers choose `side` through [`super::anchored_side`] and nothing else:
    /// a removed line looked up on the head side would be painted with the
    /// colours of whatever now stands at its number, which is a lie told in a
    /// colour rather than in words.
    pub fn highlights(&self, side: Side) -> Option<&Highlights> {
        let file = self.selected_file()?;
        let session = &self.review.session;
        let (commit, path) = match side {
            Side::Left => (
                session.base_commit.as_str(),
                file.source_path.as_deref().unwrap_or(&file.path),
            ),
            Side::Right => (session.head_commit.as_str(), file.path.as_str()),
        };
        self.highlights.get(&(commit.to_owned(), path.to_owned()))
    }

    /// The comment being typed, empty outside [`Mode::Comment`].
    pub fn buffer(&self) -> &str {
        &self.buffer
    }

    /// The one-line message under the reviewer's last action.
    pub fn status(&self) -> &str {
        &self.status
    }

    /// Where the wheel has parked the diff pane's window, as the first row on
    /// screen — [`None`] while the view is following the cursor.
    pub fn diff_scroll(&self) -> Option<usize> {
        self.diff_scroll
    }

    /// The same for the sidebar's list.
    pub fn sidebar_scroll(&self) -> Option<usize> {
        self.sidebar_scroll
    }
}
