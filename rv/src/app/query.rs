//! What the renderer, the keyboard and the tests read off an [`App`],
//! excluding diff content and the row plan — see [`super::diffview`] for
//! those.
//!
//! Everything here is derived on demand rather than cached. A node list is a
//! pure function of the state it is built from, and a cache would be one more
//! thing to keep in step — which is the shape of the defect spec §10
//! describes.

use rv_core::model::FileChange;
use rv_core::store::Session;

use super::App;
use super::Context;
use super::Focus;
use super::Mode;
use super::SidebarTab;
use crate::gradient::Stat;
use crate::layout::Split;
use crate::tree;

impl App {
    /// The file the sidebar has selected, or `None` when the range changed no
    /// files at all.
    pub fn selected_file(&self) -> Option<&FileChange> {
        self.review.files.get(self.file_index)
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

    /// Whether a sidebar row's name is tinted by its change. Session-only.
    pub fn tint(&self) -> bool {
        self.tint
    }

    /// Whether the sidebar shows its `+n -n` column. Session-only.
    pub fn counts_shown(&self) -> bool {
        self.counts
    }

    /// Which row of the file list the cursor is on — see the field.
    /// The changes the review covers, oldest first.
    #[must_use]
    pub fn changes(&self) -> &[rv_core::model::ChangeRef] {
        &self.review.session.changes
    }

    /// How many symbols the last-built index holds.
    ///
    /// Read off the cache rather than building one: the renderer takes `&App`
    /// and indexing is not a thing to do per frame. The picker builds it on `/`,
    /// which is the only key that needs the number.
    #[must_use]
    pub fn symbols_in_scope(&self) -> usize {
        self.symbol_index.len()
    }

    pub(super) fn engine(&self) -> crate::app::DiffEngine {
        self.engine
    }

    /// How far down the `i` popup is scrolled.
    #[must_use]
    pub fn info_scroll(&self) -> usize {
        self.info_scroll
    }

    /// Whether the reviewer has put the sidebar away.
    #[must_use]
    pub fn sidebar_hidden(&self) -> bool {
        self.sidebar_hidden
    }

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
        self.zoom_view(tree::build(
            &paths,
            &self.collapsed_dirs,
            self.tree,
            self.sort,
            &|index| self.stat(index),
        ))
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

    /// Where the reviewer is working, as the bar names it and the `?` tooltip
    /// filters by it. Derived, never stored — see [`Context`].
    pub fn context(&self) -> Context {
        match &self.mode {
            Mode::Comment => Context::Writing,
            Mode::ConfirmDelete { .. } => Context::Confirming,
            Mode::Pick => Context::Finding,
            Mode::Browse => match self.focus {
                Focus::Diff => Context::Diff,
                Focus::Stack => Context::Stack,
                Focus::Sidebar => match self.sidebar_tab {
                    SidebarTab::Files => Context::Files,
                    SidebarTab::Commits => Context::Commits,
                    SidebarTab::Comments => Context::Comments,
                },
            },
        }
    }

    /// What the left column is listing. Files, on launch.
    pub fn sidebar_tab(&self) -> SidebarTab {
        self.sidebar_tab
    }

    /// How the width is divided between the sidebar and the diff. Session-only.
    pub fn split(&self) -> Split {
        self.split
    }

    /// Whether the `?` keymap is up, at either size.
    pub fn help_open(&self) -> bool {
        self.help != super::HelpStage::Closed
    }

    /// Whether it is up at full size rather than as the corner tip.
    pub fn help_full(&self) -> bool {
        self.help == super::HelpStage::Full
    }

    /// How far the keymap has been scrolled, in rows. Clamped by the renderer
    /// against the popup it actually has.
    pub fn help_scroll(&self) -> usize {
        self.help_scroll
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

    /// How many columns of each diff line are scrolled off the pane's left edge.
    pub fn diff_hscroll(&self) -> usize {
        self.diff_hscroll
    }

    /// The same for the sidebar's rows.
    pub fn sidebar_hscroll(&self) -> usize {
        self.sidebar_hscroll
    }

    /// Whether the diff is grouped (removals-then-additions per hunk) rather
    /// than interleaved — the `v g` toggle, read by the pane's title.
    pub fn grouped(&self) -> bool {
        self.grouped
    }

    /// Which side of the change the diff pane is showing — the `v b` cycle, read
    /// by the pane's title.
    pub fn view_side(&self) -> super::ViewSide {
        self.view_side
    }

    /// The leader whose which-key submenu is open, if any — drawn by the popup.
    pub fn pending_leader(&self) -> Option<super::Leader> {
        self.pending_leader
    }
}
