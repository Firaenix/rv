//! The one table every browse key is dispatched from, and drawn from.
//!
//! What a row *is* lives here — [`Binding`], its [`Group`] and the [`Command`]
//! it names. The rows themselves are in [`table`], which is a split for length
//! and nothing else: [`BINDINGS`] is still the single table, re-exported from
//! here so every caller names one path.

use crossterm::event::KeyCode;

use super::Context;

mod table;

pub use table::BINDINGS;

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
    /// Leaving the reviewer to change the code it is showing.
    Edit,
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
        Group::Edit,
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
            Group::Edit => "Edit",
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
    NextHunk,
    PreviousHunk,
    NextSymbol,
    PreviousSymbol,
    ScrollLeft,
    ScrollRight,
    Pick,
    FocusLeft,
    FocusRight,
    SwitchTab,
    FilesTab,
    CommitsTab,
    CommentsTab,
    Enter,
    FoldRow,
    Escape,
    Comment,
    Delete,
    Resolve,
    Abandon,
    Export,
    OpenEditor,
    Fold,
    Narrower,
    Wider,
    ToggleSidebar,
    ToggleTree,
    CycleSort,
    ToggleTint,
    ToggleCounts,
    ToggleFullContext,
    Info,
    Refresh,
    Help,
    Quit,
}
