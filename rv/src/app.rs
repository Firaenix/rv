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
mod build;
mod changes;
mod comment;
mod comments;
mod commit_diff;
mod commits;
mod delete;
mod diffs;
mod diffview;
mod editor;
mod enabled;
mod focus;
mod fold;
mod hunks;
pub mod keymap;
mod keys;
mod measure;
mod merges;
mod mode;
mod mouse;
mod navigate;
mod paging;
mod paint;
mod query;
mod refresh;
mod regroup;
mod run;
mod settle;
mod sidebar;
mod stack;
mod status;
mod symbols;
mod viewside;
mod watch;
mod zoom;

pub use alerts::Alert;
pub use changes::ChangeInfo;
pub use sidebar::BrowserRow;
pub use sidebar::Suppression;

pub use anchor::anchored_side;
pub use anchor::comment_id;
pub use bindings::BINDINGS;
pub use bindings::Binding;
pub use bindings::Group;
pub use bindings::Leader;
pub use keymap::Keymap;
pub use mode::Action;
pub use mode::Context;
pub use mode::DiffEngine;
pub use mode::Focus;
pub use mode::HelpStage;
pub use mode::Mode;
pub use mode::SidebarTab;
pub use viewside::ViewSide;

use std::cell::Cell;
use std::collections::HashMap;
use std::collections::HashSet;

use rv_core::diff::FileDiff;
use rv_core::highlight::Highlights;
use rv_core::store::Comment;

use crate::gradient::Stat;
use crate::layout::Layout;
use crate::layout::Split;
use crate::session::Review;
use crate::tree::Sort;

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
    /// The `(old, new)` blob bytes each `diffs` entry was computed from,
    /// parallel to it — `None` exactly where `diffs`' entry is `None`.
    ///
    /// Kept so [`super::merges`]' background worker can synthesize full-file
    /// context without a second blob read, per the branch-reviewer design's
    /// lazy-blob rule (spec §7): the bytes are already in hand the moment the
    /// diff is, and re-reading them for a second purpose is exactly the eager
    /// read that rule forbids.
    blobs: Vec<Option<(Vec<u8>, Vec<u8>)>>,
    /// The full-file-context merge for each file, parallel to `blobs` — see
    /// [`super::merges::MergeState`]. Populated by the same lifecycle that
    /// fills `blobs`: a fresh load slots [`MergeState::Pending`] and kicks
    /// the background merger; the result lands as [`MergeState::Ready`] or
    /// [`MergeState::Bailed`]. Read through [`App::displayed_lines`], which
    /// falls back to the diff's own changed-only lines while the merge is
    /// pending — the reviewer sees the shipped-before-this-feature view
    /// during the sub-second the worker is running, not a blank pane.
    ///
    /// See `docs/superpowers/specs/2026-08-21-rv-full-file-context-design.md`
    /// Appendix A (the caching architecture the shipped feature omitted).
    merges: Vec<Option<merges::MergeState>>,
    /// Whether the reviewer wants full-file context (spec §5, walked back to
    /// let a toggle stand — see [`crate::app::context`]). Reviewer default is
    /// `true`, and `f` flips it. Not persisted: this is a display preference
    /// scoped to one run, not a review artefact.
    full_context: bool,
    /// The merge worker: single-slot, latest-wins, mirroring
    /// [`diffs::Refiner`]. See [`super::merges`].
    merger: merges::Merger,
    comments: Vec<Comment>,
    /// What each comment's anchor has done since it was written, by id —
    /// surveyed alongside `comments` and refreshed with them, because every
    /// entry costs a blob read and the box that shows it is drawn per frame.
    drift: HashMap<String, crate::stale::Drift>,
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
    /// Which **row** of the comment browser the cursor is on.
    ///
    /// A row and not a comment: the browser draws a heading above each file's
    /// comments, so the two numbers differ. **This is the state; the comment is
    /// derived from it** by [`App::browsed_comment`] — the same ruling
    /// `cursor_rows` records for the diff pane, and for the same reason, that
    /// two cursors kept in step is the defect rather than the fix.
    ///
    /// Kept on the list, and off a heading, by `clamp_browser`, so that
    /// deleting the comment it was on leaves it somewhere `d` still means
    /// something.
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
    /// The built sidebar rows, memoized against a fingerprint of what shapes
    /// them. Rebuilding the whole tree on every `nodes()` call — and it is
    /// reached many times per keystroke and per frame — is what made scrolling
    /// the commits list crawl. A refresh builds a fresh `App`, so the cache
    /// never outlives the files it was built from.
    nodes_cache: std::cell::RefCell<Option<(u64, Vec<crate::tree::Node>)>>,
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
    /// The leader whose which-key submenu is open, waiting for its second key,
    /// or [`None`] when the next key is read from the top level. A leader is
    /// pressed, its children are shown, and the following key either runs one of
    /// them or (on `Esc`, or a key none of them claim) cancels back to browsing.
    keymap: keymap::Keymap,
    /// The op-head watch behind auto-refresh — see [`watch`].
    watch: watch::Watch,
    pending_leader: Option<bindings::Leader>,
    /// Whether the diff pane groups each hunk's removals before its additions,
    /// the way a unified diff prints — rather than difftastic's interleaving of
    /// the two sides. Session-only, `v g` flips it. See [`crate::app::regroup`].
    grouped: bool,
    /// Which side of the change the diff pane shows: both (the default), the
    /// base alone, or the head alone. Session-only, `v b` cycles it. See
    /// [`crate::app::viewside`].
    view_side: viewside::ViewSide,
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
    /// When the event loop first saw the current `status`, and the text it
    /// stamped — see [`alerts`], whose no-clock discipline this mirrors: the
    /// key handlers that write `status` have no clock in reach, so the loop
    /// stamps a changed status on its next pass and the bar drops it roughly
    /// eight seconds later (viewport spec §9: a status expires).
    status_stamp: Option<std::time::Instant>,
    status_seen: String,
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
    /// The `(old, new)` bytes each `commit_diffs` entry was computed from,
    /// keyed the same way — see `blobs`' doc comment for why this exists.
    commit_blobs: HashMap<usize, (Vec<u8>, Vec<u8>)>,
    /// The symbols in scope, and which scope they were indexed for.
    ///
    /// Two fields rather than an `Option<(Scope, Index)>` because the index is
    /// handed out by reference and the scope is compared by value on every
    /// press of `n`.
    symbol_index: crate::index::Index,
    indexed_scope: Option<symbols::Scope>,
    /// Diffs showing the fast fallback while difftastic is still being asked,
    /// and diffs whose structural answer has landed — whatever it was. Keyed by
    /// [`diffs::Target`] so the file list and the commits view share one worker
    /// and one set of flags. See [`diffs`].
    refining: HashSet<diffs::Target>,
    refined: HashSet<diffs::Target>,
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
    /// What `v` resolved to open, waiting for [`run`] to leave the terminal and
    /// run it. `None` at every other moment: [`App::take_edit`] is the only
    /// reader and it takes.
    pending_edit: Option<editor::Edit>,
}
