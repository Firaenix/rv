//! What the keyboard is doing, what it acts on, and what it asks for next.

use ratatui::style::Color;

use crate::theme;

/// Not [`Copy`]: `ConfirmDelete` carries the question and its subject as one
/// state, so there is no way to be asking without knowing what about, and no way
/// for a stale id to survive the answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Mode {
    Browse,
    Comment,
    /// Every key answers this — `y` deletes, anything else cancels — so it cannot
    /// become a state the reviewer is stuck in.
    ConfirmDelete {
        id: String,
        label: String,
    },
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

/// What the event loop does next with the key it just handed over.
///
/// `Edit` is the one that needs the terminal: the state machine has resolved
/// what to open and cannot open it, because nothing in it may touch a screen.
/// [`crate::app::run`] answers it and hands the reviewer back afterwards.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Continue,
    Edit,
    Quit,
}

/// Where the reviewer is working right now, as one word.
///
/// Derived from the mode, the focus and the sidebar tab — never stored, so it
/// cannot fall out of step with any of them. It is what the bar's mode segment
/// names and what the `?` tooltip filters the keymap by: a richer fact than
/// [`Mode`], which says only whether keys are being *typed*.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Context {
    /// Browsing the file list.
    Files,
    /// Browsing the commits list.
    Commits,
    /// Browsing the comment browser.
    Comments,
    /// The cursor is on a diff line.
    Diff,
    /// Inside a line's comment stack.
    Stack,
    /// Typing a comment.
    Writing,
    /// Answering a delete confirmation.
    Confirming,
    /// Typing a symbol query.
    Finding,
}

impl Context {
    /// The word the bar's mode segment shows.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Context::Files => "FILES",
            Context::Commits => "COMMITS",
            Context::Comments => "COMMENTS",
            Context::Diff => "DIFF",
            Context::Stack => "STACK",
            Context::Writing => "COMMENT",
            Context::Confirming => "CONFIRM",
            Context::Finding => "FIND",
        }
    }

    /// The hue the mode segment takes: an existing meaning, never a new one.
    ///
    /// The panes take the focus magenta — the segment names where the focus is.
    /// Anything about comments takes comment blue, the commits list the hash
    /// teal, and a confirmation the alert yellow, because a destructive
    /// question *is* something that wants attention.
    #[must_use]
    pub fn colour(self) -> Color {
        match self {
            Context::Files | Context::Diff | Context::Finding => theme::FOCUS,
            Context::Commits => theme::HASH,
            Context::Comments | Context::Stack | Context::Writing => theme::COMMENT,
            Context::Confirming => theme::ALERT,
        }
    }
}

/// How much of the keymap `?` is showing.
///
/// Two sizes on one key: the first press answers "what can I do *here*" with a
/// tip in the corner, the second unrolls the whole manual, the third puts it
/// away. An enum rather than two booleans, because "the tip and the popup at
/// once" is not a state a reviewer can be in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum HelpStage {
    #[default]
    Closed,
    /// The contextual tip, in the corner above the bar's `? help` hint.
    Tip,
    /// The whole keymap, centred over the panes.
    Full,
}

/// Which engine a review's diffs come from.
///
/// A parameter rather than two constructors, one of them named after a fallback.
/// It is *configuration*: it is consulted on every `load_selected`, not per call
/// the way `diff::compute_with`'s flag is, and a value stored on the app and read
/// later is a setting whatever it is called.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum DiffEngine {
    /// The in-process diff at once, difftastic's behind it, and a swap when it
    /// lands — see [`diffs`]. What a reviewer gets, because it is the only one of
    /// the three where a keystroke never waits for a process to spawn.
    #[default]
    Auto,
    /// difftastic, computed before the call returns.
    ///
    /// No swap, so the lines on screen are the final ones from the first frame.
    /// This is what a test asserting about difftastic's output wants: the async
    /// path is worth exercising deliberately, and not worth racing accidentally in
    /// every other test in the suite.
    Structural,
    /// The `similar` fallback, always.
    ///
    /// This is the diff a user with no `difft` gets, and it is a distinct set of
    /// branches rather than a degraded copy: only it carries
    /// [`LineKind::Context`] lines and a [`rv_core::diff::DiffSource::Similar`]
    /// label. `rv --no-difft` selects it, so it is a capability a reviewer has
    /// rather than a hook the tests reach through.
    Fallback,
}
