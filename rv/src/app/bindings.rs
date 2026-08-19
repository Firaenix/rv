//! The one table every browse key is dispatched from, and drawn from.

use crossterm::event::KeyCode;

use super::Context;

/// The browse contexts, for a binding that belongs in every tip.
const EVERYWHERE: &[Context] = &[
    Context::Files,
    Context::Commits,
    Context::Comments,
    Context::Diff,
    Context::Stack,
];

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
    /// The contexts whose `?` tip lists this key. Empty means the key appears
    /// only in the full keymap: it exists everywhere, but a tip is a prompt for
    /// *here*, and a key that is never the next thing to reach for would crowd
    /// out the ones that are.
    pub contexts: &'static [Context],
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
        contexts: EVERYWHERE,
        codes: &[KeyCode::Down, KeyCode::Char('j')],
        command: Command::Forward,
    },
    Binding {
        keys: "↑ (k)",
        group: Group::Move,
        what: "previous row",
        contexts: EVERYWHERE,
        codes: &[KeyCode::Up, KeyCode::Char('k')],
        command: Command::Back,
    },
    Binding {
        keys: "n",
        group: Group::Move,
        what: "next symbol",
        contexts: &[],
        codes: &[KeyCode::Char('n')],
        command: Command::NextSymbol,
    },
    Binding {
        keys: "N",
        group: Group::Move,
        what: "previous symbol",
        contexts: &[],
        codes: &[KeyCode::Char('N')],
        command: Command::PreviousSymbol,
    },
    Binding {
        keys: "/",
        group: Group::Move,
        what: "find a symbol",
        contexts: &[Context::Diff],
        codes: &[KeyCode::Char('/')],
        command: Command::Pick,
    },
    Binding {
        keys: "]",
        group: Group::Move,
        what: "next file",
        contexts: &[Context::Diff],
        codes: &[KeyCode::Char(']')],
        command: Command::NextFile,
    },
    Binding {
        keys: "[",
        group: Group::Move,
        what: "previous file",
        contexts: &[Context::Diff],
        codes: &[KeyCode::Char('[')],
        command: Command::PreviousFile,
    },
    Binding {
        keys: "L",
        group: Group::Move,
        what: "scroll right",
        contexts: &[Context::Files, Context::Commits, Context::Diff],
        codes: &[KeyCode::Char('L')],
        command: Command::ScrollRight,
    },
    Binding {
        keys: "H",
        group: Group::Move,
        what: "scroll left",
        contexts: &[Context::Files, Context::Commits, Context::Diff],
        codes: &[KeyCode::Char('H')],
        command: Command::ScrollLeft,
    },
    Binding {
        keys: "← (h)",
        group: Group::Focus,
        what: "the file list",
        contexts: &[Context::Diff, Context::Stack],
        codes: &[KeyCode::Left, KeyCode::Char('h')],
        command: Command::FocusLeft,
    },
    Binding {
        keys: "→ (l)",
        group: Group::Focus,
        what: "the diff",
        contexts: &[Context::Files, Context::Commits, Context::Comments],
        codes: &[KeyCode::Right, KeyCode::Char('l')],
        command: Command::FocusRight,
    },
    Binding {
        keys: "Tab",
        group: Group::Focus,
        what: "the next tab",
        contexts: &[Context::Files, Context::Commits, Context::Comments],
        codes: &[KeyCode::Tab],
        command: Command::SwitchTab,
    },
    Binding {
        keys: "Enter",
        group: Group::Focus,
        what: "open / zoom in",
        contexts: &[
            Context::Files,
            Context::Commits,
            Context::Diff,
            Context::Comments,
        ],
        codes: &[KeyCode::Enter],
        command: Command::Enter,
    },
    Binding {
        keys: "Space",
        group: Group::Focus,
        what: "fold directory",
        contexts: &[Context::Files, Context::Commits],
        codes: &[KeyCode::Char(' ')],
        command: Command::FoldRow,
    },
    Binding {
        keys: "Esc",
        group: Group::Focus,
        what: "back out",
        contexts: &[Context::Files, Context::Commits, Context::Stack],
        codes: &[KeyCode::Esc],
        command: Command::Escape,
    },
    Binding {
        keys: "c",
        group: Group::Comment,
        what: "write a comment",
        contexts: &[Context::Diff, Context::Stack],
        codes: &[KeyCode::Char('c')],
        command: Command::Comment,
    },
    Binding {
        keys: "d",
        group: Group::Comment,
        what: "delete comment",
        contexts: &[Context::Stack, Context::Comments],
        codes: &[KeyCode::Char('d')],
        command: Command::Delete,
    },
    Binding {
        keys: "r",
        group: Group::Comment,
        what: "resolve/reopen",
        contexts: &[Context::Stack, Context::Comments],
        codes: &[KeyCode::Char('r')],
        command: Command::Resolve,
    },
    Binding {
        keys: "a",
        group: Group::Comment,
        what: "abandon/reopen",
        contexts: &[Context::Stack, Context::Comments],
        codes: &[KeyCode::Char('a')],
        command: Command::Abandon,
    },
    Binding {
        keys: "s",
        group: Group::Comment,
        what: "fold a comment",
        contexts: &[Context::Stack],
        codes: &[KeyCode::Char('s')],
        command: Command::Fold,
    },
    Binding {
        keys: "e",
        group: Group::Comment,
        what: "export review",
        contexts: &[Context::Comments],
        codes: &[KeyCode::Char('e')],
        command: Command::Export,
    },
    Binding {
        keys: "<",
        group: Group::View,
        what: "narrow sidebar",
        contexts: &[],
        codes: &[KeyCode::Char('<')],
        command: Command::Narrower,
    },
    Binding {
        keys: ">",
        group: Group::View,
        what: "widen sidebar",
        contexts: &[],
        codes: &[KeyCode::Char('>')],
        command: Command::Wider,
    },
    Binding {
        keys: "z",
        group: Group::View,
        what: "hide sidebar",
        contexts: &[],
        codes: &[KeyCode::Char('z')],
        command: Command::ToggleSidebar,
    },
    Binding {
        keys: "t",
        group: Group::View,
        what: "list / tree",
        contexts: &[Context::Files, Context::Commits],
        codes: &[KeyCode::Char('t')],
        command: Command::ToggleTree,
    },
    Binding {
        keys: "o",
        group: Group::View,
        what: "order the files",
        contexts: &[Context::Files, Context::Commits],
        codes: &[KeyCode::Char('o')],
        command: Command::CycleSort,
    },
    Binding {
        keys: "g",
        group: Group::View,
        what: "tint the names",
        contexts: &[Context::Files, Context::Commits],
        codes: &[KeyCode::Char('g')],
        command: Command::ToggleTint,
    },
    Binding {
        keys: "#",
        group: Group::View,
        what: "show the counts",
        contexts: &[Context::Files, Context::Commits],
        codes: &[KeyCode::Char('#')],
        command: Command::ToggleCounts,
    },
    Binding {
        keys: "R",
        group: Group::View,
        what: "refresh",
        contexts: &[],
        codes: &[KeyCode::Char('R')],
        command: Command::Refresh,
    },
    Binding {
        keys: "i",
        group: Group::View,
        what: "change details",
        contexts: &[Context::Commits],
        codes: &[KeyCode::Char('i')],
        command: Command::Info,
    },
    Binding {
        keys: "?",
        group: Group::View,
        what: "all the keys",
        contexts: EVERYWHERE,
        codes: &[KeyCode::Char('?')],
        command: Command::Help,
    },
    Binding {
        keys: "q",
        group: Group::Quit,
        what: "quit the review",
        contexts: EVERYWHERE,
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
    ScrollLeft,
    ScrollRight,
    Pick,
    FocusLeft,
    FocusRight,
    SwitchTab,
    Enter,
    FoldRow,
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
    ToggleTint,
    ToggleCounts,
    Info,
    Refresh,
    Help,
    Quit,
}
