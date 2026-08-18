//! The reviewer's state machine and its event loop.
//!
//! The split in this module is the point of it: [`App::on_key`] and everything
//! it calls are **terminal-free**. They take a [`KeyCode`], change state, read
//! and write `.review/`, and return — no `Terminal`, no raw mode, no PTY. That
//! is what lets `rv/tests/app.rs` drive a whole review, comment and all, as an
//! ordinary unit test. Only [`App::run`] touches the terminal, and it does
//! nothing else: set up, loop, tear down.
//!
//! [`App::on_key_event`] sits in front of [`App::on_key`] for the one decision
//! that cannot be made from a [`KeyCode`] alone — Ctrl+C, which raw mode leaves
//! to the program — and is terminal-free in exactly the same way.
//!
//! # Restoring the terminal
//!
//! A TUI that panics in raw mode leaves the user's shell unusable — no echo,
//! no line editing, a cursor that never came back. [`App::run`] therefore
//! installs a panic hook that restores the terminal *before* the default hook
//! prints its message, so the backtrace lands on a working terminal, and calls
//! [`ratatui::restore`] on every ordinary exit path too, including the error
//! one.
//!
//! # What a comment costs
//!
//! Saving a comment writes `.review/comments.json` and its snapshot (both
//! atomically, through the store) and then rewrites `REVIEW-FEEDBACK.md` via
//! [`session::write_markdown`], which folds in any reply an LLM appended
//! first. So the file an agent reads is never stale by more than one
//! keystroke, and a comment survives the process being killed the instant
//! after Enter. The in-memory copy the pane draws from is then re-read from
//! the store, so what is on screen is what is on disk rather than what this
//! process believes it wrote.
//!
//! # What a delete costs
//!
//! Deleting one goes through the store and stops there: the entry and its
//! snapshot go, the in-memory copy is re-read, and `REVIEW-FEEDBACK.md` is
//! **not** rewritten. The asymmetry with saving is deliberate rather than an
//! omission — the markdown is an *export* (see
//! `docs/superpowers/specs/2026-08-17-rv-storage-model-design.md`), produced by
//! `rv render` from whatever the store holds, and the save path's rewrite is
//! the thing on its way out rather than the behaviour to copy. A delete that
//! rewrote it would also be rewriting whatever reply an LLM had appended since,
//! for a document nobody asked for.
//!
//! Blobs are read lazily, for the selected file only (spec §7), and the
//! computed [`FileDiff`] is cached per file so that stepping back to a file
//! does not re-run difftastic.

use std::collections::HashMap;
use std::collections::HashSet;

use anyhow::Context as _;
use anyhow::Result;
use crossterm::event;
use crossterm::event::Event;
use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyEventKind;
use crossterm::event::KeyModifiers;
use ratatui::DefaultTerminal;
use rv_core::anchor;
use rv_core::diff;
use rv_core::diff::DiffLine;
use rv_core::diff::FileDiff;
use rv_core::diff::LineKind;
use rv_core::highlight::Highlights;
use rv_core::model::Anchor;
use rv_core::model::FileChange;
use rv_core::model::Side;
use rv_core::store::Comment;
use rv_core::store::CommentState;
use rv_core::store::Session;

use crate::layout::Split;
use crate::session;
use crate::session::Review;
use crate::ui;

/// How many hex characters of the digest make up a comment id.
///
/// Eight, not the four the plan and spec §10 write, because a collision here
/// is not a cosmetic clash: [`rv_core::store::Store::append_comment`] upserts
/// by id, so two *different* comments sharing a prefix mean the second save
/// silently replaces the first in `comments.json` and overwrites its snapshot
/// — under a "comment saved" status line. Four hex characters is a 65,536-value
/// space, which by the birthday bound is a ~2% chance of losing a comment at 50
/// of them and ~7% at 100: reachable on one real review. Spec §10's guarantee
/// that nothing loses a comment, and Task 5's write-through durability, outrank
/// the literal width. Eight still reads out of a marker at a glance, and
/// `markdown::parse_replies` binds whatever id the marker carries, so nothing
/// else changes.
const ID_CHARS: usize = 8;

/// The status line shown before the reviewer has done anything.
///
/// Every key that changes something is in here, `d` above all: a key that
/// destroys written work with no way back must be discoverable from inside the
/// app rather than only from the README. One bar row is the whole budget (see
/// [`crate::ui`]), so each entry is a key and one word — 68 columns, which fits
/// the 80-column terminal that is the narrowest anyone reviews in.
const HELP: &str = "j/k line  [/] file  c comment  enter stack  d delete  s fold  q quit";

/// What `d` says from the sidebar's **Files** tab, where there is no comment
/// under the cursor to delete.
///
/// It names the way out rather than only refusing: the reviewer pressed a key
/// meaning "delete this", and the answer they need is where "this" lives —
/// which is now either pane, since `tab` puts a list of comments in this very
/// column.
const DELETE_NEEDS_A_COMMENT: &str =
    "the file list selects files, not comments: tab for those, right for the diff";

/// What `d` and `s` say from the sidebar's **Comments** tab when the review has
/// no comments in it at all.
///
/// Both keys act on the browsed comment from there, so both refuse with a
/// sentence about the *review* rather than about a line: the browser is not
/// showing a line, and answering "no comments on this line" would send the
/// reviewer looking at the diff for the reason.
const NO_COMMENTS_IN_REVIEW: &str = "no comments in this review yet";

/// What `Enter`, `d` and `s` say when the selected line carries no comments.
///
/// One sentence for all three because it is one fact about the line, and a
/// reviewer who has just pressed a key wants to know why nothing happened
/// rather than which of three phrasings this key prefers.
const NO_COMMENTS: &str = "no comments on this line";

/// What the reviewer is doing with the keyboard.
///
/// Not [`Copy`] since [`Mode::ConfirmDelete`] gained its two fields. That is
/// the point of putting them here rather than in a pair of `Option` fields on
/// [`App`]: the question and the answer are one state, so there is no way to
/// be *asking* without knowing what is being asked about, and no way for a
/// stale id to survive the answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Keys navigate the diff.
    Browse,
    /// Keys go into the comment buffer.
    Comment,
    /// Waiting for `y` before removing the comment with this `id`, which is
    /// shown as `label` (`path:line`).
    ///
    /// Deletion is unrecoverable — the comment leaves the store and its
    /// snapshot is deleted with it — so a mistyped `d` while browsing must not
    /// cost a reviewer written work. Every key answers this question (`y`
    /// deletes, anything else cancels) precisely so that it cannot become a
    /// state the reviewer is stuck in.
    ConfirmDelete { id: String, label: String },
}

/// What the left column is listing.
///
/// The sidebar browses comments the same way it browses files — same column,
/// same keys, one idiom rather than two — because the alternative is what the
/// first real session on `rv` actually did: 2,200 of its 11,101 keystrokes went
/// on `j` and `]` hunting down its own remarks, one of them 940 consecutive
/// presses of `j`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SidebarTab {
    /// The review's changed files.
    Files,
    /// Every comment in the review, wherever it is anchored.
    Comments,
}

/// Which pane the keys act on.
///
/// A focus rather than a [`Mode`] because modes are for *typing*: a mode
/// changes what a keystroke means, while this only changes what it moves. That
/// is why `j`, `k` and the arrows keep their meaning across all three, and why
/// `[` and `]` are answered before the focus is consulted at all — a reviewer
/// walking the files never has to think about where the cursor is first.
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

/// What [`App::on_key`] wants the event loop to do next.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Continue,
    Quit,
}

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
///
/// `keys` and `what` are what the popup prints; `codes` is what actually
/// matches a key press and `command` is what running it does. All four live in
/// the same row of [`BINDINGS`] on purpose — see that constant for why.
pub struct Binding {
    /// How the popup spells the key, arrows and aliases included.
    pub keys: &'static str,
    pub group: Group,
    /// What it does, short enough to sit beside the key in a column.
    pub what: &'static str,
    /// The key presses this row answers. Private: it is the table's business
    /// which codes a row claims, and a caller comparing codes would be a second
    /// dispatcher.
    codes: &'static [KeyCode],
    /// What running the row does. Private for the same reason.
    command: Command,
}

/// Every key [`App::on_key_browse`] answers.
///
/// This is the **only** thing that handler dispatches from, which is what makes
/// the popup and the keyboard impossible to drift apart:
///
/// * a key that is not in a row here reaches no code at all, so a binding
///   cannot ship undocumented;
/// * a row here names a [`Command`], and [`App::run_command`] matches on
///   `Command` exhaustively, so a row cannot point at nothing — deleting the
///   arm that answers it does not compile;
/// * [`crate::ui`] draws the popup *from this table*, so a row cannot be
///   dispatched and left out of the manual.
///
/// The order is the order the popup reads in, grouped by [`Group`]. It is not
/// a priority order: no key appears in two rows.
pub const BINDINGS: &[Binding] = &[
    Binding {
        keys: "j / ↓",
        group: Group::Move,
        what: "next line",
        codes: &[KeyCode::Char('j'), KeyCode::Down],
        command: Command::Forward,
    },
    Binding {
        keys: "k / ↑",
        group: Group::Move,
        what: "previous line",
        codes: &[KeyCode::Char('k'), KeyCode::Up],
        command: Command::Back,
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
        keys: "←",
        group: Group::Focus,
        what: "the file list",
        codes: &[KeyCode::Left],
        command: Command::FocusLeft,
    },
    Binding {
        keys: "→",
        group: Group::Focus,
        what: "the diff",
        codes: &[KeyCode::Right],
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
        keys: "s",
        group: Group::Comment,
        what: "fold a comment",
        codes: &[KeyCode::Char('s')],
        command: Command::Fold,
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
/// Private, and deliberately not a `&'static str` or a function pointer: an
/// enum is what makes [`App::run_command`]'s match exhaustive, so a row of the
/// table cannot name a command nothing answers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Command {
    Forward,
    Back,
    NextFile,
    PreviousFile,
    FocusLeft,
    FocusRight,
    SwitchTab,
    Enter,
    Escape,
    Comment,
    Delete,
    Fold,
    Narrower,
    Wider,
    Help,
    Quit,
}

/// How many percentage points one press of `<` or `>` moves the divider.
///
/// Two rather than one: a keyboard resize the reviewer cannot see happen is a
/// resize they will hold the key down for, and one column of an 80-column
/// terminal is below the noise of the pane's own borders.
const NUDGE: i16 = 2;

/// One interactive review.
///
/// `diffs` is parallel to `review.files`: `None` means "not computed yet",
/// which is the whole of the lazy-loading scheme (spec §7).
///
/// `comments` is a *read-through copy* of the store, not a cache in front of
/// it: [`rv_core::store::Store`] stays the authority, every save still goes
/// straight to disk, and this vector is refreshed from disk immediately
/// afterwards (see [`App::reload_comments`]). It exists because the diff pane
/// has to draw the comments on the line it is drawing, and re-reading
/// `comments.json` once per line per frame is not a thing to do sixty times a
/// second.
pub struct App {
    review: Review,
    diffs: Vec<Option<FileDiff>>,
    comments: Vec<Comment>,
    file_index: usize,
    /// Where the highlight sits in each file's diff, parallel to
    /// `review.files`.
    ///
    /// One position per file rather than one shared between them, because
    /// `[`/`]` is how a reviewer compares two files and a shared cursor makes
    /// every round trip cost them their place: the first real review of `rv`
    /// spent a fifth of its keystrokes on `j` walking back down to where it had
    /// just been.
    line_indices: Vec<usize>,
    focus: Focus,
    /// What the left column lists.
    sidebar_tab: SidebarTab,
    /// Which row of the comment browser the cursor is on: an index into
    /// [`App::comments`], which is the whole review's comments in store order.
    ///
    /// Kept in range by [`App::clamp_browser`] rather than left to answer
    /// `None`, so that deleting the comment the browser was on leaves the
    /// cursor on the list instead of past the end of it.
    browser_index: usize,
    /// Which comment of the selected line's stack the cursor is on, meaningful
    /// only while the focus is [`Focus::Stack`].
    ///
    /// An index into [`App::comments_for_line`]'s answer rather than an id,
    /// because the stack is a list the reviewer walks with `j`/`k` and the
    /// store is what says which comments are in it. It is reset by
    /// [`App::reset_stack`] whenever the selection moves, so it can never
    /// address a comment on a line the reviewer has left.
    comment_index: usize,
    /// The comments the reviewer has folded away, by id.
    ///
    /// A **session-only view preference**, deliberately not review state: it
    /// never reaches `.review/`, so nothing another reviewer (or an LLM
    /// reading the export) sees depends on which boxes happened to be in this
    /// reviewer's way. Keyed by id rather than by position so that folding
    /// survives a delete, a save, or a walk to another file and back.
    collapsed: HashSet<String>,
    /// How the width is divided between the two panes.
    ///
    /// A **session-only view preference**, exactly like `collapsed`: it never
    /// reaches `.review/`, because how wide one reviewer likes their file list
    /// is not something another reviewer — or an LLM reading the export —
    /// should inherit.
    split: Split,
    /// Whether the `?` keymap is up.
    ///
    /// While it is, every key but the five it answers is inert. That is the
    /// point rather than a limitation: a reviewer reading about `d` must not
    /// discover what it does by pressing it.
    help_open: bool,
    /// How far the keymap has been scrolled, in rows.
    ///
    /// Only ever non-zero on a terminal too small to show the whole table at
    /// once — [`crate::ui`] clamps it against the geometry it has, because the
    /// geometry is the one thing this module deliberately does not know.
    help_scroll: usize,
    /// Highlight spans per `(commit, path)`, parsed once per blob.
    ///
    /// Keyed by the blob rather than by the file, because a diff line's colours
    /// come from **its own side**: a removed line is text that only exists at
    /// the base commit, under the base-side path, which for a rename is not the
    /// path the file is listed under. Filled beside the diff cache in
    /// [`App::load_selected`], from the same two blobs the diff is computed
    /// from, so opening a file costs one parse per side and revisiting it costs
    /// none.
    highlights: HashMap<(String, String), Highlights>,
    mode: Mode,
    buffer: String,
    status: String,
    /// Set to skip difftastic for every file in this review and take
    /// [`diff::compute_with`]'s `similar` fallback instead. See
    /// [`App::with_fallback_diffs`].
    force_fallback: bool,
}

impl App {
    /// Opens `review` in the reviewer, loading the first file's diff.
    ///
    /// Which diff engine each file goes through is left to
    /// [`diff::compute`], which honours `RV_NO_DIFFT`.
    pub fn new(review: Review) -> Result<Self> {
        Self::open(review, false)
    }

    /// Opens `review` with difftastic bypassed: every file's diff comes from
    /// the `similar` fallback.
    ///
    /// That is the diff a user with no `difft` on `PATH` gets, and the only one
    /// that carries [`LineKind::Context`] lines and a
    /// [`rv_core::diff::DiffSource::Similar`] label — so it is a distinct set
    /// of branches through this module and through [`crate::ui`], not a
    /// degraded copy of the difftastic path.
    ///
    /// Per-`App` rather than through `RV_NO_DIFFT`, for the same reason
    /// [`diff::compute_with`] takes the choice as an argument: the environment
    /// variable is process-wide, and a caller that wants the fallback for
    /// *this* review should not have to change what every other review in the
    /// process sees.
    pub fn with_fallback_diffs(review: Review) -> Result<Self> {
        Self::open(review, true)
    }

    fn open(review: Review, force_fallback: bool) -> Result<Self> {
        let diffs = vec![None; review.files.len()];
        // Read before the review is moved into `Self`, and before the first
        // diff is computed: a reviewer who quit halfway through yesterday
        // opens on the notes they already made, not on an empty pane that
        // fills in only once they save something new.
        let comments = review
            .store
            .comments()
            .context("could not read the saved comments")?;
        let line_indices = vec![0; review.files.len()];
        // A comment that is no longer open starts folded: it is still exactly
        // where the reviewer left it, in file and line order, without competing
        // for the screen with the comments that are still asking for an answer.
        // Seeded here rather than forced at every frame so that `s` can expand
        // one like any other box — a box a reviewer cannot open is a worse
        // failure than a loud one.
        let collapsed = comments
            .iter()
            .filter(|comment| comment.state != CommentState::Open)
            .map(|comment| comment.id.clone())
            .collect();
        let mut app = Self {
            review,
            diffs,
            comments,
            file_index: 0,
            line_indices,
            focus: Focus::Diff,
            sidebar_tab: SidebarTab::Files,
            browser_index: 0,
            comment_index: 0,
            collapsed,
            split: Split::default(),
            help_open: false,
            help_scroll: 0,
            highlights: HashMap::new(),
            mode: Mode::Browse,
            buffer: String::new(),
            status: HELP.to_owned(),
            force_fallback,
        };
        app.load_selected()?;
        Ok(app)
    }

    /// Runs the reviewer on the terminal until the user quits.
    ///
    /// Everything that can fail without a terminal — opening the repository,
    /// reading the first diff — has already failed by the time raw mode is
    /// entered, so such an error prints as a sentence rather than into a
    /// half-initialized screen. `try_init` rather than `init` for the same
    /// reason: a `rv` that was piped somewhere has no terminal to take over,
    /// and that is a sentence too, not a panic.
    pub fn run(review: Review) -> Result<()> {
        let mut app = Self::new(review)?;

        // Before `try_init`, which chains its own restoring hook in front of
        // whatever it finds — so the terminal is restored twice on a panic
        // (harmless) rather than depending on ratatui to keep doing it.
        install_panic_hook();
        let mut terminal = ratatui::try_init().context("could not start the terminal")?;
        let result = app.event_loop(&mut terminal);
        // Unconditional, and before the error is returned: a failed loop must
        // still hand the shell back in a usable state.
        ratatui::restore();
        result
    }

    /// The file the sidebar has selected, or `None` when the range changed no
    /// files at all.
    pub fn selected_file(&self) -> Option<&FileChange> {
        self.review.files.get(self.file_index)
    }

    /// The selected file's diff, once it has been loaded.
    pub fn selected_diff(&self) -> Option<&FileDiff> {
        self.diffs.get(self.file_index).and_then(Option::as_ref)
    }

    /// Every file in the review, in sidebar order.
    pub fn files(&self) -> &[FileChange] {
        &self.review.files
    }

    /// The range under review: its two endpoint commits and the changes
    /// between them.
    pub fn session(&self) -> &Session {
        &self.review.session
    }

    /// Which file the sidebar has selected.
    pub fn file_index(&self) -> usize {
        self.file_index
    }

    /// Which line of the selected diff is highlighted.
    ///
    /// Zero when the review has no files, which is the only way this can be
    /// asked about a file that does not exist.
    pub fn line_index(&self) -> usize {
        self.line_indices.get(self.file_index).copied().unwrap_or(0)
    }

    /// What the keyboard is doing right now.
    ///
    /// Returned by value — a clone — rather than as a borrow. [`Mode`] stopped
    /// being [`Copy`] when [`Mode::ConfirmDelete`] gained the id it is about,
    /// and every caller either compares the answer against a literal or holds
    /// it across the next [`App::on_key`], which takes `&mut self`: a borrow
    /// would make each of them clone anyway, or fight the borrow checker for
    /// no gain. The clone is one short id, and only while a confirmation is up.
    pub fn mode(&self) -> Mode {
        self.mode.clone()
    }

    /// Which pane the movement keys act on. The diff, on launch: that is what
    /// a reviewer came to read.
    pub fn focus(&self) -> Focus {
        self.focus
    }

    /// What the left column is listing. Files, on launch: a review starts with
    /// no comments in it.
    pub fn sidebar_tab(&self) -> SidebarTab {
        self.sidebar_tab
    }

    /// Which row of the comment browser the cursor is on.
    pub fn browser_index(&self) -> usize {
        self.browser_index
    }

    /// The comment the browser's cursor is on, or `None` when the sidebar is
    /// not listing comments.
    ///
    /// Gated on the tab for the same reason [`App::selected_comment`] is gated
    /// on the focus: `d` asks this question to decide what it destroys — and
    /// `s` to decide what it folds — and answering it with a comment that is
    /// not on screen is how a delete hits the wrong one. The Files tab has a
    /// file selected and no comment, which is what the `None` says. Not gated
    /// on the *focus*, though — the browser draws its selection whether or not
    /// the keys are pointed at it, so the selection is real either way.
    pub fn browsed_comment(&self) -> Option<&Comment> {
        if self.sidebar_tab != SidebarTab::Comments {
            return None;
        }
        self.comments.get(self.browser_index)
    }

    /// Every comment in the review, in store order (oldest first).
    pub fn comments(&self) -> &[Comment] {
        &self.comments
    }

    /// The ids of the comments the reviewer has folded away.
    ///
    /// [`crate::ui`] draws a collapsed comment as a single line instead of a
    /// box. Nothing else reads it, and nothing writes it to disk — see the
    /// field's own doc comment for why that is the point rather than an
    /// omission.
    pub fn collapsed(&self) -> &HashSet<String> {
        &self.collapsed
    }

    /// Which comment of the selected line's stack the cursor is on.
    ///
    /// Only meaningful while [`App::focus`] is [`Focus::Stack`]; it is 0 the
    /// rest of the time, which is where entering a stack starts.
    pub fn comment_index(&self) -> usize {
        self.comment_index
    }

    /// The comment the stack cursor is on, or `None` when the cursor is not in
    /// a stack.
    ///
    /// Deliberately `None` off [`Focus::Stack`] rather than "whatever comment
    /// index 0 would be": `d` and `s` both ask this question to decide what a
    /// keystroke acts on, and answering it with a comment the reviewer has not
    /// selected is how a delete hits the wrong one.
    pub fn selected_comment(&self) -> Option<&Comment> {
        if self.focus != Focus::Stack {
            return None;
        }
        self.comments_for_line(self.line_index())
            .get(self.comment_index)
            .copied()
    }

    /// The comments anchored to diff line `index` of the selected file, oldest
    /// first.
    ///
    /// A line is matched by the key it would anchor *under*, never by its raw
    /// number: [`App::anchor_target`] derives the side and the side's path from
    /// the same [`anchored_side`] rule [`App::prepare_comment`] saves through,
    /// so a comment can never be stored against one line and displayed against
    /// another. Milestone 1 shipped that bug once; there is deliberately only
    /// one side rule in this file.
    ///
    /// Filtered rather than pre-indexed, and returning borrows rather than a
    /// slice, because the matches are not contiguous in the store's order.
    pub fn comments_for_line(&self, index: usize) -> Vec<&Comment> {
        let Some(line) = self.selected_diff().and_then(|diff| diff.lines.get(index)) else {
            return Vec::new();
        };
        let Some(target) = self.anchor_target(line) else {
            return Vec::new();
        };
        self.comments
            .iter()
            .filter(|comment| {
                comment.anchor.file == target.path
                    && comment.anchor.side == target.side
                    && comment.anchor.line == target.number
            })
            .collect()
    }

    /// How the width is divided between the sidebar and the diff.
    ///
    /// Session-only — see the field. [`crate::ui`] hands it straight to
    /// [`crate::layout::layout`], which is the only thing that turns it into a
    /// rectangle.
    pub fn split(&self) -> Split {
        self.split
    }

    /// Whether the `?` keymap is up.
    pub fn help_open(&self) -> bool {
        self.help_open
    }

    /// How far the keymap has been scrolled, in rows. Clamped by the renderer
    /// against the popup it actually has; see the field.
    pub fn help_scroll(&self) -> usize {
        self.help_scroll
    }

    /// The highlight spans for the selected file's blob **on `side`** — the
    /// base blob at its base-side path for [`Side::Left`], the head blob at the
    /// file's own path for [`Side::Right`].
    ///
    /// `None` for a side the commit has no plain file at (an add has no base,
    /// a delete has no head) and for a file whose extension names no grammar
    /// rv ships. [`crate::ui`] shows the second of those in the pane's title
    /// rather than letting a reviewer wonder whether the colour is broken.
    ///
    /// Callers choose `side` through [`anchored_side`] and nothing else: a
    /// removed line looked up on the head side would be painted with the
    /// colours of whatever now stands at its number, which is a lie told in a
    /// colour rather than in words.
    pub fn highlights(&self, side: Side) -> Option<&Highlights> {
        let file = self.selected_file()?;
        let session = &self.review.session;
        let (commit, path) = match side {
            Side::Left => (
                session.base_commit.as_str(),
                file.source_path.as_deref().unwrap_or(&file.path),
            ),
            Side::Right => (session.head_commit.as_str(), file.path.as_str()),
        };
        // One allocation per side per frame: `ui` asks once and hands the
        // answer down its row loop rather than asking per line.
        self.highlights.get(&(commit.to_owned(), path.to_owned()))
    }

    /// Whether `binding` would do anything from where the cursor is now.
    ///
    /// The popup dims the ones that would not, rather than hiding them: a
    /// reviewer learning the tool should see that `d` exists and that the file
    /// list is the wrong place for it, not wonder whether they misread the
    /// manual.
    pub fn binding_enabled(&self, binding: &Binding) -> bool {
        match binding.command {
            // Always something to do: they change what is on screen, never what
            // is under the cursor.
            Command::SwitchTab
            | Command::Narrower
            | Command::Wider
            | Command::Help
            | Command::Quit => true,
            Command::Forward => self.can_move_forward(),
            Command::Back => self.can_move_back(),
            Command::NextFile => self.file_index + 1 < self.review.files.len(),
            Command::PreviousFile => self.file_index > 0,
            // `Left` leads out of every focus except the leftmost; `Right`
            // stops at the diff.
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
            Command::Fold => !self.fold_targets().is_empty(),
        }
    }

    /// Whether `j` has anywhere to go in the pane that has the cursor.
    fn can_move_forward(&self) -> bool {
        match self.focus {
            Focus::Sidebar => match self.sidebar_tab {
                SidebarTab::Files => self.file_index + 1 < self.review.files.len(),
                SidebarTab::Comments => self.browser_index + 1 < self.comments.len(),
            },
            Focus::Diff => self.line_index() + 1 < self.line_count(),
            Focus::Stack => self.comment_index + 1 < self.stack_len(),
        }
    }

    /// The same for `k`.
    fn can_move_back(&self) -> bool {
        match self.focus {
            Focus::Sidebar => match self.sidebar_tab {
                SidebarTab::Files => self.file_index > 0,
                SidebarTab::Comments => self.browser_index > 0,
            },
            Focus::Diff => self.line_index() > 0,
            Focus::Stack => self.comment_index > 0,
        }
    }

    /// The comment being typed, empty outside [`Mode::Comment`].
    pub fn buffer(&self) -> &str {
        &self.buffer
    }

    /// The one-line message under the reviewer's last action.
    pub fn status(&self) -> &str {
        &self.status
    }

    /// Handles one key press, modifiers and all.
    ///
    /// Ctrl+C is answered here rather than in [`App::on_key`] because the state
    /// machine below is written against plain [`KeyCode`]s — which is what
    /// makes it testable without a pty — and `Char('c')` with CONTROL held is
    /// indistinguishable from a plain `c` once the modifiers are dropped. In
    /// raw mode the terminal raises no SIGINT on the reviewer's behalf and `rv`
    /// offers no other abort, so without this the one key every terminal user
    /// reaches for would open the comment box and type into it.
    ///
    /// It quits from any mode, including a half-typed comment: an abort that
    /// first asks you to `Esc` is not an abort. The buffer is dropped
    /// unsaved, which is the same thing `Esc` does with it.
    pub fn on_key_event(&mut self, event: KeyEvent) -> Result<Action> {
        if event.modifiers.contains(KeyModifiers::CONTROL)
            && matches!(event.code, KeyCode::Char('c'))
        {
            return Ok(Action::Quit);
        }
        self.on_key(event.code)
    }

    /// Handles one key press. Terminal-free by construction — see the module
    /// docs.
    ///
    /// The keymap is answered ahead of the mode because it is a modal window
    /// rather than a mode: it can only be raised from [`Mode::Browse`] (nothing
    /// else binds `?`) and nothing raised behind it can change the mode, so
    /// `help_open` implies browsing and this branch is the whole of what the
    /// popup consumes.
    pub fn on_key(&mut self, key: KeyCode) -> Result<Action> {
        if self.help_open {
            return Ok(self.on_key_help(key));
        }
        match self.mode {
            Mode::Browse => self.on_key_browse(key),
            Mode::Comment => self.on_key_comment(key),
            Mode::ConfirmDelete { .. } => self.on_key_confirm_delete(key),
        }
    }

    /// The five keys the `?` popup answers; everything else is inert while it
    /// is up.
    ///
    /// `q` **closes** rather than quits. A reviewer with the manual open is by
    /// definition the one least sure what the keys do, and ending their review
    /// is the most expensive way to find out. `?` closes it too, because the
    /// key that raised a thing is the first one a hand reaches for to be rid of
    /// it, and `Esc` because that is how everything else in this reviewer is
    /// dismissed.
    ///
    /// `j`/`k` (and their arrows) scroll, which only ever moves anything on a
    /// terminal too small for the whole table — see [`App::help_scroll`].
    fn on_key_help(&mut self, key: KeyCode) -> Action {
        match key {
            KeyCode::Char('?') | KeyCode::Char('q') | KeyCode::Esc => self.help_open = false,
            KeyCode::Char('j') | KeyCode::Down => {
                self.help_scroll = self.help_scroll.saturating_add(1);
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.help_scroll = self.help_scroll.saturating_sub(1);
            }
            _ => {}
        }
        Action::Continue
    }

    fn event_loop(&mut self, terminal: &mut DefaultTerminal) -> Result<()> {
        loop {
            terminal
                .draw(|frame| ui::draw(frame, self))
                .context("could not draw the review")?;

            // Key *releases* and repeats are reported by terminals that speak
            // the kitty protocol; acting on presses only keeps one keystroke
            // from typing two characters there.
            if let Event::Key(key) = event::read().context("could not read a key")?
                && key.kind == KeyEventKind::Press
                && self.on_key_event(key)? == Action::Quit
            {
                return Ok(());
            }
        }
    }

    /// Looks `key` up in [`BINDINGS`] and runs whatever row claims it.
    ///
    /// A lookup rather than a `match` on purpose: the table is what the `?`
    /// popup is drawn from, so dispatching through it is what makes an
    /// undocumented binding unrepresentable. A key no row claims is inert,
    /// which is what every `unbound_*` case in `rv/tests/app_cases.rs` pins.
    fn on_key_browse(&mut self, key: KeyCode) -> Result<Action> {
        let Some(binding) = BINDINGS.iter().find(|binding| binding.codes.contains(&key)) else {
            return Ok(Action::Continue);
        };
        self.run_command(binding.command)
    }

    /// Runs one row of [`BINDINGS`].
    ///
    /// Exhaustive over [`Command`] by construction: a row that named a command
    /// nothing answered would not compile, which is the other half of the
    /// anti-drift guarantee the table's doc comment claims.
    fn run_command(&mut self, command: Command) -> Result<Action> {
        match command {
            Command::Quit => return Ok(Action::Quit),
            Command::FocusLeft => self.focus_left(),
            Command::FocusRight => self.focus_right(),
            Command::Forward => self.move_forward()?,
            Command::Back => self.move_back()?,
            // `[` and `]` mean "the next file" wherever the cursor happens to
            // be — they consult no focus at all — so walking a review never
            // costs a trip through the sidebar.
            Command::NextFile => self.select_file(self.file_index.saturating_add(1))?,
            Command::PreviousFile => self.select_file(self.file_index.saturating_sub(1))?,
            Command::Comment => self.begin_comment(),
            Command::Delete => self.begin_delete(),
            Command::Fold => self.toggle_collapse(),
            // Focus-free, like `[` and `]`: what the left column lists is not a
            // question about where the cursor is, and a reviewer who wants
            // their comments should not have to travel to the sidebar first to
            // ask for them.
            Command::SwitchTab => self.switch_tab(),
            Command::Enter => self.on_enter()?,
            Command::Escape => self.leave_stack(),
            Command::Narrower => self.split = self.split.nudged(-NUDGE),
            Command::Wider => self.split = self.split.nudged(NUDGE),
            Command::Help => {
                self.help_open = true;
                // Opened at the top, always: a popup that remembered where it
                // was last scrolled to would open somewhere the reviewer did
                // not leave it, since the geometry it was scrolled against may
                // have changed.
                self.help_scroll = 0;
            }
        }
        Ok(Action::Continue)
    }

    /// Flips the left column between the files and the comments.
    ///
    /// Says nothing in the status line: it is navigation, and the pane's own
    /// title reports which tab is up. A key that overwrote the help text to
    /// announce itself would cost the reviewer the line they read the rest of
    /// the keymap off.
    fn switch_tab(&mut self) {
        self.sidebar_tab = match self.sidebar_tab {
            SidebarTab::Files => SidebarTab::Comments,
            SidebarTab::Comments => SidebarTab::Files,
        };
        self.clamp_browser();
    }

    /// `Enter`: into the selected line's comment stack, or — from the comment
    /// browser — to the code the browsed comment is about.
    fn on_enter(&mut self) -> Result<()> {
        if self.focus == Focus::Sidebar && self.sidebar_tab == SidebarTab::Comments {
            return self.jump_to_comment(self.browser_index);
        }
        self.enter_stack();
        Ok(())
    }

    /// Selects the file and line a comment is anchored to and hands the focus
    /// to the diff, so that reading a comment and looking at the code it is
    /// about are one keystroke apart.
    ///
    /// Two honest failure cases, both reported rather than papered over:
    ///
    /// * the anchored **file** may no longer be in the review's file list — the
    ///   range moved under the comment — in which case nothing moves at all,
    ///   because there is nowhere to move to;
    /// * the anchored **line** may not be in the current diff — the content
    ///   moved — in which case the file is opened anyway, at its top, with the
    ///   line named in the status. Being in the right file with a warning beats
    ///   staying put and saying nothing.
    ///
    /// Which line that is comes from [`App::line_of_anchor`], which asks the
    /// same question the save path asked, so a jump and a save can never
    /// disagree about which line a comment belongs to.
    fn jump_to_comment(&mut self, index: usize) -> Result<()> {
        let Some(comment) = self.comments.get(index) else {
            return Ok(());
        };
        let anchor = comment.anchor.clone();

        // Either side's path: a comment on a removed line is filed under the
        // base-side path, which for a rename is not the path the file is listed
        // under.
        let found = self.review.files.iter().position(|file| {
            file.path == anchor.file || file.source_path.as_deref() == Some(anchor.file.as_str())
        });
        let Some(file_index) = found else {
            self.status = format!("{} is not in this review's range any more", anchor.file);
            return Ok(());
        };

        self.file_index = file_index;
        self.load_selected()?;
        match self.line_of_anchor(&anchor) {
            Some(line) => {
                self.set_line_index(line);
                self.status = format!("jumped to {}:{}", anchor.file, anchor.line);
            }
            None => {
                self.set_line_index(0);
                self.status = format!(
                    "{}: line {} is not in this diff any more",
                    anchor.file, anchor.line
                );
            }
        }
        self.focus = Focus::Diff;
        Ok(())
    }

    /// The diff line whose anchor key matches `anchor`, using the same
    /// [`App::anchor_target`] the save path goes through — so the line a jump
    /// lands on is by construction the line the comment was stored against,
    /// rename, side rule and all.
    fn line_of_anchor(&self, anchor: &Anchor) -> Option<usize> {
        let diff = self.selected_diff()?;
        (0..diff.lines.len()).find(|index| {
            self.anchor_target(&diff.lines[*index])
                .is_some_and(|target| {
                    target.path == anchor.file
                        && target.side == anchor.side
                        && target.number == anchor.line
                })
        })
    }

    /// Keeps the browser's cursor on the list after the list has changed under
    /// it. An empty list parks it at 0, which is where the next comment lands.
    fn clamp_browser(&mut self) {
        self.browser_index = self
            .browser_index
            .min(self.comments.len().saturating_sub(1));
    }

    /// Steps the cursor into the selected line's comment stack.
    ///
    /// From [`Focus::Diff`] only. From the sidebar's **Files** tab `Enter` is
    /// unbound — a file is already selected by being highlighted — and from
    /// inside the stack it is inert rather than a jump back to the first
    /// comment: a key that quietly moved the cursor while the reviewer was
    /// already choosing with `j`/`k` would be a key they had to be careful of.
    /// From the **Comments** tab it jumps instead; see [`App::on_enter`].
    ///
    /// A line with nothing on it is refused with a sentence rather than
    /// entered. An empty stack is a focus containing nothing, which the
    /// reviewer would then have to guess their way out of.
    fn enter_stack(&mut self) {
        if self.focus != Focus::Diff {
            return;
        }
        if self.comments_for_line(self.line_index()).is_empty() {
            self.status = NO_COMMENTS.to_owned();
            return;
        }
        self.focus = Focus::Stack;
        self.comment_index = 0;
    }

    /// `Esc` out of the stack, and a no-op anywhere else — the other way out of
    /// the one focus that is entered deliberately, beside [`App::focus_left`].
    ///
    /// Two ways out, on the two keys a terminal user reaches for, is what keeps
    /// the stack from being somewhere a reviewer can get stuck.
    fn leave_stack(&mut self) {
        if self.focus == Focus::Stack {
            self.focus = Focus::Diff;
        }
    }

    /// How many comments the selected line carries.
    fn stack_len(&self) -> usize {
        self.comments_for_line(self.line_index()).len()
    }

    /// Takes the cursor out of the comment stack and puts the stack index back
    /// at the top, because the *selection* moved out from under both.
    ///
    /// Called wherever a line or a file is selected. The stack index means "the
    /// nth comment on the selected line", so it is only ever valid for the line
    /// it was set on; leaving it alone across a `j` would point it at a comment
    /// of a line the reviewer is no longer looking at.
    ///
    /// The focus leaves **unconditionally** — not only when the new line's stack
    /// happens to be empty. Entering a stack is a deliberate act (`Enter`, on a
    /// line the reviewer chose), so it is never something navigation may hand
    /// on: `]` off a stack onto a file whose current line also carries comments
    /// would otherwise land the cursor *inside that line's stack*, having never
    /// entered it, with `d` and `s` aimed at a comment nobody selected. A
    /// conditional version of this shipped once and its test passed vacuously,
    /// because the fixture's other file had no comment on the line `]` landed
    /// on.
    fn reset_stack(&mut self) {
        self.comment_index = 0;
        if self.focus == Focus::Stack {
            self.focus = Focus::Diff;
        }
    }

    fn focus_left(&mut self) {
        self.focus = match self.focus {
            Focus::Stack => Focus::Diff,
            Focus::Diff | Focus::Sidebar => Focus::Sidebar,
        };
    }

    /// `Right` from the comment stack does nothing: the stack is drawn inside
    /// the diff pane, so there is no pane to its right. `Left` leads out of
    /// every focus, which is what keeps none of them a trap.
    fn focus_right(&mut self) {
        self.focus = match self.focus {
            Focus::Sidebar => Focus::Diff,
            Focus::Diff | Focus::Stack => self.focus,
        };
    }

    /// `j` / `Down` in the focused pane — and, in the sidebar, in whichever
    /// list that pane is showing.
    fn move_forward(&mut self) -> Result<()> {
        match self.focus {
            Focus::Sidebar => match self.sidebar_tab {
                SidebarTab::Files => self.select_file(self.file_index.saturating_add(1))?,
                SidebarTab::Comments => {
                    let last = self.comments.len().saturating_sub(1);
                    self.browser_index = self.browser_index.saturating_add(1).min(last);
                }
            },
            Focus::Diff => self.set_line_index(self.line_index().saturating_add(1)),
            Focus::Stack => {
                let last = self.stack_len().saturating_sub(1);
                self.comment_index = self.comment_index.saturating_add(1).min(last);
            }
        }
        Ok(())
    }

    /// `k` / `Up` in the focused pane.
    fn move_back(&mut self) -> Result<()> {
        match self.focus {
            // `select_file(0)` from file 0 is a no-op by its own guard, so `k`
            // at the top of the list stays put rather than wrapping.
            Focus::Sidebar => match self.sidebar_tab {
                SidebarTab::Files => self.select_file(self.file_index.saturating_sub(1))?,
                SidebarTab::Comments => {
                    self.browser_index = self.browser_index.saturating_sub(1);
                }
            },
            Focus::Diff => self.set_line_index(self.line_index().saturating_sub(1)),
            Focus::Stack => self.comment_index = self.comment_index.saturating_sub(1),
        }
        Ok(())
    }

    /// Moves the highlight to `index` of the selected file, clamped to that
    /// file's last diff line.
    ///
    /// The one place a line position is written, so the clamp cannot be
    /// forgotten on some path: a diff with no lines pins it at 0, and a review
    /// with no files has nowhere to put it at all.
    fn set_line_index(&mut self, index: usize) {
        let clamped = index.min(self.line_count().saturating_sub(1));
        if let Some(position) = self.line_indices.get_mut(self.file_index) {
            *position = clamped;
        }
        // The stack belongs to the line, so it goes back to the top with it.
        self.reset_stack();
    }

    fn on_key_comment(&mut self, key: KeyCode) -> Result<Action> {
        match key {
            KeyCode::Esc => {
                self.mode = Mode::Browse;
                self.buffer.clear();
                self.status = "comment discarded".to_owned();
            }
            KeyCode::Backspace => {
                self.buffer.pop();
            }
            KeyCode::Enter => {
                self.commit_comment()?;
                self.mode = Mode::Browse;
                self.buffer.clear();
            }
            KeyCode::Char(character) => self.buffer.push(character),
            _ => {}
        }
        Ok(Action::Continue)
    }

    /// Folds comment boxes away, or unfolds them — the view preference `s`
    /// toggles.
    ///
    /// What it acts on follows the cursor, exactly as `d` does, and for the
    /// same reason: a key acts on what the reviewer is looking at.
    ///
    /// * **inside the stack**, the one box the cursor is on;
    /// * **in the sidebar's Comments tab**, the comment the browser is on —
    ///   which is the comment on screen there, and need not be on the selected
    ///   diff line or even in the selected file;
    /// * **anywhere else** — the diff, and the sidebar's Files tab — the whole
    ///   of the selected line's stack, because a file row selects no comment
    ///   and the line the diff is on is the only comment the screen is showing.
    ///
    /// A line whose boxes are in *mixed* states collapses rather than expands.
    /// The reason to press `s` on a line is to get it out of the way, and a
    /// toggle that flipped each box independently would leave the line half
    /// folded and need a second press to finish a job the reviewer asked for
    /// once. Expanding is then the answer to "they are all folded already".
    ///
    /// Nothing here writes: see [`App::collapsed`].
    fn toggle_collapse(&mut self) {
        let ids = self.fold_targets();
        if ids.is_empty() {
            // Said about the review from the browser, which is not showing a
            // line, and about the line everywhere else — the same split `d`
            // makes, because it is the same question about the same two
            // cursors.
            self.status = match (self.focus, self.sidebar_tab) {
                (Focus::Sidebar, SidebarTab::Comments) => NO_COMMENTS_IN_REVIEW,
                _ => NO_COMMENTS,
            }
            .to_owned();
            return;
        }

        let folded = ids.iter().all(|id| self.collapsed.contains(id));
        for id in ids {
            if folded {
                self.collapsed.remove(&id);
            } else {
                self.collapsed.insert(id);
            }
        }
    }

    /// Which comments `s` would fold, as ids. Empty where it would fold
    /// nothing, which is also how [`App::binding_enabled`] knows to dim it —
    /// one rule, asked twice, rather than a copy in the renderer that could
    /// disagree with the key.
    fn fold_targets(&self) -> Vec<String> {
        match (self.focus, self.sidebar_tab) {
            (Focus::Stack, _) => self
                .selected_comment()
                .map(|comment| comment.id.clone())
                .into_iter()
                .collect(),
            (Focus::Sidebar, SidebarTab::Comments) => self
                .browsed_comment()
                .map(|comment| comment.id.clone())
                .into_iter()
                .collect(),
            (Focus::Diff | Focus::Sidebar, _) => self
                .comments_for_line(self.line_index())
                .iter()
                .map(|comment| comment.id.clone())
                .collect(),
        }
    }

    /// Which comment `d` would ask about, or `None` where it would refuse.
    ///
    /// The rules differ by cursor because the situations do; see
    /// [`App::begin_delete`], which is the only caller that acts on the answer.
    /// [`App::binding_enabled`] asks the same question to decide whether to dim
    /// the row, so the popup cannot claim `d` is live somewhere it refuses.
    fn delete_target(&self) -> Option<&Comment> {
        match self.focus {
            Focus::Stack => self.selected_comment(),
            Focus::Diff => self.comments_for_line(self.line_index()).last().copied(),
            // `browsed_comment` is already `None` on the Files tab, so this
            // covers both of the sidebar's shapes.
            Focus::Sidebar => self.browsed_comment(),
        }
    }

    /// Asks before deleting: picks what `d` would remove and enters
    /// [`Mode::ConfirmDelete`] with the question in the status line.
    ///
    /// Which comment that is depends on where the cursor is, and the two rules
    /// are different because the two situations are:
    ///
    /// * **inside the stack**, `d` takes the comment the cursor is on — the
    ///   reviewer is looking at one comment of several and pointing at it;
    /// * **on the diff**, it takes the *newest* on the line, which is the one
    ///   just written and the one a reviewer reaching for `d` means. The
    ///   oldest would be the strange choice: it is the note they have lived
    ///   with longest.
    ///
    /// * **in the sidebar**, it depends on what the sidebar is listing. The
    ///   **Comments** tab has a comment selected and on screen, so `d` takes
    ///   exactly that one — the unambiguous path, and the one to prefer. The
    ///   **Files** tab deletes nothing and says why: `c` does write against the
    ///   selected diff line from there and the symmetry is tempting, but the
    ///   two keys are not symmetrical. `c` creates, and a comment made by
    ///   mistake is undone by `d`; `d` destroys, and nothing undoes it. A `d`
    ///   pressed at a list of *files* would be aimed at a comment the reviewer
    ///   cannot see, on a diff line they may never have opened.
    ///
    /// With nothing to delete there is no question worth asking, so it says so
    /// and stays in [`Mode::Browse`] rather than opening a confirmation about
    /// nothing.
    fn begin_delete(&mut self) {
        let Some(comment) = self.delete_target() else {
            self.status = match (self.focus, self.sidebar_tab) {
                (Focus::Sidebar, SidebarTab::Files) => DELETE_NEEDS_A_COMMENT,
                (Focus::Sidebar, SidebarTab::Comments) => NO_COMMENTS_IN_REVIEW,
                _ => NO_COMMENTS,
            }
            .to_owned();
            return;
        };

        let label = format!("{}:{}", comment.anchor.file, comment.anchor.line);
        let id = comment.id.clone();
        self.status = format!("delete comment at {label}? (y/n)");
        self.mode = Mode::ConfirmDelete { id, label };
    }

    /// Answers the delete confirmation — `y` deletes, anything else cancels —
    /// and leaves [`Mode::ConfirmDelete`] either way.
    ///
    /// The mode is taken out *first*, with [`std::mem::replace`], so that
    /// leaving it is not a thing any branch below could forget: whatever
    /// happens after this line, including the `?` on a store that could not be
    /// written, the reviewer is back in [`Mode::Browse`] and their keyboard
    /// does what it did before. A confirmation nobody can dismiss is worse
    /// than no confirmation at all.
    ///
    /// Only a lowercase `y` confirms. Every ambiguity here — a shifted key, a
    /// stray arrow, a repeated `d` — resolves toward keeping the comment,
    /// because one of the two mistakes is recoverable by pressing `d` again and
    /// the other is not recoverable at all.
    ///
    /// It deliberately does **not** rewrite `REVIEW-FEEDBACK.md`. That document
    /// is an *export* (see the storage-model spec): `rv render` produces it from
    /// the store, and a delete leaves it alone until the next one.
    fn on_key_confirm_delete(&mut self, key: KeyCode) -> Result<Action> {
        let Mode::ConfirmDelete { id, label } = std::mem::replace(&mut self.mode, Mode::Browse)
        else {
            // Unreachable: `on_key` dispatches here only from `ConfirmDelete`.
            return Ok(Action::Continue);
        };

        if key != KeyCode::Char('y') {
            self.status = format!("deletion cancelled, {label} kept");
            return Ok(Action::Continue);
        }

        // Counted before the removal, and from the line rather than the whole
        // review: "1 of 3" is what a reviewer needs in order to know how much
        // of what they were looking at is still there.
        let before = self.stack_len();
        let removed = self
            .review
            .store
            .remove_comment(&id)
            .with_context(|| format!("could not delete the comment at {label}"))?;
        self.reload_comments()?;
        // A folded comment that is gone is not folded, it is gone. Leaving the
        // id behind would fold a later comment that hashed to it — the same
        // body on the same line — under a preference about a comment the
        // reviewer deleted.
        self.collapsed.remove(&id);
        self.status = if removed {
            format!("deleted {label} (1 of {before} on this line)")
        } else {
            // The store had no such comment: another process deleted it, or
            // this one is re-answering a question about a comment that has
            // already gone. Idempotent, and said out loud rather than reported
            // as a deletion that did not happen.
            format!("nothing to delete at {label}, it was already gone")
        };
        self.sync_stack();
        Ok(Action::Continue)
    }

    /// Puts the stack cursor back inside the stack after the stack has changed
    /// under it.
    ///
    /// The sibling of [`App::reset_stack`], which is for when the *selection*
    /// moves: there the cursor should go back to the top, here it should stay
    /// as close as it can to the comment it was on, because a delete is
    /// something the reviewer does *inside* a stack they are working through.
    /// An emptied stack hands the focus back to the diff — a pane with nothing
    /// in it is not somewhere to leave a cursor.
    fn sync_stack(&mut self) {
        match self.stack_len() {
            0 => {
                self.comment_index = 0;
                if self.focus == Focus::Stack {
                    self.focus = Focus::Diff;
                }
            }
            total => self.comment_index = self.comment_index.min(total - 1),
        }
    }

    /// Enters [`Mode::Comment`] on an empty buffer, unless there is nothing to
    /// anchor a comment to — better to say so now than to take a typed comment
    /// and drop it at Enter.
    fn begin_comment(&mut self) {
        if self.selected_line().is_none() {
            self.status = "no diff line selected, nothing to comment on".to_owned();
            return;
        }
        self.mode = Mode::Comment;
        self.buffer.clear();
    }

    /// Moves the sidebar selection to `index` and loads that file's diff.
    /// Out-of-range indices are ignored, which is what makes `[` at the top
    /// and `]` at the bottom no-ops rather than errors.
    ///
    /// The file is reopened where it was left, not at its top — see
    /// `line_indices`. The position is re-clamped on the way in because it was
    /// clamped against whatever the diff was when it was written, and a file
    /// visited before its diff was computed has none to have been clamped to.
    fn select_file(&mut self, index: usize) -> Result<()> {
        if index >= self.review.files.len() || index == self.file_index {
            return Ok(());
        }
        self.file_index = index;
        self.load_selected()?;
        self.set_line_index(self.line_index());
        Ok(())
    }

    /// Computes the selected file's diff if it has not been computed yet.
    ///
    /// Both sides are read at their own path and their own commit, so a
    /// rename diffs its base-side source against its head-side target rather
    /// than against a file that does not exist. A side the commit has no plain
    /// file at — an add, a delete, a symlink — reads as absent, which is
    /// exactly what [`diff::compute`] wants for a whole-file change.
    fn load_selected(&mut self) -> Result<()> {
        let Some(file) = self.review.files.get(self.file_index) else {
            return Ok(());
        };
        if self.diffs[self.file_index].is_some() {
            return Ok(());
        }

        let session = &self.review.session;
        let base_commit = session.base_commit.clone();
        let head_commit = session.head_commit.clone();
        let base_path = file.source_path.as_deref().unwrap_or(&file.path).to_owned();
        let head_path = file.path.clone();
        let old = self
            .review
            .repo
            .read_blob(&base_commit, &base_path)
            .with_context(|| format!("could not read {base_path} at the base of the review"))?;
        let new = self
            .review
            .repo
            .read_blob(&head_commit, &head_path)
            .with_context(|| format!("could not read {head_path} at the head of the review"))?;

        let diff = if self.force_fallback {
            diff::compute_with(old.as_deref(), new.as_deref(), &head_path, false)
        } else {
            diff::compute(old.as_deref(), new.as_deref(), &head_path)
        };
        self.diffs[self.file_index] = Some(diff);
        // Parsed from the very blobs the diff was computed from, so the spans a
        // line is painted with describe the text that line came from. Lazy per
        // file, like the diff beside it: a review of a hundred files parses the
        // two the reviewer has opened.
        self.cache_highlights(base_commit, base_path, old.as_deref());
        self.cache_highlights(head_commit, head_path, new.as_deref());
        // The clamp is [`App::select_file`]'s, applied once the diff it clamps
        // against is in place.
        Ok(())
    }

    /// Parses `blob`'s highlight spans under `(commit, path)` unless they are
    /// already there.
    ///
    /// A side the commit has no plain file at — an add's base, a delete's head
    /// — caches nothing, so [`App::highlights`] answers `None` for it and the
    /// renderer draws that side plain. [`Highlights::of`] itself never fails,
    /// including on bytes that are not UTF-8.
    fn cache_highlights(&mut self, commit: String, path: String, blob: Option<&[u8]>) {
        let Some(bytes) = blob else {
            return;
        };
        let key = (commit, path);
        if self.highlights.contains_key(&key) {
            return;
        }
        let highlights = Highlights::of(bytes, &key.1);
        self.highlights.insert(key, highlights);
    }

    /// Where a comment on `line` of the selected file belongs.
    ///
    /// `None` when the line carries no number on the side it belongs to, which
    /// is the same condition [`App::prepare_comment`] refuses to save under —
    /// so a line that cannot be commented on shows no comments either, rather
    /// than borrowing some other line's.
    fn anchor_target(&self, line: &DiffLine) -> Option<AnchorTarget<'_>> {
        let file = self.selected_file()?;
        let session = &self.review.session;
        let side = anchored_side(line.kind);
        let (path, number, commit) = match side {
            Side::Left => (
                file.source_path.as_deref().unwrap_or(&file.path),
                line.left,
                session.base_commit.as_str(),
            ),
            Side::Right => (file.path.as_str(), line.right, session.head_commit.as_str()),
        };
        Some(AnchorTarget {
            side,
            path,
            number: number?,
            commit,
        })
    }

    /// Re-reads the comments from disk.
    ///
    /// Called after every write, so the pane shows what is stored rather than
    /// what this process believes it stored: the store is the authority, and
    /// its upsert may have replaced an entry rather than added one.
    fn reload_comments(&mut self) -> Result<()> {
        self.comments = self
            .review
            .store
            .comments()
            .context("could not re-read the saved comments")?;
        // The browser indexes this vector, so it is clamped where the vector is
        // written: a delete from the browser must leave the cursor on a row
        // rather than one past the end of the list it just shortened.
        self.clamp_browser();
        Ok(())
    }

    fn line_count(&self) -> usize {
        self.selected_diff().map_or(0, |diff| diff.lines.len())
    }

    fn selected_line(&self) -> Option<&DiffLine> {
        self.selected_diff()
            .and_then(|diff| diff.lines.get(self.line_index()))
    }

    /// Saves the typed comment against the selected line, then rewrites the
    /// markdown export.
    ///
    /// Anything that makes the comment unanchorable — an empty body, a diff
    /// with no lines to select at all (a binary file, or difftastic reporting
    /// no semantic change), a diff line with no number on the side it belongs
    /// to — leaves the store untouched and the reason in the status line. A
    /// comment that cannot be placed is never worth storing somewhere
    /// approximate.
    ///
    /// A *suppressed* diff is not on that list, and used to be described as if
    /// it were. Suppression says the difference between the two sides is not
    /// visible in the lines — difftastic's `unchanged`, or the `similar`
    /// fallback's terminator-only change — not that the lines are unreal. The
    /// difftastic case carries no lines, so it is refused by the clause above
    /// and needs no clause of its own; the fallback case carries every line of
    /// the file as `Context`, [`crate::ui`] draws them under a note saying the
    /// difference is elsewhere, and a comment on one of them anchors to a real
    /// line, at a real number, whose text the anchor hashes out of the file
    /// itself. Refusing it would mean refusing a line the reviewer is looking
    /// at.
    fn commit_comment(&mut self) -> Result<()> {
        let comment = match self.prepare_comment()? {
            Ok(comment) => comment,
            Err(reason) => {
                self.status = reason;
                return Ok(());
            }
        };

        self.review
            .store
            .append_comment(&comment)
            .context("could not save the comment")?;
        self.reload_comments()?;
        session::write_markdown(&self.review)?;

        self.status = format!(
            "comment saved at {}:{}",
            comment.anchor.file, comment.anchor.line
        );
        Ok(())
    }

    /// Builds the [`Comment`] the current selection and buffer describe, or —
    /// as the inner `Err` — the sentence to show instead of saving anything.
    ///
    /// The outer [`Result`] is reserved for a repository that could not be
    /// read, which is a real failure rather than a refusal.
    ///
    /// Two of the refusals below cannot be provoked from the keyboard alone.
    ///
    /// "the review covers no change to comment on" needs an empty
    /// `session.changes`. [`session::build`] never produces one —
    /// [`rv_core::vcs::Repository::stack`] returns `EmptyRange` for an empty
    /// range — but [`Review`] is `pub` with `pub` fields, so a caller that
    /// assembles one by hand can, and `rv/tests/app_cases.rs` does exactly that
    /// (`a_review_with_no_changes_refuses_to_attribute_a_comment`). It is a
    /// tested refusal rather than an unreachable branch.
    ///
    /// "this line has no number on the side it belongs to" is the one that
    /// really is unreachable, and is kept as defence in depth: it needs a
    /// [`rv_core::diff::DiffLine`] whose anchored side
    /// carries no number, and every producer in [`rv_core::diff`] numbers the
    /// side it dispatches to: difftastic's paired entries set both sides, an
    /// unpaired lhs is `Removed` with `left`, an unpaired rhs is `Added` with
    /// `right`, `all_added`/`all_removed` number their own side, and the
    /// `similar` fallback's Equal/Delete/Insert each set the side
    /// [`anchored_side`] sends them to.
    ///
    /// The body is stored trimmed: surrounding whitespace is a slip of the
    /// keyboard, and it would otherwise end up in the comment id.
    fn prepare_comment(&self) -> Result<Result<Comment, String>> {
        let body = self.buffer.trim();
        if body.is_empty() {
            return Ok(Err("empty comment, nothing saved".to_owned()));
        }
        let Some(line) = self.selected_line() else {
            return Ok(Err("no diff line selected, nothing saved".to_owned()));
        };
        // What `change_id` on the stored comment actually is, stated plainly
        // because the name invites a stronger reading: the *first change of the
        // reviewed range*, the same one for every comment in the review, and
        // not the change that introduced the line being commented on.
        // `Repository::stack` streams newest first, so for the default
        // `trunk()..@` this is `@` — the working copy, which is usually an
        // empty change.
        //
        // Two things follow, both of them current behaviour rather than
        // problems this function should solve: `markdown::render` orders each
        // section by the comment's index in `session.changes` and prints the id
        // in every anchor marker, so today that ordering key is constant and
        // every marker names the same change; and `comment_id`'s digest gets
        // the same `change_id` from every comment, so the seed's whole
        // discriminating power is the location and the body. Attributing a
        // comment to the change that touched its line is Milestone 2's work
        // (spec §14) and needs per-change diffs, which this milestone does not
        // compute.
        //
        // `commit_id` is *not* taken from that change: it comes from the
        // anchored side, along with the path and the number — see
        // [`AnchorTarget`].
        let Some(change) = self.review.session.changes.first() else {
            return Ok(Err("the review covers no change to comment on".to_owned()));
        };

        let Some(target) = self.anchor_target(line) else {
            return Ok(Err(
                "this line has no number on the side it belongs to".to_owned()
            ));
        };

        // The anchor hashes the line as it stands in the file, not as the diff
        // rendered it, so it resolves against the file's own future text.
        let blob = self
            .review
            .repo
            .read_blob(target.commit, target.path)
            .with_context(|| format!("could not read {} to anchor the comment", target.path))?;
        let text = blob.map(|bytes| String::from_utf8_lossy(&bytes).into_owned());
        let anchor = anchor::create(
            target.path,
            target.side,
            target.number,
            text.as_deref().unwrap_or_default(),
        );

        Ok(Ok(Comment {
            id: comment_id(
                &change.change_id,
                target.path,
                target.side,
                target.number,
                body,
            ),
            change_id: change.change_id.clone(),
            commit_id: target.commit.to_owned(),
            anchor,
            body: body.to_owned(),
            state: CommentState::Open,
            reply: None,
        }))
    }
}

/// Where a comment on one diff line belongs: which side it is anchored to, and
/// the path, line number and commit **on that side**.
///
/// Four values, one function ([`App::anchor_target`]), because they have to
/// agree: the pane labels a line with `number`, the store anchors it at
/// `path`:`number` on `side`, and `commit` is the revision whose blob that text
/// is read and hashed from. Milestone 1 shipped a version where the pane and
/// the anchor each decided the side for themselves and disagreed; the first
/// real review of `rv` then found `commit` deciding separately too, and
/// recording the head for text that only exists on the base. A comment on a
/// removed line whose `commit` names the head points at a revision the quoted
/// text cannot be read back from, which is `commit`'s only job.
struct AnchorTarget<'a> {
    side: Side,
    path: &'a str,
    number: u32,
    commit: &'a str,
}

/// Which side of the diff a comment on a line of this kind belongs to: a
/// removed line only exists on the base side, and everything else — added and
/// context alike — is commented against the head.
///
/// Public because [`crate::ui`] labels each line with the number on the side
/// this returns. A pane that showed one number while the anchor stored another
/// would be lying to the reviewer about what they just commented on.
pub fn anchored_side(kind: LineKind) -> Side {
    match kind {
        LineKind::Removed => Side::Left,
        LineKind::Added | LineKind::Context => Side::Right,
    }
}

/// A comment's id: the first [`ID_CHARS`] hex characters of the blake3 digest
/// of the change, location and body it covers.
///
/// Derived rather than random so that re-typing the same comment on the same
/// line of the same change upserts the entry it already made instead of
/// stacking a duplicate beside it.
///
/// `change_id` is the same string for every comment in a review — it is the
/// range's first change, never the change that touched the line, as
/// [`App::prepare_comment`] spells out — so within one review the location and
/// the body carry the whole of the seed's discriminating power. It stays in
/// because ids outlive the review that made them: `.review/` from another
/// range, keyed by these ids, must not collide with this one's.
///
/// # Why `side` is part of the seed
///
/// The *whole* location has to be in here, and a location is a side as well as
/// a path and a number. difftastic aligns a rewritten line with its counterpart
/// and gives both halves of the pair both numbers, so a rewrite that stays at
/// the same line number (nothing inserted above it) produces a removed line and
/// an added line at, say, `same.rs:2` on the base and head sides respectively.
/// Without the side, one sentence typed on each half — "which of these two is
/// right?" — seeds two identical ids, and
/// [`rv_core::store::Store::append_comment`] upserts by id: the second save
/// silently replaces the first, snapshot and all, under a "comment saved"
/// status line. That is the loss [`ID_CHARS`] argues must never happen, and
/// unlike a digest collision it happens with probability 1.
///
/// The path alone is not enough, even though [`App::prepare_comment`] resolves
/// it per side: the two paths differ only for a rename.
///
/// Adding the side changed every id this function produces. Nothing recomputes
/// an id to find a comment — `comments.json` is keyed by the id it stored,
/// snapshots are filed under it, and `session::fold_replies` matches the id a
/// document's marker carries against the stored one — so a review in progress
/// keeps working across the change: its comments, snapshots and replies all
/// still resolve. The only visible effect is that re-typing a comment saved
/// *before* the change no longer upserts that entry; it appends a second one
/// beside it. A duplicate is recoverable; the loss above is not.
fn comment_id(change_id: &str, path: &str, side: Side, line: u32, body: &str) -> String {
    let side = match side {
        Side::Left => "left",
        Side::Right => "right",
    };
    let seed = format!("{change_id}:{path}:{side}:{line}:{body}");
    let digest = blake3::hash(seed.as_bytes()).to_hex();
    digest[..ID_CHARS].to_owned()
}

/// Makes a panic restore the terminal before it prints.
///
/// The previous hook runs afterwards, so the message and backtrace land on a
/// terminal that has left raw mode and the alternate screen — visible, and on
/// a shell the user can keep using.
fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        ratatui::restore();
        previous(info);
    }));
}
