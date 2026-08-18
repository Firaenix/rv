//! The one table every browse key is dispatched from, and drawn from.

use crossterm::event::KeyCode;

use super::App;
use super::Focus;
use super::SidebarTab;

/// What a binding acts on, and therefore which heading the `?` popup lists it
/// under. A reviewer looking for "how do I get to the next file" scans a group,
/// not an alphabet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Group {
    /// Moving the cursor inside whatever pane has it.
    Move,
    /// Moving the cursor *between* panes, and between what the sidebar lists.
    Focus,
    /// Writing, deleting and folding comments.
    Comment,
    /// How the screen is arranged. Session-only, every one of them.
    View,
    /// Leaving.
    Quit,
}

impl Group {
    /// Every group, in the order the popup lists them.
    pub const ALL: &'static [Group] = &[
        Group::Move,
        Group::Focus,
        Group::Comment,
        Group::View,
        Group::Quit,
    ];

    /// The heading the popup writes above the group.
    #[must_use]
    pub fn heading(self) -> &'static str {
        match self {
            Group::Move => "Move",
            Group::Focus => "Panes",
            Group::Comment => "Comments",
            Group::View => "View",
            Group::Quit => "Leave",
        }
    }
}

/// What one key does, spelled once.
pub struct Binding {
    /// How the popup spells the key, arrows and aliases included.
    pub keys: &'static str,
    pub group: Group,
    /// What it does, short enough to sit beside the key in a column.
    pub what: &'static str,
    /// The key presses this row answers. Not public: it is the table's business
    /// which codes a row claims, and a caller comparing codes would be a second
    /// dispatcher.
    pub(super) codes: &'static [KeyCode],
    /// What running the row does. Not public for the same reason.
    pub(super) command: Command,
}

/// Every key browse mode answers.
///
/// This is the **only** thing the browse handler dispatches from, which is what
/// makes the popup and the keyboard impossible to drift apart: a key in no row
/// reaches no code, a row names a [`Command`] that
/// [`App::run_command`](super::App) matches exhaustively, and
/// [`crate::ui`] draws the popup from this very table.
///
/// The order is the order the popup reads in, grouped by [`Group`]; it is not a
/// priority order, since no key appears in two rows.
///
/// Everywhere the keymap is presented the **arrow leads and the vim key follows
/// in parentheses**, and the arrow is listed first in `codes` too, so the
/// spelling and the dispatch cannot disagree about which is which.
pub const BINDINGS: &[Binding] = &[
    Binding {
        keys: "↓ (j)",
        group: Group::Move,
        what: "next row",
        codes: &[KeyCode::Down, KeyCode::Char('j')],
        command: Command::Forward,
    },
    Binding {
        keys: "↑ (k)",
        group: Group::Move,
        what: "previous row",
        codes: &[KeyCode::Up, KeyCode::Char('k')],
        command: Command::Back,
    },
    Binding {
        keys: "n",
        group: Group::Move,
        what: "next symbol",
        codes: &[KeyCode::Char('n')],
        command: Command::NextSymbol,
    },
    Binding {
        keys: "N",
        group: Group::Move,
        what: "previous symbol",
        codes: &[KeyCode::Char('N')],
        command: Command::PreviousSymbol,
    },
    Binding {
        keys: "/",
        group: Group::Move,
        what: "find a symbol",
        codes: &[KeyCode::Char('/')],
        command: Command::Pick,
    },
    Binding {
        keys: "]",
        group: Group::Move,
        what: "next file",
        codes: &[KeyCode::Char(']')],
        command: Command::NextFile,
    },
    Binding {
        keys: "[",
        group: Group::Move,
        what: "previous file",
        codes: &[KeyCode::Char('[')],
        command: Command::PreviousFile,
    },
    Binding {
        keys: "← (h)",
        group: Group::Focus,
        what: "the file list",
        codes: &[KeyCode::Left, KeyCode::Char('h')],
        command: Command::FocusLeft,
    },
    Binding {
        keys: "→ (l)",
        group: Group::Focus,
        what: "the diff",
        codes: &[KeyCode::Right, KeyCode::Char('l')],
        command: Command::FocusRight,
    },
    Binding {
        keys: "Tab",
        group: Group::Focus,
        what: "files / comments",
        codes: &[KeyCode::Tab],
        command: Command::SwitchTab,
    },
    Binding {
        keys: "Enter",
        group: Group::Focus,
        what: "open the stack",
        codes: &[KeyCode::Enter],
        command: Command::Enter,
    },
    Binding {
        keys: "Space",
        group: Group::Focus,
        what: "fold a directory",
        codes: &[KeyCode::Char(' ')],
        command: Command::Enter,
    },
    Binding {
        keys: "Esc",
        group: Group::Focus,
        what: "leave the stack",
        codes: &[KeyCode::Esc],
        command: Command::Escape,
    },
    Binding {
        keys: "c",
        group: Group::Comment,
        what: "write a comment",
        codes: &[KeyCode::Char('c')],
        command: Command::Comment,
    },
    Binding {
        keys: "d",
        group: Group::Comment,
        what: "delete a comment",
        codes: &[KeyCode::Char('d')],
        command: Command::Delete,
    },
    Binding {
        keys: "r",
        group: Group::Comment,
        what: "resolve / reopen",
        codes: &[KeyCode::Char('r')],
        command: Command::Resolve,
    },
    Binding {
        keys: "a",
        group: Group::Comment,
        what: "abandon / reopen",
        codes: &[KeyCode::Char('a')],
        command: Command::Abandon,
    },
    Binding {
        keys: "s",
        group: Group::Comment,
        what: "fold a comment",
        codes: &[KeyCode::Char('s')],
        command: Command::Fold,
    },
    Binding {
        keys: "e",
        group: Group::Comment,
        what: "export the review",
        codes: &[KeyCode::Char('e')],
        command: Command::Export,
    },
    Binding {
        keys: "<",
        group: Group::View,
        what: "narrower sidebar",
        codes: &[KeyCode::Char('<')],
        command: Command::Narrower,
    },
    Binding {
        keys: ">",
        group: Group::View,
        what: "wider sidebar",
        codes: &[KeyCode::Char('>')],
        command: Command::Wider,
    },
    Binding {
        keys: "z",
        group: Group::View,
        what: "hide the sidebar",
        codes: &[KeyCode::Char('z')],
        command: Command::ToggleSidebar,
    },
    Binding {
        keys: "t",
        group: Group::View,
        what: "list / tree",
        codes: &[KeyCode::Char('t')],
        command: Command::ToggleTree,
    },
    Binding {
        keys: "o",
        group: Group::View,
        what: "order the files",
        codes: &[KeyCode::Char('o')],
        command: Command::CycleSort,
    },
    Binding {
        keys: "i",
        group: Group::View,
        what: "change in full",
        codes: &[KeyCode::Char('i')],
        command: Command::Info,
    },
    Binding {
        keys: "?",
        group: Group::View,
        what: "this keymap",
        codes: &[KeyCode::Char('?')],
        command: Command::Help,
    },
    Binding {
        keys: "q",
        group: Group::Quit,
        what: "quit the review",
        codes: &[KeyCode::Char('q')],
        command: Command::Quit,
    },
];

/// What running one row of [`BINDINGS`] does.
///
/// An enum rather than a string or a function pointer: that is what makes the
/// dispatch match exhaustive, so a row cannot name a command nothing answers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Command {
    Forward,
    Back,
    NextFile,
    PreviousFile,
    NextSymbol,
    PreviousSymbol,
    Pick,
    FocusLeft,
    FocusRight,
    SwitchTab,
    Enter,
    Escape,
    Comment,
    Delete,
    Resolve,
    Abandon,
    Export,
    Fold,
    Narrower,
    Wider,
    ToggleSidebar,
    ToggleTree,
    CycleSort,
    Info,
    Help,
    Quit,
}

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
            | Command::Narrower
            | Command::Wider
            | Command::Help
            | Command::ToggleSidebar
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
            Command::FocusLeft => self.focus != Focus::Sidebar,
            Command::FocusRight => self.focus == Focus::Sidebar,
            Command::Enter => match (self.focus, self.sidebar_tab) {
                (Focus::Sidebar, SidebarTab::Comments) => self.browsed_comment().is_some(),
                (Focus::Diff, _) => !self.comments_for_line(self.line_index()).is_empty(),
                _ => false,
            },
            Command::Escape => self.focus == Focus::Stack,
            Command::Comment => self.selected_line().is_some(),
            Command::Delete => self.delete_target().is_some(),
            Command::Resolve | Command::Abandon => self.settle_target().is_some(),
            // Two things under one key, so two ways for it to have a target.
            Command::Fold => self.sidebar_fold_key().is_some() || !self.fold_targets().is_empty(),
            Command::ToggleTree | Command::CycleSort => {
                self.sidebar_tab != SidebarTab::Comments
            }
            Command::Info => self.change_under_cursor().is_some(),
        }
    }

    /// Whether `j` has anywhere to go in the pane that has the cursor.
    fn can_move_forward(&self) -> bool {
        match self.focus {
            Focus::Sidebar => match self.sidebar_tab {
                SidebarTab::Files | SidebarTab::Commits => {
                    self.sidebar_row + 1 < self.nodes().len()
                }
                SidebarTab::Comments => self.browser_index + 1 < self.comments.len(),
            },
            Focus::Diff => self.cursor_row() + 1 < self.row_count(),
            Focus::Stack => self.comment_index + 1 < self.stack_len(),
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
