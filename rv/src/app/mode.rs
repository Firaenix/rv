//! What the keyboard is doing, what it acts on, and what it asks for next.

/// Not [`Copy`]: `ConfirmDelete` carries the question and its subject as one
/// state, so there is no way to be asking without knowing what about, and no way
/// for a stale id to survive the answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Mode {
    Browse,
    Comment,
    /// Every key answers this — `y` deletes, anything else cancels — so it cannot
    /// become a state the reviewer is stuck in.
    ConfirmDelete { id: String, label: String },
    /// The query shares the comment buffer: one place text arrives in this
    /// reviewer, rather than two places to get backspace and escape right.
    Pick,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SidebarTab {
    Files,
    Commits,
    Comments,
}

/// A focus rather than a [`Mode`]: a mode changes what a keystroke *means*, this
/// only changes what it moves. `[` and `]` are answered before it is consulted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Focus {
    Sidebar,
    Diff,
    /// Inside the selected line's comment stack, where `j`/`k` move between
    /// comments rather than between lines.
    Stack,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Continue,
    Quit,
}
