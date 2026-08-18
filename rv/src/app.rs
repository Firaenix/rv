//! The reviewer's state machine.
//!
//! Everything here but [`run`](App::run) is **terminal-free**: [`App::on_key`]
//! and [`App::on_mouse`] take a crossterm event *value*, change state, read and
//! write `.review/`, and return. That is what lets `rv/tests/app.rs` drive a
//! whole review, comment and all, as an ordinary unit test.
//!
//! **Nothing in this module calls [`Instant::now`] except the event loop.**
//! Everything that ages — alerts, and the frame that draws them — takes the
//! time as an argument, so "the toast is gone after five seconds" is an
//! assertion rather than a sleep. A clock is ambient input, and every state
//! machine here has stayed testable by refusing ambient input.
//!
//! The module is split by what a keystroke *does*: [`mode`] and [`status`] hold
//! the vocabulary, [`bindings`] the one table browse keys dispatch from,
//! [`keys`] the dispatch itself, [`navigate`]/[`sidebar`]/[`stack`] the three
//! cursors, [`comment`]/[`delete`]/[`fold`]/[`anchor`] what happens to a
//! comment, [`mouse`] the pointer, [`alerts`] what went wrong, [`query`] what
//! the renderer reads, and [`run`] the terminal.
//!
//! Design: `docs/superpowers/specs/2026-08-18-rv-viewport-design.md`, and
//! `2026-08-17-rv-storage-model-design.md` for what a comment costs to save.

mod alerts;
mod anchor;
mod bindings;
mod comment;
mod commits;
mod delete;
mod export;
mod fold;
mod keys;
mod mode;
mod mouse;
mod navigate;
mod query;
mod run;
mod settle;
mod sidebar;
mod stack;
mod status;

pub use alerts::Alert;
pub use anchor::anchored_side;
pub use bindings::BINDINGS;
pub use bindings::Binding;
pub use bindings::Group;
pub use mode::Action;
pub use mode::Focus;
pub use mode::Mode;
pub use mode::SidebarTab;

use std::cell::Cell;
use std::collections::HashMap;
use std::collections::HashSet;

use anyhow::Context as _;
use anyhow::Result;
use rv_core::diff;
use rv_core::diff::FileDiff;
use rv_core::diff::LineKind;
use rv_core::highlight::Highlights;
use rv_core::store::Comment;
use rv_core::store::CommentState;

use crate::gradient::Stat;
use crate::layout::Layout;
use crate::layout::Split;
use crate::session::Review;
use crate::statusbar;
use crate::tree::Sort;
use crate::ui;
use status::HELP;

/// One interactive review.
///
/// `diffs` is parallel to `review.files`: `None` means "not computed yet",
/// which is the whole of the lazy-loading scheme (spec §7).
///
/// `comments` is a *read-through copy* of the store, not a cache in front of
/// it: every save goes straight to disk and this vector is refreshed from disk
/// immediately afterwards. It exists because the diff pane has to draw the
/// comments on the line it is drawing, and re-reading `comments.json` once per
/// line per frame is not a thing to do sixty times a second.
pub struct App {
    review: Review,
    diffs: Vec<Option<FileDiff>>,
    comments: Vec<Comment>,
    file_index: usize,
    /// Where the cursor sits in each file, as a **row of that file's plan** —
    /// parallel to `review.files`.
    ///
    /// Rows, not diff lines, and that is the whole of spec §10's fix: a comment
    /// box is several rows tall, so a cursor that moved by diff line stepped
    /// *over* a box rather than through it and the middle rows of a tall one
    /// were in no window at any cursor position. **This is the state;
    /// [`App::line_index`] is derived from it** — the reverse would leave two
    /// cursors to keep in step, which is what caused the defect.
    ///
    /// One position per file, because `[`/`]` is how a reviewer compares two
    /// files and a shared cursor makes every round trip cost them their place.
    cursor_rows: Vec<usize>,
    focus: Focus,
    sidebar_tab: SidebarTab,
    /// Which row of the comment browser the cursor is on: an index into
    /// `comments`, kept in range by `clamp_browser` so that deleting the
    /// comment it was on leaves the cursor on the list.
    browser_index: usize,
    /// Which comment of the selected line's stack the cursor is on, meaningful
    /// only while the focus is [`Focus::Stack`].
    ///
    /// An index rather than an id, because the stack is a list the reviewer
    /// walks with `j`/`k`. Reset whenever the selection moves, so it can never
    /// address a comment on a line the reviewer has left.
    comment_index: usize,
    /// The comments the reviewer has folded away, by id — keyed by id rather
    /// than position so that folding survives a delete, a save, or a walk to
    /// another file and back.
    collapsed: HashSet<String>,
    /// The directory rows of the file list the reviewer has folded away. Kept
    /// apart from `collapsed` because one set holding both would let a comment
    /// id and a path collide.
    collapsed_dirs: HashSet<String>,
    /// Whether the file list is drawn as a directory tree rather than as a flat
    /// list of whole paths.
    tree: bool,
    /// The order the file list's rows are in.
    sort: Sort,
    /// Which **row of the file list** the cursor is on.
    ///
    /// A row rather than a file, because a tree has rows that are not files and
    /// a directory row is what `s` folds. `file_index` stays the *selection*,
    /// and the two are kept in step where either moves.
    sidebar_row: usize,
    /// How many lines each file adds and removes, computed **once** when the
    /// review is opened — parallel to `review.files`.
    ///
    /// The sidebar tints and counts every row from these, so they have to exist
    /// before the first frame; computing them lazily would mean the colours
    /// moved as the reviewer browsed, which is the one thing a change bar must
    /// not do.
    stats: Vec<Stat>,
    /// Whether the status bar draws its separators in ASCII, read from
    /// `RV_ASCII` **once** at startup: the renderer runs on every keystroke and
    /// the environment cannot change under a running process.
    ascii: bool,
    /// How the width is divided between the two panes.
    split: Split,
    /// Whether the `?` keymap is up. While it is, every key but the five it
    /// answers is inert: a reviewer reading about `d` must not discover what it
    /// does by pressing it.
    help_open: bool,
    /// How far the keymap has been scrolled, in rows. Held unclamped, because
    /// only the renderer knows how tall the popup got.
    help_scroll: usize,
    /// How many columns of body text a comment box was drawn with on the last
    /// frame — reported by [`crate::ui::visible`], never decided here.
    ///
    /// How many rows a plan has is a fact about the pane's width, and
    /// `cursor_rows` indexes that plan. A [`Cell`] because [`crate::ui::draw`]
    /// takes `&App` — it must not be able to *decide* anything — and reporting
    /// the width it drew at is a measurement rather than a decision.
    body_width: Cell<usize>,
    /// Highlight spans per `(commit, path)`, parsed once per blob.
    ///
    /// Keyed by the blob rather than the file, because a diff line's colours
    /// come from **its own side**: a removed line only exists at the base
    /// commit, under the base-side path, which for a rename is not the path the
    /// file is listed under.
    highlights: HashMap<(String, String), Highlights>,
    /// The rectangles the last frame was painted with — see [`mouse`], which is
    /// the whole of "one layout, two consumers". It starts at
    /// [`crate::ui::default_layout`] so that a gesture arriving before the
    /// first frame resolves against something plausible.
    painted: Cell<Layout>,
    /// Whether the pointer is holding the divider, so that a drag resizes only
    /// when it began on the handle.
    dragging: bool,
    /// Where the wheel has parked the diff pane's window, as the first row on
    /// screen — or [`None`] when the view is following the cursor.
    ///
    /// **Scrolling is looking; clicking is choosing.** An absolute row rather
    /// than a delta from the cursor: a delta would move the view every time the
    /// selection moved under it, which is the opposite of parking.
    diff_scroll: Option<usize>,
    /// The same for the sidebar's list.
    sidebar_scroll: Option<usize>,
    /// What has gone wrong lately, newest last, and none of it on disk: an
    /// alert is a fact about *this* run, and a failure another reviewer
    /// inherited would be a claim about the present that was never true for
    /// them.
    alerts: Vec<Alert>,
    mode: Mode,
    buffer: String,
    status: String,
    /// Set to skip difftastic for every file in this review — see
    /// [`App::with_fallback_diffs`].
    force_fallback: bool,
    /// The commits view, built the first frame that asks for it.
    ///
    /// A [`std::cell::OnceCell`] because [`crate::ui::draw`] takes `&App`, and
    /// enumerating a stack's changes is a cost to pay once rather than a
    /// decision to make.
    commits: commits::Commits,
}

impl App {
    /// Opens `review` in the reviewer, loading the first file's diff.
    ///
    /// Which diff engine each file goes through is left to [`diff::compute`],
    /// which honours `RV_NO_DIFFT`.
    pub fn new(review: Review) -> Result<Self> {
        Self::open(review, false)
    }

    /// Opens `review` with difftastic bypassed: every file's diff comes from
    /// the `similar` fallback.
    ///
    /// That is the diff a user with no `difft` on `PATH` gets, and the only one
    /// carrying [`LineKind::Context`] lines and a
    /// [`rv_core::diff::DiffSource::Similar`] label — a distinct set of
    /// branches rather than a degraded copy. Per-`App` rather than through
    /// `RV_NO_DIFFT`, which is process-wide and would swap the engine under
    /// every other review in the process.
    pub fn with_fallback_diffs(review: Review) -> Result<Self> {
        Self::open(review, true)
    }

    fn open(review: Review, force_fallback: bool) -> Result<Self> {
        let diffs = vec![None; review.files.len()];
        // Read before the first diff is computed: a reviewer who quit halfway
        // through yesterday opens on the notes they already made.
        let comments = review
            .store
            .comments()
            .context("could not read the saved comments")?;
        let cursor_rows = vec![0; review.files.len()];
        // A comment that is no longer open starts folded: still exactly where
        // the reviewer left it, without competing for the screen with the ones
        // still asking for an answer. Seeded here rather than forced every
        // frame so that `s` can expand one like any other box.
        let collapsed = comments
            .iter()
            .filter(|comment| comment.state != CommentState::Open)
            .map(|comment| comment.id.clone())
            .collect();
        // Before anything is drawn: the sidebar's tint and counts are facts
        // about the whole review. Unreadable blobs are measured as zero *and
        // said out loud*, unstamped — opening a review has no more clock in
        // reach than a key press does.
        let (stats, unreadable) = Self::measure(&review);
        let mut app = Self {
            review,
            diffs,
            comments,
            file_index: 0,
            cursor_rows,
            focus: Focus::Diff,
            sidebar_tab: SidebarTab::Files,
            browser_index: 0,
            comment_index: 0,
            collapsed,
            collapsed_dirs: HashSet::new(),
            tree: false,
            sort: Sort::default(),
            sidebar_row: 0,
            stats,
            ascii: statusbar::ascii_from_env(),
            split: Split::default(),
            help_open: false,
            help_scroll: 0,
            body_width: Cell::new(ui::default_body_width()),
            highlights: HashMap::new(),
            painted: Cell::new(ui::default_layout()),
            dragging: false,
            diff_scroll: None,
            sidebar_scroll: None,
            alerts: Vec::new(),
            mode: Mode::Browse,
            buffer: String::new(),
            status: HELP.to_owned(),
            force_fallback,
            commits: commits::Commits::default(),
        };
        for message in unreadable {
            app.raise(message);
        }
        app.load_selected()?;
        Ok(app)
    }

    /// How many lines every file in the review adds and removes, in sidebar
    /// order, and what could not be read.
    ///
    /// Through [`diff::compute_with`] with difftastic **off**, always. It is a
    /// subprocess per file and this runs over *every* file before the first
    /// frame: a hundred files is a hundred process spawns between the reviewer
    /// pressing enter and seeing anything. The `similar` path is in-process and
    /// its line counts answer the same question about the same two blobs.
    ///
    /// A file whose blobs cannot be read measures zero rather than failing the
    /// whole review, and **says so**: measuring it as zero in silence draws the
    /// row exactly like a file nobody touched.
    fn measure(review: &Review) -> (Vec<Stat>, Vec<String>) {
        let mut unreadable = Vec::new();
        let stats = review
            .files
            .iter()
            .map(|file| {
                let base = file.source_path.as_deref().unwrap_or(&file.path);
                let old = Self::measured_blob(
                    review,
                    &review.session.base_commit,
                    base,
                    "the base",
                    &mut unreadable,
                );
                let new = Self::measured_blob(
                    review,
                    &review.session.head_commit,
                    &file.path,
                    "the head",
                    &mut unreadable,
                );
                let diff = diff::compute_with(old.as_deref(), new.as_deref(), &file.path, false);
                diff.lines
                    .iter()
                    .fold(Stat::default(), |stat, line| match line.kind {
                        LineKind::Added => Stat {
                            added: stat.added.saturating_add(1),
                            ..stat
                        },
                        LineKind::Removed => Stat {
                            removed: stat.removed.saturating_add(1),
                            ..stat
                        },
                        LineKind::Context => stat,
                    })
            })
            .collect();
        (stats, unreadable)
    }

    /// One side's blob for [`App::measure`], with a failure recorded rather
    /// than swallowed.
    ///
    /// A side the commit has no plain file at reads as `Ok(None)` — an add has
    /// no base, a delete has no head — and is not a failure; only an `Err` is.
    fn measured_blob(
        review: &Review,
        commit: &str,
        path: &str,
        end: &str,
        unreadable: &mut Vec<String>,
    ) -> Option<Vec<u8>> {
        match review.repo.read_blob(commit, path) {
            Ok(blob) => blob,
            Err(_) => {
                unreadable.push(format!("could not read {path} at {end} of the review"));
                None
            }
        }
    }
}
