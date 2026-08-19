//! The left column: which list it shows, what shape and order that list is in,
//! and where its cursor sits after the rows underneath have been rebuilt.
//!
//! Every preference here is **session-only** — none of it reaches `.review/`,
//! because how one reviewer likes their file list is not something another
//! reviewer, or an LLM reading the export, should inherit.

use anyhow::Result;

use super::App;
use super::Focus;
use super::SidebarTab;
use super::status::VIEW_KEYS_ARE_FOR_THE_FILE_LIST;
use crate::tree;
use crate::tree::NodeKind;

impl App {
    /// Flips the left column between the files and the comments.
    ///
    /// Says nothing in the status line: it is navigation, the pane's own title
    /// reports which tab is up, and a key that overwrote the help text to
    /// announce itself would cost the reviewer the keymap they read off it.
    pub(super) fn switch_tab(&mut self) -> Result<()> {
        self.sidebar_tab = match self.sidebar_tab {
            SidebarTab::Files => SidebarTab::Commits,
            SidebarTab::Commits => SidebarTab::Comments,
            SidebarTab::Comments => SidebarTab::Files,
        };
        // A zoom is an address in the tab it was made in; carried across it
        // would go dormant here and ambush the reviewer on the way back.
        self.zoom.clear();
        // An enumeration failure renders as a change with no files under it, so
        // it is also *said*: the alert dedupes, so revisiting the tab does not
        // stack toasts.
        if self.sidebar_tab == SidebarTab::Commits {
            for error in self.commit_index().errors().to_vec() {
                self.raise(error);
            }
        }
        // The two node tabs share one cursor — they are never both on screen —
        // so it has to be clamped onto whichever list just appeared.
        self.sidebar_row = self.sidebar_row.min(self.nodes().len().saturating_sub(1));
        // And the row it lands on has to be *selected*, not merely highlighted.
        // Without this the commits tab opened with a file row under the cursor
        // and the previous tab's diff still on screen — the sidebar naming one
        // thing and the pane showing another, which is the one state this
        // interface must never be in.
        if let Some(tree::NodeKind::File { index }) = self
            .nodes()
            .get(self.sidebar_row)
            .map(|node| node.kind.clone())
        {
            self.select_node_file(index)?;
        }
        // A parked view is a row of *the list that was showing*; the other tab
        // is a different list of a different length.
        self.sidebar_scroll = None;
        self.clamp_browser();
        Ok(())
    }

    /// Keeps the browser's cursor on the list after the list has changed under
    /// it. An empty list parks it at 0, which is where the next comment lands.
    pub(super) fn clamp_browser(&mut self) {
        self.browser_index = self
            .browser_index
            .min(self.comments.len().saturating_sub(1));
    }

    /// `t`: flips the file list between a flat list of whole paths and a
    /// directory tree.
    pub(super) fn toggle_tree(&mut self) {
        if self.sidebar_tab == SidebarTab::Comments {
            self.status = VIEW_KEYS_ARE_FOR_THE_FILE_LIST.to_owned();
            return;
        }
        self.tree = !self.tree;
        self.resettle_sidebar();
    }

    /// `z`: puts the sidebar away, or brings it back.
    ///
    /// The focus comes with it. A sidebar that is not on screen must not still
    /// hold the cursor — every key would then be acting on a list the reviewer
    /// cannot see.
    pub(super) fn toggle_sidebar(&mut self) {
        self.sidebar_hidden = !self.sidebar_hidden;
        if self.sidebar_hidden && self.focus == Focus::Sidebar {
            self.focus = Focus::Diff;
        }
        self.status = if self.sidebar_hidden {
            "sidebar hidden — z brings it back".to_owned()
        } else {
            "sidebar shown".to_owned()
        };
    }

    /// `g`: tints the row names by their change's proportion, or stops.
    pub(super) fn toggle_tint(&mut self) {
        if self.sidebar_tab == SidebarTab::Comments {
            self.status = VIEW_KEYS_ARE_FOR_THE_FILE_LIST.to_owned();
            return;
        }
        self.tint = !self.tint;
        self.status = if self.tint {
            "names tinted by their change — g turns it off".to_owned()
        } else {
            "names untinted".to_owned()
        };
    }

    /// `#`: shows the sidebar's `+n -n` column, or puts it away.
    pub(super) fn toggle_counts(&mut self) {
        if self.sidebar_tab == SidebarTab::Comments {
            self.status = VIEW_KEYS_ARE_FOR_THE_FILE_LIST.to_owned();
            return;
        }
        self.counts = !self.counts;
        self.status = if self.counts {
            "counts shown".to_owned()
        } else {
            "counts hidden — # brings them back".to_owned()
        };
    }

    /// `o`: cycles the file list's order. See [`crate::tree::Sort`], whose
    /// `next` is what "cycles" means, declared beside the orders themselves.
    pub(super) fn cycle_sort(&mut self) {
        if self.sidebar_tab == SidebarTab::Comments {
            self.status = VIEW_KEYS_ARE_FOR_THE_FILE_LIST.to_owned();
            return;
        }
        self.sort = self.sort.next();
        self.resettle_sidebar();
    }

    /// Puts the file list's cursor back on the row that holds the selected
    /// file, after something rebuilt the rows under it.
    ///
    /// The *file* is what survives such a change; a row index is an address in
    /// a list that has just been rewritten. A selected file with no row of its
    /// own — it is inside a folded directory — leaves the cursor where it was,
    /// clamped onto the list.
    pub(super) fn resettle_sidebar(&mut self) {
        self.sidebar_scroll = None;
        if self.sidebar_tab == SidebarTab::Commits {
            // A commits row is a (change, file) pair, not a bookmark file, so
            // there is no file index to look up. Clamp and leave the cursor
            // where the reviewer put it.
            let rows = self.nodes().len();
            self.sidebar_row = self.sidebar_row.min(rows.saturating_sub(1));
            return;
        }
        let nodes = self.sidebar_nodes();
        let selected = self.file_index;
        self.sidebar_row = nodes
            .iter()
            .position(|node| matches!(node.kind, NodeKind::File { index } if index == selected))
            .unwrap_or_else(|| self.sidebar_row.min(nodes.len().saturating_sub(1)));
    }

    /// Which directory (or change) `s` would fold, as the key it folds under,
    /// or `None` where the cursor is not on a row that holds anything.
    ///
    /// Only from the file list: `s` means *fold the thing under the cursor*,
    /// and everywhere else that thing is a comment.
    pub(super) fn sidebar_fold_key(&self) -> Option<String> {
        if self.focus != Focus::Sidebar || self.sidebar_tab == SidebarTab::Comments {
            return None;
        }
        match &self.nodes().get(self.sidebar_row)?.kind {
            NodeKind::Dir { key, .. } => Some(key.clone()),
            NodeKind::Commit { change_id, .. } => Some(change_id.clone()),
            NodeKind::File { .. } | NodeKind::Up => None,
        }
    }

    /// Folds the directory row under the cursor, or unfolds it.
    ///
    /// The folded row is still under the cursor — folding only ever removes
    /// rows *below* it — so the cursor is clamped rather than resettled onto
    /// the selected file, which may now be inside what was just folded away.
    pub(super) fn toggle_dir_fold(&mut self, key: String) {
        if !self.collapsed_dirs.remove(&key) {
            self.collapsed_dirs.insert(key);
        }
        let rows = self.nodes().len();
        self.sidebar_row = self.sidebar_row.min(rows.saturating_sub(1));
        // The list is a different length, so a parked view is an address in it
        // that no longer means what it did.
        self.sidebar_scroll = None;
    }
}
