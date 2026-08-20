//! Driving_the_app shared by the case modules.

use crossterm::event::KeyCode;
use proptest::prelude::*;
use proptest::test_runner::TestCaseResult;
use proptest::test_runner::TestRunner;
use rv::app::Action;
use rv::app::App;
use rv::app::Focus;
use rv::app::Mode;
use rv::app::SidebarTab;
use rv::app::anchored_side;
use rv::tree::Sort;
use rv_core::diff::DiffLine;
use rv_core::diff::DiffSource;
use rv_core::model::Side;
use std::cell::RefCell;

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
pub fn rewind(app: &mut App) {
    app.on_key(KeyCode::Esc).expect("leave comment mode");
    app.on_key(KeyCode::Left).expect("out of the stack");
    app.on_key(KeyCode::Left).expect("onto the sidebar");
    to_comments(app);
    for _ in 0..=app.browser_rows().len() {
        // Bounded for the same reason the line loop below is: this presses the
        // very key the browser's clamp is about.
        app.on_key(KeyCode::Up).expect("first comment");
    }
    // Round the cycle rather than one press back: `Tab` goes forward only, and
    // the cycle is Files → Commits → Comments, so one press from the browser
    // lands on the file list only because it is the next one along.
    to_files(app);
    // The file list's shape and order are session preferences a generated `t`
    // or `o` will have moved, and both change what `j` walks in that pane: a
    // sorted list puts a different file under the cursor, and a tree puts rows
    // there that are not files at all. Reset from the Files tab, which is the
    // one place either key does anything.
    if app.tree_view() {
        app.on_key(KeyCode::Char('t')).expect("back to the list");
    }
    for _ in 0..3 {
        if app.sort() == Sort::Natural {
            break;
        }
        app.on_key(KeyCode::Char('o')).expect("back to path order");
    }
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
    // The browser's cursor is back at the top of its list. Said as "on the
    // first comment" rather than as a literal row: the browser groups its
    // comments under file headings, so row 0 is a heading and the first
    // *comment* is row 1 — and what the next case needs reset is which comment
    // `j` steps from, not which number holds it. A review with no comments has
    // no first comment, and parks at 0.
    assert_eq!(
        app.browser_index(),
        usize::from(!app.comments().is_empty()),
        "the comment browser's cursor is not on its first comment"
    );
    assert!(!app.tree_view());
    assert_eq!(app.sort(), Sort::Natural);
    assert_eq!(app.sidebar_row(), 0);
    // The `Esc` above is also what puts the `?` popup away — a generated `?`
    // would otherwise leave every key of the next case inert.
    assert!(!app.help_open());
    // The split is deliberately *not* restored: no key the reviewer has moves
    // it back to a named ratio, and nothing downstream of `rewind` asserts on
    // the geometry. A property that starts doing so has to reset it itself.
}

pub fn press(app: &mut App, key: KeyCode) -> Action {
    app.on_key(key).expect("handle a key")
}

pub fn press_n(app: &mut App, key: KeyCode, times: usize) {
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
pub fn walk_to_line(app: &mut App, index: usize) -> usize {
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

pub fn type_text(app: &mut App, text: &str) {
    for character in text.chars() {
        press(app, KeyCode::Char(character));
    }
}

/// The lines of the selected file's diff.
pub fn lines(app: &App) -> Vec<DiffLine> {
    app.selected_diff()
        .map(|diff| diff.lines.clone())
        .unwrap_or_default()
}

/// Rewinds and then walks the sidebar to `path` with `]`, the way a reviewer
/// would.
pub fn select_path(app: &mut App, path: &str) {
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
pub fn side_tag(side: Side) -> &'static str {
    match side {
        Side::Left => "left",
        Side::Right => "right",
    }
}

/// The anchored-side number of the selected line: what the pane prints, what
/// the status line reports, and what the anchor stores.
pub fn anchored_number(line: &DiffLine) -> Option<u32> {
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
pub fn assert_difftastic(app: &App) {
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
pub struct Coverage {
    names: Vec<&'static str>,
    hits: RefCell<Vec<usize>>,
}

impl Coverage {
    pub fn new(names: &[&'static str]) -> Self {
        Self {
            names: names.to_vec(),
            hits: RefCell::new(vec![0; names.len()]),
        }
    }

    pub fn hit(&self, branch: usize) {
        self.hits.borrow_mut()[branch] += 1;
    }

    pub fn assert_all(&self) {
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
pub fn run_cases<S: Strategy>(cases: u32, strategy: S, test: impl Fn(S::Value) -> TestCaseResult) {
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

/// Presses `Tab` until the sidebar is showing the review's comments.
///
/// The cycle is Files → Commits → Comments; a test that wants the browser wants
/// it whatever the cycle's length is this week.
pub fn to_comments(app: &mut App) {
    for _ in 0..8 {
        if app.sidebar_tab() == SidebarTab::Comments {
            return;
        }
        app.on_key(KeyCode::Tab).expect("switch the sidebar tab");
    }
    panic!("the comments tab is not in the Tab cycle");
}

/// The same, for the tab that lists the stack's changes.
pub fn to_commits(app: &mut App) {
    for _ in 0..8 {
        if app.sidebar_tab() == SidebarTab::Commits {
            return;
        }
        app.on_key(KeyCode::Tab).expect("switch the sidebar tab");
    }
    panic!("the commits tab is not in the Tab cycle");
}

/// The same, for the file list — which is also where `t` and `o` mean
/// something, so `rewind` resets them from here.
pub fn to_files(app: &mut App) {
    for _ in 0..8 {
        if app.sidebar_tab() == SidebarTab::Files {
            return;
        }
        app.on_key(KeyCode::Tab).expect("switch the sidebar tab");
    }
    panic!("the files tab is not in the Tab cycle");
}
