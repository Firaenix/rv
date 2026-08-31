//! The one table every browse key is dispatched from, and drawn from.
//!
//! What a row *is* lives here — [`Binding`], its [`Group`] and the [`Command`]
//! it names. The rows themselves are in [`table`], which is a split for length
//! and nothing else: [`BINDINGS`] is still the single table, re-exported from
//! here so every caller names one path.

use crossterm::event::KeyCode;

use super::Context;

mod table;
mod view;

pub use table::BINDINGS;

/// What a binding acts on, and therefore which heading the `?` popup lists it
/// under. A reviewer looking for "how do I get to the next file" scans a group,
/// not an alphabet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Group {
    /// Moving the cursor inside whatever pane has it.
    Move,
    /// Jumping the cursor by a page or to an end, and scrolling text sideways.
    Scroll,
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
        Group::Scroll,
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
            Group::Scroll => "Jump & scroll",
            Group::Focus => "Panes",
            Group::Comment => "Comments",
            Group::Edit => "Edit",
            Group::View => "View",
            Group::Quit => "Leave",
        }
    }
}

/// The leader a binding sits under: a first keystroke that opens a which-key
/// submenu, after which the binding's own `codes` are the second keystroke.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Leader {
    /// `Space`: the contextual menu — its children are whichever actions suit
    /// the mode the reviewer is in, filtered by [`Binding::contexts`].
    Context,
    /// `m`: jump straight to one of the review's modes.
    Mode,
    Goto,
    Comment,
    View,
}

impl Leader {
    /// Every leader, in the order the `?` popup lists them.
    pub const ALL: &'static [Leader] = &[
        Leader::Context,
        Leader::Mode,
        Leader::Goto,
        Leader::Comment,
        Leader::View,
    ];

    /// The key that opens the leader's submenu.
    #[must_use]
    pub fn key(self) -> char {
        match self {
            Leader::Context => ' ',
            Leader::Mode => 'm',
            Leader::Goto => 'g',
            Leader::Comment => 'c',
            Leader::View => 'v',
        }
    }

    /// How the leader key is spelled in the keymap and the which-key title —
    /// the letter itself, except `Space`, whose glyph is invisible.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Leader::Context => "Space",
            Leader::Mode => "m",
            Leader::Goto => "g",
            Leader::Comment => "c",
            Leader::View => "v",
        }
    }

    /// Whether the leader's submenu is filtered by the current context — only
    /// the [`Leader::Context`] menu is, so `Space` shows a mode's own actions.
    #[must_use]
    pub fn is_contextual(self) -> bool {
        matches!(self, Leader::Context)
    }

    /// What the which-key popup titles the submenu.
    #[must_use]
    pub fn title(self) -> &'static str {
        match self {
            Leader::Context => "actions here",
            Leader::Mode => "mode",
            Leader::Goto => "goto",
            Leader::Comment => "comment",
            Leader::View => "view",
        }
    }
}

/// What one key does, spelled once.
#[derive(Clone, Copy)]
pub struct Binding {
    /// How the popup spells the key, arrows and aliases included. For a binding
    /// under a leader this is the *second* keystroke alone.
    pub keys: &'static str,
    pub group: Group,
    /// The leader this binding sits under, or `None` for a direct key.
    pub leader: Option<Leader>,
    /// Which modes a [`Leader::Context`] (`Space`) child appears in — empty for
    /// every other binding, since only the contextual menu is filtered.
    pub contexts: &'static [Context],
    /// What it does, short enough to sit beside the key in a column.
    pub what: &'static str,
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
    PageForward,
    PageBackward,
    JumpFirst,
    JumpLast,
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
    CycleMode,
    FilesTab,
    CommitsTab,
    CommentsTab,
    ModeDiff,
    Enter,
    Escape,
    Comment,
    Delete,
    Resolve,
    Abandon,
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
    GroupDiff,
    BeforeAfter,
    Help,
    Quit,
}

impl Command {
    /// Whether the command acts on what the cursor is on, rather than being
    /// *ambient* (a view or session change). Only cursor-targeting children
    /// count when a leader decides it can skip its submenu for one live child.
    pub(super) fn targets_cursor(self) -> bool {
        matches!(
            self,
            Command::Comment
                | Command::Delete
                | Command::Resolve
                | Command::Abandon
                | Command::OpenEditor
                | Command::NextHunk
                | Command::PreviousHunk
                | Command::NextSymbol
                | Command::PreviousSymbol
                | Command::Pick
                | Command::Fold
                | Command::Enter
                | Command::Forward
                | Command::Back
                | Command::PageForward
                | Command::PageBackward
                | Command::JumpFirst
                | Command::JumpLast
                | Command::ScrollLeft
                | Command::ScrollRight
        )
    }
}
