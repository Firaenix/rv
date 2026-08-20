//! The table itself: every key browse mode answers, one row each.
//!
//! Its own file only because the list is long. [`super`] holds what a row *is*
//! — [`Binding`], [`Group`], [`Command`] — and the reasoning about why one
//! table is the only dispatcher; this is the data.

use crossterm::event::KeyCode;

use super::Binding;
use super::Command;
use super::Group;
use crate::app::Context;

/// The browse contexts, for a binding that belongs in every tip.
const EVERYWHERE: &[Context] = &[
    Context::Files,
    Context::Commits,
    Context::Comments,
    Context::Diff,
    Context::Stack,
];

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
        keys: "J",
        group: Group::Move,
        what: "next hunk",
        contexts: &[Context::Diff],
        codes: &[KeyCode::Char('J')],
        command: Command::NextHunk,
    },
    Binding {
        keys: "K",
        group: Group::Move,
        what: "previous hunk",
        contexts: &[Context::Diff],
        codes: &[KeyCode::Char('K')],
        command: Command::PreviousHunk,
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
        keys: "1",
        group: Group::Focus,
        what: "files tab",
        contexts: &[],
        codes: &[KeyCode::Char('1')],
        command: Command::FilesTab,
    },
    Binding {
        keys: "2",
        group: Group::Focus,
        what: "commits tab",
        contexts: &[],
        codes: &[KeyCode::Char('2')],
        command: Command::CommitsTab,
    },
    Binding {
        keys: "3",
        group: Group::Focus,
        what: "comments tab",
        contexts: &[],
        codes: &[KeyCode::Char('3')],
        command: Command::CommentsTab,
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
        keys: "v",
        group: Group::Edit,
        what: "open in $EDITOR",
        contexts: &[Context::Diff, Context::Stack],
        codes: &[KeyCode::Char('v')],
        command: Command::OpenEditor,
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
