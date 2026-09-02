//! Whether a binding would do anything from where the cursor is: what dims a
//! row of the `?` popup and the tip.

use rv_core::model::ChangeKind;

use super::App;
use super::Focus;
use super::SidebarTab;
use super::bindings::AppCommand;
use super::bindings::Binding;
use super::bindings::Command;
use super::bindings::CommentCommand;
use super::bindings::CursorCommand;
use super::bindings::DiffCommand;
use super::bindings::FilesCommand;
use super::bindings::LayoutCommand;
use super::bindings::PaneCommand;
use super::hunks;
use super::keymap::RuntimeBinding;
use crate::tree::NodeKind;

impl App {
    /// Whether `binding` would do anything from where the cursor is.
    ///
    /// The popup dims the ones that would not rather than hiding them: a
    /// reviewer should see that `d` exists and that the file list is the wrong
    /// place for it. Every arm asks the same question the key itself asks, so
    /// the popup cannot claim a key is live where it would refuse.
    pub fn rt_binding_enabled(&self, binding: &RuntimeBinding) -> bool {
        self.command_enabled(binding.command)
    }

    pub fn binding_enabled(&self, binding: &Binding) -> bool {
        self.command_enabled(binding.command)
    }

    fn command_enabled(&self, command: Command) -> bool {
        match command {
            Command::Cursor(command) => self.cursor_enabled(command),
            Command::Pane(command) => self.pane_enabled(command),
            Command::Files(command) => self.files_enabled(command),
            Command::Diff(command) => self.diff_enabled(command),
            Command::Comment(command) => self.comment_enabled(command),
            // Always something to do: the layout changes what is on screen,
            // never what is under the cursor.
            Command::Layout(
                LayoutCommand::SidebarWider
                | LayoutCommand::SidebarNarrower
                | LayoutCommand::ToggleSidebar,
            ) => true,
            Command::App(command) => self.app_enabled(command),
        }
    }

    fn cursor_enabled(&self, command: CursorCommand) -> bool {
        match command {
            CursorCommand::NextRow | CursorCommand::PageDown | CursorCommand::LastRow => {
                self.can_move_forward()
            }
            CursorCommand::PrevRow | CursorCommand::PageUp | CursorCommand::FirstRow => {
                self.can_move_back()
            }
            // Rightwards is always live — only the renderer knows how long the
            // longest line is — and leftwards has somewhere to go once it is.
            CursorCommand::ScrollRight => true,
            CursorCommand::ScrollLeft => match self.focus {
                Focus::Sidebar => self.sidebar_hscroll > 0,
                Focus::Diff | Focus::Stack => self.diff_hscroll > 0,
            },
        }
    }

    fn pane_enabled(&self, command: PaneCommand) -> bool {
        match command {
            PaneCommand::CycleTab
            | PaneCommand::GotoFiles
            | PaneCommand::GotoCommits
            | PaneCommand::GotoComments
            | PaneCommand::GotoDiff => true,
            // `←` always leads somewhere (out of a pane, or up the tree); `→`
            // acts only from the sidebar.
            PaneCommand::FocusLeft => true,
            PaneCommand::FocusRight => self.focus == Focus::Sidebar,
            PaneCommand::Open => match (self.focus, self.sidebar_tab) {
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
            PaneCommand::BackOut => {
                self.focus == Focus::Stack || (self.focus == Focus::Sidebar && self.zoomed())
            }
        }
    }

    fn files_enabled(&self, command: FilesCommand) -> bool {
        match command {
            FilesCommand::Next => self.file_index + 1 < self.review.files.len(),
            FilesCommand::Prev => self.file_index > 0,
            FilesCommand::ToggleTree
            | FilesCommand::CycleSort
            | FilesCommand::ToggleTint
            | FilesCommand::ToggleCounts => self.sidebar_tab != SidebarTab::Comments,
        }
    }

    fn diff_enabled(&self, command: DiffCommand) -> bool {
        match command {
            // Live wherever the file has a hunk to reach from where the cursor
            // is; the key itself says which end it has run out at.
            DiffCommand::NextHunk => self.hunk_ahead(true),
            DiffCommand::PrevHunk => self.hunk_ahead(false),
            // Asked of the cache rather than of a fresh index: the popup is
            // drawn from `&self` and indexing is not a thing to do per frame. An
            // unbuilt index reports live, and the key says so itself.
            DiffCommand::NextSymbol | DiffCommand::PrevSymbol | DiffCommand::FindSymbol => {
                self.indexed_scope.is_none() || !self.symbol_index.is_empty()
            }
            DiffCommand::ToggleFullContext => true,
            DiffCommand::GroupBySide | DiffCommand::CycleSide => self.selected_diff().is_some(),
        }
    }

    fn comment_enabled(&self, command: CommentCommand) -> bool {
        match command {
            // A write is about the diff line under the cursor, live only where
            // that cursor is the one being steered.
            CommentCommand::Write => {
                matches!(self.focus, Focus::Diff | Focus::Stack) && self.selected_line().is_some()
            }
            CommentCommand::Delete => self.delete_target().is_some(),
            CommentCommand::Resolve | CommentCommand::Abandon => self.settle_target().is_some(),
            // Two things under one key, so two ways for it to have a target.
            CommentCommand::ToggleFold => {
                self.sidebar_fold_key().is_some() || !self.fold_targets().is_empty()
            }
        }
    }

    fn app_enabled(&self, command: AppCommand) -> bool {
        match command {
            AppCommand::Help | AppCommand::Quit | AppCommand::Refresh => true,
            // A file that exists at the head is one an editor can open. Whether
            // `$EDITOR` is set is not asked here: the popup is drawn every
            // frame, and dimming a key over an environment variable would teach
            // that the key does not exist rather than that the variable is
            // unset — which is what the key says when it is pressed.
            AppCommand::OpenEditor => self
                .selected_file()
                .is_some_and(|file| file.kind != ChangeKind::Removed),
            AppCommand::ToggleChangeDetails => self.sidebar_tab == SidebarTab::Commits,
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
        let displayed = self.displayed_lines();
        let mut starts = hunks::hunk_starts(&displayed);
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
