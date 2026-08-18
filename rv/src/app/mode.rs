//! What the keyboard is doing, what it acts on, and what it asks for next.

/// What the reviewer is doing with the keyboard.
///
/// Not [`Copy`]: [`Mode::ConfirmDelete`] carries the question *and* its subject
/// as one state, so there is no way to be asking without knowing what about,
/// and no way for a stale id to survive the answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Keys navigate the diff.
    Browse,
    /// Keys go into the comment buffer.
    Comment,
    /// Waiting for `y` before removing the comment with this `id`, which is
    /// shown as `label` (`path:line`).
    ///
    /// Deletion is unrecoverable, and every key answers this question — `y`
    /// deletes, anything else cancels — so it cannot become a state the
    /// reviewer is stuck in.
    ConfirmDelete { id: String, label: String },
}

/// What the left column is listing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SidebarTab {
    /// The review's changed files.
    Files,
    /// Every comment in the review, wherever it is anchored.
    Comments,
}

/// Which pane the keys act on.
///
/// A focus rather than a [`Mode`]: a mode changes what a keystroke *means*,
/// this only changes what it moves. `[` and `]` are answered before it is
/// consulted at all.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Focus {
    /// The left column, which lists either the review's files or its comments
    /// — see [`SidebarTab`].
    Sidebar,
    /// The diff of the selected file.
    Diff,
    /// Inside the comment stack of the selected diff line: `Enter` steps in,
    /// `Esc` and `Left` step back out, and `j`/`k` move between the comments
    /// rather than between the lines.
    Stack,
}

/// What a key press wants the event loop to do next.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Continue,
    Quit,
}
