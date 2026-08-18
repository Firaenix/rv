//! Parameterized cases and properties for [`rv::app`] and [`rv::ui`].
//!
//! Additive to `rv/tests/app.rs`, which pins a handful of exact end-to-end
//! behaviours. This file covers the *contract*: the documented keybinding
//! tables (cross-checked against README's two tables), the state invariants
//! that must survive any key sequence at all, the byte-identity of a comment
//! from keystrokes to `comments.json` and back out through the markdown
//! export, and the total-ness of [`rv::ui::draw`] at every terminal size.
//!
//! # Why the fixtures are shared, and how cases stay independent
//!
//! Building a fixture costs ~200 ms of `jj` process time and `App::new` costs
//! ~30 ms of `difft`, so a fresh `App` per proptest case would put this file
//! well over its wall-clock budget. Instead each property owns **one** `App`
//! and drives the proptest runner by hand (see [`run_cases`]), calling
//! [`rewind`] before every case. `rewind` uses only the public keyboard API —
//! `Esc`, then `[` off the left edge, then `k` off the top — so it lands on the
//! same state a fresh `App` starts in (file 0, line 0, `Browse`, empty
//! buffer), and clears `.review/comments.json` where the store matters. That
//! keeps shrinking sound: a shrunk key sequence replays from the same state
//! the failing one did.
//!
//! Fixtures that no test in this file writes comments into are shared through
//! a `OnceLock` across the whole binary; anything that saves a comment gets a
//! fixture of its own, because integration tests run in parallel threads and
//! `.review/` is process-wide state.
//!
//! # Which diff engine each fixture is reviewed through
//!
//! Most fixtures go through difftastic, and the properties whose oracles depend
//! on difftastic's *pairing* of a rewritten line with its counterpart say so out
//! loud with [`assert_difftastic`] — `rv_core::diff::compute` degrades to
//! `similar` when `difft` is missing or `RV_NO_DIFFT` is exported, and every
//! fixture guard in this file survives that swap, so without the assertion the
//! suite would report green while covering different branches than every doc
//! comment here describes.
//!
//! [`Fixture::fallback`] is the deliberate other side of that: it is reviewed
//! through [`App::with_fallback_diffs`], which is the only way this file
//! produces a [`LineKind::Context`] line or a `DiffSource::Similar` label —
//! the diff every user with no `difft` on `PATH` actually sees. Per-`App`
//! rather than by setting `RV_NO_DIFFT`, which is process-wide and would swap
//! the engine under the other tests running in parallel threads.

use std::cell::RefCell;
use std::fs;
use std::path::Path;
use std::process::Command;
use std::sync::OnceLock;
use std::time::SystemTime;

use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyModifiers;
use proptest::prelude::*;
use proptest::test_runner::TestCaseResult;
use proptest::test_runner::TestRunner;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::layout::Rect;
use ratatui::style::Color;
use rstest::fixture;
use rstest::rstest;
use rv::app::Action;
use rv::app::App;
use rv::app::Focus;
use rv::app::Mode;
use rv::app::SidebarTab;
use rv::app::anchored_side;
use rv::layout::Chrome;
use rv::layout::Split;
use rv::layout::layout;
use rv::session;
use rv::ui;
use rv_core::anchor;
use rv_core::diff;
use rv_core::diff::DiffLine;
use rv_core::diff::DiffSource;
use rv_core::diff::FileDiff;
use rv_core::diff::LineKind;
use rv_core::model::Side;
use rv_core::store::Comment;
use rv_core::store::CommentState;
use rv_core::store::Store;
use tempfile::TempDir;

/// The status line `App::new` starts on. Copied from `app.rs`'s private `HELP`
/// on purpose: if the help text changes, these cases should be re-read rather
/// than silently following it.
const HELP: &str = "↓↑ line  [/] file  c comment  enter stack  d delete  s fold  ? help  q quit";

/// What `Enter`, `d` and `s` report on a line carrying no comments. Copied from
/// `app.rs`'s private constant for the same reason [`HELP`] is.
const NO_COMMENTS: &str = "no comments on this line";

/// `alpha.rs` at the base of [`Fixture::multi`]. Every line is distinct, so a
/// diff line can be located in the frame unambiguously.
const ALPHA_BASE: &str = "\
fn alpha() {
    let a01 = 1;
    let a02 = 2;
    let a03 = 3;
    let a04 = 4;
    let a05 = 5;
}
";

/// `alpha.rs` at the head: a header line inserted *above* the rewritten one,
/// so a changed line sits at a different number on each side.
const ALPHA_HEAD: &str = "\
// alpha header
fn alpha() {
    let a01 = 1;
    let a02 = 22;
    let a03 = 3;
    let a04 = 4;
    let a05 = 5;
}
";

/// `a.rs` at the base of [`Fixture::renamed`].
///
/// Long enough that the head side below still counts as the same file to jj's
/// copy detection: two rewritten lines out of ten leaves the similarity high.
/// A shorter pair is reported as a delete plus an add, which has no left side
/// to anchor anything to.
const RENAME_BASE: &str = "\
fn a() {
    let x = 1;
    let y = 2;
    let z = 3;
    let w = 4;
    let v = 5;
    let u = 6;
    let t = 7;
    let s = 8;
}
";

/// `b.rs` at the head of [`Fixture::renamed`]: renamed, one line inserted at
/// the top and two rewritten, so several diff lines carry a left number that
/// differs from their right one.
const RENAME_HEAD: &str = "\
// header
fn a() {
    let x = 42;
    let y = 2;
    let z = 33;
    let w = 4;
    let v = 5;
    let u = 6;
    let t = 7;
    let s = 8;
}
";

/// `same.rs` at the base of [`Fixture::collisions`]: the line that gets
/// rewritten sits at line 2, and nothing is inserted above it.
///
/// Verified with `DFT_UNSTABLE=yes difft --display json` that difftastic pairs
/// the two versions of line 2 as `lhs 1 rhs 1`, so *both* halves of the pair
/// come back with `left == right == Some(2)`: same file, same number, opposite
/// sides. That is the one shape in which a side-blind comment id loses a
/// comment.
const SAME_BASE: &str = "\
fn same() {
    let b = 1;
}
";

/// `same.rs` at the head of [`Fixture::collisions`]: line 2 rewritten in place.
const SAME_HEAD: &str = "\
fn same() {
    let X = 1;
}
";

/// `ctx.rs` at the base of [`Fixture::fallback`]: two lines that survive
/// unchanged around one that does not, so the `similar` fallback has context to
/// emit.
const CTX_BASE: &str = "\
fn ctx() {
    let keep1 = 1;
    let change = 2;
    let keep2 = 3;
}
";

/// `ctx.rs` at the head of [`Fixture::fallback`]: one line rewritten and one
/// appended, so the fallback diff carries `Context`, `Removed` and `Added`
/// lines at once.
const CTX_HEAD: &str = "\
fn ctx() {
    let keep1 = 1;
    let change = 22;
    let keep2 = 3;
    let added = 4;
}
";

/// `eol.rs` at the base of [`Fixture::terminator`]: three lines, the last of
/// them *not* terminated.
const EOL_BASE: &str = "fn eol() {\n    let e = 1;\n}";

/// `eol.rs` at the head: the same three lines, now with a final newline. The
/// files differ, and no line of either differs from its counterpart.
const EOL_HEAD: &str = "fn eol() {\n    let e = 1;\n}\n";

/// `crlf.txt` at the base of [`Fixture::terminator`]: CRLF terminators.
const CRLF_BASE: &str = "alpha\r\nbeta\r\n";

/// `crlf.txt` at the head: the same two lines, LF-terminated. The other shape
/// of a terminator-only change, and the one that touches every line at once.
const CRLF_HEAD: &str = "alpha\nbeta\n";

/// The sentence the diff pane shows for a diff `rv_core::diff` suppressed.
/// Both of the pane's suppressed branches — the note above a suppressed diff's
/// lines, and the whole body of one that has none — start with it, so a test
/// can look for it without pinning either wording.
const SUPPRESSED: &str = "no semantic change";

/// How many lines [`Fixture::multi`]'s `long.rs` has: comfortably more than
/// any pane height the rendering properties sweep, so the diff pane has to
/// scroll.
const LONG_LINES: usize = 40;

/// The terminal sizes that have historically broken ratatui layout arithmetic:
/// a single cell, a single row, a single column, and the ones where a bar
/// asking for three rows meets a frame that has one or two.
///
/// Spelled out rather than sampled wherever they are swept, because these are
/// *the* cases and a uniform draw over a plausible range visits them almost
/// never.
const PATHOLOGICAL: [(u16, u16); 8] = [
    (1, 1),
    (80, 1),
    (1, 40),
    (2, 5),
    (5, 2),
    (3, 3),
    (40, 2),
    (40, 3),
];

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

struct Fixture {
    tempdir: TempDir,
}

impl Fixture {
    fn init() -> Self {
        let fixture = Self {
            tempdir: tempfile::tempdir().expect("create temp dir"),
        };
        fixture.jj(&["git", "init", "--colocate"]);
        fixture
    }

    /// Five changed files chosen to cover the branches the diff pane and the
    /// comment refusal take *on difftastic's output*: a text file changed on
    /// both sides, a binary file, an added empty file, a long added file, and a
    /// file whose only change is indentation (which difftastic reports as no
    /// semantic change).
    ///
    /// Not every branch: difftastic emits only changed lines, so no diff here
    /// carries a [`LineKind::Context`] line, and none is labelled
    /// `DiffSource::Similar`. [`Fixture::fallback`] owns those.
    fn multi() -> Self {
        let fixture = Self::init();
        fixture.write("alpha.rs", ALPHA_BASE);
        fixture.write_bytes("bin.dat", b"\x00\x01\x02 base binary\x00");
        fixture.write("reindent.rs", "fn reindent() {\n    let r = 1;\n}\n");
        fixture.write("stable.txt", "never changes\n");
        fixture.jj(&["describe", "-m", "base change"]);
        fixture.jj(&["new"]);

        fixture.write("alpha.rs", ALPHA_HEAD);
        fixture.write_bytes("bin.dat", b"\x00\x09\x08 head binary\x00");
        fixture.write("reindent.rs", "fn reindent() {\n\t\tlet r = 1;\n}\n");
        fixture.write("blank.txt", "");
        fixture.write("long.rs", &long_source());
        fixture.jj(&["describe", "-m", "head change"]);
        fixture.jj(&["new"]);
        fixture
    }

    /// Two text files whose diffs between them cover both ways two comments can
    /// land on the same line number: `same.rs` rewrites line 2 *in place*, so
    /// difftastic's pair carries `left == right` and the two halves differ only
    /// by side; `alpha.rs` inserts a line above the rewrite, so its two halves
    /// carry different numbers.
    ///
    /// The fixture for [`distinct_comments_are_never_lost_to_each_other`] and
    /// [`both_halves_of_a_same_position_rewrite_keep_their_own_comment`]: both
    /// write comments, so neither may share a workspace with anything else.
    fn collisions() -> Self {
        let fixture = Self::init();
        fixture.write("same.rs", SAME_BASE);
        fixture.write("alpha.rs", ALPHA_BASE);
        fixture.jj(&["describe", "-m", "base change"]);
        fixture.jj(&["new"]);

        fixture.write("same.rs", SAME_HEAD);
        fixture.write("alpha.rs", ALPHA_HEAD);
        fixture.jj(&["describe", "-m", "head change"]);
        fixture.jj(&["new"]);
        fixture
    }

    /// One text file with unchanged lines around a rewritten one, reviewed
    /// through the `similar` fallback rather than difftastic (see
    /// [`Fixture::fallback_app`]).
    ///
    /// This is the only fixture that produces [`LineKind::Context`] lines and a
    /// [`DiffSource::Similar`] diff — the diff every user without `difft` on
    /// `PATH` sees, and the one `RV_NO_DIFFT=1` forces.
    fn fallback() -> Self {
        let fixture = Self::init();
        fixture.write("ctx.rs", CTX_BASE);
        fixture.jj(&["describe", "-m", "base change"]);
        fixture.jj(&["new"]);

        fixture.write("ctx.rs", CTX_HEAD);
        fixture.jj(&["describe", "-m", "head change"]);
        fixture.jj(&["new"]);
        fixture
    }

    /// Two files whose only change is in their line *terminators*: `eol.rs`
    /// gains a final newline, `crlf.txt` goes from CRLF to LF.
    ///
    /// The one input on which the two diff engines disagree about shape while
    /// agreeing about meaning. Both call it suppressed — but difftastic says so
    /// with `status: "unchanged"` and emits no chunks, while the `similar`
    /// fallback renders both sides to the same lines and reports them as
    /// `Context` (see `rv_core::diff::similar_diff`). So the fallback's diff is
    /// suppressed *and* has lines, which is the case the pane, `j`/`k` and the
    /// comment anchor have to agree about.
    ///
    /// Reviewed through [`Fixture::fallback_app`] for the fallback half and
    /// [`Fixture::app`] for the difftastic contrast, from the very same
    /// workspace.
    fn terminator() -> Self {
        let fixture = Self::init();
        fixture.write("crlf.txt", CRLF_BASE);
        fixture.write("eol.rs", EOL_BASE);
        fixture.jj(&["describe", "-m", "base change"]);
        fixture.jj(&["new"]);

        fixture.write("crlf.txt", CRLF_HEAD);
        fixture.write("eol.rs", EOL_HEAD);
        fixture.jj(&["describe", "-m", "head change"]);
        fixture.jj(&["new"]);
        fixture
    }

    /// A rename plus two rewritten lines: the fixture for everything about
    /// which *side* a comment anchors to.
    fn renamed() -> Self {
        let fixture = Self::init();
        fixture.write("a.rs", RENAME_BASE);
        fixture.jj(&["describe", "-m", "base change"]);
        fixture.jj(&["new"]);

        fs::remove_file(fixture.root().join("a.rs")).expect("remove a.rs");
        fixture.write("b.rs", RENAME_HEAD);
        fixture.jj(&["describe", "-m", "rename and edit"]);
        fixture.jj(&["new"]);
        fixture
    }

    /// A non-empty stack that changes no files at all: `@--..@` spans two
    /// described-but-empty changes, so `session::build` succeeds (the range is
    /// not empty) while `App` opens with zero files. That is the state
    /// `ui::draw`'s "no changed files in this range" branch exists for, and
    /// the one where every accessor on `App` has to answer `None`.
    fn no_files() -> Self {
        let fixture = Self::init();
        fixture.write("only.rs", "fn only() {}\n");
        fixture.jj(&["describe", "-m", "base change"]);
        fixture.jj(&["new"]);
        fixture.jj(&["describe", "-m", "an empty change"]);
        fixture.jj(&["new"]);
        fixture
    }

    fn root(&self) -> &Path {
        self.tempdir.path()
    }

    fn jj(&self, args: &[&str]) -> String {
        let output = Command::new("jj")
            .args(args)
            .current_dir(self.root())
            .env("JJ_CONFIG", "/dev/null")
            .env("JJ_USER", "rv-test")
            .env("JJ_EMAIL", "rv-test@localhost")
            .output()
            .expect("run jj");
        assert!(
            output.status.success(),
            "jj {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).expect("jj stdout is utf-8")
    }

    fn write(&self, rel: &str, contents: &str) {
        self.write_bytes(rel, contents.as_bytes());
    }

    fn write_bytes(&self, rel: &str, contents: &[u8]) {
        let path = self.root().join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parent dir");
        }
        fs::write(&path, contents).expect("write file");
    }

    /// The reviewer over `base..@`, where `base` defaults to `@--` — the
    /// change below the one every fixture above puts its head state in.
    fn app(&self) -> App {
        let review = session::build(self.root(), Some("@--"), None).expect("build the review");
        App::new(review).expect("open the reviewer")
    }

    /// The reviewer over the same range, with difftastic bypassed: every diff
    /// comes from the `similar` fallback.
    ///
    /// Per-`App` rather than `RV_NO_DIFFT`, which is process-wide: integration
    /// tests run in parallel threads, so setting that variable here would
    /// silently swap the diff engine under every other property in this binary.
    fn fallback_app(&self) -> App {
        let review = session::build(self.root(), Some("@--"), None).expect("build the review");
        App::with_fallback_diffs(review).expect("open the reviewer")
    }

    /// A store handle that shares nothing with the app's own, so an assertion
    /// through it is about what reached the disk.
    fn store(&self) -> Store {
        Store::open(self.root()).expect("open the store")
    }

    fn comments(&self) -> Vec<Comment> {
        self.store().comments().expect("read comments.json")
    }

    fn markdown(&self) -> String {
        fs::read_to_string(self.root().join(".review/REVIEW-FEEDBACK.md")).unwrap_or_default()
    }

    /// Forgets every stored comment, so the next case starts from an empty
    /// store. Snapshots are left behind: nothing reads one except by an id
    /// `comments.json` still lists.
    fn clear_comments(&self) {
        let _ = fs::remove_file(self.root().join(".review/comments.json"));
        let _ = fs::remove_file(self.root().join(".review/REVIEW-FEEDBACK.md"));
    }
}

fn long_source() -> String {
    (1..=LONG_LINES)
        .map(|index| format!("let long{index:03} = {index};\n"))
        .collect()
}

/// The fixtures no test in this file writes a comment into, shared across the
/// whole binary so their `jj` cost is paid once.
fn shared_multi() -> &'static Fixture {
    static MULTI: OnceLock<Fixture> = OnceLock::new();
    MULTI.get_or_init(Fixture::multi)
}

fn shared_no_files() -> &'static Fixture {
    static NO_FILES: OnceLock<Fixture> = OnceLock::new();
    NO_FILES.get_or_init(Fixture::no_files)
}

/// The keybinding tables' own workspace, separate from [`shared_multi`].
///
/// The tables assert that no case of theirs put a comment in the store, and
/// that assertion has to be about a store nothing else is touching:
/// `the_buffer_is_exactly_what_was_typed`, `drawing_never_panics_at_any_size`
/// and `drawing_survives_pathological_sizes` all press `c` and type into
/// [`shared_multi`] from other test threads. They never press `Enter` today, so
/// the old shared assertion held — but by luck of what those properties happen
/// not to do, and a failure would have named the wrong test.
fn shared_tables() -> &'static Fixture {
    static TABLES: OnceLock<Fixture> = OnceLock::new();
    TABLES.get_or_init(Fixture::multi)
}

/// A read-only reviewer over the keybinding tables' fixture. Not `#[once]`:
/// each case wants its own `App`, and building one is cheap next to building
/// the workspace.
#[fixture]
fn multi_app() -> App {
    shared_tables().app()
}

/// The comment browser's own workspace: [`Fixture::multi`] with two comments
/// already saved into `alpha.rs`, so that the browser has rows to move between.
///
/// Shared, and read-only from here on: no case in the browser's table saves or
/// deletes anything, which is what lets them share one `jj` workspace. The
/// table asserts that at the end of every case.
fn shared_browser() -> &'static Fixture {
    static BROWSER: OnceLock<Fixture> = OnceLock::new();
    BROWSER.get_or_init(|| {
        let fixture = Fixture::multi();
        let mut app = fixture.app();
        assert_eq!(app.selected_file().expect("a file").path, "alpha.rs");
        for (downs, body) in [(0, "first finding"), (1, "second finding")] {
            press_n(&mut app, KeyCode::Char('j'), downs);
            press(&mut app, KeyCode::Char('c'));
            type_text(&mut app, body);
            press(&mut app, KeyCode::Enter);
        }
        assert_eq!(fixture.comments().len(), 2, "{:?}", fixture.comments());
        fixture
    })
}

/// A reviewer over [`shared_browser`], already in the comment browser with the
/// sidebar focused — reached the way a reviewer reaches it.
#[fixture]
fn browser_app() -> App {
    let mut app = shared_browser().app();
    press(&mut app, KeyCode::Tab);
    press(&mut app, KeyCode::Left);
    assert_eq!(app.sidebar_tab(), SidebarTab::Comments);
    assert_eq!(app.focus(), Focus::Sidebar);
    assert_eq!(app.browser_index(), 0);
    app
}

// ---------------------------------------------------------------------------
// Driving the app
// ---------------------------------------------------------------------------

/// Returns the app to the state `App::new` leaves it in, using nothing but
/// keys the reviewer has.
///
/// `Esc` is a no-op in `Browse` and discards in `Comment`, so it is safe from
/// any state; `[` past the left edge and `k` past the top are no-ops by
/// design, which is what makes this a total reset rather than a guess.
///
/// Two things make this longer than the state it restores looks:
///
/// * **The focus is reset first.** `j` and `k` move the *file* selection while
///   the sidebar has focus, so a walk that ended on the sidebar would leave the
///   line loop below pressing `k` at the file list and never touching the line
///   at all. `Left` twice then `Right` lands on `Focus::Diff` from any of the
///   three, since `Left` leads out of every focus and `Right` stops at the
///   diff.
/// * **Every file's line is reset, not just the selected one.** The highlight
///   is remembered per file, so returning to file 0 restores file 0's position
///   and leaves the others where the last case left them — which would make the
///   next case's `]` land somewhere it did not ask for.
/// * **The sidebar's tab and the comment browser's own cursor are reset too.**
///   `Tab` is bound from every focus, so a random walk leaves the left column
///   listing whichever of the two it last flipped to, with its cursor wherever
///   `j` left it — and `j` then means something different in the next case
///   than it did in this one.
fn rewind(app: &mut App) {
    app.on_key(KeyCode::Esc).expect("leave comment mode");
    app.on_key(KeyCode::Left).expect("out of the stack");
    app.on_key(KeyCode::Left).expect("onto the sidebar");
    if app.sidebar_tab() != SidebarTab::Comments {
        app.on_key(KeyCode::Tab).expect("onto the comment browser");
    }
    for _ in 0..=app.comments().len() {
        // Bounded for the same reason the line loop below is: this presses the
        // very key the browser's clamp is about.
        app.on_key(KeyCode::Up).expect("first comment");
    }
    app.on_key(KeyCode::Tab).expect("back to the file list");
    app.on_key(KeyCode::Right).expect("back onto the diff");
    for _ in 0..=app.files().len() {
        app.on_key(KeyCode::Char('[')).expect("first file");
    }
    for _ in 0..app.files().len() {
        // Bounded, not `while cursor_row() > 0`: this reset presses the very
        // key `line_navigation_clamps_at_both_ends` exists to pin, and an
        // unbounded loop would turn a regression in the `k`/`Up` binding into a
        // silent hang of the whole binary instead of a failed assertion below.
        //
        // The bound is the **plan's** rows rather than the diff's lines,
        // because that is what `k` walks: a comment box is rows, so a file
        // carrying comments has more of them than it has lines, and a bound of
        // one per line would stop partway up such a file.
        for _ in 0..=app.plan().rows.len() {
            app.on_key(KeyCode::Char('k')).expect("first line");
        }
        app.on_key(KeyCode::Char(']')).expect("next file");
    }
    for _ in 0..=app.files().len() {
        app.on_key(KeyCode::Char('[')).expect("first file");
    }
    assert_eq!(app.focus(), Focus::Diff);
    assert_eq!(app.file_index(), 0);
    assert_eq!(app.line_index(), 0);
    assert_eq!(app.mode(), Mode::Browse);
    assert_eq!(app.buffer(), "");
    assert_eq!(app.sidebar_tab(), SidebarTab::Files);
    assert_eq!(app.browser_index(), 0);
    // The `Esc` above is also what puts the `?` popup away — a generated `?`
    // would otherwise leave every key of the next case inert.
    assert!(!app.help_open());
    // The split is deliberately *not* restored: no key the reviewer has moves
    // it back to a named ratio, and nothing downstream of `rewind` asserts on
    // the geometry. A property that starts doing so has to reset it itself.
}

fn press(app: &mut App, key: KeyCode) -> Action {
    app.on_key(key).expect("handle a key")
}

fn press_n(app: &mut App, key: KeyCode, times: usize) {
    for _ in 0..times {
        press(app, key);
    }
}

/// Walks the cursor down onto diff line `index` with `j`, the way a reviewer
/// would, and returns the number of presses it took.
///
/// **Not `press_n(j, index)`.** `j` walks the diff pane's *rows*, and a comment
/// box is rows, so how many presses reach line `index` depends on what is
/// anchored to the lines above it — see `rv/src/app.rs`'s `cursor_rows` for why
/// the cursor moves by row rather than by line. What every case below means is
/// "put the cursor on that line", and this is that sentence said once.
///
/// The first row a line owns is its own diff row, so this always lands on the
/// line rather than inside one of its boxes.
///
/// A file with fewer lines than `index` stops at its last one rather than
/// failing, which is where holding `j` down actually leaves a reviewer and what
/// `press_n(j, n)` used to do. Every caller that means "and it must be exactly
/// that line" asserts it on the next line of its own body.
///
/// Bounded rather than `while`, for the reason [`rewind`]'s loops are: a
/// regression in the `j` binding should fail an assertion here rather than hang
/// the whole binary. The bound is the plan's own length, which is the space `j`
/// walks.
fn walk_to_line(app: &mut App, index: usize) -> usize {
    let bound = app.plan().rows.len();
    for pressed in 0..=bound {
        if app.line_index() == index {
            return pressed;
        }
        let row = app.cursor_row();
        press(app, KeyCode::Char('j'));
        if app.cursor_row() == row {
            return pressed;
        }
    }
    panic!(
        "`j` never reached diff line {index} in {bound} presses: stopped on {}",
        app.line_index()
    );
}

fn type_text(app: &mut App, text: &str) {
    for character in text.chars() {
        press(app, KeyCode::Char(character));
    }
}

/// The lines of the selected file's diff.
fn lines(app: &App) -> Vec<DiffLine> {
    app.selected_diff()
        .map(|diff| diff.lines.clone())
        .unwrap_or_default()
}

/// Rewinds and then walks the sidebar to `path` with `]`, the way a reviewer
/// would.
fn select_path(app: &mut App, path: &str) {
    rewind(app);
    let index = app
        .files()
        .iter()
        .position(|file| file.path == path)
        .unwrap_or_else(|| panic!("{path} is not in the review: {:?}", app.files()));
    press_n(app, KeyCode::Char(']'), index);
    assert_eq!(
        app.selected_file().expect("a file is selected").path,
        path,
        "walking to {path} landed elsewhere"
    );
}

/// A side as a sortable, printable tag, so an oracle keyed on the whole
/// location can be sorted and compared ([`Side`] is deliberately not `Ord`).
fn side_tag(side: Side) -> &'static str {
    match side {
        Side::Left => "left",
        Side::Right => "right",
    }
}

/// The anchored-side number of the selected line: what the pane prints, what
/// the status line reports, and what the anchor stores.
fn anchored_number(line: &DiffLine) -> Option<u32> {
    match anchored_side(line.kind) {
        Side::Left => line.left,
        Side::Right => line.right,
    }
}

/// Fails the *setup* of a property whose oracle assumes difftastic produced the
/// diff.
///
/// `rv_core::diff::compute` silently falls back to `similar` when `difft` is
/// missing or `RV_NO_DIFFT` is exported, which changes which branches a
/// property covers (and, for a paired rewrite, the numbers it reasons about)
/// without breaking any of the fixture guards. A property that means to test
/// difftastic's pairing should say so out loud rather than pass while testing
/// something else. [`Fixture::fallback_app`] is the deliberate other side of
/// this coin.
fn assert_difftastic(app: &App) {
    let diff = app.selected_diff().expect("a loaded diff");
    assert!(
        matches!(diff.source, DiffSource::Difftastic { .. }),
        "{} was diffed by {:?}, not difftastic — is difft on PATH, or is RV_NO_DIFFT set? \
         this property's oracle assumes difftastic's line pairing",
        diff.path,
        diff.source
    );
}

/// Records which branches of a property's claim the generated cases actually
/// reached, so a property whose interesting half was never sampled fails
/// loudly instead of passing vacuously.
///
/// Several properties below are conditionals ("if a line is selected then …,
/// otherwise …"). Sampling can leave one arm unvisited, and an unvisited arm
/// is an assertion that never ran — exactly the kind of protection that is
/// advertised but not provided. [`Coverage::assert_all`] is the receipt.
struct Coverage {
    names: Vec<&'static str>,
    hits: RefCell<Vec<usize>>,
}

impl Coverage {
    fn new(names: &[&'static str]) -> Self {
        Self {
            names: names.to_vec(),
            hits: RefCell::new(vec![0; names.len()]),
        }
    }

    fn hit(&self, branch: usize) {
        self.hits.borrow_mut()[branch] += 1;
    }

    fn assert_all(&self) {
        let hits = self.hits.borrow();
        for (name, count) in self.names.iter().zip(hits.iter()) {
            assert!(
                *count > 0,
                "no generated case reached {name:?}, so that half of the property never ran \
                 (branch counts: {:?})",
                self.names.iter().zip(hits.iter()).collect::<Vec<_>>()
            );
        }
    }
}

/// Runs `test` over `strategy` with an explicit runner, so the caller can keep
/// a single `App` alive across cases (see the module docs). Panics with
/// proptest's own shrunk-counterexample report on failure.
fn run_cases<S: Strategy>(cases: u32, strategy: S, test: impl Fn(S::Value) -> TestCaseResult) {
    let config = ProptestConfig {
        cases,
        max_shrink_iters: 4096,
        ..ProptestConfig::default()
    };
    let mut runner = TestRunner::new(config);
    if let Err(error) = runner.run(&strategy, test) {
        panic!("{error}");
    }
}

// ---------------------------------------------------------------------------
// Strategies
// ---------------------------------------------------------------------------

/// Every key the reviewer might see, weighted so a random walk actually
/// navigates instead of drowning in inert keys.
fn any_key() -> impl Strategy<Value = KeyCode> {
    prop_oneof![
        18 => prop_oneof![
            Just(KeyCode::Char('j')),
            Just(KeyCode::Char('k')),
            Just(KeyCode::Char(']')),
            Just(KeyCode::Char('[')),
            Just(KeyCode::Char('c')),
            Just(KeyCode::Char('q')),
            // The comment keys, and the one that answers `d`'s question: a
            // random walk that could not press them would leave deleting and
            // collapsing out of every fuzzed invariant below.
            Just(KeyCode::Char('d')),
            Just(KeyCode::Char('y')),
            Just(KeyCode::Char('s')),
            Just(KeyCode::Down),
            Just(KeyCode::Up),
            Just(KeyCode::Enter),
            Just(KeyCode::Esc),
            Just(KeyCode::Backspace),
        ],
        6 => any::<char>().prop_map(KeyCode::Char),
        2 => (1u8..=20).prop_map(KeyCode::F),
        4 => prop_oneof![
            Just(KeyCode::Left),
            Just(KeyCode::Right),
            Just(KeyCode::Home),
            Just(KeyCode::End),
            Just(KeyCode::PageUp),
            Just(KeyCode::PageDown),
            Just(KeyCode::Tab),
            Just(KeyCode::BackTab),
            Just(KeyCode::Delete),
            Just(KeyCode::Insert),
            Just(KeyCode::Null),
            Just(KeyCode::CapsLock),
            Just(KeyCode::NumLock),
            Just(KeyCode::PrintScreen),
            Just(KeyCode::Pause),
            Just(KeyCode::Menu),
            Just(KeyCode::KeypadBegin),
            Just(KeyCode::Modifier(
                crossterm::event::ModifierKeyCode::LeftControl
            )),
            Just(KeyCode::Media(crossterm::event::MediaKeyCode::Play)),
        ],
    ]
}

/// Comment bodies a reviewer might plausibly type, plus every markdown and
/// `rv`-protocol marker that the export has to survive verbatim.
///
/// No `'\n'`: `Enter` commits, so a newline is not typeable, and generating
/// one would test a state the keyboard cannot reach.
fn any_body() -> impl Strategy<Value = String> {
    let piece = prop_oneof![
        5 => "[ -~]{0,8}",
        2 => prop_oneof![
            Just("**bold**".to_owned()),
            Just("`code`".to_owned()),
            Just("```".to_owned()),
            Just("~~~".to_owned()),
            Just("### 1. heading".to_owned()),
            Just("## Open (1)".to_owned()),
            Just("**Reply:** not a real reply".to_owned()),
            Just("**Comment:** not a real comment".to_owned()),
            Just("<!-- rv:anchor id=deadbeef -->".to_owned()),
            Just("<details><summary>x</summary>".to_owned()),
        ],
        2 => prop_oneof![
            Just("héllo wörld".to_owned()),
            Just("日本語のテキスト".to_owned()),
            Just("🎉🙈 emoji".to_owned()),
            Just("عربى".to_owned()),
            Just("a\u{0301}combining".to_owned()),
        ],
        1 => prop_oneof![
            Just(" ".to_owned()),
            Just("\t".to_owned()),
            Just("\r".to_owned()),
            Just("\u{a0}".to_owned()),
            Just("\u{2028}".to_owned()),
        ],
    ];
    prop::collection::vec(piece, 0..4).prop_map(|parts| parts.concat())
}

// ---------------------------------------------------------------------------
// The documented keybinding tables
// ---------------------------------------------------------------------------

/// Every row of README's **Browsing** table, plus the keys it deliberately
/// does not bind.
///
/// Cross-checked against `app.rs::BINDINGS`, which is now the *only* thing
/// `on_key_browse` dispatches from: `Down`/`j`, `Up`/`k`, `Left`/`h`,
/// `Right`/`l`, `]`, `[`, `c`, `d`, `s`, `Tab`, `Enter`, `Esc`, `<`, `>`, `?`,
/// `q` — every one of which has a row below, and the rows after them are keys
/// the table deliberately leaves inert. The arrow is the binding and the vim
/// key its alias, so each of the four movement rows appears twice: a pair that
/// stopped agreeing would be two keys doing different things under one heading. A key that reached the handler without a row in
/// `BINDINGS` would fail one of the `unbound_*` rows here; a row in `BINDINGS`
/// that reached nothing would fail its own row.
///
/// README's table carries one row more than this one: `Ctrl+C`, which
/// `on_key_event` answers before the mode is dispatched at all and which the
/// page lists beside the rest because a reviewer looking for the way out does
/// not care which function answers them. It is pinned in `rv/tests/app.rs`
/// (`ctrl_c_quits_instead_of_opening_a_comment`), where a `KeyEvent` with
/// modifiers can be built; every row here is a bare `KeyCode`.
///
/// That README table is itself held to this key set by
/// `rv/tests/app.rs::the_readme_documents_every_browse_binding`, in both
/// directions, so a binding cannot ship undocumented and a row cannot outlive
/// its key.
///
/// The start state is a fresh reviewer on `alpha.rs` (five-plus diff lines,
/// first of five files) with the diff focused and **no comments anywhere in the
/// review**, so every direction has somewhere to go except `k`/`Up` and `Left`,
/// which are checked at their clamp — and the four comment keys take their
/// empty-line branch. What each of them does on a line that *has* comments is
/// pinned end-to-end in `rv/tests/app.rs`, which is where a stack can be built
/// by typing one; that is also where `d`'s confirmation lives, so no row here
/// can leave the reviewer in `Mode::ConfirmDelete`.
///
/// The focus column is what makes the movement rows mean anything now that
/// there is more than one pane: `j` moves the line *because the diff has
/// focus*, and every row here says which pane the key left the cursor in. The
/// status column is the other half of that: a key that refuses has to say so,
/// and a key that navigates has to leave the help text alone.
#[rstest]
#[case::next_line_letter(KeyCode::Char('j'), Action::Continue, Mode::Browse, Focus::Diff, (0, 1), None)]
#[case::next_line_arrow(KeyCode::Down, Action::Continue, Mode::Browse, Focus::Diff, (0, 1), None)]
#[case::previous_line_letter(KeyCode::Char('k'), Action::Continue, Mode::Browse, Focus::Diff, (0, 0), None)]
#[case::previous_line_arrow(KeyCode::Up, Action::Continue, Mode::Browse, Focus::Diff, (0, 0), None)]
#[case::focus_sidebar_arrow(KeyCode::Left, Action::Continue, Mode::Browse, Focus::Sidebar, (0, 0), None)]
#[case::focus_sidebar_letter(KeyCode::Char('h'), Action::Continue, Mode::Browse, Focus::Sidebar, (0, 0), None)]
#[case::focus_diff_arrow(KeyCode::Right, Action::Continue, Mode::Browse, Focus::Diff, (0, 0), None)]
#[case::focus_diff_letter(KeyCode::Char('l'), Action::Continue, Mode::Browse, Focus::Diff, (0, 0), None)]
#[case::next_file(KeyCode::Char(']'), Action::Continue, Mode::Browse, Focus::Diff, (1, 0), None)]
#[case::previous_file(KeyCode::Char('['), Action::Continue, Mode::Browse, Focus::Diff, (0, 0), None)]
#[case::comment(KeyCode::Char('c'), Action::Continue, Mode::Comment, Focus::Diff, (0, 0), None)]
#[case::quit(KeyCode::Char('q'), Action::Quit, Mode::Browse, Focus::Diff, (0, 0), None)]
// The three comment keys, on a line with no comments on it.
#[case::enter_an_empty_stack(KeyCode::Enter, Action::Continue, Mode::Browse, Focus::Diff, (0, 0), Some(NO_COMMENTS))]
#[case::escape_outside_a_stack(KeyCode::Esc, Action::Continue, Mode::Browse, Focus::Diff, (0, 0), None)]
#[case::delete_nothing(KeyCode::Char('d'), Action::Continue, Mode::Browse, Focus::Diff, (0, 0), Some(NO_COMMENTS))]
#[case::collapse_nothing(KeyCode::Char('s'), Action::Continue, Mode::Browse, Focus::Diff, (0, 0), Some(NO_COMMENTS))]
// `Tab` changes what the left column *lists* and nothing else: not the focus,
// not the selection, and not the status line — which is where the reviewer
// reads the rest of the keymap, and is not a place for a navigation key to
// announce itself. Which tab it left behind is asserted in the body.
#[case::switch_sidebar_tab(KeyCode::Tab, Action::Continue, Mode::Browse, Focus::Diff, (0, 0), None)]
// The three view keys. None of them says anything in the status line: they are
// about how the screen is arranged, and the bar is where the reviewer reads the
// rest of the keymap. What each of them actually moved is asserted in the body.
#[case::narrower_sidebar(KeyCode::Char('<'), Action::Continue, Mode::Browse, Focus::Diff, (0, 0), None)]
#[case::wider_sidebar(KeyCode::Char('>'), Action::Continue, Mode::Browse, Focus::Diff, (0, 0), None)]
#[case::open_the_help(KeyCode::Char('?'), Action::Continue, Mode::Browse, Focus::Diff, (0, 0), None)]
// Not in the table, and therefore inert.
#[case::unbound_letter(KeyCode::Char('x'), Action::Continue, Mode::Browse, Focus::Diff, (0, 0), None)]
#[case::unbound_uppercase(KeyCode::Char('J'), Action::Continue, Mode::Browse, Focus::Diff, (0, 0), None)]
#[case::unbound_backspace(KeyCode::Backspace, Action::Continue, Mode::Browse, Focus::Diff, (0, 0), None)]
#[case::unbound_backtab(KeyCode::BackTab, Action::Continue, Mode::Browse, Focus::Diff, (0, 0), None)]
#[case::unbound_function(KeyCode::F(1), Action::Continue, Mode::Browse, Focus::Diff, (0, 0), None)]
#[case::unbound_page_down(KeyCode::PageDown, Action::Continue, Mode::Browse, Focus::Diff, (0, 0), None)]
#[case::unbound_home(KeyCode::Home, Action::Continue, Mode::Browse, Focus::Diff, (0, 0), None)]
#[case::unbound_delete(KeyCode::Delete, Action::Continue, Mode::Browse, Focus::Diff, (0, 0), None)]
fn browse_keybindings(
    mut multi_app: App,
    #[case] key: KeyCode,
    #[case] action: Action,
    #[case] mode: Mode,
    #[case] focus: Focus,
    // The selection the key should leave behind, as `(file, line)`: one
    // column rather than two, so a row still reads as a row.
    #[case] selection: (usize, usize),
    #[case] status: Option<&str>,
) {
    let app = &mut multi_app;
    assert!(
        app.files().len() >= 3,
        "the fixture lost files: {:?}",
        app.files()
    );
    assert!(
        lines(app).len() > 2,
        "alpha.rs has too few diff lines to navigate"
    );
    assert_eq!(app.focus(), Focus::Diff, "a fresh reviewer reads the diff");
    assert!(
        shared_tables().comments().is_empty(),
        "the tables' fixture has comments in it, so the three comment keys \
         would take a branch these rows do not describe"
    );

    assert_eq!(app.on_key(key).expect("handle the key"), action);
    assert_eq!(app.mode(), mode);
    assert_eq!(app.focus(), focus);
    assert_eq!((app.file_index(), app.line_index()), selection);
    // Navigating never writes the status line: the help text is what a reviewer
    // reads while they move around, and `c` on a commentable line has nothing
    // to report. Only a refusal speaks.
    assert_eq!(app.status(), status.unwrap_or(HELP));
    // Whatever the key did, it did not start a comment body behind the
    // reviewer's back.
    if mode == Mode::Browse {
        assert_eq!(app.buffer(), "");
    }
    assert_eq!(
        app.comment_index(),
        0,
        "no browsing key moves the stack cursor off the top"
    );
    // Exactly one key in the table changes what the left column lists.
    assert_eq!(
        app.sidebar_tab(),
        if key == KeyCode::Tab {
            SidebarTab::Comments
        } else {
            SidebarTab::Files
        },
        "{key:?} left the sidebar listing the wrong thing"
    );
    // ...and exactly one raises the keymap.
    assert_eq!(
        app.help_open(),
        key == KeyCode::Char('?'),
        "{key:?} left the help in the wrong state"
    );
    // ...and exactly two move the divider, in the two directions their glyphs
    // point. Asserted as a direction rather than a number, so the size of one
    // nudge stays `app.rs`'s business.
    let ratio = app.split().ratio();
    match key {
        KeyCode::Char('>') => assert!(ratio > Split::DEFAULT, "> did not widen the sidebar"),
        KeyCode::Char('<') => assert!(ratio < Split::DEFAULT, "< did not narrow the sidebar"),
        _ => assert_eq!(ratio, Split::DEFAULT, "{key:?} moved the divider"),
    }
}

/// README draws the reviewer as an ASCII mock-up, status bar and all, and that
/// bar is the keymap a reader meets *first* — before either table, and in the
/// one place on the page that claims to be a picture of the running program.
///
/// So it is held to the real one rather than to a list of keys: [`HELP`] is
/// asserted equal to `App::status()` on a fresh reviewer by every row of
/// [`browse_keybindings`] above, and asserted to appear in the page here, which
/// chains the drawing to the program through the constant. The previous wave
/// changed `HELP` and left the mock-up showing the old bar, noted that nothing
/// tested it, and left it to this task; a mock-up that has drifted teaches a
/// keymap the binary does not have, which is worse than no picture at all.
///
/// Substring rather than a whole line, because the mock-up wraps the bar in the
/// box-drawing characters that make it a picture. What is pinned is the bar's
/// text, not the frame drawn around it.
#[test]
fn the_readme_mockup_draws_the_status_bar_the_reviewer_starts_on() {
    let readme = fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../README.md"))
        .expect("read README.md");
    assert!(
        readme.contains(HELP),
        "README's mock-up of the reviewer shows a status bar that is not the \
         one `App::new` starts on ({HELP:?}), so the first keymap a reader sees \
         is not this binary's"
    );
}

/// Every key, from inside the sidebar's **Comments** tab — the one focus whose
/// keys mean something different from everywhere else, since `j`/`k` walk the
/// review's comments there rather than its files or its lines.
///
/// The start state is the browser, focused, on the first of two comments, both
/// of them on `alpha.rs` (the first file). Nothing here saves or deletes: `d`
/// is checked as far as its question, because what answering it does is pinned
/// end-to-end in `rv/tests/app.rs` and doing it here would empty the shared
/// fixture under the other cases.
#[rstest]
#[case::next_comment_letter(
    KeyCode::Char('j'),
    Action::Continue,
    Mode::Browse,
    Focus::Sidebar,
    SidebarTab::Comments,
    (1, 0)
)]
#[case::next_comment_arrow(
    KeyCode::Down,
    Action::Continue,
    Mode::Browse,
    Focus::Sidebar,
    SidebarTab::Comments,
    (1, 0)
)]
#[case::previous_comment_letter(
    KeyCode::Char('k'),
    Action::Continue,
    Mode::Browse,
    Focus::Sidebar,
    SidebarTab::Comments,
    (0, 0)
)]
#[case::previous_comment_arrow(
    KeyCode::Up,
    Action::Continue,
    Mode::Browse,
    Focus::Sidebar,
    SidebarTab::Comments,
    (0, 0)
)]
// `Enter` jumps to the code, which hands the focus to the diff.
#[case::jump(
    KeyCode::Enter,
    Action::Continue,
    Mode::Browse,
    Focus::Diff,
    SidebarTab::Comments,
    (0, 0)
)]
// `d` asks about the *browsed* comment, and stays in the browser to be answered.
#[case::delete_asks(KeyCode::Char('d'), Action::Continue, Mode::ConfirmDelete { id: String::new(), label: String::new() }, Focus::Sidebar, SidebarTab::Comments, (0, 0))]
#[case::back_to_files(
    KeyCode::Tab,
    Action::Continue,
    Mode::Browse,
    Focus::Sidebar,
    SidebarTab::Files,
    (0, 0)
)]
#[case::nothing_further_left(
    KeyCode::Left,
    Action::Continue,
    Mode::Browse,
    Focus::Sidebar,
    SidebarTab::Comments,
    (0, 0)
)]
#[case::out_to_the_diff(
    KeyCode::Right,
    Action::Continue,
    Mode::Browse,
    Focus::Diff,
    SidebarTab::Comments,
    (0, 0)
)]
// File navigation still means files, from here as from everywhere.
#[case::next_file(
    KeyCode::Char(']'),
    Action::Continue,
    Mode::Browse,
    Focus::Sidebar,
    SidebarTab::Comments,
    (0, 1)
)]
#[case::previous_file(
    KeyCode::Char('['),
    Action::Continue,
    Mode::Browse,
    Focus::Sidebar,
    SidebarTab::Comments,
    (0, 0)
)]
#[case::comment(
    KeyCode::Char('c'),
    Action::Continue,
    Mode::Comment,
    Focus::Sidebar,
    SidebarTab::Comments,
    (0, 0)
)]
#[case::quit(
    KeyCode::Char('q'),
    Action::Quit,
    Mode::Browse,
    Focus::Sidebar,
    SidebarTab::Comments,
    (0, 0)
)]
#[case::escape_is_inert(
    KeyCode::Esc,
    Action::Continue,
    Mode::Browse,
    Focus::Sidebar,
    SidebarTab::Comments,
    (0, 0)
)]
#[case::unbound_function(
    KeyCode::F(1),
    Action::Continue,
    Mode::Browse,
    Focus::Sidebar,
    SidebarTab::Comments,
    (0, 0)
)]
#[case::unbound_home(
    KeyCode::Home,
    Action::Continue,
    Mode::Browse,
    Focus::Sidebar,
    SidebarTab::Comments,
    (0, 0)
)]
#[case::unbound_backspace(
    KeyCode::Backspace,
    Action::Continue,
    Mode::Browse,
    Focus::Sidebar,
    SidebarTab::Comments,
    (0, 0)
)]
fn comment_browser_keybindings(
    mut browser_app: App,
    #[case] key: KeyCode,
    #[case] action: Action,
    #[case] mode: Mode,
    #[case] focus: Focus,
    #[case] tab: SidebarTab,
    // The selection the key should leave behind, as `(browsed row, file)`: one
    // column rather than two, so a row still reads as a row.
    #[case] selection: (usize, usize),
) {
    let app = &mut browser_app;
    assert_eq!(app.comments().len(), 2, "the browser has nothing to walk");

    assert_eq!(app.on_key(key).expect("handle the key"), action);
    // `ConfirmDelete` carries the id it is about, which no row can spell out;
    // the rows say *which* mode, and `rv/tests/app.rs` says which comment.
    assert_eq!(
        std::mem::discriminant(&app.mode()),
        std::mem::discriminant(&mode),
        "{key:?} left the reviewer in {:?}",
        app.mode()
    );
    assert_eq!(
        app.focus(),
        focus,
        "{key:?} left the cursor in the wrong pane"
    );
    assert_eq!(app.sidebar_tab(), tab);
    assert_eq!(
        (app.browser_index(), app.file_index()),
        selection,
        "{key:?} left the browser or the file list somewhere else"
    );
    assert_eq!(
        shared_browser().comments().len(),
        2,
        "{key:?} wrote to the browser's shared fixture"
    );
}

/// Every row of README's **Typing a comment** table, plus the keys it does not
/// bind. Start state: `c` pressed on `alpha.rs`'s first diff line, empty
/// buffer.
///
/// Cross-checked against `app.rs::on_key_comment`: `Esc`, `Backspace`,
/// `Enter`, `Char(_)`, and nothing else — which matches README's four rows
/// ("any character" being `Char`). No case here saves anything, so they may
/// share the read-only fixture: `Enter` on an empty buffer is a refusal.
#[rstest]
#[case::append_letter(KeyCode::Char('a'), Mode::Comment, "a", None)]
#[case::append_space(KeyCode::Char(' '), Mode::Comment, " ", None)]
#[case::append_bracket(KeyCode::Char(']'), Mode::Comment, "]", None)]
#[case::append_q_does_not_quit(KeyCode::Char('q'), Mode::Comment, "q", None)]
#[case::append_unicode(KeyCode::Char('日'), Mode::Comment, "日", None)]
#[case::backspace_on_empty(KeyCode::Backspace, Mode::Comment, "", None)]
#[case::escape_discards(KeyCode::Esc, Mode::Browse, "", Some("comment discarded"))]
#[case::enter_on_empty(KeyCode::Enter, Mode::Browse, "", Some("empty comment, nothing saved"))]
// Not in the table: a comment is a single line of text, so nothing else moves.
#[case::unbound_tab(KeyCode::Tab, Mode::Comment, "", None)]
#[case::unbound_down(KeyCode::Down, Mode::Comment, "", None)]
#[case::unbound_left(KeyCode::Left, Mode::Comment, "", None)]
#[case::unbound_delete(KeyCode::Delete, Mode::Comment, "", None)]
#[case::unbound_function(KeyCode::F(4), Mode::Comment, "", None)]
#[case::unbound_home(KeyCode::Home, Mode::Comment, "", None)]
fn comment_keybindings(
    mut multi_app: App,
    #[case] key: KeyCode,
    #[case] mode: Mode,
    #[case] buffer: &str,
    #[case] status: Option<&str>,
) {
    let app = &mut multi_app;
    assert_eq!(press(app, KeyCode::Char('c')), Action::Continue);
    assert_eq!(
        app.mode(),
        Mode::Comment,
        "the fixture's first line is not commentable"
    );

    // Nothing typed while a comment is open ever ends the program: the whole
    // point of the mode is that `q` is a letter here.
    assert_eq!(app.on_key(key).expect("handle the key"), Action::Continue);
    assert_eq!(app.mode(), mode);
    assert_eq!(app.buffer(), buffer);
    assert_eq!(app.status(), status.unwrap_or(HELP));
    assert_eq!(
        app.focus(),
        Focus::Diff,
        "typing moved the cursor to another pane"
    );
    assert!(
        shared_tables().comments().is_empty(),
        "a keybinding case saved a comment into the tables' fixture"
    );
}

/// `on_key_event` is a thin gate in front of `on_key`: it answers Ctrl+C
/// itself and hands every other key on by its code alone.
///
/// The gate exists because the terminal is in raw mode, where no SIGINT is
/// raised on the reviewer's behalf — and where `Char('c')` with CONTROL held is
/// indistinguishable from a typed `c` once the modifiers are dropped, which is
/// what used to make the universal abort open the comment box.
#[rstest]
#[case::ctrl_c_quits(KeyCode::Char('c'), KeyModifiers::CONTROL, Action::Quit, Mode::Browse)]
#[case::ctrl_shift_c_quits(
    KeyCode::Char('c'),
    KeyModifiers::CONTROL.union(KeyModifiers::SHIFT),
    Action::Quit,
    Mode::Browse
)]
#[case::plain_c_comments(
    KeyCode::Char('c'),
    KeyModifiers::NONE,
    Action::Continue,
    Mode::Comment
)]
#[case::alt_c_comments(KeyCode::Char('c'), KeyModifiers::ALT, Action::Continue, Mode::Comment)]
#[case::ctrl_q_is_still_q(KeyCode::Char('q'), KeyModifiers::CONTROL, Action::Quit, Mode::Browse)]
#[case::ctrl_x_is_still_inert(
    KeyCode::Char('x'),
    KeyModifiers::CONTROL,
    Action::Continue,
    Mode::Browse
)]
fn modified_keys_reach_the_state_machine_by_their_code(
    mut multi_app: App,
    #[case] code: KeyCode,
    #[case] modifiers: KeyModifiers,
    #[case] action: Action,
    #[case] mode: Mode,
) {
    let app = &mut multi_app;
    assert_eq!(
        app.on_key_event(KeyEvent::new(code, modifiers))
            .expect("handle the key"),
        action
    );
    assert_eq!(app.mode(), mode);
    assert!(
        shared_tables().comments().is_empty(),
        "a modifier case saved a comment into the tables' fixture"
    );
}

// ---------------------------------------------------------------------------
// Totality and state invariants under fuzz
// ---------------------------------------------------------------------------

/// Every state invariant `App` has, checked after every key of an arbitrary
/// sequence — unbound keys, arbitrary `char`s, function, media and modifier
/// keys included.
///
/// The invariants, none of which any keystroke may break:
///
/// 1. `on_key` never returns `Err` and never panics.
/// 2. `line_index` indexes the selected diff's lines, or is `0` when that diff
///    has none.
/// 3. The comment buffer is empty whenever the mode is `Browse`.
/// 4. A selected file always has a loaded diff (the lazy-load invariant
///    `ui::draw`'s "no diff loaded" branch documents as unreachable), that diff
///    is *that file's* — the `diffs` vector stays parallel to `files` — and the
///    sidebar selects the file at `file_index`.
/// 5. The cursor is a **row of the plan it indexes**, and `line_index` is the
///    line that owns that row. The cursor is the state and the line is derived
///    from it (see `rv/src/app.rs`'s `cursor_rows`), so a cursor that has
///    fallen off the end of a plan something shortened under it — a fold, a
///    delete — is a reviewer whose selection and whose scroll position have
///    stopped describing the same place.
///
/// Four, not the five this used to advertise. `selected_file()` is
/// `self.review.files.get(self.file_index)`, so "the sidebar selects
/// `paths[file_index]`" is true by the definition of `Vec::get` once
/// `file_index` is in range, and cannot distinguish one implementation of `App`
/// from another; it is folded into invariant 4 as a consistency check on this
/// test's own `paths` snapshot rather than billed as a property of the app. The
/// in-range check on `file_index` that guards it stays — deleting
/// `select_file`'s bound check is what it exists to catch.
#[test]
fn state_invariants_survive_any_key_sequence() {
    let fixture = Fixture::multi();
    let app = RefCell::new(fixture.app());
    let paths: Vec<String> = app
        .borrow()
        .files()
        .iter()
        .map(|file| file.path.clone())
        .collect();
    assert!(paths.len() >= 3, "the fixture lost files: {paths:?}");

    run_cases(48, prop::collection::vec(any_key(), 0..24), |keys| {
        fixture.clear_comments();
        rewind(&mut app.borrow_mut());
        for (step, key) in keys.iter().enumerate() {
            let app = &mut *app.borrow_mut();
            // Invariant 1.
            app.on_key(*key)
                .map_err(|error| TestCaseError::fail(format!("key {step} {key:?}: {error}")))?;

            // The bound check `select_file` owes the invariants below.
            prop_assert!(
                app.file_index() < paths.len(),
                "after {key:?} at step {step}: file_index {} out of range for {} files",
                app.file_index(),
                paths.len()
            );
            // Invariant 4.
            let selected = app.selected_file().map(|file| file.path.clone());
            prop_assert_eq!(
                selected.as_deref(),
                Some(paths[app.file_index()].as_str()),
                "after {:?} at step {}: the sidebar selects nothing",
                key,
                step
            );
            let diff = app.selected_diff().ok_or_else(|| {
                TestCaseError::fail(format!(
                    "after {key:?} at step {step}: {} is selected with no diff loaded",
                    paths[app.file_index()]
                ))
            })?;
            prop_assert_eq!(
                &diff.path,
                &paths[app.file_index()],
                "after {:?} at step {}: the loaded diff belongs to another file",
                key,
                step
            );
            // Invariant 2.
            let total = diff.lines.len();
            if total == 0 {
                prop_assert_eq!(
                    app.line_index(),
                    0,
                    "after {:?} at step {}: a line is highlighted in an empty diff",
                    key,
                    step
                );
            } else {
                prop_assert!(
                    app.line_index() < total,
                    "after {key:?} at step {step}: line_index {} out of range for {total} lines",
                    app.line_index()
                );
            }
            // Invariant 5.
            let plan = app.plan();
            if plan.rows.is_empty() {
                prop_assert_eq!(
                    app.cursor_row(),
                    0,
                    "after {:?} at step {}: the cursor is off an empty plan",
                    key,
                    step
                );
            } else {
                prop_assert_eq!(
                    plan.line_of_row(app.cursor_row()),
                    Some(app.line_index()),
                    "after {:?} at step {}: the cursor is on row {} of a {}-row plan",
                    key,
                    step,
                    app.cursor_row(),
                    plan.rows.len()
                );
            }

            // Invariant 3.
            if app.mode() == Mode::Browse {
                prop_assert_eq!(
                    app.buffer(),
                    "",
                    "after {:?} at step {}: a comment body outlived comment mode",
                    key,
                    step
                );
            }
        }
        Ok(())
    });
}

/// `Quit` is returned for exactly one key in exactly one mode.
///
/// The `Comment` half is the one that matters to a reviewer: `q` is a letter
/// in a sentence, and a reviewer typing "queries the cache" must not lose the
/// review to it. The `Browse` half says `q` always works, whatever else the
/// app is in the middle of.
#[test]
fn quit_is_exactly_q_in_browse_mode() {
    let fixture = Fixture::multi();
    let app = RefCell::new(fixture.app());
    // `q` is pressed unconditionally rather than sampled: leaving it to the
    // key strategy meant the `Comment` half of the claim — the half a reviewer
    // notices — was only reached in some runs. See `Coverage`.
    let seen = Coverage::new(&["q while browsing", "q while typing"]);

    // `c` is weighted up in the prefix so the `Comment` arm is reached in most
    // cases rather than a handful: `Coverage` below is a hard assertion, and a
    // rarely-sampled arm makes it a flaky one.
    let prefix_key = prop_oneof![3 => any_key(), 1 => Just(KeyCode::Char('c'))];
    run_cases(
        48,
        (prop::collection::vec(prefix_key, 0..16), any_key()),
        |(prefix, other)| {
            fixture.clear_comments();
            let app = &mut *app.borrow_mut();
            rewind(app);
            for key in &prefix {
                app.on_key(*key).expect("handle a prefix key");
            }

            let mode = app.mode();
            // `q` closes the `?` popup rather than quitting, because quitting
            // from a help screen surprises the reviewer least sure what the
            // keys do — so "browsing" is not on its own enough to expect
            // `Quit`, and the prefix can raise the popup with a generated `?`.
            let browsing = mode == Mode::Browse && !app.help_open();
            seen.hit(if mode == Mode::Browse { 0 } else { 1 });
            let action = app
                .on_key(KeyCode::Char('q'))
                .map_err(|error| TestCaseError::fail(error.to_string()))?;
            prop_assert_eq!(
                action == Action::Quit,
                browsing,
                "q in {:?} returned {:?}",
                mode,
                action
            );
            if mode == Mode::Comment {
                prop_assert!(
                    app.buffer().ends_with('q'),
                    "q did not reach the comment buffer {:?}",
                    app.buffer()
                );
            }

            // ...and no other key returns `Quit` in either mode.
            let mode = app.mode();
            let browsing = mode == Mode::Browse && !app.help_open();
            let action = app
                .on_key(other)
                .map_err(|error| TestCaseError::fail(error.to_string()))?;
            prop_assert_eq!(
                action == Action::Quit,
                browsing && other == KeyCode::Char('q'),
                "{:?} in {:?} returned {:?}",
                other,
                mode,
                action
            );
            Ok(())
        },
    );
    seen.assert_all();
}

// ---------------------------------------------------------------------------
// Navigation
// ---------------------------------------------------------------------------

/// README's `j` / `↓` and `k` / `↑` are aliases, so two reviewers pressing the
/// same moves — one on letters, one on arrows — must end up looking at exactly
/// the same thing.
///
/// Differential rather than re-derived: neither app is the oracle, which is
/// what makes this fail if either binding drifts.
///
/// `Left` and `Right` are in the sequence because they are what makes the claim
/// non-trivial now: the pair moves the *file* selection while the sidebar has
/// focus and the *line* while the diff does, so an alias that was only wired up
/// in one of the two arms shows up here.
#[test]
fn arrow_keys_are_aliases_of_the_letters() {
    let fixture = shared_multi();
    let letters = RefCell::new(fixture.app());
    let arrows = RefCell::new(fixture.app());

    #[derive(Clone, Copy, Debug)]
    enum Move {
        Forward,
        Back,
        FileNext,
        FilePrevious,
        FocusLeft,
        FocusRight,
    }

    let moves = prop_oneof![
        3 => Just(Move::Forward),
        3 => Just(Move::Back),
        1 => Just(Move::FileNext),
        1 => Just(Move::FilePrevious),
        2 => Just(Move::FocusLeft),
        2 => Just(Move::FocusRight),
    ];

    let seen = Coverage::new(&[
        "moving with the sidebar focused",
        "moving with the diff focused",
    ]);
    run_cases(48, prop::collection::vec(moves, 0..24), |sequence| {
        let letters = &mut *letters.borrow_mut();
        let arrows = &mut *arrows.borrow_mut();
        rewind(letters);
        rewind(arrows);

        for step in &sequence {
            let (letter, arrow) = match step {
                Move::Forward => (KeyCode::Char('j'), KeyCode::Down),
                Move::Back => (KeyCode::Char('k'), KeyCode::Up),
                Move::FileNext => (KeyCode::Char(']'), KeyCode::Char(']')),
                Move::FilePrevious => (KeyCode::Char('['), KeyCode::Char('[')),
                Move::FocusLeft => (KeyCode::Left, KeyCode::Left),
                Move::FocusRight => (KeyCode::Right, KeyCode::Right),
            };
            if matches!(step, Move::Forward | Move::Back) {
                seen.hit(usize::from(letters.focus() == Focus::Diff));
            }
            letters.on_key(letter).expect("letter key");
            arrows.on_key(arrow).expect("arrow key");

            prop_assert_eq!(
                (letters.file_index(), letters.line_index()),
                (arrows.file_index(), arrows.line_index()),
                "after {:?}: letters and arrows disagree",
                step
            );
            prop_assert_eq!(letters.focus(), arrows.focus());
            prop_assert_eq!(letters.mode(), arrows.mode());
            prop_assert_eq!(letters.buffer(), arrows.buffer());
            prop_assert_eq!(letters.status(), arrows.status());
        }
        Ok(())
    });
    seen.assert_all();
}

/// The highlight's closed form: `n` presses of `j` from the top land on
/// `min(n, lines - 1)`, and `m` presses of `k` from there land on
/// `saturating_sub`. Recomputed from the diff's own length rather than by
/// replaying the loop, so an off-by-one in either clamp shows up.
///
/// The round trip is the second half: `j` cannot outrun the file, so `j` then
/// `k` the same number of times is always back at the top — however far past
/// the end the walk tried to go.
#[test]
fn line_navigation_clamps_at_both_ends() {
    let fixture = shared_multi();
    let app = RefCell::new(fixture.app());
    // `long.rs` is the file with enough lines for a walk to be interesting.
    let long = {
        let app = app.borrow();
        app.files()
            .iter()
            .position(|file| file.path == "long.rs")
            .expect("long.rs is in the review")
    };

    run_cases(64, (0usize..60, 0usize..60), |(downs, ups)| {
        let app = &mut *app.borrow_mut();
        rewind(app);
        press_n(app, KeyCode::Char(']'), long);
        prop_assert_eq!(app.file_index(), long);

        let total = lines(app).len();
        prop_assert!(total >= 20, "long.rs produced only {} diff lines", total);
        let last = total - 1;

        press_n(app, KeyCode::Char('j'), downs);
        prop_assert_eq!(
            app.line_index(),
            downs.min(last),
            "{} presses of j on {} lines",
            downs,
            total
        );

        press_n(app, KeyCode::Char('k'), ups);
        prop_assert_eq!(
            app.line_index(),
            downs.min(last).saturating_sub(ups),
            "{} presses of j then {} of k on {} lines",
            downs,
            ups,
            total
        );

        if ups >= downs {
            prop_assert_eq!(
                app.line_index(),
                0,
                "j x{} then k x{} did not return to the top",
                downs,
                ups
            );
        }
        Ok(())
    });
}

/// The sidebar's closed form, and the invariant that replaced the line reset:
/// every file keeps its *own* place, and `[` `]` gives it back.
///
/// Walking away from a file and back used to drop the highlight to line 1, so
/// comparing two files cost the reviewer their position in both. The oracle is
/// one remembered position per file, clamped to that file's own diff — a single
/// shared position, or a reset on the way in, both fail it.
#[test]
fn file_navigation_walks_in_range_and_keeps_each_files_place() {
    let fixture = shared_multi();
    let app = RefCell::new(fixture.app());
    let count = app.borrow().files().len();
    assert!(count >= 3, "the fixture lost files");

    // How long each file's diff is, so the oracle clamps `j` where the app
    // does. Two of `multi`'s files have no lines at all, which is the clamp
    // worth having in the model.
    let totals: Vec<usize> = {
        let app = &mut *app.borrow_mut();
        rewind(app);
        (0..count)
            .map(|index| {
                press_n(app, KeyCode::Char(']'), usize::from(index > 0));
                lines(app).len()
            })
            .collect()
    };
    assert!(
        totals.iter().any(|total| *total > 2),
        "no file has room to move in: {totals:?}"
    );

    // `true` is `]`, `false` is `[`; the `j`s before each step are what gives
    // the file being left a place to be remembered at.
    let step = (any::<bool>(), 0usize..6);
    let seen = Coverage::new(&["a file returned to at a line it was left on"]);
    run_cases(48, prop::collection::vec(step, 0..20), |steps| {
        let app = &mut *app.borrow_mut();
        rewind(app);

        let mut expected = 0usize;
        let mut places = vec![0usize; count];
        for (forward, downs) in &steps {
            press_n(app, KeyCode::Char('j'), *downs);
            places[expected] = (places[expected] + downs).min(totals[expected].saturating_sub(1));
            prop_assert_eq!(
                app.line_index(),
                places[expected],
                "{} presses of j in file {}",
                downs,
                expected
            );

            press(app, KeyCode::Char(if *forward { ']' } else { '[' }));
            expected = if *forward {
                (expected + 1).min(count - 1)
            } else {
                expected.saturating_sub(1)
            };
            prop_assert_eq!(
                app.file_index(),
                expected,
                "walking {:?} left the sidebar somewhere else",
                steps
            );
            if places[expected] > 0 {
                seen.hit(0);
            }
            prop_assert_eq!(
                app.line_index(),
                places[expected],
                "file {} did not come back to the line it was left on",
                expected
            );
        }
        Ok(())
    });
    seen.assert_all();
}

// ---------------------------------------------------------------------------
// Typing
// ---------------------------------------------------------------------------

/// The comment buffer is exactly the characters typed into it, minus the ones
/// backspaced off — nothing lost, nothing invented, and nothing contributed by
/// a key that is not a character.
///
/// The oracle is a `Vec<char>` built alongside, which is what catches a buffer
/// edited by bytes rather than by characters: a `truncate` in place of `pop`
/// splits a multi-byte character and panics.
#[test]
fn the_buffer_is_exactly_what_was_typed() {
    let fixture = shared_multi();
    let app = RefCell::new(fixture.app());

    let key = prop_oneof![
        6 => any::<char>().prop_filter("newlines commit", |c| *c != '\n').prop_map(KeyCode::Char),
        3 => Just(KeyCode::Backspace),
        3 => prop_oneof![
            Just(KeyCode::Tab),
            Just(KeyCode::Left),
            Just(KeyCode::Delete),
            Just(KeyCode::Home),
            Just(KeyCode::F(9)),
            Just(KeyCode::Null),
            Just(KeyCode::Up),
            Just(KeyCode::PageDown),
        ],
    ];

    run_cases(48, prop::collection::vec(key, 0..40), |keys| {
        let app = &mut *app.borrow_mut();
        rewind(app);
        press(app, KeyCode::Char('c'));
        prop_assert_eq!(app.mode(), Mode::Comment);

        let mut expected: Vec<char> = Vec::new();
        for key in &keys {
            app.on_key(*key).expect("handle a key");
            match key {
                KeyCode::Char(character) => expected.push(*character),
                KeyCode::Backspace => {
                    expected.pop();
                }
                _ => {}
            }
            let oracle: String = expected.iter().collect();
            prop_assert_eq!(app.buffer(), oracle.as_str(), "after {:?}", key);
            prop_assert_eq!(app.mode(), Mode::Comment);
        }
        // Leave the shared fixture as it was found.
        press(app, KeyCode::Esc);
        prop_assert!(fixture.comments().is_empty());
        Ok(())
    });
}

/// `Esc` discards, whatever was typed: the buffer is gone, the mode is back to
/// browsing, the store never heard of it, and the status line says so.
#[test]
fn escape_never_saves_anything() {
    let fixture = Fixture::multi();
    let app = RefCell::new(fixture.app());

    let seen = Coverage::new(&["an all-whitespace body", "a body worth losing"]);
    run_cases(48, (any_body(), 0usize..8), |(body, downs)| {
        fixture.clear_comments();
        let app = &mut *app.borrow_mut();
        rewind(app);
        press_n(app, KeyCode::Char('j'), downs);

        press(app, KeyCode::Char('c'));
        prop_assert_eq!(app.mode(), Mode::Comment);
        type_text(app, &body);
        prop_assert_eq!(app.buffer().chars().count(), body.chars().count());

        press(app, KeyCode::Esc);
        prop_assert_eq!(app.mode(), Mode::Browse);
        prop_assert_eq!(app.buffer(), "");
        prop_assert_eq!(app.status(), "comment discarded");
        prop_assert!(
            fixture.comments().is_empty(),
            "escaping saved {:?}",
            fixture.comments()
        );
        seen.hit(usize::from(!body.trim().is_empty()));
        Ok(())
    });
    seen.assert_all();
}

// ---------------------------------------------------------------------------
// Saving a comment
// ---------------------------------------------------------------------------

/// A comment typed one keystroke at a time reaches `comments.json` — and the
/// markdown export beside it — byte for byte, modulo the one documented
/// normalization (the body is stored trimmed).
///
/// Independent oracles, none of which re-run the app's own code:
///
/// * the stored body is `typed.trim()`;
/// * the anchor's hash and snapshot are what `anchor::create` would produce
///   from the *fixture's own constant* for the anchored side, at the number the
///   anchor stores — so reading the wrong side or the wrong commit shows up;
/// * `REVIEW-FEEDBACK.md` carries the body as one whole line, whatever
///   markdown or `rv:anchor` markers it contains;
/// * a snapshot file exists under the comment's id.
#[test]
fn a_typed_comment_reaches_the_store_byte_identically() {
    let fixture = Fixture::multi();
    let app = RefCell::new(fixture.app());

    // Indices are drawn over the diff's real length, not a fixed range: with a
    // wider range every draw past the end clamps onto the last line, and the
    // base-side arm of the coverage assertion below becomes rare.
    let total = {
        let app = app.borrow();
        assert_eq!(app.selected_file().expect("a file").path, "alpha.rs");
        assert_difftastic(&app);
        app.selected_diff().expect("a diff").lines.len()
    };
    assert!(total >= 3, "alpha.rs produced only {total} diff lines");
    let seen = Coverage::new(&[
        "an all-whitespace body",
        "a body with something in it",
        "a base-side (removed line) anchor",
        "a head-side anchor",
    ]);
    run_cases(64, (any_body(), 0usize..total), |(typed, downs)| {
        fixture.clear_comments();
        let app = &mut *app.borrow_mut();
        rewind(app);
        walk_to_line(app, downs);

        let line = lines(app)
            .get(app.line_index())
            .cloned()
            .expect("alpha.rs has a line here");
        let side = anchored_side(line.kind);
        let (source, number) = match side {
            Side::Left => (ALPHA_BASE, line.left),
            Side::Right => (ALPHA_HEAD, line.right),
        };
        let number = number.expect("an anchored side always carries its number");
        seen.hit(if side == Side::Left { 2 } else { 3 });
        seen.hit(usize::from(!typed.trim().is_empty()));

        press(app, KeyCode::Char('c'));
        prop_assert_eq!(app.mode(), Mode::Comment);
        type_text(app, &typed);
        press(app, KeyCode::Enter);

        prop_assert_eq!(app.mode(), Mode::Browse);
        prop_assert_eq!(app.buffer(), "");

        let expected = typed.trim();
        let comments = fixture.comments();
        if expected.is_empty() {
            prop_assert!(comments.is_empty(), "an all-whitespace body was saved");
            prop_assert_eq!(app.status(), "empty comment, nothing saved");
            return Ok(());
        }

        prop_assert_eq!(comments.len(), 1, "{:?}", comments);
        let comment = &comments[0];
        prop_assert_eq!(comment.body.as_str(), expected);
        prop_assert_eq!(comment.state, CommentState::Open);
        prop_assert_eq!(comment.reply.as_deref(), None);
        prop_assert_eq!(comment.anchor.file.as_str(), "alpha.rs");
        prop_assert_eq!(comment.anchor.side, side);
        prop_assert_eq!(comment.anchor.line, number);
        let saved = format!("comment saved at alpha.rs:{number}");
        prop_assert_eq!(app.status(), saved.as_str());

        // The recorded commit follows the side too: it is advisory, and its one
        // job is being a revision the quoted text can still be read out of, so
        // a comment on a removed line has to name the base.
        let expected_commit = match side {
            Side::Left => &app.session().base_commit,
            Side::Right => &app.session().head_commit,
        };
        prop_assert_eq!(
            comment.commit_id.as_str(),
            expected_commit.as_str(),
            "a {:?}-side comment recorded the other side's commit",
            side
        );

        // The hash and the snapshot come from the side the anchor names, at
        // the number it stores.
        let recomputed = anchor::create("alpha.rs", side, number, source);
        prop_assert_eq!(
            comment.anchor.content_hash.as_str(),
            recomputed.content_hash.as_str()
        );
        prop_assert_eq!(
            &comment.anchor.context,
            &anchor::snapshot_of(source, number)
        );

        prop_assert_eq!(comment.id.len(), 8, "{:?}", comment.id);
        prop_assert!(
            comment
                .id
                .chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
            "{:?} is not a lowercase hex id",
            comment.id
        );
        prop_assert!(
            fixture
                .root()
                .join(".review/snapshots")
                .join(&comment.id)
                .exists(),
            "no snapshot was written for {}",
            comment.id
        );

        // The export is rewritten with the comment in it, on one line: the
        // body cannot have been split, escaped or re-indented.
        let document = fixture.markdown();
        prop_assert!(
            document
                .lines()
                .any(|line| line == format!("**Comment:** {expected}")),
            "the export does not carry {:?} verbatim:\n{}",
            expected,
            document
        );
        Ok(())
    });
    seen.assert_all();
}

/// The same body on the two halves of a same-position rewrite is two comments,
/// not one.
///
/// difftastic pairs a rewritten line with its counterpart and gives *both*
/// halves *both* numbers, so on `same.rs` the removed half anchors to base-side
/// line 2 and the added half to head-side line 2: same change, same file, same
/// number, same body — different side. A comment id that leaves the side out of
/// its seed therefore gives both halves one id, and
/// `Store::append_comment`'s upsert replaces the reviewer's first note with
/// their second while the status line reports "comment saved" for both. That is
/// the loss `ID_CHARS = 8` spends fourteen lines of doc comment arguing must
/// never happen — reachable here with probability 1 rather than by birthday
/// chance.
#[test]
fn both_halves_of_a_same_position_rewrite_keep_their_own_comment() {
    let fixture = Fixture::collisions();
    let mut app = fixture.app();
    select_path(&mut app, "same.rs");
    assert_difftastic(&app);

    let diff_lines = lines(&app);
    let (removed_index, removed) = diff_lines
        .iter()
        .enumerate()
        .find(|(_, line)| {
            line.kind == LineKind::Removed && line.left.is_some() && line.left == line.right
        })
        .unwrap_or_else(|| {
            panic!("no paired removed line whose two numbers agree: {diff_lines:?}")
        });
    let (added_index, added) = diff_lines
        .iter()
        .enumerate()
        .find(|(_, line)| {
            line.kind == LineKind::Added && line.right.is_some() && line.left == line.right
        })
        .unwrap_or_else(|| panic!("no paired added line whose two numbers agree: {diff_lines:?}"));
    let number = removed
        .left
        .expect("a paired removed line carries its left");
    assert_eq!(
        added.right,
        Some(number),
        "the fixture's rewrite is not at the same number on both sides: {diff_lines:?}"
    );
    assert_eq!(anchored_side(removed.kind), Side::Left);
    assert_eq!(anchored_side(added.kind), Side::Right);

    // The same reviewer, the same sentence, on each half of the rewrite.
    for index in [removed_index, added_index] {
        select_path(&mut app, "same.rs");
        walk_to_line(&mut app, index);
        assert_eq!(app.line_index(), index);
        press(&mut app, KeyCode::Char('c'));
        assert_eq!(app.mode(), Mode::Comment);
        type_text(&mut app, "which of these two is right?");
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.status(), format!("comment saved at same.rs:{number}"));
    }

    let comments = fixture.comments();
    assert_eq!(
        comments.len(),
        2,
        "the second comment overwrote the first: {comments:?}"
    );
    let sides: Vec<Side> = comments.iter().map(|comment| comment.anchor.side).collect();
    assert!(
        sides.contains(&Side::Left) && sides.contains(&Side::Right),
        "both comments landed on the same side: {comments:?}"
    );
    assert_ne!(
        comments[0].id, comments[1].id,
        "two comments share one id: {comments:?}"
    );
    for comment in &comments {
        assert_eq!(comment.anchor.file, "same.rs");
        assert_eq!(comment.anchor.line, number);
        assert_eq!(comment.body, "which of these two is right?");
        // Each id owns its own snapshot, so neither comment's context was
        // overwritten by the other's.
        assert!(
            fixture
                .root()
                .join(".review/snapshots")
                .join(&comment.id)
                .exists(),
            "no snapshot for {}",
            comment.id
        );
    }
    // The base-side snapshot quotes the base file and the head-side one the
    // head file: the two comments are about genuinely different text.
    let left = comments
        .iter()
        .find(|comment| comment.anchor.side == Side::Left)
        .expect("a base-side comment");
    let right = comments
        .iter()
        .find(|comment| comment.anchor.side == Side::Right)
        .expect("a head-side comment");
    assert_eq!(left.anchor.context, SAME_BASE.lines().collect::<Vec<_>>());
    assert_eq!(right.anchor.context, SAME_HEAD.lines().collect::<Vec<_>>());
    assert_ne!(left.anchor.content_hash, right.anchor.content_hash);
}

/// A jump tells the two halves of a same-position rewrite apart.
///
/// `same.rs` rewrites line 2 without moving it, so difftastic pairs the halves
/// and both come back with `left == right == 2`: same file, same *path* (there
/// is no rename here), same number, opposite sides. The side is therefore the
/// only thing that distinguishes the two comments, and a jump that dropped it
/// from its lookup would send the reviewer to whichever half the diff lists
/// first — for both of them — while the status line named the right place.
///
/// That is not a hypothetical: dropping the side from `line_of_anchor` survives
/// every rename-based test in this suite, because for a rename the *path*
/// happens to carry the same information. This is the shape where it does not.
#[test]
fn a_jump_tells_the_two_halves_of_a_rewrite_apart() {
    let fixture = Fixture::collisions();
    let mut app = fixture.app();
    select_path(&mut app, "same.rs");
    assert_difftastic(&app);

    let diff_lines = lines(&app);
    let paired = |kind: LineKind| {
        diff_lines
            .iter()
            .position(|line| line.kind == kind && line.left.is_some() && line.left == line.right)
            .unwrap_or_else(|| {
                panic!("no paired {kind:?} line whose two numbers agree: {diff_lines:?}")
            })
    };
    let removed = paired(LineKind::Removed);
    let added = paired(LineKind::Added);
    assert_ne!(removed, added, "the two halves are the same diff line");

    // One comment on each half, in diff order, so the browser lists them in
    // that order too.
    for (index, body) in [(removed, "the old one"), (added, "the new one")] {
        select_path(&mut app, "same.rs");
        walk_to_line(&mut app, index);
        press(&mut app, KeyCode::Char('c'));
        assert_eq!(app.mode(), Mode::Comment);
        type_text(&mut app, body);
        press(&mut app, KeyCode::Enter);
    }
    assert_eq!(fixture.comments().len(), 2, "{:?}", fixture.comments());

    for (row, expected, kind) in [(0, removed, LineKind::Removed), (1, added, LineKind::Added)] {
        rewind(&mut app);
        press(&mut app, KeyCode::Tab);
        press(&mut app, KeyCode::Left);
        press_n(&mut app, KeyCode::Down, row);
        press(&mut app, KeyCode::Enter);

        assert_eq!(app.selected_file().expect("a file").path, "same.rs");
        assert_eq!(
            app.line_index(),
            expected,
            "row {row} jumped to the other half of the rewrite: {:?}",
            lines(&app)[app.line_index()]
        );
        assert_eq!(lines(&app)[app.line_index()].kind, kind);
        assert_eq!(
            app.comments_for_line(app.line_index()).len(),
            1,
            "the line jumped to shows both comments, so the two halves are not \
             being told apart"
        );
    }
    fixture.clear_comments();
}

/// Nothing a reviewer saves is ever lost or duplicated: after any run of
/// comments, `comments.json` holds exactly one entry per distinct
/// **(file, side, line, trimmed body)** the reviewer committed — the same body
/// re-typed at the same place upserts, and two comments that differ anywhere in
/// that tuple never collapse into one.
///
/// The key is the whole location, side included, because the *side* is where the
/// collision actually lives: `same.rs` rewrites a line without moving it, so its
/// removed and added halves are `same.rs:2` on opposite sides, and a side-blind
/// id makes one body typed on each half overwrite itself (see
/// [`both_halves_of_a_same_position_rewrite_keep_their_own_comment`], and the
/// `comment_id` doc comment for why the path cannot stand in for the side).
/// `alpha.rs` is the contrast: its rewrite sits one line lower on the head side,
/// so its two halves carry different numbers.
///
/// This is also the property `ID_CHARS` exists for: an id short enough to
/// collide makes `Store::append_comment`'s upsert overwrite an unrelated
/// comment, under a "comment saved" status line. Shrinking the id width is
/// exactly what this fails on.
///
/// Bodies are drawn from four short strings rather than `[a-z]{1,4}`: the
/// interesting cases are the ones where two writes *share* a body, and a
/// half-million-value alphabet makes them vanishingly rare. The coverage
/// receipt below is what proves the same-position pair is reached.
#[test]
fn distinct_comments_are_never_lost_to_each_other() {
    let fixture = Fixture::collisions();
    let app = RefCell::new(fixture.app());
    let count = app.borrow().files().len();
    assert_eq!(
        count,
        2,
        "the fixture lost files: {:?}",
        app.borrow().files()
    );

    // The pair whose two halves share a number is what makes this property a
    // test of the id's side-awareness rather than of its path-awareness.
    {
        let app = &mut *app.borrow_mut();
        select_path(app, "same.rs");
        assert_difftastic(app);
        let same = lines(app);
        assert!(
            same.iter().any(|line| line.kind == LineKind::Removed
                && line.left.is_some()
                && line.left == line.right)
                && same.iter().any(|line| line.kind == LineKind::Added
                    && line.right.is_some()
                    && line.left == line.right),
            "same.rs is not a same-position rewrite any more, so the collision this \
             property is about is unreachable: {same:?}"
        );
        select_path(app, "alpha.rs");
        assert_difftastic(app);
        let alpha = lines(app);
        assert!(
            alpha.iter().any(|line| match (line.left, line.right) {
                (Some(left), Some(right)) => left != right,
                _ => false,
            }),
            "alpha.rs no longer carries a pair with two different numbers: {alpha:?}"
        );
    }

    let body = prop_oneof![
        Just("a".to_owned()),
        Just("b".to_owned()),
        Just("ab".to_owned()),
        Just("ba".to_owned()),
    ];
    let write = (0usize..count, 0usize..4, body);
    let seen = Coverage::new(&["two comments distinguished only by their side"]);
    run_cases(32, prop::collection::vec(write, 1..9), |writes| {
        fixture.clear_comments();
        let app = &mut *app.borrow_mut();

        let mut expected: Vec<(String, &'static str, u32, String)> = Vec::new();
        for (file, downs, body) in &writes {
            rewind(app);
            press_n(app, KeyCode::Char(']'), *file);
            press_n(app, KeyCode::Char('j'), *downs);
            let selected = app.selected_file().cloned().expect("a file");
            let line = lines(app)
                .get(app.line_index())
                .cloned()
                .expect("both files have lines here");
            let side = anchored_side(line.kind);
            let number =
                anchored_number(&line).expect("an anchored side always carries its number");
            // The anchored path follows the side, exactly as the id's seed does.
            let path = match side {
                Side::Left => selected
                    .source_path
                    .clone()
                    .unwrap_or_else(|| selected.path.clone()),
                Side::Right => selected.path.clone(),
            };

            press(app, KeyCode::Char('c'));
            prop_assert_eq!(app.mode(), Mode::Comment);
            type_text(app, body);
            press(app, KeyCode::Enter);
            let saved = format!("comment saved at {path}:{number}");
            prop_assert_eq!(app.status(), saved.as_str());

            let entry = (path, side_tag(side), number, body.clone());
            if !expected.contains(&entry) {
                expected.push(entry);
            }
        }

        // Did this case reach the shape the id used to lose: one body on both
        // halves of one rewrite?
        if expected.iter().any(|(file, side, line, body)| {
            expected
                .iter()
                .any(|(other_file, other_side, other_line, other_body)| {
                    other_file == file
                        && other_line == line
                        && other_body == body
                        && other_side != side
                })
        }) {
            seen.hit(0);
        }

        let mut stored: Vec<(String, &'static str, u32, String)> = fixture
            .comments()
            .into_iter()
            .map(|comment| {
                (
                    comment.anchor.file,
                    side_tag(comment.anchor.side),
                    comment.anchor.line,
                    comment.body,
                )
            })
            .collect();
        let mut ids: Vec<String> = fixture
            .comments()
            .into_iter()
            .map(|comment| comment.id)
            .collect();
        stored.sort();
        expected.sort();
        prop_assert_eq!(
            &stored,
            &expected,
            "{} writes produced {} stored comments",
            writes.len(),
            stored.len()
        );
        // Two entries sharing an id would already have collapsed above, so this
        // is a receipt rather than a second chance: the store holds one id per
        // distinct location and body.
        ids.sort();
        let total = ids.len();
        ids.dedup();
        prop_assert_eq!(ids.len(), total, "two stored comments share an id");
        Ok(())
    });
    seen.assert_all();
}

/// A comment is refused *before* it is typed, never after.
///
/// The promise in `begin_comment`'s doc comment is that a reviewer is told
/// there is nothing to anchor to at the moment they press `c` — so the
/// contract is a disjunction, and both halves are checked at every reachable
/// (file, line): either `c` is refused outright and the store is untouched, or
/// the mode opens and a non-empty body *is* saved. There is no third case
/// where a typed comment is accepted and then dropped.
///
/// The fixture is built so both halves fire: `bin.dat` (binary) and
/// `blank.txt` (empty) have no diff lines at all, `alpha.rs` and `long.rs`
/// have plenty.
#[test]
fn commenting_is_refused_before_typing_or_not_at_all() {
    let fixture = Fixture::multi();
    let app = RefCell::new(fixture.app());
    let count = app.borrow().files().len();

    // Both halves of the disjunction have to be reachable, or this property
    // proves nothing.
    let (mut empty, mut nonempty) = (0, 0);
    {
        let app = &mut *app.borrow_mut();
        for index in 0..count {
            rewind(app);
            press_n(app, KeyCode::Char(']'), index);
            if lines(app).is_empty() {
                empty += 1;
            } else {
                nonempty += 1;
            }
        }
    }
    assert!(
        empty >= 2,
        "no file has an uncommentable diff; the fixture is wrong"
    );
    assert!(
        nonempty >= 2,
        "no file has a commentable diff; the fixture is wrong"
    );

    let seen = Coverage::new(&["a refused `c`", "an accepted `c`"]);
    run_cases(48, (0usize..count, 0usize..48), |(file, downs)| {
        fixture.clear_comments();
        let app = &mut *app.borrow_mut();
        rewind(app);
        press_n(app, KeyCode::Char(']'), file);
        press_n(app, KeyCode::Char('j'), downs);

        let selected = lines(app).get(app.line_index()).cloned();
        seen.hit(usize::from(selected.is_some()));
        press(app, KeyCode::Char('c'));

        if selected.is_none() {
            prop_assert_eq!(
                app.mode(),
                Mode::Browse,
                "comment mode opened on a diff with no lines"
            );
            prop_assert_eq!(app.status(), "no diff line selected, nothing to comment on");
            // Everything the reviewer types next is browsing, not a body.
            type_text(app, "wasted");
            press(app, KeyCode::Enter);
            prop_assert!(fixture.comments().is_empty(), "{:?}", fixture.comments());
            prop_assert_eq!(app.buffer(), "");
            return Ok(());
        }

        prop_assert_eq!(app.mode(), Mode::Comment);
        type_text(app, "kept");
        press(app, KeyCode::Enter);
        let comments = fixture.comments();
        prop_assert_eq!(
            comments.len(),
            1,
            "an accepted comment was dropped: {:?}",
            comments
        );
        prop_assert_eq!(comments[0].body.as_str(), "kept");
        prop_assert!(
            app.status().starts_with("comment saved at "),
            "{:?}",
            app.status()
        );
        Ok(())
    });
    seen.assert_all();
}

/// The number the diff pane prints beside a line, the number the status line
/// reports after saving, and the number `comments.json` stores are the same
/// number — on the same file.
///
/// The fixture renames `a.rs` to `b.rs` and rewrites two lines, so
/// difftastic pairs them and every paired line carries *both* a left and a
/// right number, and the base-side path differs from the head-side one. A pane
/// that labelled a removed line by its head number, or a status line that
/// named the head path, would disagree with the anchor here.
#[test]
fn the_pane_the_status_and_the_anchor_agree_on_the_line() {
    let fixture = Fixture::renamed();
    let app = RefCell::new(fixture.app());
    let lines = {
        let app = app.borrow();
        assert_difftastic(&app);
        let file = app.selected_file().expect("a file");
        assert_eq!(
            file.path,
            "b.rs",
            "jj did not record the rename; the base side has nothing to anchor to: {:?}",
            app.files()
        );
        assert_eq!(file.source_path.as_deref(), Some("a.rs"));
        app.selected_diff().expect("a diff").lines.clone()
    };
    let total = lines.len();
    // The property only bites where a line's two numbers disagree: that is the
    // case a pane labelling by the wrong side would get away with.
    let disagreeing = lines
        .iter()
        .filter(|line| match (line.left, line.right) {
            (Some(left), Some(right)) => left != right,
            _ => false,
        })
        .count();
    assert!(
        disagreeing >= 1,
        "no diff line carries two different numbers, so this proves nothing: {lines:?}"
    );

    let seen = Coverage::new(&["a base-side anchor", "a head-side anchor"]);
    run_cases((total * 8) as u32, 0usize..total, |index| {
        fixture.clear_comments();
        let app = &mut *app.borrow_mut();
        rewind(app);
        walk_to_line(app, index);
        prop_assert_eq!(app.line_index(), index);

        let printed = printed_number(app, 120, 44).ok_or_else(|| {
            TestCaseError::fail(format!(
                "no highlighted row in the diff pane at line {index}"
            ))
        })?;

        press(app, KeyCode::Char('c'));
        prop_assert_eq!(app.mode(), Mode::Comment);
        type_text(app, "why");
        press(app, KeyCode::Enter);

        let comments = fixture.comments();
        prop_assert_eq!(comments.len(), 1, "{:?}", comments);
        let anchor = &comments[0].anchor;
        let saved = format!("comment saved at {}:{}", anchor.file, anchor.line);
        prop_assert_eq!(app.status(), saved.as_str());
        prop_assert_eq!(
            printed,
            anchor.line,
            "the pane printed {} for line {} but the anchor stored {}",
            printed,
            index,
            anchor.line
        );
        // ...and the path follows the side, so the pane's file and the
        // anchor's file are the same file.
        let (expected_file, source) = match anchor.side {
            Side::Left => ("a.rs", RENAME_BASE),
            Side::Right => ("b.rs", RENAME_HEAD),
        };
        seen.hit(usize::from(anchor.side == Side::Right));
        prop_assert_eq!(anchor.file.as_str(), expected_file);
        let recomputed = anchor::create(expected_file, anchor.side, anchor.line, source);
        prop_assert_eq!(
            anchor.content_hash.as_str(),
            recomputed.content_hash.as_str()
        );
        Ok(())
    });
    seen.assert_all();
}

// ---------------------------------------------------------------------------
// Deleting a comment
// ---------------------------------------------------------------------------

/// Every file in the **whole workspace**, as `(path relative to the root,
/// mtime, bytes)`, sorted: a snapshot of everything on disk an action could
/// have touched.
///
/// Used to assert that an action wrote *nothing at all*, which is a stronger
/// and more durable claim than checking one filename — it holds whatever the
/// store keeps its comments in, so it survives the move to `session.toml`.
///
/// The whole root rather than `.review/`, which is what this used to walk. A
/// guard scoped to one directory only forbids writing *there*: a mutant that
/// spilled the fold set into `rv-folds.txt` beside `.review/` — one level up,
/// in the workspace the reviewer is reading — passed both collapse guards
/// untouched. "Nothing reached disk" is the claim, and the workspace is where
/// disk is.
///
/// The mtime is part of the snapshot on purpose. Bytes alone would let a
/// rewrite that produced the same document pass as "nothing happened", and a
/// rewrite *is* something that happened: the export exists to be read by
/// another program, which sees the write whatever the bytes say.
fn workspace_tree(root: &Path) -> Vec<(String, SystemTime, Vec<u8>)> {
    let mut files = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        let Ok(entries) = fs::read_dir(&directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
            } else {
                let name = path
                    .strip_prefix(root)
                    .expect("a path under the workspace root")
                    .display()
                    .to_string();
                let Ok(metadata) = fs::metadata(&path) else {
                    // A dangling symlink, or a file that went between the
                    // listing and the stat. Neither is something `rv` wrote.
                    continue;
                };
                let modified = metadata.modified().expect("an mtime");
                files.push((name, modified, fs::read(&path).unwrap_or_default()));
            }
        }
    }
    files.sort();
    files
}

/// `d`'s question is answered by *every* key, and the disk agrees with the
/// answer: `y` deletes, and anything else writes nothing whatsoever.
///
/// Both halves matter for different reasons. A confirmation that some key fails
/// to dismiss is a reviewer stuck in a mode with no way out but Ctrl+C, which is
/// the failure `on_key_confirm_delete` takes the mode out of the app *before*
/// branching in order to make unrepresentable. And a cancel that still touched
/// the workspace would mean "no" cost the reviewer something — checked here as
/// byte-identity of the whole tree rather than as a comment count, because
/// the export, the snapshots and the comments are all things a cancel must
/// leave alone.
#[test]
fn no_key_leaves_the_reviewer_stuck_at_a_confirmation() {
    let fixture = Fixture::multi();
    let app = RefCell::new(fixture.app());

    // `y` is weighted in rather than left to the key strategy: the confirmed
    // branch is the one that writes, and sampling it rarely would make the
    // receipt below flaky instead of informative.
    let answer = prop_oneof![3 => any_key(), 1 => Just(KeyCode::Char('y'))];
    let seen = Coverage::new(&["a cancelled deletion", "a confirmed deletion"]);
    run_cases(64, (answer, 0usize..4), |(key, downs)| {
        fixture.clear_comments();
        let app = &mut *app.borrow_mut();
        rewind(app);
        press_n(app, KeyCode::Char('j'), downs);

        press(app, KeyCode::Char('c'));
        prop_assert_eq!(app.mode(), Mode::Comment);
        type_text(app, "delete me");
        press(app, KeyCode::Enter);
        prop_assert_eq!(
            fixture.comments().len(),
            1,
            "the case has nothing to delete"
        );

        press(app, KeyCode::Char('d'));
        prop_assert!(
            matches!(app.mode(), Mode::ConfirmDelete { .. }),
            "d deleted without asking, or did not ask: {:?}",
            app.mode()
        );
        let before = workspace_tree(fixture.root());

        let action = app
            .on_key(key)
            .map_err(|error| TestCaseError::fail(error.to_string()))?;
        prop_assert_eq!(
            action,
            Action::Continue,
            "{:?} ended the review from inside a confirmation",
            key
        );
        prop_assert_eq!(
            app.mode(),
            Mode::Browse,
            "{:?} left the reviewer waiting at the question",
            key
        );

        let confirmed = key == KeyCode::Char('y');
        seen.hit(usize::from(confirmed));
        if confirmed {
            prop_assert!(
                fixture.comments().is_empty(),
                "y did not delete: {:?}",
                fixture.comments()
            );
        } else {
            prop_assert_eq!(
                fixture.comments().len(),
                1,
                "{:?} deleted a comment it was not asked to",
                key
            );
            prop_assert_eq!(
                workspace_tree(fixture.root()),
                before,
                "{:?} wrote to the workspace while cancelling",
                key
            );
        }
        Ok(())
    });
    seen.assert_all();
}

// ---------------------------------------------------------------------------
// Jumping to a comment's code
// ---------------------------------------------------------------------------

/// Every comment in the browser jumps to a line that shows it.
///
/// The oracle is the app's own display index — `comments_for_line` — rather
/// than the anchor arithmetic the jump uses, which is the point: the jump and
/// the save go through one `anchor_target`, so a jump that landed anywhere else
/// would mean the reviewer's own comment was not on the line the reviewer was
/// sent to. Written through the keyboard, so every anchor under test is one the
/// save path actually made.
#[test]
fn jumping_to_any_comment_lands_on_a_line_that_shows_it() {
    let fixture = Fixture::multi();
    let app = RefCell::new(fixture.app());

    // Only the files with diff lines can carry a comment at all; drawing from
    // the others would spend most cases writing nothing.
    let commentable: Vec<usize> = {
        let app = &mut *app.borrow_mut();
        let count = app.files().len();
        (0..count)
            .filter(|index| {
                rewind(app);
                press_n(app, KeyCode::Char(']'), *index);
                !lines(app).is_empty()
            })
            .collect()
    };
    assert!(
        commentable.len() >= 2,
        "fewer than two files can hold a comment: {commentable:?}"
    );

    let write = (proptest::sample::select(commentable), 0usize..6);
    let seen = Coverage::new(&["a jump that changed file", "a jump inside the open file"]);
    run_cases(24, prop::collection::vec(write, 1..5), |writes| {
        fixture.clear_comments();
        let app = &mut *app.borrow_mut();

        for (index, (file, downs)) in writes.iter().enumerate() {
            rewind(app);
            press_n(app, KeyCode::Char(']'), *file);
            walk_to_line(app, *downs);
            press(app, KeyCode::Char('c'));
            prop_assert_eq!(app.mode(), Mode::Comment);
            // Distinct bodies, so that two writes at one location are two
            // comments rather than one upsert of the other.
            type_text(app, &format!("finding {index}"));
            press(app, KeyCode::Enter);
        }

        let ids: Vec<String> = app.comments().iter().map(|c| c.id.clone()).collect();
        prop_assert!(!ids.is_empty(), "{:?} wrote nothing", writes);

        for (row, id) in ids.iter().enumerate() {
            rewind(app);
            press(app, KeyCode::Tab);
            press(app, KeyCode::Left);
            press_n(app, KeyCode::Down, row);
            prop_assert_eq!(
                app.browsed_comment()
                    .expect("a browsed comment")
                    .id
                    .as_str(),
                id.as_str(),
                "the browser's {}th row is not the {}th comment",
                row,
                row
            );

            let before = app.file_index();
            press(app, KeyCode::Enter);
            seen.hit(usize::from(app.file_index() == before));

            prop_assert_eq!(
                app.focus(),
                Focus::Diff,
                "the jump did not hand over the diff"
            );
            let landed = app.comments_for_line(app.line_index());
            prop_assert!(
                landed.iter().any(|comment| &comment.id == id),
                "row {} jumped to {}:{} , which does not show it: {:?}",
                row,
                app.file_index(),
                app.line_index(),
                app.status()
            );
        }
        Ok(())
    });
    seen.assert_all();
    fixture.clear_comments();
}

// ---------------------------------------------------------------------------
// Collapsing a box
// ---------------------------------------------------------------------------

/// Folding boxes away writes nothing. Collapse is a view preference of *this*
/// session: the next reviewer to open this `.review/`, and every LLM reading
/// the export, must see the review as it is rather than as this reviewer
/// arranged their screen.
///
/// Asserted as byte-identity of the whole **workspace** across a run of `s`
/// from both focuses, on a folded line and an unfolded one, rather than by
/// grepping one file for one word: a preference that leaked into `session.toml`
/// under some other name would pass the grep and fail this.
///
/// The workspace, not `.review/`, and that is the difference between this guard
/// and the one it replaces. Scoped to `.review/`, it only forbade folding from
/// writing *there*: a mutant that dropped the fold set into `rv-folds.txt` in
/// the workspace root — one level up, in the tree the reviewer is reading —
/// passed this test and its sibling in `--test app` both.
#[test]
fn collapsing_never_writes_to_the_workspace() {
    let fixture = Fixture::multi();
    let mut app = fixture.app();

    // Two comments on one line, and a third on the next: enough for `s` to have
    // something to do from the diff, from inside a stack, and on a line whose
    // boxes are in mixed states.
    for body in ["first finding", "second finding"] {
        press(&mut app, KeyCode::Char('c'));
        type_text(&mut app, body);
        press(&mut app, KeyCode::Enter);
    }
    press(&mut app, KeyCode::Char('j'));
    press(&mut app, KeyCode::Char('c'));
    type_text(&mut app, "third finding");
    press(&mut app, KeyCode::Enter);
    press(&mut app, KeyCode::Char('k'));
    assert_eq!(fixture.comments().len(), 3, "{:?}", fixture.comments());

    let before = workspace_tree(fixture.root());
    assert!(!before.is_empty(), "the review wrote nothing to compare");

    for key in [
        KeyCode::Char('s'), // fold the line, from the diff
        KeyCode::Enter,     // into the stack
        KeyCode::Char('s'), // unfold the selected box, leaving the line mixed
        KeyCode::Char('j'), // onto the other box
        KeyCode::Char('s'), // and fold that one
        KeyCode::Esc,       // back to the diff
        KeyCode::Char('s'), // fold the mixed line together
        KeyCode::Char('j'), // onto the next line
        KeyCode::Char('s'), // fold it too
        KeyCode::Char('s'), // and unfold it
    ] {
        press(&mut app, key);
        assert_eq!(
            workspace_tree(fixture.root()),
            before,
            "{key:?} wrote to the workspace while arranging the screen"
        );
    }
    assert!(
        !app.collapsed().is_empty(),
        "nothing ended up folded, so this proves nothing"
    );

    fixture.clear_comments();
}

// ---------------------------------------------------------------------------
// The `similar` fallback: what a reviewer without difftastic sees
// ---------------------------------------------------------------------------

/// The fallback diff is a different shape from difftastic's, and the pane says
/// which one it is showing.
///
/// Everything else in this file reviews through difftastic, which emits only
/// *changed* lines — so no diff anywhere else here contains a
/// [`LineKind::Context`] line or a [`DiffSource::Similar`] label, and the arms
/// of `ui::body`, `ui::title` and `app::anchored_side` that handle them were
/// never rendered or taken. This is the path every user with no `difft` on
/// `PATH` is on, and the one `RV_NO_DIFFT=1` forces.
#[test]
fn the_fallback_diff_is_labelled_and_carries_context_lines() {
    let fixture = Fixture::fallback();
    let app = fixture.fallback_app();
    let diff = app.selected_diff().expect("a loaded diff");
    assert_eq!(diff.path, "ctx.rs");
    assert_eq!(diff.source, DiffSource::Similar);
    assert!(!diff.suppressed);

    let kinds = |kind: LineKind| diff.lines.iter().filter(|line| line.kind == kind).count();
    assert!(
        kinds(LineKind::Context) >= 2,
        "the fallback diff has no context lines: {:?}",
        diff.lines
    );
    assert!(kinds(LineKind::Removed) >= 1, "{:?}", diff.lines);
    assert!(kinds(LineKind::Added) >= 2, "{:?}", diff.lines);

    // A context line belongs to the head side, and carries both numbers.
    let context = diff
        .lines
        .iter()
        .find(|line| line.kind == LineKind::Context)
        .expect("a context line");
    assert_eq!(anchored_side(context.kind), Side::Right);
    assert!(
        context.left.is_some() && context.right.is_some(),
        "{context:?}"
    );

    // The pane labels the diff by its source and prints each kind's sigil:
    // ' ' for context, '-' for removed, '+' for added, after a five-wide
    // number column.
    let frame = render(&app, 120, 20).backend().to_string();
    assert!(
        frame.contains("ctx.rs — fallback"),
        "the pane does not say the diff is a fallback:\n{frame}"
    );
    for line in &diff.lines {
        let sigil = match line.kind {
            LineKind::Context => ' ',
            LineKind::Added => '+',
            LineKind::Removed => '-',
        };
        let number = anchored_number(line).expect("every fallback line is numbered");
        let row = format!("{number:>5} {sigil}{}", line.text.trim_end());
        assert!(
            frame.contains(row.trim_end()),
            "the pane does not render {line:?} as {row:?}:\n{frame}"
        );
    }

    // The contrast that makes the paragraph above true: difftastic over the
    // very same files produces no context line at all.
    let difftastic = fixture.app();
    let structural = difftastic.selected_diff().expect("a loaded diff");
    assert!(
        matches!(structural.source, DiffSource::Difftastic { .. }),
        "{structural:?}"
    );
    assert!(
        !structural
            .lines
            .iter()
            .any(|line| line.kind == LineKind::Context),
        "difftastic produced a context line, so the fallback is not the only \
         source of one any more: {:?}",
        structural.lines
    );
}

/// Navigating, commenting, anchoring and rendering all behave the same on the
/// fallback path as on difftastic's — including on a context line, which only
/// this path produces.
///
/// Checked over every line of the fallback diff at every pane height with room
/// for a row, with the coverage receipt naming the three line kinds: a case that
/// never selected a context line would leave `anchored_side`'s `Context` arm and
/// `ui::body`'s `Context` arm unexercised, which is exactly the hole this test
/// exists to close.
#[test]
fn the_fallback_path_navigates_comments_and_anchors() {
    let fixture = Fixture::fallback();
    let app = RefCell::new(fixture.fallback_app());
    let total = {
        let app = app.borrow();
        app.selected_diff().expect("a diff").lines.len()
    };
    assert!(total >= 6, "the fallback diff has only {total} lines");

    let seen = Coverage::new(&["a context line", "an added line", "a removed line"]);
    run_cases(48, (0usize..total, 4u16..24), |(index, height)| {
        fixture.clear_comments();
        let app = &mut *app.borrow_mut();
        rewind(app);
        walk_to_line(app, index);
        prop_assert_eq!(app.line_index(), index);

        let line = lines(app)[index].clone();
        seen.hit(match line.kind {
            LineKind::Context => 0,
            LineKind::Added => 1,
            LineKind::Removed => 2,
        });
        // Spelled out rather than taken from `anchored_side`: an oracle that
        // calls the function under test agrees with it by construction, and
        // "everything but a removed line is commented against the head" is a
        // claim about `rv`, not about this file's convenience helpers.
        let side = match line.kind {
            LineKind::Removed => Side::Left,
            LineKind::Added | LineKind::Context => Side::Right,
        };
        let (number, source) = match side {
            Side::Left => (line.left, CTX_BASE),
            Side::Right => (line.right, CTX_HEAD),
        };
        let number = number.expect("every fallback line is numbered on its own side");

        // The pane shows the selected line, highlighted, at this height.
        let frame = render(app, 100, height).backend().to_string();
        prop_assert!(
            frame.contains(line.text.trim_end()),
            "line {} ({:?}) is not on screen at height {}:\n{}",
            index,
            line.text,
            height,
            frame
        );
        prop_assert!(
            frame.contains("ctx.rs — fallback"),
            "the pane stopped calling this a fallback:\n{}",
            frame
        );
        prop_assert_eq!(
            printed_number(app, 100, height),
            Some(number),
            "at height {} the pane labels line {} ({:?}) with another number",
            height,
            index,
            line
        );

        // ...and a comment on it anchors where the pane said it would.
        press(app, KeyCode::Char('c'));
        prop_assert_eq!(app.mode(), Mode::Comment);
        type_text(app, "what about this line");
        press(app, KeyCode::Enter);
        let saved = format!("comment saved at ctx.rs:{number}");
        prop_assert_eq!(app.status(), saved.as_str());

        let comments = fixture.comments();
        prop_assert_eq!(comments.len(), 1, "{:?}", comments);
        let comment = &comments[0];
        prop_assert_eq!(comment.body.as_str(), "what about this line");
        prop_assert_eq!(comment.anchor.file.as_str(), "ctx.rs");
        prop_assert_eq!(comment.anchor.side, side);
        prop_assert_eq!(comment.anchor.line, number);
        let recomputed = anchor::create("ctx.rs", side, number, source);
        prop_assert_eq!(
            comment.anchor.content_hash.as_str(),
            recomputed.content_hash.as_str(),
            "the anchor hashed the wrong side or the wrong line for {:?}",
            line
        );
        prop_assert_eq!(
            &comment.anchor.context,
            &anchor::snapshot_of(source, number)
        );
        Ok(())
    });
    seen.assert_all();
}

// ---------------------------------------------------------------------------
// A suppressed diff that still has lines
// ---------------------------------------------------------------------------

/// A suppressed diff with lines in it is *shown*, not replaced by a sentence:
/// the note sits above the lines, and every line `j` can reach is on screen,
/// labelled with the number a comment on it anchors to.
///
/// `suppressed` used to imply `lines.is_empty()` — it was set only from
/// difftastic's `unchanged` status, which emits no chunks — and the pane, which
/// short-circuits on the flag, was written against that. The `similar` fallback
/// now reports a terminator-only change (a final newline appearing, CRLF
/// becoming LF) as suppressed *with* all-`Context` lines, because the difference
/// is real and the fallback says so explicitly rather than going silent. That
/// left the pane showing one sentence over a diff `line_count` was still
/// counting for `j`/`k` and `prepare_comment` was still willing to anchor to:
/// the reviewer could put the highlight, and a comment, on a line the pane was
/// not drawing.
///
/// So this pins the agreement rather than the fix: whatever the pane draws,
/// `j`/`k` walks, and whatever `j`/`k` walks can be commented on and anchors
/// where the pane said it would.
#[test]
fn a_suppressed_fallback_diff_shows_the_lines_it_lets_you_navigate() {
    let fixture = Fixture::terminator();
    let app = RefCell::new(fixture.fallback_app());

    for (path, head) in [("crlf.txt", CRLF_HEAD), ("eol.rs", EOL_HEAD)] {
        // The shape the rest of this test is about: suppressed, and not empty.
        let lines = {
            let app = &mut *app.borrow_mut();
            select_path(app, path);
            let diff = app.selected_diff().expect("a loaded diff");
            assert_eq!(diff.source, DiffSource::Similar, "{diff:?}");
            assert!(
                diff.suppressed,
                "{path} is not a suppressed diff, so this proves nothing: {diff:?}"
            );
            assert!(
                !diff.lines.is_empty(),
                "{path}'s suppressed diff has no lines, so this proves nothing: {diff:?}"
            );
            for (index, line) in diff.lines.iter().enumerate() {
                let number = u32::try_from(index + 1).expect("a small line number");
                assert_eq!(line.kind, LineKind::Context, "{line:?}");
                assert_eq!(line.left, Some(number), "{line:?}");
                assert_eq!(line.right, Some(number), "{line:?}");
            }
            diff.lines.clone()
        };

        // Every line, at every pane height with room for a row: the highlight
        // is on screen wearing the right number, and the row beside it is the
        // line's own text under the `Context` sigil.
        //
        // Swept exhaustively rather than sampled because the interesting
        // heights are the two smallest ones — a pane with room for the note and
        // one line, and a pane with room for only one row at all — and a
        // uniform draw over the range would visit them rarely enough to make
        // the receipt flaky.
        for (index, line) in lines.iter().enumerate() {
            let number = anchored_number(line).expect("a numbered context line");
            for height in 4u16..24 {
                let app = &mut *app.borrow_mut();
                select_path(app, path);
                walk_to_line(app, index);
                assert_eq!(app.line_index(), index);

                let frame = render(app, 100, height).backend().to_string();
                assert_eq!(
                    printed_number(app, 100, height),
                    Some(number),
                    "at height {height} the pane does not show line {index} of {path} \
                     ({line:?}) highlighted:\n{frame}"
                );
                let row = format!("{number:>5}  {}", line.text);
                assert!(
                    frame.contains(&row),
                    "at height {height} the pane does not draw {line:?} as {row:?}:\n{frame}"
                );
                // The note is a *header*, not a replacement: it appears above
                // the lines wherever the pane has a row to spare for it, and
                // gives that row back rather than hiding the selection when it
                // does not. A `Browse` bar takes one row and the pane's borders
                // two, so five is the first height with room for both.
                assert_eq!(
                    frame.contains(SUPPRESSED),
                    height >= 5,
                    "at height {height} the suppression note is in the wrong place:\n{frame}"
                );
            }
        }

        // ...and a comment on any of those lines anchors where the pane said.
        for (index, line) in lines.iter().enumerate() {
            let number = anchored_number(line).expect("a numbered context line");
            fixture.clear_comments();
            let app = &mut *app.borrow_mut();
            select_path(app, path);
            walk_to_line(app, index);

            press(app, KeyCode::Char('c'));
            assert_eq!(
                app.mode(),
                Mode::Comment,
                "commenting was refused on line {index} of {path}, which the pane draws: \
                 {:?}",
                app.status()
            );
            type_text(app, "is this terminator deliberate?");
            press(app, KeyCode::Enter);
            assert_eq!(app.status(), format!("comment saved at {path}:{number}"));

            let comments = fixture.comments();
            assert_eq!(comments.len(), 1, "{comments:?}");
            let anchor = &comments[0].anchor;
            assert_eq!(anchor.side, Side::Right);
            assert_eq!(anchor.file, path);
            assert_eq!(anchor.line, number);
            let recomputed = anchor::create(path, Side::Right, number, head);
            assert_eq!(
                anchor.content_hash, recomputed.content_hash,
                "the anchor hashed something other than {line:?}"
            );
        }
    }
    fixture.clear_comments();
}

/// The other half of the same flag: a suppressed diff with *no* lines is the
/// sentence and nothing else, and `c` on it is refused.
///
/// difftastic reports both files of [`Fixture::terminator`] as `unchanged` and
/// emits no chunks for either, so the very same workspace that produces the
/// case above produces this one through the other engine. Without this, a pane
/// that simply deleted the suppression branch would still pass the test above.
#[test]
fn a_suppressed_diff_with_no_lines_is_the_sentence_alone() {
    let fixture = Fixture::terminator();
    let mut app = fixture.app();

    for path in ["crlf.txt", "eol.rs"] {
        select_path(&mut app, path);
        assert_difftastic(&app);
        let diff = app.selected_diff().expect("a loaded diff");
        assert!(diff.suppressed, "{diff:?}");
        assert!(
            diff.lines.is_empty(),
            "difftastic emitted chunks for a terminator-only change, so this \
             case is no longer the empty one: {diff:?}"
        );

        let frame = render(&app, 100, 24).backend().to_string();
        assert!(
            frame.contains(SUPPRESSED),
            "the pane does not say the diff is suppressed:\n{frame}"
        );

        // Nothing to put the highlight on, so nothing to comment on either.
        press(&mut app, KeyCode::Char('c'));
        assert_eq!(app.mode(), Mode::Browse);
        assert_eq!(app.status(), "no diff line selected, nothing to comment on");
    }
    assert!(fixture.comments().is_empty(), "{:?}", fixture.comments());
}

/// A [`session::Review`] whose session covers no change refuses to save a
/// comment instead of storing one attributed to nothing.
///
/// `prepare_comment` calls this refusal defence in depth, and it used to call it
/// unreachable on the grounds that `rv_core::vcs::Repository::stack` returns
/// `EmptyRange` for an empty range. That is true of `session::build`, and only
/// of `session::build`: `session::Review` is `pub` with `pub` fields, so a
/// caller can assemble one with an empty `changes` — which is exactly what this
/// does, so that the branch has a test behind it rather than a claim.
#[test]
fn a_review_with_no_changes_refuses_to_attribute_a_comment() {
    let fixture = Fixture::fallback();
    let mut review = session::build(fixture.root(), Some("@--"), None).expect("build the review");
    assert!(
        !review.session.changes.is_empty(),
        "the range was empty before this test emptied it"
    );
    review.session.changes.clear();

    let mut app = App::new(review).expect("open the reviewer");
    assert!(!lines(&app).is_empty(), "the fixture has nothing to select");

    // The refusal is at Enter, not at `c`: there *is* a line to anchor to, and
    // what is missing is the change to attribute the comment to.
    press(&mut app, KeyCode::Char('c'));
    assert_eq!(app.mode(), Mode::Comment);
    type_text(&mut app, "who changed this?");
    press(&mut app, KeyCode::Enter);

    assert_eq!(app.status(), "the review covers no change to comment on");
    assert_eq!(app.mode(), Mode::Browse);
    assert_eq!(app.buffer(), "");
    assert!(fixture.comments().is_empty(), "{:?}", fixture.comments());
    assert!(
        fixture.markdown().is_empty(),
        "a refused comment rewrote the export:\n{}",
        fixture.markdown()
    );
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

/// The geometry `ui::draw` lays out, asked of the same function it paints
/// from, only to know *where* to look in the buffer. Nothing about the layout
/// is under test here — `rv/tests/layout.rs` owns that.
fn diff_area(width: u16, height: u16, mode: Mode) -> Rect {
    let bar_rows = if mode == Mode::Browse { 1 } else { 3 };
    layout(
        Rect::new(0, 0, width, height),
        Split::default(),
        Chrome {
            bar_rows,
            help_open: false,
            toast: false,
        },
    )
    .diff
}

fn render(app: &App, width: u16, height: u16) -> Terminal<TestBackend> {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("test terminal");
    terminal
        .draw(|frame| ui::draw(frame, app))
        .expect("draw a frame");
    terminal
}

/// The five-character line number printed on the highlighted row of the diff
/// pane at a `width` x `height` terminal, found by the **selection's
/// background** rather than by matching text — so a duplicated line cannot be
/// mistaken for the selected one.
///
/// It used to be found by the `REVERSED` modifier. The wave that put syntax
/// colours inside the diff took the reverse away deliberately: reversing swaps
/// the foreground and the background, so on a tinted line it turns the syntax
/// colours into the wash and the wash into the text. The selection is now a
/// *brighter* version of the line's own tint, and `ui::line_background` is the
/// one function that decides both — so this asks it rather than keeping a
/// second copy of the palette here.
///
/// The geometry is a parameter because it is the interesting variable: the
/// pane's window only scrolls once the diff is taller than the pane, so a
/// number checked at one generous height says nothing about what a reviewer on
/// a short terminal is shown.
fn printed_number(app: &App, width: u16, height: u16) -> Option<u32> {
    let terminal = render(app, width, height);
    let area = diff_area(width, height, app.mode());
    let buffer = terminal.backend().buffer().clone();

    let selected: Vec<Color> = [LineKind::Added, LineKind::Removed, LineKind::Context]
        .into_iter()
        .filter_map(|kind| ui::line_background(kind, true))
        .collect();
    let inner_x = area.x + 1;
    let row = (area.y + 1..area.y + area.height.saturating_sub(1)).find(|y| {
        buffer[(inner_x, *y)]
            .style()
            .bg
            .is_some_and(|background| selected.contains(&background))
    })?;
    let text: String = (inner_x..inner_x + 5)
        .map(|x| buffer[(x, row)].symbol().to_owned())
        .collect();
    text.trim().parse().ok()
}

/// The highlighted line is always on screen, and it is the *selected* line that
/// is highlighted.
///
/// `ui::window` centers the visible slice on the selection wherever the file
/// is long enough to center anything, which is what keeps `j` from walking the
/// highlight off the bottom of a pane. Checked over every line of a
/// forty-line diff at every pane height that has room for a row at all —
/// the failure a fixed `0..height` window would cause is invisible until the
/// selection passes the fold.
///
/// Both halves are checked *at the swept height*, which is the whole point of
/// sweeping it: `long.rs` has forty lines, so every height below ~42 makes the
/// window scroll, and the highlight then sits at a row whose position in the
/// pane is not its index in the diff. A number read off a fixed, generous
/// geometry would agree with the app on every one of those heights without ever
/// exercising the scrolled case.
#[test]
fn the_highlighted_line_is_always_rendered() {
    let fixture = shared_multi();
    let app = RefCell::new(fixture.app());
    let long = {
        let app = app.borrow();
        app.files()
            .iter()
            .position(|file| file.path == "long.rs")
            .expect("long.rs is in the review")
    };
    let total = {
        let app = &mut *app.borrow_mut();
        rewind(app);
        press_n(app, KeyCode::Char(']'), long);
        lines(app).len()
    };
    assert!(total >= 20, "long.rs produced only {total} diff lines");

    // A `Browse` bar takes one row and the pane's borders two, so a height of
    // four is the smallest that can show a line at all.
    run_cases(64, (0usize..total, 4u16..48), |(index, height)| {
        let app = &mut *app.borrow_mut();
        rewind(app);
        press_n(app, KeyCode::Char(']'), long);
        press_n(app, KeyCode::Char('j'), index);
        prop_assert_eq!(app.line_index(), index);

        let line = lines(app)[index].clone();
        let frame = render(app, 120, height).backend().to_string();
        prop_assert!(
            frame.contains(line.text.trim_end()),
            "line {} ({:?}) is not on screen at height {}:\n{}",
            index,
            line.text,
            height,
            frame
        );

        // The number beside it is the highlighted one, at this height: "on
        // screen" means highlighted rather than merely present, and the row
        // wearing the highlight is the selected line rather than whatever
        // happens to sit at the same offset inside a scrolled window.
        prop_assert_eq!(
            printed_number(app, 120, height),
            anchored_number(&line),
            "at height {} the highlight is not on line {} ({:?}):\n{}",
            height,
            index,
            line,
            render(app, 120, height).backend().to_string()
        );
        Ok(())
    });
}

/// `ui::draw` is total and deterministic: no terminal size, no mode, no
/// selection and no comment body makes it panic, and painting the same app
/// twice paints the same cells.
///
/// Totality is the load-bearing half — a one-row or one-column terminal is
/// where ratatui layout code classically panics, a `Comment` bar asks for three
/// rows out of a frame that may have one, and the diff pane subtracts two rows
/// of border from whatever is left. Degenerate sizes are therefore *weighted
/// in* rather than left to a uniform draw over `1..40`, and the coverage receipt
/// records that they were reached: a sweep that never went below four rows
/// would report green while leaving the arithmetic this exists to check
/// untried.
///
/// What this deliberately does **not** claim any more is that "drawing is a
/// pure projection". `ui::draw(frame, app)` takes `&App`: that it moves neither
/// the selection nor the mode nor the buffer is enforced by the borrow checker
/// before a single case runs, so asserting it here proved nothing (and the old
/// `file_index == file.min(count - 1)` assertion was literally `file == file`,
/// since `file` was drawn from `0..count`). Determinism is the part the types
/// do not give for free: it is what fails if a frame ever comes to depend on a
/// clock, a counter or any other state the app does not own.
#[test]
fn drawing_never_panics_at_any_size() {
    let fixture = shared_multi();
    let app = RefCell::new(fixture.app());
    let count = app.borrow().files().len();

    let inputs = (
        prop_oneof![3 => 1u16..60, 1 => 1u16..4],
        prop_oneof![3 => 1u16..40, 1 => 1u16..4],
        0usize..count,
        0usize..48,
        prop_oneof![Just(None), any_body().prop_map(Some)],
    );
    let seen = Coverage::new(&[
        "a status bar",
        "a comment box",
        "a terminal with no room for a diff row",
    ]);
    run_cases(64, inputs, |(width, height, file, downs, body)| {
        let app = &mut *app.borrow_mut();
        rewind(app);
        press_n(app, KeyCode::Char(']'), file);
        press_n(app, KeyCode::Char('j'), downs);
        // Only type once the box is actually open. `c` on a binary or empty
        // diff is refused, and the body would then be pressed *as browse keys*
        // — `[` and `j` and all — which moves the selection out from under the
        // case rather than filling a comment box.
        let typing = match &body {
            Some(body) => {
                press(app, KeyCode::Char('c'));
                let opened = app.mode() == Mode::Comment;
                if opened {
                    type_text(app, body);
                }
                opened
            }
            None => false,
        };
        seen.hit(usize::from(typing));
        let bar = if typing { 3 } else { 1 };
        if height.saturating_sub(bar) <= 2 {
            seen.hit(2);
        }

        let frame = render(app, width, height);
        let buffer = frame.backend().buffer().clone();
        prop_assert_eq!(buffer.area, Rect::new(0, 0, width, height));

        // Same app, same frame: nothing outside `App` decides what is painted.
        let again = render(app, width, height);
        prop_assert_eq!(
            again.backend().buffer(),
            &buffer,
            "drawing {}x{} twice painted two different frames",
            width,
            height
        );

        press(app, KeyCode::Esc);
        Ok(())
    });
    seen.assert_all();
}

/// The same totality claim, with **comment boxes on screen** — which is where
/// the arithmetic actually is.
///
/// A box subtracts a seven-column gutter, two borders and their padding from
/// whatever width it is given, and its body is wrapped to what is left; the
/// row model then windows over rows rather than lines. Every one of those is a
/// subtraction that panics if it is not saturating, and none of them runs at
/// all in `drawing_never_panics_at_any_size`, whose fixture has no comments in
/// it by construction.
///
/// The walk is navigation only. `d` and `y` would empty the fixture under the
/// sweep — and what a delete does is pinned elsewhere — while every key here
/// changes what is *drawn*: the focus, the tab, the browser's row, which boxes
/// are folded, and which line the window is centred on.
#[test]
fn drawing_never_panics_with_comment_boxes_on_screen() {
    let fixture = Fixture::multi();
    let app = RefCell::new(fixture.app());
    {
        let app = &mut *app.borrow_mut();
        rewind(app);
        for body in [
            "first finding",
            "a second finding, long enough that it has to wrap several times over in any pane \
             narrow enough to be worth sweeping",
        ] {
            press(app, KeyCode::Char('c'));
            type_text(app, body);
            press(app, KeyCode::Enter);
        }
        press(app, KeyCode::Char('j'));
        press(app, KeyCode::Char('c'));
        type_text(app, "third finding");
        press(app, KeyCode::Enter);
    }
    assert_eq!(fixture.comments().len(), 3, "{:?}", fixture.comments());

    let key = prop_oneof![
        Just(KeyCode::Char('j')),
        Just(KeyCode::Char('k')),
        Just(KeyCode::Enter),
        Just(KeyCode::Esc),
        Just(KeyCode::Left),
        Just(KeyCode::Right),
        Just(KeyCode::Tab),
        Just(KeyCode::Char('s')),
        Just(KeyCode::Char(']')),
        Just(KeyCode::Char('[')),
    ];
    let inputs = (
        prop_oneof![3 => 1u16..60, 1 => 1u16..5],
        prop_oneof![3 => 1u16..40, 1 => 1u16..5],
        prop::collection::vec(key, 0..12),
    );
    let seen = Coverage::new(&[
        "a terminal with no room for a diff row",
        "a box actually drawn",
    ]);
    run_cases(48, inputs, |(width, height, keys)| {
        let app = &mut *app.borrow_mut();
        rewind(app);
        for key in &keys {
            app.on_key(*key)
                .map_err(|error| TestCaseError::fail(format!("{key:?}: {error}")))?;
            let frame = render(app, width, height);
            prop_assert_eq!(
                frame.backend().buffer().area,
                Rect::new(0, 0, width, height)
            );
        }
        if height <= 3 {
            seen.hit(0);
        }
        // Whatever the walk left behind, drawn at the sizes that have
        // historically broken ratatui layout arithmetic — spelled out rather
        // than sampled, because 1x1 is the case and a uniform draw would visit
        // it once in a hundred runs.
        for (width, height) in PATHOLOGICAL {
            let frame = render(app, width, height);
            prop_assert_eq!(
                frame.backend().buffer().area,
                Rect::new(0, 0, width, height)
            );
        }
        if render(app, 120, 44).backend().to_string().contains('╭') {
            seen.hit(1);
        }
        Ok(())
    });
    seen.assert_all();
    fixture.clear_comments();
}

// ---------------------------------------------------------------------------
// A review that changed no files
// ---------------------------------------------------------------------------

/// A range with changes but no changed files: every accessor answers `None`,
/// no key can panic, commenting is refused, and the pane says so at every
/// size.
///
/// `session::build` rejects an *empty range*, so this state is only reachable
/// the way the fixture builds it — two described changes that touch nothing —
/// and it is the state `ui::draw`'s "no changed files in this range" branch
/// and `ListState::with_selected(None)` exist for.
#[test]
fn a_review_with_no_files_is_inert_but_alive() {
    let fixture = shared_no_files();
    let app = RefCell::new(fixture.app());
    {
        let app = app.borrow();
        assert!(app.files().is_empty(), "{:?}", app.files());
        assert!(app.selected_file().is_none());
        assert!(app.selected_diff().is_none());
    }

    run_cases(
        32,
        (prop::collection::vec(any_key(), 0..16), 1u16..40, 1u16..24),
        |(keys, width, height)| {
            let app = &mut *app.borrow_mut();
            for key in &keys {
                app.on_key(*key)
                    .map_err(|error| TestCaseError::fail(format!("{key:?}: {error}")))?;
                prop_assert_eq!(app.file_index(), 0);
                prop_assert_eq!(app.line_index(), 0);
                prop_assert!(app.selected_file().is_none());
                prop_assert!(app.selected_diff().is_none());
                // With nothing to anchor to, comment mode can never open, so
                // no keystroke can ever become a body.
                prop_assert_eq!(app.mode(), Mode::Browse);
                prop_assert_eq!(app.buffer(), "");
            }

            // `Esc` first: a generated `?` leaves the keymap up, and every
            // other key is inert behind it — including the `c` this case is
            // about. `Esc` closes it and is a no-op otherwise.
            press(app, KeyCode::Esc);
            prop_assert!(!app.help_open());
            press(app, KeyCode::Char('c'));
            prop_assert_eq!(app.mode(), Mode::Browse);
            prop_assert_eq!(app.status(), "no diff line selected, nothing to comment on");
            prop_assert!(fixture.comments().is_empty());

            let frame = render(app, width, height);
            prop_assert_eq!(frame.backend().buffer().area.width, width);
            Ok(())
        },
    );
}

/// The pathological terminal sizes, spelled out as cases rather than sampled,
/// including the `Comment` bar asking for three rows out of one.
#[rstest]
#[case::single_cell(1, 1)]
#[case::one_row(80, 1)]
#[case::one_column(1, 40)]
#[case::two_by_five(2, 5)]
#[case::five_by_two(5, 2)]
#[case::three_by_three(3, 3)]
#[case::bar_only(40, 1)]
#[case::bar_plus_one(40, 2)]
#[case::comment_bar_exactly(40, 3)]
#[case::tall_and_thin(2, 60)]
fn drawing_survives_pathological_sizes(#[case] width: u16, #[case] height: u16) {
    let fixture = shared_multi();
    let mut app = fixture.app();

    let count = app.files().len();
    for file in 0..count {
        rewind(&mut app);
        press_n(&mut app, KeyCode::Char(']'), file);
        press_n(&mut app, KeyCode::Char('j'), 60);

        let browse = render(&app, width, height);
        assert_eq!(
            browse.backend().buffer().area,
            Rect::new(0, 0, width, height)
        );

        press(&mut app, KeyCode::Char('c'));
        if app.mode() == Mode::Comment {
            type_text(&mut app, "a comment being typed into a very small terminal");
        }
        let comment = render(&app, width, height);
        assert_eq!(
            comment.backend().buffer().area,
            Rect::new(0, 0, width, height)
        );
        press(&mut app, KeyCode::Esc);
    }
    assert!(fixture.comments().is_empty());
}

/// Every file's diff is the diff of *that file's* two blobs, on every visit:
/// the sidebar's cache can neither serve one file's lines under another's name
/// nor hand back something the repository does not say.
///
/// The oracle is an independent recomputation — the base and head blobs read
/// straight out of the repository (the base side at its own path, so a rename
/// still diffs against the file it came from) and handed to
/// [`rv_core::diff::compute`] — rather than the app's own earlier answer.
/// Comparing pass 2 with pass 1 would prove nothing: `load_selected`
/// early-returns on a cached diff, so all three passes read the same value, and
/// the comparison could only fail if `Clone` or `PartialEq` were broken.
/// "Stable" has to mean "still equal to what these blobs diff to".
#[test]
fn revisiting_a_file_returns_the_same_diff() {
    let fixture = shared_multi();
    let mut app = fixture.app();
    let count = app.files().len();
    assert!(count >= 3, "the fixture lost files");

    // Built once, from a second review of the same range: nothing here goes
    // through `App`.
    let review = session::build(fixture.root(), Some("@--"), None).expect("build the review");
    assert_eq!(review.files.len(), count);
    let expected: Vec<FileDiff> = review
        .files
        .iter()
        .map(|file| {
            let base_path = file.source_path.as_deref().unwrap_or(&file.path);
            let old = review
                .repo
                .read_blob(&review.session.base_commit, base_path)
                .expect("read the base blob");
            let new = review
                .repo
                .read_blob(&review.session.head_commit, &file.path)
                .expect("read the head blob");
            diff::compute(old.as_deref(), new.as_deref(), &file.path)
        })
        .collect();

    for pass in 0..3 {
        rewind(&mut app);
        for (index, expected) in expected.iter().enumerate() {
            press_n(&mut app, KeyCode::Char(']'), if index == 0 { 0 } else { 1 });
            assert_eq!(app.file_index(), index);
            let path = app.files()[index].path.clone();
            assert_eq!(path, review.files[index].path);
            let diff = app.selected_diff().expect("a loaded diff");
            assert_eq!(diff.path, path);
            assert_eq!(
                diff, expected,
                "pass {pass}: the diff the app serves for {path} is not the diff of its blobs"
            );
        }
    }
}
