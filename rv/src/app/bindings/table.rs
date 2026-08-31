//! The table itself: every key browse mode answers, one row each. `leader: None`
//! is a direct key; `leader: Some(_)` sits under that leader's which-key submenu,
//! its `codes` the *second* keystroke. `contexts` is set only on the `Space`
//! (contextual) children, which the menu filters by the mode the reviewer is in.

use crossterm::event::KeyCode;

use super::Binding;
use super::Command;
use super::Context;
use super::Group;
use super::Leader;
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
        command: Command::Forward,
    },
    Binding {
        keys: "↑",
        group: Group::Move,
        leader: None,
        contexts: ANY,
        what: "prev row",
        codes: &[KeyCode::Up],
        command: Command::Back,
    },
    Binding {
        keys: "←",
        group: Group::Focus,
        leader: None,
        contexts: ANY,
        what: "out / up",
        codes: &[KeyCode::Left],
        command: Command::FocusLeft,
    },
    Binding {
        keys: "→",
        group: Group::Focus,
        leader: None,
        contexts: ANY,
        what: "into / open",
        codes: &[KeyCode::Right],
        command: Command::FocusRight,
    },
    Binding {
        keys: "PgDn",
        group: Group::Scroll,
        leader: None,
        contexts: ANY,
        what: "page down",
        codes: &[KeyCode::PageDown],
        command: Command::PageForward,
    },
    Binding {
        keys: "PgUp",
        group: Group::Scroll,
        leader: None,
        contexts: ANY,
        what: "page up",
        codes: &[KeyCode::PageUp],
        command: Command::PageBackward,
    },
    Binding {
        keys: "Home",
        group: Group::Scroll,
        leader: None,
        contexts: ANY,
        what: "first row",
        codes: &[KeyCode::Home],
        command: Command::JumpFirst,
    },
    Binding {
        keys: "End",
        group: Group::Scroll,
        leader: None,
        contexts: ANY,
        what: "last row",
        codes: &[KeyCode::End],
        command: Command::JumpLast,
    },
    Binding {
        keys: "]",
        group: Group::Move,
        leader: None,
        contexts: ANY,
        what: "next file",
        codes: &[KeyCode::Char(']')],
        command: Command::NextFile,
    },
    Binding {
        keys: "[",
        group: Group::Move,
        leader: None,
        contexts: ANY,
        what: "prev file",
        codes: &[KeyCode::Char('[')],
        command: Command::PreviousFile,
    },
    Binding {
        keys: "Enter",
        group: Group::Focus,
        leader: None,
        contexts: ANY,
        what: "open / into",
        codes: &[KeyCode::Enter],
        command: Command::Enter,
    },
    Binding {
        keys: "Tab",
        group: Group::Focus,
        leader: None,
        contexts: ANY,
        what: "next mode",
        codes: &[KeyCode::Tab],
        command: Command::CycleMode,
    },
    Binding {
        keys: "s",
        group: Group::Focus,
        leader: None,
        contexts: ANY,
        what: "fold",
        codes: &[KeyCode::Char('s')],
        command: Command::Fold,
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
        command: Command::ToggleFullContext,
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
        command: Command::Info,
    },
    Binding {
        keys: "E",
        group: Group::Edit,
        leader: None,
        contexts: ANY,
        what: "edit ($EDITOR)",
        codes: &[KeyCode::Char('E')],
        command: Command::OpenEditor,
    },
    Binding {
        keys: "+",
        group: Group::Scroll,
        leader: None,
        contexts: ANY,
        what: "wider",
        codes: &[KeyCode::Char('+')],
        command: Command::Wider,
    },
    Binding {
        keys: "_",
        group: Group::Scroll,
        leader: None,
        contexts: ANY,
        what: "narrower",
        codes: &[KeyCode::Char('_')],
        command: Command::Narrower,
    },
    Binding {
        keys: "Esc",
        group: Group::Focus,
        leader: None,
        contexts: ANY,
        what: "back out",
        codes: &[KeyCode::Esc],
        command: Command::Escape,
    },
    Binding {
        keys: "?",
        group: Group::Quit,
        leader: None,
        contexts: ANY,
        what: "keys",
        codes: &[KeyCode::Char('?')],
        command: Command::Help,
    },
    Binding {
        keys: "q",
        group: Group::Quit,
        leader: None,
        contexts: ANY,
        what: "quit the review",
        codes: &[KeyCode::Char('q')],
        command: Command::Quit,
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
        command: Command::ToggleTree,
    },
    Binding {
        keys: "o",
        group: Group::View,
        leader: Some(Leader::Context),
        contexts: LISTS,
        what: "order",
        codes: &[KeyCode::Char('o')],
        command: Command::CycleSort,
    },
    Binding {
        keys: "#",
        group: Group::View,
        leader: Some(Leader::Context),
        contexts: LISTS,
        what: "counts",
        codes: &[KeyCode::Char('#')],
        command: Command::ToggleCounts,
    },
    Binding {
        keys: "c",
        group: Group::View,
        leader: Some(Leader::Context),
        contexts: LISTS,
        what: "tint",
        codes: &[KeyCode::Char('c')],
        command: Command::ToggleTint,
    },
    // Diff: the toggles that change how the change itself is shown.
    Binding {
        keys: "g",
        group: Group::View,
        leader: Some(Leader::Context),
        contexts: &[Context::Diff],
        what: "group",
        codes: &[KeyCode::Char('g')],
        command: Command::GroupDiff,
    },
    Binding {
        keys: "b",
        group: Group::View,
        leader: Some(Leader::Context),
        contexts: &[Context::Diff],
        what: "side",
        codes: &[KeyCode::Char('b')],
        command: Command::BeforeAfter,
    },
    Binding {
        keys: "f",
        group: Group::View,
        leader: Some(Leader::Context),
        contexts: &[Context::Diff],
        what: "context",
        codes: &[KeyCode::Char('f')],
        command: Command::ToggleFullContext,
    },
    // On a comment: the verbs that settle it.
    Binding {
        keys: "d",
        group: Group::Comment,
        leader: Some(Leader::Context),
        contexts: COMMENTED,
        what: "delete comment",
        codes: &[KeyCode::Char('d')],
        command: Command::Delete,
    },
    Binding {
        keys: "r",
        group: Group::Comment,
        leader: Some(Leader::Context),
        contexts: COMMENTED,
        what: "resolve",
        codes: &[KeyCode::Char('r')],
        command: Command::Resolve,
    },
    Binding {
        keys: "a",
        group: Group::Comment,
        leader: Some(Leader::Context),
        contexts: COMMENTED,
        what: "abandon",
        codes: &[KeyCode::Char('a')],
        command: Command::Abandon,
    },
    // ── m: mode ──────────────────────────────────────────────────────────────
    Binding {
        keys: "f",
        group: Group::Focus,
        leader: Some(Leader::Mode),
        contexts: ANY,
        what: "files",
        codes: &[KeyCode::Char('f')],
        command: Command::FilesTab,
    },
    Binding {
        keys: "c",
        group: Group::Focus,
        leader: Some(Leader::Mode),
        contexts: ANY,
        what: "commits",
        codes: &[KeyCode::Char('c')],
        command: Command::CommitsTab,
    },
    Binding {
        keys: "o",
        group: Group::Focus,
        leader: Some(Leader::Mode),
        contexts: ANY,
        what: "comments",
        codes: &[KeyCode::Char('o')],
        command: Command::CommentsTab,
    },
    Binding {
        keys: "d",
        group: Group::Focus,
        leader: Some(Leader::Mode),
        contexts: ANY,
        what: "diff",
        codes: &[KeyCode::Char('d')],
        command: Command::ModeDiff,
    },
];
