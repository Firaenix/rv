//! The left column: which list it shows, what shape and order that list is in,
//! and where its cursor sits after the rows underneath have been rebuilt.
//!
//! Every preference here is **session-only** — none of it reaches `.review/`,
//! because how one reviewer likes their file list is not something another
//! reviewer, or an LLM reading the export, should inherit.

use anyhow::Result;

use super::App;
use super::DiffEngine;
use super::Focus;
use super::SidebarTab;
use super::status::VIEW_KEYS_ARE_FOR_THE_FILE_LIST;
use crate::tree;
use crate::tree::NodeKind;

/// One row of the comment browser.
///
/// The browser draws file headings between the comments, so a row and a
/// comment stopped being the same number. **The cursor is a row and the
/// comment is derived from it** — the sibling of the ruling `cursor_rows`
/// records for the diff pane, and for the same reason: two cursors kept in
/// step is the defect, not the fix.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BrowserRow {
    /// The file every comment row under it is anchored in.
    File(String),
    /// A comment, addressing [`App::comments`] — its *store* position, which
    /// is not this row's number and never becomes it.
    Comment(usize),
}

/// How many of the review's files carry no semantic change, over how many the
/// review actually has a structural answer for.
///
/// Two numbers rather than one because spec §7 loads blobs lazily: rv knows a
/// file is suppressed only once its diff has been computed, so a bare count
/// would be a claim about the whole review made from the part of it the
/// reviewer happens to have opened.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Suppression {
    /// Files whose computed diff came back suppressed.
    pub suppressed: usize,
    /// Files rv has a settled answer for.
    pub checked: usize,
    /// Files in the review.
    pub total: usize,
}

impl App {
    /// Flips the left column between the files and the comments.
    ///
    /// Says nothing in the status line: it is navigation, the pane's own title
    /// reports which tab is up, and a key that overwrote the help text to
    /// announce itself would cost the reviewer the keymap they read off it.
    pub(super) fn switch_tab(&mut self) -> Result<()> {
        self.goto_tab(match self.sidebar_tab {
            SidebarTab::Files => SidebarTab::Commits,
            SidebarTab::Commits => SidebarTab::Comments,
            SidebarTab::Comments => SidebarTab::Files,
        })
    }

    /// Shows `tab` in the left column — `Tab`'s cycle, and `1`/`2`/`3`
    /// directly.
    pub(super) fn goto_tab(&mut self, tab: SidebarTab) -> Result<()> {
        if tab == self.sidebar_tab {
            return Ok(());
        }
        self.sidebar_tab = tab;
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
        // Switching views preserves position (navigation spec §3): the cursor
        // lands on the selected file's row in the list that just appeared — in
        // the commits tab, under the newest change that touched it. The raw
        // row index is meaningless across two different lists; the file is
        // what the reviewer was on.
        if let Some(row) = self.row_of_selected_file() {
            self.sidebar_row = row;
            if let Some(tree::NodeKind::File { index }) = self
                .nodes()
                .get(self.sidebar_row)
                .map(|node| node.kind.clone())
            {
                self.select_node_file(index)?;
            }
        } else {
            // No row shows the file — folded away, or the Comments tab — so
            // the cursor is clamped onto whichever list appeared.
            self.sidebar_row = self.sidebar_row.min(self.nodes().len().saturating_sub(1));
            // And the row it lands on has to be *selected*, not merely
            // highlighted: a file row under the cursor with the previous
            // tab's diff still on screen is the sidebar naming one thing and
            // the pane showing another.
            if let Some(tree::NodeKind::File { index }) = self
                .nodes()
                .get(self.sidebar_row)
                .map(|node| node.kind.clone())
            {
                self.select_node_file(index)?;
            }
        }
        // A parked view is a row of *the list that was showing*; the other tab
        // is a different list of a different length.
        self.sidebar_scroll = None;
        self.clamp_browser();
        Ok(())
    }

    /// The row of the current tab's list that names the selected file, or
    /// `None` where no row does. In the commits tab the first match wins,
    /// which — the stack being listed newest first — is the newest change
    /// that touched the file.
    fn row_of_selected_file(&self) -> Option<usize> {
        let selected = self.selected_file()?.path.clone();
        self.nodes().iter().position(|node| {
            let tree::NodeKind::File { index } = node.kind else {
                return false;
            };
            match self.sidebar_tab {
                SidebarTab::Files => index == self.file_index,
                SidebarTab::Commits => self.commit_path(index) == Some(selected.as_str()),
                SidebarTab::Comments => false,
            }
        })
    }

    /// The comment browser's rows: every comment in the review under a heading
    /// naming its file, ordered by `(file, line)`.
    ///
    /// Grouped rather than flat because a reviewer returning to a review moves
    /// through what they *said*, and what they said is organised by where they
    /// said it. The heading is a row and not a collapsible node — inline
    /// comments spec §3 — so nothing here can hide a comment behind an
    /// expansion.
    ///
    /// Derived per call like every other list in [`super::query`]: it is a pure
    /// function of `comments`, and a cached copy would be one more thing to
    /// keep in step.
    #[must_use]
    pub fn browser_rows(&self) -> Vec<BrowserRow> {
        let mut order: Vec<usize> = (0..self.comments.len()).collect();
        // By file and line, with the store position last so that two comments
        // on one line keep the order they were written in — the browser has
        // always opened on the oldest.
        order.sort_by(|a, b| {
            let (left, right) = (&self.comments[*a].anchor, &self.comments[*b].anchor);
            left.file
                .cmp(&right.file)
                .then(left.line.cmp(&right.line))
                .then(a.cmp(b))
        });
        let mut rows = Vec::with_capacity(order.len());
        let mut heading: Option<&str> = None;
        for index in order {
            let file = self.comments[index].anchor.file.as_str();
            if heading != Some(file) {
                heading = Some(file);
                rows.push(BrowserRow::File(file.to_owned()));
            }
            rows.push(BrowserRow::Comment(index));
        }
        rows
    }

    /// Which comment the browser's cursor is on, as a position in
    /// [`App::comments`] — `None` on a heading row and on an empty review.
    pub(super) fn browsed_index(&self) -> Option<usize> {
        match self.browser_rows().get(self.browser_index)? {
            BrowserRow::Comment(index) => Some(*index),
            BrowserRow::File(_) => None,
        }
    }

    /// Keeps the browser's cursor on the list after the list has changed under
    /// it, and off a heading.
    ///
    /// A heading is not selectable, so a clamp that landed on one would leave
    /// `d` and `s` with no target on a row the reviewer can see. The step is
    /// **forwards**, onto the first comment of the file the heading names,
    /// because that is the row the reviewer was reaching for; only a cursor
    /// clamped onto a trailing heading — which cannot happen, every heading has
    /// a comment under it — would have nowhere forward to go. An empty list
    /// parks it at 0, which is where the next comment lands.
    pub(super) fn clamp_browser(&mut self) {
        let rows = self.browser_rows();
        self.browser_index = self.browser_index.min(rows.len().saturating_sub(1));
        if matches!(rows.get(self.browser_index), Some(BrowserRow::File(_))) {
            self.browser_index = self.browser_index.saturating_add(1);
        }
    }

    /// How much of the review carries no semantic change, and how much of that
    /// question rv can currently answer.
    ///
    /// Read off the diffs already computed, **never computing one**. Spec §7
    /// loads blobs lazily for the file being viewed, and this note is not worth
    /// overturning that: measured on this machine difftastic costs a flat
    /// ~26 ms per file whatever its size, so answering eagerly for a 40-file
    /// review costs about a second of dead time before the first frame — paid
    /// on every run, to print one sentence.
    ///
    /// So the count grows as the reviewer browses, and the renderer says which
    /// number it is out of. A bare "N files with no semantic change" that
    /// silently meant "N of the ones you have opened" is exactly the guess
    /// presented as a fact that this project refuses.
    ///
    /// **Only settled diffs are counted.** Under [`DiffEngine::Auto`] the
    /// in-process engine answers first and difftastic replaces it, and the two
    /// disagree about exactly this flag: a reindentation is `Added`/`Removed`
    /// lines to `similar` and `unchanged` to difftastic. Counting the fast
    /// answer would print a number that moved without the reviewer doing
    /// anything, which is worse than a number that is still growing.
    #[must_use]
    pub fn suppression(&self) -> Suppression {
        let settled = |file: usize| self.engine != DiffEngine::Auto || self.refined.contains(&file);
        self.diffs
            .iter()
            .enumerate()
            .filter(|(file, _)| settled(*file))
            .filter_map(|(_, diff)| diff.as_ref())
            .fold(
                Suppression {
                    total: self.review.files.len(),
                    ..Suppression::default()
                },
                |counted, diff| Suppression {
                    suppressed: counted.suppressed + usize::from(diff.suppressed),
                    checked: counted.checked + 1,
                    ..counted
                },
            )
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
