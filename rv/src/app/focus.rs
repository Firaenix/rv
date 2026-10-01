//! Which panel has the focus, and moving between them: `←`/`→` drill the tree
//! and cross the panes — in the diff they walk the column cursor first — `Tab`
//! swaps between the two panels, and a file row opens into the diff.

use anyhow::Result;

use super::App;
use super::Focus;
use super::SidebarTab;
use crate::tree::NodeKind;

impl App {
    /// `←`: one character left along the diff line, and out to the sidebar once
    /// there is no line left to walk; out of a comment stack and back to the
    /// diff; and — inside the file list or the commits list — up one level of
    /// the tree the reviewer drilled into with `→`.
    pub(super) fn focus_left(&mut self) -> Result<()> {
        // A symbol is picked out by walking to it, so the arrow belongs to the
        // column while there is somewhere to go; the start of the line is where
        // it means the pane again.
        if self.focus == Focus::Diff && self.step_column(false) {
            return Ok(());
        }
        match self.focus {
            Focus::Stack | Focus::Flag => self.focus = Focus::Diff,
            Focus::Diff => self.focus = Focus::Sidebar,
            Focus::Sidebar => match self.sidebar_tab {
                // The comment browser has no tree to climb, so `←` leads out to
                // the diff the way it does from every other focus.
                SidebarTab::Comments | SidebarTab::Flags => self.focus = Focus::Diff,
                SidebarTab::Files | SidebarTab::Commits => self.zoom_out(),
            },
        }
        Ok(())
    }

    /// `→`: into whatever the cursor is on. In the diff it is one character
    /// right along the line, and in the stack there is nothing to its right. In
    /// the file list a directory or change is drilled into, and a file is
    /// opened with the focus following it to the diff; in the comment browser it
    /// jumps to the browsed comment's code.
    pub(super) fn focus_right(&mut self) -> Result<()> {
        if self.focus == Focus::Diff {
            self.step_column(true);
            return Ok(());
        }
        if self.focus != Focus::Sidebar {
            return Ok(());
        }
        match self.sidebar_tab {
            SidebarTab::Comments | SidebarTab::Flags => self.enter_browser_row(),
            SidebarTab::Files | SidebarTab::Commits => {
                if self.zoom_into_under_cursor() {
                    return Ok(());
                }
                // Not a row that drills — a file. Opening it moves the focus to
                // the diff, which is the one thing `→` can mean on a leaf.
                self.enter_file_under_cursor()
            }
        }
    }

    /// `Tab`: from the diff a plain focus toggle — the sidebar, on whatever tab
    /// it was left on — and from the sidebar the mode rotation: the files list,
    /// the commits list, the comment browser, the flag browser, then back to the
    /// diff. A comment stack counts as the diff it lives in.
    ///
    /// Rotation is scoped to the sidebar on purpose. `Tab` used to force `Files`
    /// on the way in, which discarded the tab the reviewer had chosen: a trip to
    /// the diff and back was a trip to a different list.
    pub(super) fn cycle_mode(&mut self) -> Result<()> {
        let on_diff = matches!(self.focus, Focus::Diff | Focus::Stack | Focus::Flag);
        match (on_diff, self.sidebar_tab) {
            (true, tab) => self.goto_mode(tab),
            (false, SidebarTab::Files) => self.goto_mode(SidebarTab::Commits),
            (false, SidebarTab::Commits) => self.goto_mode(SidebarTab::Comments),
            (false, SidebarTab::Comments) => self.goto_mode(SidebarTab::Flags),
            (false, SidebarTab::Flags) => {
                self.focus = Focus::Diff;
                Ok(())
            }
        }
    }

    /// Opens the file under the sidebar cursor and moves the focus to the diff;
    /// a non-file row is left alone.
    pub(super) fn enter_file_under_cursor(&mut self) -> Result<()> {
        let nodes = self.nodes();
        let Some(NodeKind::File { index }) = nodes.get(self.sidebar_row).map(|n| &n.kind) else {
            return Ok(());
        };
        let index = *index;
        self.select_node_file(index)?;
        self.focus = Focus::Diff;
        Ok(())
    }
}
