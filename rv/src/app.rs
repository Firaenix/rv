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
mod changes;
mod comment;
mod commits;
mod delete;
mod diffs;
mod enabled;
mod export;
mod fold;
mod keys;
mod measure;
mod mode;
mod mouse;
mod navigate;
mod paint;
mod query;
mod refresh;
mod run;
mod settle;
mod sidebar;
mod stack;
mod status;
mod symbols;
mod zoom;

pub use alerts::Alert;
pub use changes::ChangeInfo;

pub use anchor::anchored_side;
pub use anchor::comment_id;
pub use bindings::BINDINGS;
pub use bindings::Binding;
pub use bindings::Group;
pub use mode::Action;
pub use mode::Context;
pub use mode::DiffEngine;
pub use mode::Focus;
pub use mode::HelpStage;
pub use mode::Mode;
pub use mode::SidebarTab;

use std::cell::Cell;
use std::collections::HashMap;
use std::collections::HashSet;

use anyhow::Context as _;
use anyhow::Result;
use rv_core::diff::FileDiff;
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
    /// Whether a sidebar row's name is tinted by its change's proportion —
    /// green through the seam to red, across the text itself.
    tint: bool,
    /// Where the sidebar is zoomed into, innermost last — see [`zoom`].
    zoom: Vec<zoom::Zoom>,
    /// Whether the sidebar shows the `+n -n` column at all.
    counts: bool,
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
    /// Whether the `?` keymap is up, and at which size. While it is, every key
    /// but the five it answers is inert: a reviewer reading about `d` must not
    /// discover what it does by pressing it.
    help: HelpStage,
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
    /// How many columns of each diff line are scrolled off the pane's left
    /// edge — `H`/`L`, and the horizontal wheel. Reset when another file is
    /// selected: a scroll chosen for one file's long lines is noise on the
    /// next file's short ones.
    diff_hscroll: usize,
    /// The same for the sidebar's rows, surviving tab and file changes: the
    /// names' length is a fact about the review, not about one file.
    sidebar_hscroll: usize,
    /// What has gone wrong lately, newest last, and none of it on disk: an
    /// alert is a fact about *this* run, and a failure another reviewer
    /// inherited would be a claim about the present that was never true for
    /// them.
    alerts: Vec<Alert>,
    mode: Mode,
    buffer: String,
    status: String,
    engine: DiffEngine,
    /// Blobs whose highlight spans are being parsed on another thread, and the
    /// channel they come back on — see [`paint`].
    parsing: HashSet<(String, String)>,
    painter: paint::Painter,
    /// Which commits-view row is selected, and that row's own diff.
    ///
    /// Keyed by the row's pair number rather than by file, because two changes
    /// touching one file are two rows with two diffs — which is the whole reason
    /// the view exists.
    commit_pair: Option<usize>,
    commit_diffs: HashMap<usize, FileDiff>,
    /// The symbols in scope, and which scope they were indexed for.
    ///
    /// Two fields rather than an `Option<(Scope, Index)>` because the index is
    /// handed out by reference and the scope is compared by value on every
    /// press of `n`.
    symbol_index: crate::index::Index,
    indexed_scope: Option<symbols::Scope>,
    /// Files showing the fast diff while difftastic is still being asked, and
    /// files whose structural answer has landed — whatever it was. See [`diffs`].
    refining: HashSet<usize>,
    refined: HashSet<usize>,
    refiner: diffs::Refiner,
    /// Whether `i` has put the change tooltip away, and how far down it is
    /// scrolled.
    info_dismissed: bool,
    info_scroll: usize,
    /// Whether the reviewer has put the sidebar away with `z`.
    ///
    /// What they asked for, not what they get: a terminal narrow enough hides
    /// it regardless, and that decision belongs to [`crate::layout`], which is
    /// the only place that knows how wide the screen is.
    sidebar_hidden: bool,
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
    /// # Why the engine is a parameter and not an environment variable
    ///
    /// `RV_NO_DIFFT` exists and `diff::compute` honours it, but a *test* cannot
    /// use it: `std::env::set_var` is unsafe in edition 2024 precisely because it
    /// races any concurrent `getenv`, and cargo runs integration tests on many
    /// threads. The alternative this replaces is therefore a data race, not an
    /// inconvenience — which is the argument for the parameter, and a stronger one
    /// than the process-wide-and-impolite reasoning that used to stand here.
    pub fn open(review: Review, engine: DiffEngine) -> Result<Self> {
        Self::build(review, engine)
    }

    fn build(review: Review, engine: DiffEngine) -> Result<Self> {
        let diffs = vec![None; review.files.len()];
        // Read before the first diff is computed: a reviewer who quit halfway
        // through yesterday opens on the notes they already made.
        let mut comments = crate::session::in_range(
            &review,
            review
                .store
                .comments()
                .context("could not read the saved comments")?,
        );
        // Derived, never stored: a comment whose anchor no longer resolves is
        // outdated for as long as that stays true and no longer.
        crate::stale::mark_outdated(&review, &mut comments);
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
            tint: true,
            counts: true,
            zoom: Vec::new(),
            sidebar_row: 0,
            stats,
            ascii: statusbar::ascii_from_env(),
            split: Split::default(),
            help: HelpStage::Closed,
            help_scroll: 0,
            body_width: Cell::new(ui::default_body_width()),
            highlights: HashMap::new(),
            painted: Cell::new(ui::default_layout()),
            dragging: false,
            diff_scroll: None,
            sidebar_scroll: None,
            diff_hscroll: 0,
            sidebar_hscroll: 0,
            alerts: Vec::new(),
            mode: Mode::Browse,
            buffer: String::new(),
            status: HELP.to_owned(),
            engine,
            parsing: HashSet::new(),
            painter: paint::Painter::default(),
            commit_pair: None,
            commit_diffs: HashMap::new(),
            symbol_index: crate::index::Index::default(),
            indexed_scope: None,
            refining: HashSet::new(),
            refined: HashSet::new(),
            refiner: diffs::Refiner::default(),
            info_dismissed: false,
            info_scroll: 0,
            sidebar_hidden: false,
            commits: commits::Commits::default(),
        };
        for message in unreadable {
            app.raise(message);
        }
        app.load_selected()?;
        Ok(app)
    }
}
