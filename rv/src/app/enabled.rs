//! Whether a binding would do anything from where the cursor is: what dims a
//! row of the `?` popup and the tip.

use rv_core::model::ChangeKind;

use super::App;
use super::Focus;
use super::SidebarTab;
use super::bindings::Binding;
use super::bindings::Command;
use super::hunks;
use crate::tree::NodeKind;

impl App {
    /// Whether `binding` would do anything from where the cursor is now.
    ///
    /// The popup dims the ones that would not rather than hiding them: a
    /// reviewer should see that `d` exists and that the file list is the wrong
    /// place for it. Every arm asks the same question the key itself asks, so
    /// the popup cannot claim a key is live where it would refuse.
    pub fn binding_enabled(&self, binding: &Binding) -> bool {
        match binding.command {
            // Always something to do: they change what is on screen, never what
            // is under the cursor.
            Command::SwitchTab
            | Command::FilesTab
            | Command::CommitsTab
            | Command::CommentsTab
            | Command::Narrower
            | Command::Wider
            | Command::Help
            | Command::ToggleSidebar
            | Command::Refresh
            // A review with no comments still has a session to export.
            | Command::Export
            | Command::Quit => true,
            Command::Forward => self.can_move_forward(),
            Command::Back => self.can_move_back(),
            Command::NextFile => self.file_index + 1 < self.review.files.len(),
            // Asked of the cache rather than of a fresh index: the popup is
            // drawn from `&self` and indexing is not a thing to do per frame. An
            // unbuilt index reports live, and the key says so itself.
            Command::NextSymbol | Command::PreviousSymbol | Command::Pick => {
                self.indexed_scope.is_none() || !self.symbol_index.is_empty()
            }
            Command::PreviousFile => self.file_index > 0,
            // Live wherever the file has a hunk to reach from where the cursor
            // is; the key itself says which end it has run out at.
            Command::NextHunk => self.hunk_ahead(true),
            Command::PreviousHunk => self.hunk_ahead(false),
            // A file that exists at the head is one an editor can open. Whether
            // `$EDITOR` is set is not asked here: the popup is drawn every
            // frame, and dimming a key over an environment variable would teach
            // that the key does not exist rather than that the variable is
            // unset — which is what the key says when it is pressed.
            Command::OpenEditor => self
                .selected_file()
                .is_some_and(|file| file.kind != ChangeKind::Removed),
            // Rightwards is always live — only the renderer knows how long the
            // longest line is — and leftwards has somewhere to go once it is.
            Command::ScrollRight => true,
            Command::ScrollLeft => match self.focus {
                Focus::Sidebar => self.sidebar_hscroll > 0,
                Focus::Diff | Focus::Stack => self.diff_hscroll > 0,
            },
            Command::FocusLeft => self.focus != Focus::Sidebar,
            Command::FocusRight => self.focus == Focus::Sidebar,
            Command::Enter => match (self.focus, self.sidebar_tab) {
                // Every browser row leads somewhere: a comment to its code, a
                // heading to the top of the file it names.
                (Focus::Sidebar, SidebarTab::Comments) => !self.browser_rows().is_empty(),
                // A row that holds things can be zoomed into, and the Up row
                // zoomed back out of; only a file row leaves `Enter` nothing.
                (Focus::Sidebar, _) => matches!(
                    self.nodes().get(self.sidebar_row).map(|node| &node.kind),
                    Some(NodeKind::Dir { .. } | NodeKind::Commit { .. } | NodeKind::Up)
                ),
                (Focus::Diff, _) => !self.comments_for_line(self.line_index()).is_empty(),
                (Focus::Stack, _) => false,
            },
            Command::FoldRow => {
                self.sidebar_fold_key().is_some()
                    || (self.focus == Focus::Diff
                        && !self.comments_for_line(self.line_index()).is_empty())
            }
            Command::Escape => {
                self.focus == Focus::Stack || (self.focus == Focus::Sidebar && self.zoomed())
            }
            Command::Comment => self.selected_line().is_some(),
            Command::Delete => self.delete_target().is_some(),
            Command::Resolve | Command::Abandon => self.settle_target().is_some(),
            // Two things under one key, so two ways for it to have a target.
            Command::Fold => self.sidebar_fold_key().is_some() || !self.fold_targets().is_empty(),
            Command::ToggleTree
            | Command::CycleSort
            | Command::ToggleTint
            | Command::ToggleCounts => self.sidebar_tab != SidebarTab::Comments,
            Command::ToggleFullContext => true,
            Command::Info => self.sidebar_tab == SidebarTab::Commits,
        }
    }

    /// Whether `j` has anywhere to go in the pane that has the cursor.
    fn can_move_forward(&self) -> bool {
        match self.focus {
            Focus::Sidebar => match self.sidebar_tab {
                SidebarTab::Files | SidebarTab::Commits => {
                    self.sidebar_row + 1 < self.nodes().len()
                }
                SidebarTab::Comments => self.browser_index + 1 < self.browser_rows().len(),
            },
            Focus::Diff => self.cursor_row() + 1 < self.row_count(),
            Focus::Stack => self.comment_index + 1 < self.stack_len(),
        }
    }
    fn hunk_ahead(&self, forward: bool) -> bool {
        if self.selected_diff().is_none() {
            return false;
        }
        let line = self.line_index();
        let mut starts = hunks::hunk_starts(self.displayed());
        if forward {
            starts.any(|start| start > line)
        } else {
            starts.any(|start| start < line)
        }
    }

    /// The same for `k`.
    fn can_move_back(&self) -> bool {
        match self.focus {
            Focus::Sidebar => match self.sidebar_tab {
                SidebarTab::Files | SidebarTab::Commits => self.sidebar_row > 0,
                SidebarTab::Comments => self.browser_index > 0,
            },
            Focus::Diff => self.cursor_row() > 0,
            Focus::Stack => self.comment_index > 0,
        }
    }
}
