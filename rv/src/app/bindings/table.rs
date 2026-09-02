//! The table itself: every key browse mode answers, one row each. `leader: None`
//! is a direct key; `leader: Some(_)` sits under that leader's which-key submenu,
//! its `codes` the *second* keystroke. `contexts` is set only on the `Space`
//! (contextual) children, which the menu filters by the mode the reviewer is in.

use crossterm::event::KeyCode;

use super::AppCommand;
use super::Binding;
use super::Command;
use super::CommentCommand;
use super::Context;
use super::CursorCommand;
use super::DiffCommand;
use super::FilesCommand;
use super::Group;
use super::LayoutCommand;
use super::Leader;
use super::PaneCommand;
use super::view::JUMPS;
use super::view::VIEW;

/// Every key browse mode answers, the direct keys and the `Space`/`m`/`g`/`c`
/// leaders' children here and the stable `v` view list in [`view`], concatenated
/// so callers still see one slice.
pub const BINDINGS: &[Binding] = &concat();

/// Concatenates the two halves of the table at compile time. Sizes are spelled
/// out because stable Rust cannot add const generics in an array length.
const fn concat() -> [Binding; 56] {
    let mut out = [HEAD[0]; 56];
    let mut n = 0;
    let mut i = 0;
    while i < HEAD.len() {
        out[n] = HEAD[i];
        n += 1;
        i += 1;
    }
    let mut j = 0;
    while j < JUMPS.len() {
        out[n] = JUMPS[j];
        n += 1;
        j += 1;
    }
    let mut k = 0;
    while k < VIEW.len() {
        out[n] = VIEW[k];
        n += 1;
        k += 1;
    }
    out
}

/// The modes whose `Space` menu offers the file-list toggles.
const LISTS: &[Context] = &[Context::Files, Context::Commits];

/// The modes whose `Space` menu offers the comment verbs.
const COMMENTED: &[Context] = &[Context::Stack, Context::Comments];

/// A direct key or a non-contextual leader child: no `contexts`.
const ANY: &[Context] = &[];

/// Every key browse mode answers, and the **only** thing the browse handler
/// dispatches from, so the popup and the keyboard cannot drift. The order is the
/// order the popup reads in, grouped by [`Group`].
/// The direct keys and the `Space`/`m`/`g`/`c` leaders' children. The `v` view
/// list lives in [`view`] and is concatenated on above.
const HEAD: [Binding; 35] = [
    // ── Direct motion & controls ─────────────────────────────────────────────
    Binding {
        keys: "↓",
        group: Group::Move,
        leader: None,
        contexts: ANY,
        what: "next row",
        codes: &[KeyCode::Down],
        command: Command::Cursor(CursorCommand::NextRow),
    },
    Binding {
        keys: "↑",
        group: Group::Move,
        leader: None,
        contexts: ANY,
        what: "prev row",
        codes: &[KeyCode::Up],
        command: Command::Cursor(CursorCommand::PrevRow),
    },
    Binding {
        keys: "←",
        group: Group::Focus,
        leader: None,
        contexts: ANY,
        what: "out / up",
        codes: &[KeyCode::Left],
        command: Command::Pane(PaneCommand::FocusLeft),
    },
    Binding {
        keys: "→",
        group: Group::Focus,
        leader: None,
        contexts: ANY,
        what: "into / open",
        codes: &[KeyCode::Right],
        command: Command::Pane(PaneCommand::FocusRight),
    },
    Binding {
        keys: "PgDn",
        group: Group::Scroll,
        leader: None,
        contexts: ANY,
        what: "page down",
        codes: &[KeyCode::PageDown],
        command: Command::Cursor(CursorCommand::PageDown),
    },
    Binding {
        keys: "PgUp",
        group: Group::Scroll,
        leader: None,
        contexts: ANY,
        what: "page up",
        codes: &[KeyCode::PageUp],
        command: Command::Cursor(CursorCommand::PageUp),
    },
    Binding {
        keys: "Home",
        group: Group::Scroll,
        leader: None,
        contexts: ANY,
        what: "first row",
        codes: &[KeyCode::Home],
        command: Command::Cursor(CursorCommand::FirstRow),
    },
    Binding {
        keys: "End",
        group: Group::Scroll,
        leader: None,
        contexts: ANY,
        what: "last row",
        codes: &[KeyCode::End],
        command: Command::Cursor(CursorCommand::LastRow),
    },
    Binding {
        keys: "]",
        group: Group::Move,
        leader: None,
        contexts: ANY,
        what: "next file",
        codes: &[KeyCode::Char(']')],
        command: Command::Files(FilesCommand::Next),
    },
    Binding {
        keys: "[",
        group: Group::Move,
        leader: None,
        contexts: ANY,
        what: "prev file",
        codes: &[KeyCode::Char('[')],
        command: Command::Files(FilesCommand::Prev),
    },
    Binding {
        keys: "Enter",
        group: Group::Focus,
        leader: None,
        contexts: ANY,
        what: "open / into",
        codes: &[KeyCode::Enter],
        command: Command::Pane(PaneCommand::Open),
    },
    Binding {
        keys: "Tab",
        group: Group::Focus,
        leader: None,
        contexts: ANY,
        what: "next mode",
        codes: &[KeyCode::Tab],
        command: Command::Pane(PaneCommand::CycleTab),
    },
    Binding {
        keys: "s",
        group: Group::Focus,
        leader: None,
        contexts: ANY,
        what: "fold",
        codes: &[KeyCode::Char('s')],
        command: Command::Comment(CommentCommand::ToggleFold),
    },
    // Direct on the diff, where a full-file toggle is the reviewer's most common
    // reach; still under `v f` and the diff's `Space f` from anywhere.
    Binding {
        keys: "f",
        group: Group::View,
        leader: None,
        contexts: &[Context::Diff],
        what: "full context",
        codes: &[KeyCode::Char('f')],
        command: Command::Diff(DiffCommand::ToggleFullContext),
    },
    // Direct in the commits list, where the change tooltip is the reviewer's
    // most common reach; still under `v i` from anywhere.
    Binding {
        keys: "i",
        group: Group::View,
        leader: None,
        contexts: &[Context::Commits],
        what: "details",
        codes: &[KeyCode::Char('i')],
        command: Command::App(AppCommand::ToggleChangeDetails),
    },
    Binding {
        keys: "E",
        group: Group::Edit,
        leader: None,
        contexts: ANY,
        what: "edit ($EDITOR)",
        codes: &[KeyCode::Char('E')],
        command: Command::App(AppCommand::OpenEditor),
    },
    Binding {
        keys: "+",
        group: Group::Scroll,
        leader: None,
        contexts: ANY,
        what: "wider",
        codes: &[KeyCode::Char('+')],
        command: Command::Layout(LayoutCommand::SidebarWider),
    },
    Binding {
        keys: "_",
        group: Group::Scroll,
        leader: None,
        contexts: ANY,
        what: "narrower",
        codes: &[KeyCode::Char('_')],
        command: Command::Layout(LayoutCommand::SidebarNarrower),
    },
    Binding {
        keys: "Esc",
        group: Group::Focus,
        leader: None,
        contexts: ANY,
        what: "back out",
        codes: &[KeyCode::Esc],
        command: Command::Pane(PaneCommand::BackOut),
    },
    Binding {
        keys: "?",
        group: Group::Quit,
        leader: None,
        contexts: ANY,
        what: "keys",
        codes: &[KeyCode::Char('?')],
        command: Command::App(AppCommand::Help),
    },
    Binding {
        keys: "q",
        group: Group::Quit,
        leader: None,
        contexts: ANY,
        what: "quit the review",
        codes: &[KeyCode::Char('q')],
        command: Command::App(AppCommand::Quit),
    },
    // ── Space: the contextual menu ───────────────────────────────────────────
    // Files & commits lists: the view toggles for a file tree.
    Binding {
        keys: "t",
        group: Group::View,
        leader: Some(Leader::Context),
        contexts: LISTS,
        what: "list / tree",
        codes: &[KeyCode::Char('t')],
        command: Command::Files(FilesCommand::ToggleTree),
    },
    Binding {
        keys: "o",
        group: Group::View,
        leader: Some(Leader::Context),
        contexts: LISTS,
        what: "order",
        codes: &[KeyCode::Char('o')],
        command: Command::Files(FilesCommand::CycleSort),
    },
    Binding {
        keys: "#",
        group: Group::View,
        leader: Some(Leader::Context),
        contexts: LISTS,
        what: "counts",
        codes: &[KeyCode::Char('#')],
        command: Command::Files(FilesCommand::ToggleCounts),
    },
    Binding {
        keys: "c",
        group: Group::View,
        leader: Some(Leader::Context),
        contexts: LISTS,
        what: "tint",
        codes: &[KeyCode::Char('c')],
        command: Command::Files(FilesCommand::ToggleTint),
    },
    // Diff: the toggles that change how the change itself is shown.
    Binding {
        keys: "g",
        group: Group::View,
        leader: Some(Leader::Context),
        contexts: &[Context::Diff],
        what: "group",
        codes: &[KeyCode::Char('g')],
        command: Command::Diff(DiffCommand::GroupBySide),
    },
    Binding {
        keys: "b",
        group: Group::View,
        leader: Some(Leader::Context),
        contexts: &[Context::Diff],
        what: "side",
        codes: &[KeyCode::Char('b')],
        command: Command::Diff(DiffCommand::CycleSide),
    },
    Binding {
        keys: "f",
        group: Group::View,
        leader: Some(Leader::Context),
        contexts: &[Context::Diff],
        what: "context",
        codes: &[KeyCode::Char('f')],
        command: Command::Diff(DiffCommand::ToggleFullContext),
    },
    // On a comment: the verbs that settle it.
    Binding {
        keys: "d",
        group: Group::Comment,
        leader: Some(Leader::Context),
        contexts: COMMENTED,
        what: "delete comment",
        codes: &[KeyCode::Char('d')],
        command: Command::Comment(CommentCommand::Delete),
    },
    Binding {
        keys: "r",
        group: Group::Comment,
        leader: Some(Leader::Context),
        contexts: COMMENTED,
        what: "resolve",
        codes: &[KeyCode::Char('r')],
        command: Command::Comment(CommentCommand::Resolve),
    },
    Binding {
        keys: "a",
        group: Group::Comment,
        leader: Some(Leader::Context),
        contexts: COMMENTED,
        what: "abandon",
        codes: &[KeyCode::Char('a')],
        command: Command::Comment(CommentCommand::Abandon),
    },
    // ── m: mode ──────────────────────────────────────────────────────────────
    Binding {
        keys: "f",
        group: Group::Focus,
        leader: Some(Leader::Mode),
        contexts: ANY,
        what: "files",
        codes: &[KeyCode::Char('f')],
        command: Command::Pane(PaneCommand::GotoFiles),
    },
    Binding {
        keys: "c",
        group: Group::Focus,
        leader: Some(Leader::Mode),
        contexts: ANY,
        what: "commits",
        codes: &[KeyCode::Char('c')],
        command: Command::Pane(PaneCommand::GotoCommits),
    },
    Binding {
        keys: "o",
        group: Group::Focus,
        leader: Some(Leader::Mode),
        contexts: ANY,
        what: "comments",
        codes: &[KeyCode::Char('o')],
        command: Command::Pane(PaneCommand::GotoComments),
    },
    Binding {
        keys: "d",
        group: Group::Focus,
        leader: Some(Leader::Mode),
        contexts: ANY,
        what: "diff",
        codes: &[KeyCode::Char('d')],
        command: Command::Pane(PaneCommand::GotoDiff),
    },
];
