//! Tests for the review TUI's state machine and its one frame of output.
//!
//! [`rv::app::App::on_key`] is deliberately terminal-free, so everything below
//! drives the reviewer the way a user does — one [`KeyCode`] at a time — and
//! then inspects the `.review/` store through a *fresh* [`Store`], never the
//! app's own handle. That is what makes these tests about persistence rather
//! than about in-memory bookkeeping.
//!
//! The fixture is a copy of the one in `cli.rs`, which is itself a copy of
//! `rv-core/tests/fixture.rs`: a `tests/` helper cannot be imported across
//! crates, and each integration test file is its own crate. Every `jj`
//! invocation is made hermetic with `JJ_CONFIG=/dev/null` plus a fixed author,
//! so the developer's own jj config cannot change what the tests see.

use std::collections::HashSet;
use std::fs;
use std::ops::Range;
use std::path::Path;
use std::process::Command;
use std::sync::OnceLock;
use std::time::Duration;
use std::time::Instant;
use std::time::SystemTime;

use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyModifiers;
use crossterm::event::MouseButton;
use crossterm::event::MouseEvent;
use crossterm::event::MouseEventKind;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::style::Modifier;
use rstest::rstest;
use rv::app::Action;
use rv::app::App;
use rv::app::BINDINGS;
use rv::app::Focus;
use rv::app::Mode;
use rv::app::SidebarTab;
use rv::gradient;
use rv::layout::Chrome;
use rv::layout::Split;
use rv::layout::layout;
use rv::session;
use rv::ui;
use rv_core::anchor;
use rv_core::diff::DiffLine;
use rv_core::diff::DiffSource;
use rv_core::diff::LineKind;
use rv_core::highlight::Capture;
use rv_core::model::ChangeKind;
use rv_core::model::Side;
use rv_core::store::CommentState;
use rv_core::store::Store;
use tempfile::TempDir;

/// The first file every fixture reviews. Two indented lines so that `j` has
/// somewhere to go and the diff pane has something recognizable to render.
const SOURCE: &str = "fn a() {\n    let x = 1;\n}\n";

/// The second file [`Fixture::new`] reviews, so that the sidebar has somewhere
/// to move to: `[`, `]` and a focused file list all need a review of more than
/// one file to say anything. Sorts after [`SOURCE`]'s `a.rs`, which keeps
/// `a.rs` the file the reviewer opens on.
const SECOND: &str = "fn b() {\n    let y = 2;\n    let z = 3;\n}\n";

/// The base side of [`Fixture::renamed`]'s review: `a.rs`, before the rename.
const BASE_SIDE: &str = "fn a() {\n    let x = 1;\n    let y = 2;\n    let z = 3;\n}\n";

/// The head side of [`Fixture::renamed`]'s review: `b.rs`, one line rewritten
/// and one line added *above* it, so that the changed line sits at a different
/// number on each side (2 on the left, 3 on the right). A comment on its
/// removed half must anchor to the left number, never the right.
const HEAD_SIDE: &str = "// header\nfn a() {\n    let x = 42;\n    let y = 2;\n    let z = 3;\n}\n";

/// The base side of [`Fixture::rewritten`]: `rewrite.rs`, whose second line
/// ends in a **string literal**.
///
/// The trap this fixture exists to set is a *rewrite that does not move*: the
/// changed line is line 2 on both sides, in a file that is not renamed, and the
/// token at columns 16..21 has a different capture on each side — a string here
/// and a number in [`REWRITE_HEAD`]. So the only thing that can tell the two
/// halves apart is the **side**, and a highlight lookup that reads the head
/// blob for a removed line paints the string yellow-turned-magenta.
///
/// A rename cannot catch that, because a rename already encodes the side in the
/// path: read the wrong side and the path is wrong too, so the lookup misses
/// entirely and the line renders plain rather than wrongly coloured.
const REWRITE_BASE: &str = "fn rewrite() {\n    let value = \"aaa\";\n}\n";

/// The head side of [`Fixture::rewritten`]: the same line, same length, same
/// number — a number literal where the base had a string.
const REWRITE_HEAD: &str = "fn rewrite() {\n    let value = 12345;\n}\n";

/// The one changed line of [`Fixture::rewritten`], as it stands on each side.
const REWRITE_BASE_LINE: &str = "    let value = \"aaa\";";

/// See [`REWRITE_BASE_LINE`].
const REWRITE_HEAD_LINE: &str = "    let value = 12345;";

/// Where the literal starts in both of them, counted in characters from the
/// start of the line. The two sides agree, which is the whole point.
const REWRITE_LITERAL_COLUMN: usize = 16;

/// [`Fixture::plain`]'s one file: a `.txt`, for which rv ships no grammar.
const PLAIN_TEXT: &str = "just some prose\nand a second line\n";

/// [`Fixture::commented`]'s one file: a `//` comment over a function.
///
/// The comment is what the "comments render too white" defect was about, and a
/// frame of this file is the only way to read the colour a real terminal would
/// have been sent. The rest of the file is there so the same frame carries a
/// keyword, a type-free binding and a number literal beside it — a row that is
/// *all* comment would prove nothing about the captures around it.
const COMMENTED: &str = "// a note about a\nfn a() -> u32 {\n    let x = 1;\n    x\n}\n";

/// How many lines [`Fixture::mixed`]'s `added.rs` is: a file that is nothing
/// but additions, so its row sits at the green end of the gradient with no
/// seam on it at all.
const ADDED_LINES: u32 = 40;

/// The same for `removed.rs`, which exists at the base and is gone by the head.
const REMOVED_LINES: u32 = 25;

/// `count` distinct lines, each naming `prefix`, so a fixture's size is a
/// number this file states rather than one a reader counts off a literal.
fn numbered(prefix: &str, count: u32) -> String {
    (0..count)
        .map(|line| format!("{prefix} line {line}\n"))
        .collect()
}

struct Fixture {
    tempdir: TempDir,
}

impl Fixture {
    /// Creates a colocated jj workspace holding one described change that adds
    /// [`SOURCE`] as `a.rs` and [`SECOND`] as `b.rs`.
    fn new() -> Self {
        let fixture = Self {
            tempdir: tempfile::tempdir().expect("create temp dir"),
        };
        fixture.jj(&["git", "init", "--colocate"]);
        fixture.write("a.rs", SOURCE);
        fixture.write("b.rs", SECOND);
        fixture.jj(&["describe", "-m", "first change"]);
        fixture.jj(&["new"]);
        fixture
    }

    /// Creates a workspace whose second change renames `a.rs` to `b.rs` and
    /// rewrites a line of it.
    ///
    /// Reviewed from `@--` (see [`Fixture::app_from`]) rather than the default
    /// `trunk()`, which degrades to the root commit here and would make every
    /// file a plain addition with no base side at all.
    fn renamed() -> Self {
        let fixture = Self {
            tempdir: tempfile::tempdir().expect("create temp dir"),
        };
        fixture.jj(&["git", "init", "--colocate"]);
        fixture.write("a.rs", BASE_SIDE);
        fixture.jj(&["describe", "-m", "first change"]);
        fixture.jj(&["new"]);

        fs::remove_file(fixture.root().join("a.rs")).expect("remove a.rs");
        fixture.write("b.rs", HEAD_SIDE);
        fixture.jj(&["describe", "-m", "rename and edit"]);
        fixture.jj(&["new"]);
        fixture
    }

    /// Creates a workspace whose second change rewrites one line of
    /// `rewrite.rs` **in place** — same path, same line number on both sides.
    /// See [`REWRITE_BASE`] for why that is the only shape that catches a
    /// side-blind highlight lookup.
    ///
    /// Reviewed from `@--` (see [`Fixture::app_from`]), like
    /// [`Fixture::renamed`]: from the default `trunk()` the whole file is one
    /// addition with no base side at all, and a removed line is exactly what
    /// this fixture is for.
    fn rewritten() -> Self {
        let fixture = Self {
            tempdir: tempfile::tempdir().expect("create temp dir"),
        };
        fixture.jj(&["git", "init", "--colocate"]);
        fixture.write("rewrite.rs", REWRITE_BASE);
        fixture.jj(&["describe", "-m", "first change"]);
        fixture.jj(&["new"]);

        fixture.write("rewrite.rs", REWRITE_HEAD);
        fixture.jj(&["describe", "-m", "rewrite a line in place"]);
        fixture.jj(&["new"]);
        fixture
    }

    /// Creates a workspace whose second change is nothing but additions in one
    /// file and nothing but removals in another — the two ends of the change
    /// gradient, in one review.
    ///
    /// Reviewed from `@--` (see [`Fixture::app_from`]): from the default
    /// `trunk()` there is no base side at all, so nothing would ever be
    /// removed.
    fn mixed() -> Self {
        let fixture = Self {
            tempdir: tempfile::tempdir().expect("create temp dir"),
        };
        fixture.jj(&["git", "init", "--colocate"]);
        fixture.write("removed.rs", &numbered("gone", REMOVED_LINES));
        fixture.jj(&["describe", "-m", "first change"]);
        fixture.jj(&["new"]);

        fs::remove_file(fixture.root().join("removed.rs")).expect("remove removed.rs");
        fixture.write("added.rs", &numbered("new", ADDED_LINES));
        fixture.jj(&["describe", "-m", "one file each way"]);
        fixture.jj(&["new"]);
        fixture
    }

    /// Creates a workspace whose second change renames a file and changes
    /// **nothing** inside it.
    ///
    /// Distinct from [`Fixture::renamed`], which rewrites a line as it moves:
    /// a review of that file has a shape, and this one deliberately has none.
    fn pure_rename() -> Self {
        let fixture = Self {
            tempdir: tempfile::tempdir().expect("create temp dir"),
        };
        fixture.jj(&["git", "init", "--colocate"]);
        fixture.write("a.rs", SOURCE);
        fixture.jj(&["describe", "-m", "first change"]);
        fixture.jj(&["new"]);

        fs::remove_file(fixture.root().join("a.rs")).expect("remove a.rs");
        fixture.write("b.rs", SOURCE);
        fixture.jj(&["describe", "-m", "rename and nothing else"]);
        fixture.jj(&["new"]);
        fixture
    }

    /// Creates a workspace whose one change adds four files at three depths,
    /// with a chain of single-child directories over two of them and a
    /// different size on every one — so a tree has something to fold, an order
    /// has something to reorder, and the two can be told apart on screen.
    fn nested() -> Self {
        let fixture = Self {
            tempdir: tempfile::tempdir().expect("create temp dir"),
        };
        fixture.jj(&["git", "init", "--colocate"]);
        fixture.write("docs/specs/a.md", &numbered("a", 10));
        fixture.write("docs/specs/b.md", &numbered("b", 5));
        fixture.write("src/lib.rs", &numbered("lib", 30));
        fixture.write("top.rs", &numbered("top", 50));
        fixture.jj(&["describe", "-m", "a change with directories in it"]);
        fixture.jj(&["new"]);
        fixture
    }

    /// Creates a workspace whose one file has no grammar rv ships.
    fn plain() -> Self {
        let fixture = Self {
            tempdir: tempfile::tempdir().expect("create temp dir"),
        };
        fixture.jj(&["git", "init", "--colocate"]);
        fixture.write("notes.txt", PLAIN_TEXT);
        fixture.jj(&["describe", "-m", "some prose"]);
        fixture.jj(&["new"]);
        fixture
    }

    /// Creates a workspace whose one file opens with a `//` comment — see
    /// [`COMMENTED`].
    fn commented() -> Self {
        let fixture = Self {
            tempdir: tempfile::tempdir().expect("create temp dir"),
        };
        fixture.jj(&["git", "init", "--colocate"]);
        fixture.write("noted.rs", COMMENTED);
        fixture.jj(&["describe", "-m", "a commented function"]);
        fixture.jj(&["new"]);
        fixture
    }

    /// Creates a workspace whose one file is a single line of `length`
    /// characters — longer than any pane in this suite, and no longer than
    /// this repository's own longest line.
    fn with_long_line(length: usize) -> Self {
        let fixture = Self {
            tempdir: tempfile::tempdir().expect("create temp dir"),
        };
        fixture.jj(&["git", "init", "--colocate"]);
        let line: String = std::iter::repeat_n('x', length).collect();
        fixture.write("long.rs", &format!("{line}\n"));
        fixture.jj(&["describe", "-m", "one very long line"]);
        fixture.jj(&["new"]);
        fixture
    }

    /// The workspace root.
    fn root(&self) -> &Path {
        self.tempdir.path()
    }

    /// Runs `jj` in the workspace and returns its stdout, panicking on failure.
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

    /// Writes a file in the working copy, creating parent directories.
    fn write(&self, rel: &str, contents: &str) {
        let path = self.root().join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parent dir");
        }
        fs::write(&path, contents).expect("write file");
    }

    /// The reviewer, opened over `trunk()..@` of this workspace.
    fn app(&self) -> App {
        let review = session::build(self.root(), None, None).expect("build the review");
        App::new(review).expect("open the reviewer")
    }

    /// The reviewer, opened over `base..@` of this workspace.
    fn app_from(&self, base: &str) -> App {
        let review = session::build(self.root(), Some(base), None).expect("build the review");
        App::new(review).expect("open the reviewer")
    }

    /// A handle on `.review/` that shares nothing with the app's own.
    fn store(&self) -> Store {
        Store::open(self.root()).expect("open the store")
    }

    fn markdown(&self) -> String {
        fs::read_to_string(self.root().join(".review/REVIEW-FEEDBACK.md"))
            .expect("read REVIEW-FEEDBACK.md")
    }
}

/// Presses every character of `text` in order.
fn type_text(app: &mut App, text: &str) {
    for character in text.chars() {
        app.on_key(KeyCode::Char(character)).expect("type");
    }
}

/// Moves the highlight down to the first diff line `wanted` accepts, the way a
/// reviewer would, and returns it.
fn select_line(app: &mut App, wanted: impl Fn(&DiffLine) -> bool) -> DiffLine {
    let diff = app.selected_diff().expect("the selected file has a diff");
    let index = diff
        .lines
        .iter()
        .position(&wanted)
        .unwrap_or_else(|| panic!("no diff line matched: {:?}", diff.lines));
    for _ in 0..index {
        app.on_key(KeyCode::Char('j')).expect("move down a line");
    }
    assert_eq!(app.line_index(), index);
    app.selected_diff().expect("a diff").lines[index].clone()
}

/// The instant every frame below is painted at unless the test says otherwise.
///
/// Fixed once per process rather than read per frame, because a frame is a
/// function of the app and the time — the toast fades — and a test that painted
/// two frames at two instants would be comparing two different questions. Only
/// the alert tests care what the number is, and they pass their own.
fn epoch() -> Instant {
    static EPOCH: OnceLock<Instant> = OnceLock::new();
    *EPOCH.get_or_init(Instant::now)
}

/// One frame of the reviewer, as a 100x24 `TestBackend` renders it.
fn render(app: &App) -> String {
    let mut terminal = Terminal::new(TestBackend::new(100, 24)).expect("build a test terminal");
    terminal
        .draw(|frame| ui::draw(frame, app, epoch()))
        .expect("draw a frame");
    terminal.backend().to_string()
}

/// One frame at an arbitrary size, as **cells** rather than as text.
///
/// The wave that draws comment boxes is a wave about style: "blue and
/// bordered", "bold where the focus is", "grey when the comment is outdated".
/// A test that only greps the text of a frame passes on an unstyled box, so
/// everything below asserts against the buffer and reads the colours out of it.
fn frame_at(app: &App, width: u16, height: u16) -> Buffer {
    frame_at_time(app, width, height, epoch())
}

/// The same, painted at an instant the test chooses — which is how a toast's
/// fade is an assertion rather than a sleep.
fn frame_at_time(app: &App, width: u16, height: u16, now: Instant) -> Buffer {
    let mut terminal =
        Terminal::new(TestBackend::new(width, height)).expect("build a test terminal");
    terminal
        .draw(|frame| ui::draw(frame, app, now))
        .expect("draw a frame");
    terminal.backend().buffer().clone()
}

/// The frame's rows, one string per terminal row.
fn rows_of(buffer: &Buffer) -> Vec<String> {
    (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect()
        })
        .collect()
}

/// The whole frame as text, rows separated by newlines.
fn buffer_text(buffer: &Buffer) -> String {
    rows_of(buffer).join("\n")
}

/// The last row of the frame, which is where the bar is drawn — see
/// [`rv::layout`], which puts it under both panes rather than over them.
fn last_row(buffer: &Buffer) -> String {
    rows_of(buffer).pop().expect("a frame has rows")
}

/// The row `needle` first appears on, as an index into [`rows_of`].
fn row_holding(buffer: &Buffer, needle: &str) -> usize {
    rows_of(buffer)
        .iter()
        .position(|row| row.contains(needle))
        .unwrap_or_else(|| panic!("{needle:?} is not on screen:\n{}", buffer_text(buffer)))
}

/// The style of the first cell of `needle` on row `y`.
fn style_of_text(buffer: &Buffer, y: u16, needle: &str) -> ratatui::style::Style {
    let row: String = (0..buffer.area.width)
        .map(|x| buffer[(x, y)].symbol())
        .collect();
    let column = row
        .char_indices()
        .position(|(offset, _)| row[offset..].starts_with(needle))
        .unwrap_or_else(|| panic!("{needle:?} is not on row {y}: {row:?}"));
    buffer[(u16::try_from(column).expect("a small column"), y)].style()
}

/// Every file in the **whole workspace**, as `(path relative to the root,
/// mtime, bytes)`, sorted: a snapshot of everything on disk that an action
/// could have touched.
///
/// Comparing two of these is how a test says "this action wrote nothing at
/// all", which is both stronger than checking one filename and durable across
/// the storage model's move to `session.toml` — it never names a file.
///
/// The whole root rather than `.review/`, which is what this used to walk. A
/// guard scoped to one directory only forbids writing *there*: a mutant that
/// spilled the fold set into `rv-folds.txt` beside `.review/` — one level up,
/// in the workspace the reviewer is reading — passed both collapse guards
/// untouched. "Nothing reached disk" is the claim, and the workspace is where
/// disk is.
///
/// The mtime is in there because bytes alone are not enough: a rewrite that
/// happens to produce the same document is still a write, and another program
/// watching `.review/` — which is the whole point of the export — sees it.
/// Dropping it lets "cancelling a delete rewrites the export" pass unnoticed.
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

/// A rectangle's interior: the pane inside its own borders.
///
/// The panes' corners are rounded, so a `╭` at the edge of the frame is a
/// *pane* and a `╭` inside one is a comment box. Everything below that asks
/// about a box therefore asks inside this, not across the whole buffer.
fn inner(area: Rect) -> Rect {
    Rect::new(
        area.x + 1,
        area.y + 1,
        area.width.saturating_sub(2),
        area.height.saturating_sub(2),
    )
}

/// One frame row, cut to the columns of `area`.
fn row_in(buffer: &Buffer, area: Rect, y: u16) -> String {
    (area.x..area.right())
        .map(|x| buffer[(x, y)].symbol())
        .collect()
}

/// The text inside `area`, one row per line.
fn text_in(buffer: &Buffer, area: Rect) -> String {
    (area.y..area.bottom())
        .map(|y| row_in(buffer, area, y))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Where `needle` first appears inside `area`, scanning rows top to bottom.
fn find_char_in(buffer: &Buffer, area: Rect, needle: char) -> Option<(u16, u16)> {
    let wanted = needle.to_string();
    (area.y..area.bottom())
        .flat_map(|y| (area.x..area.right()).map(move |x| (x, y)))
        .find(|(x, y)| buffer[(*x, *y)].symbol() == wanted)
}

/// Whether the first cell of `needle` inside `area` is drawn in blue — the
/// colour this reviewer reserves for comments.
fn styled_blue_in(buffer: &Buffer, area: Rect, needle: char) -> bool {
    find_char_in(buffer, area, needle)
        .is_some_and(|(x, y)| buffer[(x, y)].style().fg == Some(Color::Blue))
}

/// The diff pane's interior at a 100x24 terminal, which is what almost every
/// test here renders at.
fn box_area() -> Rect {
    inner(areas(100, 24, Split::default()).diff)
}

/// One of [`rv::gradient`]'s colours, as `ui` sends it to the terminal.
fn colour(gradient::Rgb(red, green, blue): gradient::Rgb) -> Color {
    Color::Rgb(red, green, blue)
}

/// The file list's own rows, with the pane's borders taken off — so nothing the
/// diff pane draws can be mistaken for a file row.
fn sidebar_rows(buffer: &Buffer, width: u16, height: u16, split: Split) -> Vec<String> {
    let area = inner(areas(width, height, split).sidebar);
    (area.y..area.bottom())
        .map(|y| {
            (area.x..area.right())
                .map(|x| buffer[(x, y)].symbol())
                .collect()
        })
        .collect()
}

/// The same, as one string.
fn sidebar_text(buffer: &Buffer, width: u16, height: u16, split: Split) -> String {
    sidebar_rows(buffer, width, height, split).join("\n")
}

/// The rows of the file list that have anything on them, in order.
fn sidebar_filled(buffer: &Buffer, width: u16, height: u16, split: Split) -> Vec<String> {
    sidebar_rows(buffer, width, height, split)
        .into_iter()
        .filter(|row| !row.trim().is_empty())
        .collect()
}

/// The frame row the file list draws `needle` on, at a 100x24 terminal.
fn sidebar_row_for(buffer: &Buffer, needle: &str) -> u16 {
    sidebar_row_for_in(
        buffer,
        inner(areas(100, 24, Split::default()).sidebar),
        needle,
    )
}

/// The same, in a file list drawn at some other size.
fn sidebar_row_for_in(buffer: &Buffer, area: Rect, needle: &str) -> u16 {
    (area.y..area.bottom())
        .find(|y| row_in(buffer, area, *y).contains(needle))
        .unwrap_or_else(|| {
            panic!(
                "{needle:?} is not in the file list:\n{}",
                text_in(buffer, area)
            )
        })
}

/// What the file list says along its bottom border: its shape and its order.
fn sidebar_shape(buffer: &Buffer) -> String {
    let area = areas(100, 24, Split::default()).sidebar;
    (area.x..area.right())
        .map(|x| buffer[(x, area.bottom() - 1)].symbol())
        .collect()
}

/// The background of one cell, or `None` where it is left on the terminal's own
/// ground.
fn bg_of(buffer: &Buffer, x: u16, y: u16) -> Option<Color> {
    match buffer[(x, y)].style().bg {
        None | Some(Color::Reset) => None,
        colour => colour,
    }
}

/// Every foreground the file list drew, row by row and cell by cell.
///
/// Foregrounds rather than backgrounds because no row of this pane carries a
/// background at all — see `no_row_of_the_file_list_is_painted_over`.
fn sidebar_inks(buffer: &Buffer) -> Vec<Option<Color>> {
    let area = inner(areas(100, 24, Split::default()).sidebar);
    (area.y..area.bottom())
        .flat_map(|y| (area.x..area.right()).map(move |x| (x, y)))
        .map(|(x, y)| match buffer[(x, y)].style().fg {
            None | Some(Color::Reset) => None,
            colour => colour,
        })
        .collect()
}

/// Presses `c`, types `body`, and presses Enter — one whole comment.
fn write_comment(app: &mut App, body: &str) {
    app.on_key(KeyCode::Char('c')).expect("enter comment mode");
    type_text(app, body);
    app.on_key(KeyCode::Enter).expect("save the comment");
}

#[test]
fn first_file_selected_and_diff_available() {
    let workspace = Fixture::new();
    let app = workspace.app();

    let file = app.selected_file().expect("a file is selected");
    assert_eq!(file.path, "a.rs");

    let diff = app.selected_diff().expect("the selected file has a diff");
    assert!(
        diff.lines
            .iter()
            .any(|line| line.text.contains("let x = 1;")),
        "the diff does not carry the file's text: {:?}",
        diff.lines
    );
}

#[test]
fn build_writes_session_toml() {
    let workspace = Fixture::new();
    let _ = workspace.app();

    let session = workspace.store().read_session().expect("read session.toml");
    assert_eq!(session.revset, "trunk()..@");
    assert!(
        session
            .changes
            .iter()
            .any(|change| change.description == "first change"),
        "session.toml does not describe the reviewed stack: {:?}",
        session.changes,
    );
}

#[test]
fn typing_a_comment_persists_against_selected_line() {
    let workspace = Fixture::new();
    let mut app = workspace.app();

    // `a.rs` is added whole, so its diff is its three lines in order and the
    // second of them is line 2 of the head-side file.
    let line = select_line(&mut app, |line| line.text.contains("let x = 1;"));
    assert_eq!(line.right, Some(2), "{line:?}");
    write_comment(&mut app, "needs a doc");

    assert_eq!(app.mode(), Mode::Browse);
    assert_eq!(app.status(), "comment saved at a.rs:2");

    let comments = workspace.store().comments().expect("read comments.json");
    assert_eq!(comments.len(), 1, "{comments:?}");
    let comment = &comments[0];
    assert_eq!(comment.body, "needs a doc");
    assert_eq!(comment.state, CommentState::Open);
    assert_eq!(comment.reply, None);
    assert_eq!(comment.anchor.file, "a.rs");
    assert_eq!(comment.anchor.side, Side::Right);
    assert_eq!(comment.anchor.line, 2);

    // The markdown export is rewritten alongside the store, so the reviewer
    // never has to run `rv render` to hand the file to an LLM.
    assert!(
        workspace.markdown().contains("**Comment:** needs a doc"),
        "the rewritten markdown is missing the comment:\n{}",
        workspace.markdown()
    );
}

#[test]
fn commenting_on_a_removed_line_anchors_to_the_base_side() {
    let workspace = Fixture::renamed();
    let mut app = workspace.app_from("@--");

    let file = app.selected_file().expect("a file is selected");
    assert_eq!(file.path, "b.rs");
    assert_eq!(file.kind, ChangeKind::Renamed, "{file:?}");
    assert_eq!(file.source_path.as_deref(), Some("a.rs"), "{file:?}");

    // Everything below is about difftastic's *pairing* of a rewritten line
    // with its counterpart, so say so: `diff::compute` falls back to `similar`
    // when `difft` is missing or `RV_NO_DIFFT` is exported, and the fallback
    // numbers the two halves separately. Without this the test would still
    // pass on the fallback, while testing something else entirely.
    assert!(
        matches!(
            app.selected_diff().expect("a diff").source,
            DiffSource::Difftastic { .. }
        ),
        "difftastic did not produce this diff — is difft on PATH, or is RV_NO_DIFFT set? {:?}",
        app.selected_diff()
    );

    // The removed half of the rewritten line: line 2 of the base-side file,
    // which difftastic pairs with line 3 of the head-side one.
    let line = select_line(&mut app, |line| {
        line.kind == LineKind::Removed && line.text.contains("let x = 1;")
    });
    // difftastic aligns the pair, so this line carries *both* numbers. The
    // pane must label it by the side it would be anchored on.
    assert_eq!(line.left, Some(2), "{line:?}");
    assert_eq!(line.right, Some(3), "{line:?}");
    let frame = render(&app);
    assert!(
        frame.contains("    2 -    let x = 1;"),
        "the pane does not label the removed line by its base-side number:\n{frame}"
    );
    assert!(
        !frame.contains("    3 -"),
        "the pane labels a removed line by its head-side number:\n{frame}"
    );

    write_comment(&mut app, "why was this rewritten?");

    // The status names the base-side path and the base-side number, both of
    // which differ from the head-side ones the file is otherwise known by.
    assert_eq!(app.status(), "comment saved at a.rs:2");

    let comments = workspace.store().comments().expect("read comments.json");
    assert_eq!(comments.len(), 1, "{comments:?}");
    let anchor = &comments[0].anchor;
    assert_eq!(anchor.side, Side::Left);
    assert_eq!(anchor.file, "a.rs");
    assert_eq!(anchor.line, 2);

    // The hash and the snapshot come from the *base* blob, read at the base
    // commit under the base-side path: reading the head side instead would
    // hash `let x = 42;` and quote a file that opens with `// header`.
    assert_eq!(anchor.content_hash, anchor::content_hash("    let x = 1;"));
    assert_eq!(
        anchor.context,
        BASE_SIDE.lines().collect::<Vec<_>>(),
        "the snapshot is not the base-side file",
    );
}

/// A comment the reviewer just saved is readable back off the line it was
/// anchored to — the whole of what the diff pane needs in order to draw it
/// there.
#[test]
fn a_saved_comment_is_visible_on_the_line_it_anchored_to() {
    let workspace = Fixture::new();
    let mut app = workspace.app();

    app.on_key(KeyCode::Char('j')).expect("move down a line");
    let line = app.line_index();
    write_comment(&mut app, "needs a doc");

    let on_line = app.comments_for_line(line);
    assert_eq!(on_line.len(), 1, "the comment shows up on its own line");
    assert_eq!(on_line[0].body, "needs a doc");
    assert!(
        app.comments_for_line(line + 1).is_empty(),
        "and not on the next line"
    );
}

/// Comments are read off disk when the reviewer opens, not only when this
/// process is the one that wrote them: a review interrupted and resumed shows
/// the notes it already has.
#[test]
fn reopening_the_reviewer_shows_the_comments_already_saved() {
    let workspace = Fixture::new();
    let mut first = workspace.app();
    first.on_key(KeyCode::Char('j')).expect("move down a line");
    let line = first.line_index();
    write_comment(&mut first, "still here tomorrow");
    drop(first);

    let reopened = workspace.app();
    let on_line = reopened.comments_for_line(line);
    assert_eq!(on_line.len(), 1, "{:?}", reopened.comments());
    assert_eq!(on_line[0].body, "still here tomorrow");
}

/// `commit_id` is advisory, and its one job is being the commit whose blob the
/// quoted text can still be read from. A comment on removed text therefore has
/// to name the base commit: the head no longer has that text at all.
#[test]
fn a_left_side_comment_records_the_base_commit() {
    let workspace = Fixture::renamed();
    let mut app = workspace.app_from("@--");
    select_line(&mut app, |line| {
        line.kind == LineKind::Removed && line.text.contains("let x = 1;")
    });
    write_comment(&mut app, "you should not have removed this");

    let comment = &app.comments()[0];
    assert_eq!(comment.anchor.side, Side::Left);
    assert_eq!(
        comment.commit_id,
        app.session().base_commit,
        "a comment on removed text points at the commit that still has that text"
    );
}

/// The other side of the same rule, so that "the anchored side chooses" cannot
/// be satisfied by naming the base commit for everything.
#[test]
fn a_head_side_comment_records_the_head_commit() {
    let workspace = Fixture::renamed();
    let mut app = workspace.app_from("@--");
    select_line(&mut app, |line| {
        line.kind == LineKind::Added && line.text.contains("let x = 42;")
    });
    write_comment(&mut app, "why 42?");

    let comment = &app.comments()[0];
    assert_eq!(comment.anchor.side, Side::Right);
    assert_eq!(
        comment.commit_id,
        app.session().head_commit,
        "a comment on added text points at the commit that has that text"
    );
    assert_ne!(
        app.session().head_commit,
        app.session().base_commit,
        "the two endpoints are the same commit, so this proves nothing"
    );
}

#[test]
fn escape_abandons() {
    let workspace = Fixture::new();
    let mut app = workspace.app();

    app.on_key(KeyCode::Char('c')).expect("enter comment mode");
    app.on_key(KeyCode::Char('x')).expect("type");
    assert_eq!(app.mode(), Mode::Comment);
    assert_eq!(app.buffer(), "x");

    app.on_key(KeyCode::Esc).expect("abandon the comment");
    assert_eq!(app.mode(), Mode::Browse);
    assert_eq!(app.buffer(), "");

    let comments = workspace.store().comments().expect("read comments.json");
    assert!(comments.is_empty(), "{comments:?}");
}

#[test]
fn a_reply_survives_the_rewrite_a_new_comment_triggers() {
    let workspace = Fixture::new();
    let mut app = workspace.app();

    write_comment(&mut app, "first");

    // What an LLM does to the document: append a reply under the entry it
    // just addressed, leaving every marker alone.
    let replied = insert_reply(&workspace.markdown(), "fixed in the next change");
    fs::write(
        workspace.root().join(".review/REVIEW-FEEDBACK.md"),
        &replied,
    )
    .expect("write the replied-to markdown");

    // A second comment rewrites the whole document from comments.json, which
    // is exactly where the reply would be lost.
    app.on_key(KeyCode::Char('j')).expect("move down a line");
    write_comment(&mut app, "second");

    let comments = workspace.store().comments().expect("read comments.json");
    assert_eq!(comments.len(), 2, "{comments:?}");
    let first = comments
        .iter()
        .find(|comment| comment.body == "first")
        .expect("the first comment is still stored");
    assert_eq!(first.reply.as_deref(), Some("fixed in the next change"));
    // A reply is not a state transition: that is Milestone 2's job.
    assert_eq!(first.state, CommentState::Open);

    assert!(
        workspace
            .markdown()
            .contains("**Reply:** fixed in the next change"),
        "the rewritten markdown dropped the reply:\n{}",
        workspace.markdown()
    );
}

/// A comment stored under an id this version of `rv` would never derive still
/// resolves: its snapshot is found, its reply folds back in, and a new comment
/// beside it neither disturbs nor duplicates it.
///
/// This is the compatibility question the `comment_id` seed change raises —
/// adding the anchor's side changed every id the function produces, so a
/// `.review/` written by the previous build carries ids that no longer match
/// what its own location and body would hash to today. Nothing recomputes an id
/// to find a comment (`comments.json` is keyed by the id it stored, snapshots
/// are filed under it, and `session::fold_replies` matches the id a document's
/// marker carries against the stored one), so a review in progress keeps
/// working across the change. Rather than assert that from reading the code,
/// this drives it: `0badc0de` is not a digest of anything here.
#[test]
fn a_comment_stored_under_a_foreign_id_keeps_working() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, "written by the previous build");

    // Rewrite `.review/` the way an older `rv` left it: the same comment under
    // an id this build's seed cannot produce, snapshot filed under that id.
    const LEGACY: &str = "0badc0de";
    let mut comments = workspace.store().comments().expect("read comments.json");
    assert_eq!(comments.len(), 1, "{comments:?}");
    let derived = comments[0].id.clone();
    assert_ne!(derived, LEGACY);
    comments[0].id = LEGACY.to_owned();
    fs::write(
        workspace.root().join(".review/comments.json"),
        serde_json::to_string_pretty(&comments).expect("serialize comments.json"),
    )
    .expect("write the legacy comments.json");
    fs::rename(
        workspace.root().join(".review/snapshots").join(&derived),
        workspace.root().join(".review/snapshots").join(LEGACY),
    )
    .expect("file the snapshot under the legacy id");

    // The export is a projection of the store, so it now carries the legacy
    // marker — and a reply written under it binds to the stored comment.
    let review = session::build(workspace.root(), None, None).expect("build the review");
    session::write_markdown(&review).expect("rewrite the export");
    assert!(
        workspace.markdown().contains(LEGACY),
        "the export does not carry the legacy id:\n{}",
        workspace.markdown()
    );
    let replied = insert_reply(&workspace.markdown(), "still addressable");
    fs::write(
        workspace.root().join(".review/REVIEW-FEEDBACK.md"),
        &replied,
    )
    .expect("write the replied-to markdown");

    // A second comment rewrites the whole document from `comments.json`, with
    // this build's id scheme, beside the legacy entry.
    app.on_key(KeyCode::Char('j')).expect("move down a line");
    write_comment(&mut app, "written by this build");

    let comments = workspace.store().comments().expect("read comments.json");
    assert_eq!(comments.len(), 2, "{comments:?}");
    let legacy = comments
        .iter()
        .find(|comment| comment.id == LEGACY)
        .unwrap_or_else(|| panic!("the legacy comment was lost or re-keyed: {comments:?}"));
    assert_eq!(legacy.body, "written by the previous build");
    assert_eq!(legacy.reply.as_deref(), Some("still addressable"));
    assert!(
        workspace
            .root()
            .join(".review/snapshots")
            .join(LEGACY)
            .exists(),
        "the legacy snapshot was dropped"
    );
    assert!(
        workspace
            .markdown()
            .contains("**Reply:** still addressable"),
        "the rewritten export dropped the reply to the legacy comment:\n{}",
        workspace.markdown()
    );
}

/// Appends a `**Reply:**` block under every rendered comment body, the way an
/// LLM following the document's own protocol block would.
fn insert_reply(document: &str, reply: &str) -> String {
    let mut out = String::new();
    for line in document.lines() {
        out.push_str(line);
        out.push('\n');
        if line.starts_with("**Comment:**") {
            out.push_str(&format!("\n**Reply:** {reply}\n"));
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Aborting
// ---------------------------------------------------------------------------

/// In raw mode the terminal raises no SIGINT, so Ctrl+C is `rv`'s to answer —
/// and answering it with the comment box (which is what dropping the modifiers
/// does) leaves a reviewer's reflexive abort typing a note.
#[test]
fn ctrl_c_quits_instead_of_opening_a_comment() {
    let workspace = Fixture::new();
    let mut app = workspace.app();

    let action = app
        .on_key_event(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL))
        .expect("ctrl-c");

    assert_eq!(action, Action::Quit, "ctrl-c aborts the review");
    assert_eq!(
        app.mode(),
        Mode::Browse,
        "and does not open the comment buffer"
    );
}

#[test]
fn a_plain_c_still_opens_a_comment() {
    let workspace = Fixture::new();
    let mut app = workspace.app();

    let action = app
        .on_key_event(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE))
        .expect("c");

    assert_eq!(action, Action::Continue);
    assert_eq!(app.mode(), Mode::Comment, "plain c is unchanged");
}

/// The abort is an abort from anywhere: a half-typed comment is not a state a
/// reviewer has to escape from before they can leave.
#[test]
fn ctrl_c_aborts_from_inside_a_half_typed_comment() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    app.on_key(KeyCode::Char('c')).expect("enter comment mode");
    type_text(&mut app, "half a thought");

    let action = app
        .on_key_event(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL))
        .expect("ctrl-c");

    assert_eq!(action, Action::Quit);
    let comments = workspace.store().comments().expect("read comments.json");
    assert!(
        comments.is_empty(),
        "aborting saved the half-typed comment: {comments:?}"
    );
}

// ---------------------------------------------------------------------------
// Which pane the keys act on
// ---------------------------------------------------------------------------

#[test]
fn left_and_right_move_focus_between_the_panes() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    assert_eq!(app.focus(), Focus::Diff, "the diff has focus on launch");

    app.on_key(KeyCode::Left).expect("left");
    assert_eq!(app.focus(), Focus::Sidebar);
    app.on_key(KeyCode::Left).expect("left again");
    assert_eq!(
        app.focus(),
        Focus::Sidebar,
        "there is nothing left of the files"
    );

    app.on_key(KeyCode::Right).expect("right");
    assert_eq!(app.focus(), Focus::Diff);
    app.on_key(KeyCode::Right).expect("right again");
    assert_eq!(
        app.focus(),
        Focus::Diff,
        "there is nothing right of the diff"
    );
}

#[rstest]
#[case(KeyCode::Char('j'), KeyCode::Char('k'))]
#[case(KeyCode::Down, KeyCode::Up)]
fn with_the_files_focused_both_key_pairs_move_the_file_selection(
    #[case] forward: KeyCode,
    #[case] back: KeyCode,
) {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    app.on_key(KeyCode::Left).expect("focus files");

    app.on_key(forward).expect("forward");
    assert_eq!(app.file_index(), 1, "moved to the second file");
    app.on_key(back).expect("back");
    assert_eq!(app.file_index(), 0, "and back to the first");
    app.on_key(back).expect("back off the top");
    assert_eq!(app.file_index(), 0, "and stays there");
}

#[rstest]
#[case(KeyCode::Char('j'), KeyCode::Char('k'))]
#[case(KeyCode::Down, KeyCode::Up)]
fn with_the_diff_focused_both_key_pairs_move_the_line(
    #[case] forward: KeyCode,
    #[case] back: KeyCode,
) {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    assert_eq!(app.focus(), Focus::Diff);

    app.on_key(forward).expect("forward");
    assert_eq!(app.line_index(), 1);
    assert_eq!(app.file_index(), 0, "the file list did not move with it");
    app.on_key(back).expect("back");
    assert_eq!(app.line_index(), 0);
}

/// Stepping to the next file and back is how a reviewer compares two files, and
/// it used to cost them their place in the first one: `]` `[` dropped the
/// highlight back to line 1 every time.
#[test]
fn leaving_a_file_and_coming_back_keeps_your_place() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    app.on_key(KeyCode::Char('j')).expect("down");
    app.on_key(KeyCode::Char('j')).expect("down");
    let was = app.line_index();
    assert!(was > 0, "the fixture has enough lines to move");

    app.on_key(KeyCode::Char(']')).expect("next file");
    assert_eq!(
        app.line_index(),
        0,
        "a file being opened for the first time opens at its top"
    );
    app.on_key(KeyCode::Char('[')).expect("back");

    assert_eq!(app.line_index(), was, "the line came back with the file");
}

/// Each file remembers its own place, not one place shared between them.
#[test]
fn each_file_keeps_its_own_place() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    app.on_key(KeyCode::Char('j')).expect("down");
    app.on_key(KeyCode::Char(']')).expect("next file");
    app.on_key(KeyCode::Char('j')).expect("down");
    app.on_key(KeyCode::Char('j')).expect("down");
    assert_eq!(app.line_index(), 2, "the second file is two lines down");

    app.on_key(KeyCode::Char('[')).expect("back");
    assert_eq!(app.line_index(), 1, "the first file is one line down");
    app.on_key(KeyCode::Char(']')).expect("forward again");
    assert_eq!(app.line_index(), 2, "and the second is still two");
}

#[test]
fn file_navigation_keys_work_from_either_pane() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    app.on_key(KeyCode::Char(']')).expect("next file");
    assert_eq!(app.file_index(), 1);
    app.on_key(KeyCode::Left).expect("focus files");
    app.on_key(KeyCode::Char('[')).expect("previous file");
    assert_eq!(app.file_index(), 0);
}

#[test]
fn frame_renders_file_list_and_diff() {
    let workspace = Fixture::new();
    let app = workspace.app();

    let rendered = render(&app);

    assert!(rendered.contains("a.rs"), "{rendered}");
    assert!(rendered.contains("let x = 1;"), "{rendered}");
}

// ---------------------------------------------------------------------------
// The comment stack
// ---------------------------------------------------------------------------

/// `Enter` steps into the comments on the selected line, and `Esc` steps back
/// out — the round trip a reviewer makes to pick one comment out of a stack.
#[test]
fn enter_steps_into_the_comment_stack_and_esc_leaves_it() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, "needs a doc");

    app.on_key(KeyCode::Enter).expect("enter the stack");
    assert_eq!(app.focus(), Focus::Stack);
    assert_eq!(
        app.comment_index(),
        0,
        "the stack opens on its first comment"
    );
    assert_eq!(
        app.selected_comment().expect("a selected comment").body,
        "needs a doc"
    );

    app.on_key(KeyCode::Esc).expect("leave the stack");
    assert_eq!(app.focus(), Focus::Diff);
    assert!(
        app.selected_comment().is_none(),
        "nothing is selected once the cursor is back on the diff"
    );
}

/// A focus a reviewer cannot get out of is a trap, so `Left` leaves the stack
/// as surely as `Esc` does — the same key that leaves every other focus.
#[test]
fn left_also_leaves_the_stack() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, "needs a doc");

    app.on_key(KeyCode::Enter).expect("enter the stack");
    assert_eq!(app.focus(), Focus::Stack);
    app.on_key(KeyCode::Left).expect("left");
    assert_eq!(app.focus(), Focus::Diff);
}

/// `Enter` on a line with nothing on it says so rather than moving the cursor
/// into an empty stack, which would be a focus with nothing in it and no
/// obvious way back.
#[test]
fn enter_on_a_line_without_comments_says_so_and_stays_put() {
    let workspace = Fixture::new();
    let mut app = workspace.app();

    app.on_key(KeyCode::Enter).expect("enter");

    assert_eq!(app.focus(), Focus::Diff, "focus did not move");
    assert!(
        app.status().contains("no comments"),
        "and it said why: {:?}",
        app.status()
    );
}

/// Inside the stack the movement keys move between comments, and they clamp at
/// both ends the way they do everywhere else in the reviewer.
#[rstest]
#[case(KeyCode::Char('j'), KeyCode::Char('k'))]
#[case(KeyCode::Down, KeyCode::Up)]
fn both_key_pairs_move_between_the_comments_in_a_stack(
    #[case] forward: KeyCode,
    #[case] back: KeyCode,
) {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, "first finding");
    write_comment(&mut app, "second finding");

    app.on_key(KeyCode::Enter).expect("enter the stack");
    assert_eq!(
        app.selected_comment().expect("the first").body,
        "first finding",
        "the stack opens on the oldest comment"
    );

    app.on_key(forward).expect("next");
    assert_eq!(
        app.selected_comment().expect("the second").body,
        "second finding"
    );
    app.on_key(forward).expect("past the end");
    assert_eq!(
        app.selected_comment().expect("still the second").body,
        "second finding",
        "the cursor stops at the newest rather than wrapping"
    );

    app.on_key(back).expect("back");
    assert_eq!(
        app.selected_comment().expect("the first again").body,
        "first finding"
    );
    app.on_key(back).expect("past the start");
    assert_eq!(
        app.selected_comment().expect("still the first").body,
        "first finding"
    );
    assert_eq!(
        app.line_index(),
        0,
        "moving inside the stack did not move the diff underneath it"
    );
}

/// `c` means the same thing inside the stack as outside it: another comment on
/// the line the reviewer is looking at, added to the end of that line's stack.
#[test]
fn c_from_the_stack_adds_another_comment_to_the_same_line() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, "first finding");
    let line = app.line_index();

    app.on_key(KeyCode::Enter).expect("enter the stack");
    write_comment(&mut app, "second finding");

    let on_line = app.comments_for_line(line);
    assert_eq!(on_line.len(), 2, "both are on the line: {on_line:?}");
    assert_eq!(on_line[1].body, "second finding", "the new one is last");
    assert_eq!(
        app.focus(),
        Focus::Stack,
        "saving from the stack leaves the cursor where it was"
    );
    assert_eq!(
        workspace.store().comments().expect("read comments").len(),
        2,
        "and both reached the store"
    );
}

/// The stack index belongs to the line it was opened on, so moving the
/// selection puts it back at the top rather than leaving it pointing at another
/// line's comment.
#[test]
fn moving_the_selection_puts_the_stack_index_back_at_the_top() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, "first finding");
    write_comment(&mut app, "second finding");

    app.on_key(KeyCode::Enter).expect("enter the stack");
    app.on_key(KeyCode::Char('j')).expect("select the second");
    assert_eq!(app.comment_index(), 1);

    app.on_key(KeyCode::Left).expect("back to the diff");
    app.on_key(KeyCode::Char('j')).expect("next line");
    assert_eq!(app.focus(), Focus::Diff);
    assert_eq!(app.comment_index(), 0, "the stack index came back to 0");
}

/// Navigating out of a stack leaves it — **whatever is on the line navigated
/// to**.
///
/// Entering a stack is something a reviewer does on purpose, with `Enter`, on a
/// line they picked. `]` is not that, so it may not hand the focus on: landing
/// inside the next file's stack, one the reviewer never opened, points `d` and
/// `s` at a comment they have not seen and did not select — and `d` is
/// unrecoverable.
///
/// Both files carry a comment on the line `]` lands on, which is the whole
/// point of the fixture below. An earlier version of this test commented on one
/// file only, so the focus left the stack because the new line's stack was
/// *empty*; it passed against an implementation that kept the focus whenever
/// the new line had comments, which is the bug. The `stack ahead` assertion is
/// there to keep it from going quiet that way again.
#[test]
fn navigating_to_another_file_leaves_the_stack_even_when_that_line_has_comments() {
    let workspace = Fixture::new();
    let mut app = workspace.app();

    // A comment on b.rs's first line, reached and left the way a reviewer
    // would, so that `]` below lands on a line that is *not* comment-free.
    app.on_key(KeyCode::Char(']')).expect("next file");
    write_comment(&mut app, "on the second file");
    app.on_key(KeyCode::Char('[')).expect("back to the first");
    assert_eq!(app.file_index(), 0);

    write_comment(&mut app, "first finding");
    write_comment(&mut app, "second finding");
    app.on_key(KeyCode::Enter).expect("enter the stack");
    app.on_key(KeyCode::Char('j')).expect("select the second");
    assert_eq!(app.focus(), Focus::Stack);
    assert_eq!(app.comment_index(), 1);

    app.on_key(KeyCode::Char(']')).expect("next file");

    assert_eq!(
        app.comments_for_line(app.line_index()).len(),
        1,
        "the line `]` landed on has no stack, so this test proves nothing"
    );
    assert_eq!(
        app.focus(),
        Focus::Diff,
        "`]` carried the cursor into a stack the reviewer never entered"
    );
    assert_eq!(app.comment_index(), 0, "the stack index came back to 0");
    assert!(
        app.selected_comment().is_none(),
        "a comment is selected on a line the reviewer only just arrived at: {:?}",
        app.selected_comment()
    );
}

/// The same rule on the way back: `[` out of a stack lands on the diff, not
/// inside the previous file's stack.
#[test]
fn navigating_back_to_a_file_with_comments_also_leaves_the_stack() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, "on the first file");

    app.on_key(KeyCode::Char(']')).expect("next file");
    write_comment(&mut app, "on the second file");
    app.on_key(KeyCode::Enter).expect("enter the stack");
    assert_eq!(app.focus(), Focus::Stack);

    app.on_key(KeyCode::Char('[')).expect("back to the first");

    assert_eq!(
        app.comments_for_line(app.line_index()).len(),
        1,
        "the line `[` landed on has no stack, so this test proves nothing"
    );
    assert_eq!(
        app.focus(),
        Focus::Diff,
        "`[` carried the cursor into a stack the reviewer never entered"
    );
}

// ---------------------------------------------------------------------------
// Deleting a comment
// ---------------------------------------------------------------------------

/// `d` asks before it deletes, and `y` answers. Deletion is unrecoverable, so
/// the one thing that must never happen is a mistyped key costing written work.
#[test]
fn d_then_y_deletes_the_comment_from_the_store() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, "needs a doc");
    let line = app.line_index();

    app.on_key(KeyCode::Char('d')).expect("ask");
    assert!(
        matches!(app.mode(), Mode::ConfirmDelete { .. }),
        "it asked first, rather than deleting: {:?}",
        app.mode()
    );
    assert!(
        app.status().contains("delete") && app.status().contains("a.rs:1"),
        "and said what it would delete: {:?}",
        app.status()
    );
    assert_eq!(
        workspace.store().comments().expect("read").len(),
        1,
        "asking the question did not delete anything on its own"
    );

    app.on_key(KeyCode::Char('y')).expect("confirm");

    assert_eq!(app.mode(), Mode::Browse);
    assert!(
        app.comments_for_line(line).is_empty(),
        "gone from the view: {:?}",
        app.comments_for_line(line)
    );
    assert!(
        workspace.store().comments().expect("read").is_empty(),
        "gone from a freshly opened store, which is the authority"
    );
}

/// Neither answer to `d` rewrites `REVIEW-FEEDBACK.md`. The markdown is an
/// *export* (see the storage-model spec) produced by `rv render`, and the store
/// is what a review is kept in; a delete that rewrote the export would be
/// reaching past the store to edit a document somebody else may be reading.
///
/// Both answers, in *this* file, because they fail differently. A confirmed
/// delete that rewrote the export drops whatever reply an LLM appended; a
/// **cancelled** one does that while the reviewer is being told nothing
/// happened, which is the worse of the two and the more likely keystroke — `d`
/// is next to `s` and `f`, and the answer to a mistyped one is `n`. The cancel
/// path had one guard, inside `--test app_cases`'s fuzz walk, and the two
/// targets are run separately: a wave that broke it while working in this file
/// would have seen this file stay green.
#[rstest]
#[case::confirmed(KeyCode::Char('y'), 0)]
#[case::cancelled(KeyCode::Char('n'), 1)]
fn deleting_a_comment_does_not_rewrite_the_export(#[case] answer: KeyCode, #[case] left: usize) {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, "needs a doc");

    // Seed an export by hand, so that what is on disk cannot have come from
    // this delete: any rewrite would drop the comment and this sentence both.
    const SEEDED: &str = "<!-- rv:v1 -->\nstale on purpose\n";
    let export = workspace.store().markdown_path();
    fs::write(&export, SEEDED).expect("seed an export");
    let before = fs::metadata(&export)
        .expect("stat")
        .modified()
        .expect("mtime");

    app.on_key(KeyCode::Char('d')).expect("ask");
    app.on_key(answer).expect("answer");

    assert_eq!(
        workspace.store().comments().expect("read").len(),
        left,
        "{answer:?} did not do what it says on the tin, so this proves nothing"
    );
    assert_eq!(
        fs::read_to_string(&export).expect("read the export"),
        SEEDED,
        "{answer:?} rewrote the export"
    );
    assert_eq!(
        fs::metadata(&export)
            .expect("stat")
            .modified()
            .expect("mtime"),
        before,
        "{answer:?} rewrote the export, even if with the same bytes"
    );
}

/// The other answer. `n` — or anything that is not `y` — leaves the comment
/// exactly where it was, in the view and on disk.
#[test]
fn d_then_anything_else_cancels_and_keeps_the_comment() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, "needs a doc");
    let line = app.line_index();

    app.on_key(KeyCode::Char('d')).expect("ask");
    app.on_key(KeyCode::Char('n')).expect("decline");

    assert_eq!(app.mode(), Mode::Browse);
    assert!(
        app.status().contains("cancelled"),
        "the reviewer is told nothing happened: {:?}",
        app.status()
    );
    assert_eq!(app.comments_for_line(line).len(), 1, "still there");
    let stored = workspace.store().comments().expect("read");
    assert_eq!(stored.len(), 1, "and still on disk: {stored:?}");
    assert_eq!(stored[0].body, "needs a doc");
}

/// No keystroke leaves the reviewer stuck at the question. Whatever is pressed,
/// the confirmation is answered and the app is back in `Browse` — deleting on
/// `y` and on nothing else.
#[rstest]
#[case::confirm(KeyCode::Char('y'), true)]
#[case::decline(KeyCode::Char('n'), false)]
#[case::uppercase_is_not_a_confirmation(KeyCode::Char('Y'), false)]
#[case::quit_does_not_leak_out_of_the_question(KeyCode::Char('q'), false)]
#[case::another_d(KeyCode::Char('d'), false)]
#[case::comment_key(KeyCode::Char('c'), false)]
#[case::escape(KeyCode::Esc, false)]
#[case::enter(KeyCode::Enter, false)]
#[case::space(KeyCode::Char(' '), false)]
#[case::backspace(KeyCode::Backspace, false)]
#[case::arrow(KeyCode::Left, false)]
#[case::movement(KeyCode::Down, false)]
#[case::tab(KeyCode::Tab, false)]
#[case::function(KeyCode::F(1), false)]
fn every_key_answers_the_confirmation(#[case] key: KeyCode, #[case] deletes: bool) {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, "needs a doc");

    app.on_key(KeyCode::Char('d')).expect("ask");
    let action = app.on_key(key).expect("answer");

    assert_eq!(
        action,
        Action::Continue,
        "{key:?} ended the review from inside a confirmation"
    );
    assert_eq!(
        app.mode(),
        Mode::Browse,
        "{key:?} left the reviewer waiting on a question it will never be asked again"
    );
    assert_eq!(
        workspace.store().comments().expect("read").len(),
        usize::from(!deletes),
        "{key:?} deleted the wrong number of comments"
    );
    // ...and the keystroke was consumed by the answer rather than also doing
    // whatever it means while browsing.
    assert_eq!(app.buffer(), "", "{key:?} opened a comment buffer");
    assert_eq!(app.focus(), Focus::Diff);
}

/// From the diff, `d` targets the newest comment on the line — the one a
/// reviewer has just written and is most likely to want back — and says which
/// of how many went.
#[test]
fn from_the_diff_d_targets_the_newest_and_reports_how_many_there_were() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, "first finding");
    write_comment(&mut app, "second finding");
    let line = app.line_index();

    app.on_key(KeyCode::Char('d')).expect("ask");
    app.on_key(KeyCode::Char('y')).expect("confirm");

    let left = app.comments_for_line(line);
    assert_eq!(left.len(), 1, "{left:?}");
    assert_eq!(left[0].body, "first finding", "the newest went");
    assert!(
        app.status().contains("1 of 2"),
        "and it said so: {:?}",
        app.status()
    );
    let stored = workspace.store().comments().expect("read");
    assert_eq!(stored.len(), 1, "{stored:?}");
    assert_eq!(stored[0].body, "first finding");
}

/// From inside the stack, `d` targets what the cursor is on. The two rules have
/// to differ: on the diff there is no cursor in the stack to mean anything.
#[test]
fn from_the_stack_d_targets_the_selected_comment() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, "first finding");
    write_comment(&mut app, "second finding");
    let line = app.line_index();

    app.on_key(KeyCode::Enter).expect("enter the stack");
    assert_eq!(
        app.selected_comment().expect("a selection").body,
        "first finding",
        "the cursor is on the oldest, which is not the one `d` would take from the diff"
    );
    app.on_key(KeyCode::Char('d')).expect("ask");
    app.on_key(KeyCode::Char('y')).expect("confirm");

    let left = app.comments_for_line(line);
    assert_eq!(left.len(), 1, "{left:?}");
    assert_eq!(left[0].body, "second finding", "the selected one went");
    assert_eq!(
        app.focus(),
        Focus::Stack,
        "a stack with a comment left in it keeps the cursor"
    );
    assert_eq!(
        app.selected_comment().expect("a selection").body,
        "second finding",
        "and the cursor is clamped onto what is left"
    );
}

/// Deleting the last comment on a line empties the stack, so the cursor comes
/// back to the diff rather than sitting in a pane with nothing in it.
#[test]
fn deleting_the_last_comment_on_a_line_returns_focus_to_the_diff() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, "needs a doc");

    app.on_key(KeyCode::Enter).expect("enter the stack");
    app.on_key(KeyCode::Char('d')).expect("ask");
    app.on_key(KeyCode::Char('y')).expect("confirm");

    assert_eq!(app.focus(), Focus::Diff, "no cursor left in an empty stack");
    assert_eq!(app.comment_index(), 0);
    assert!(app.selected_comment().is_none());
}

/// From the file list, `d` deletes nothing and says what it would need.
///
/// `c` does write against the selected diff line from the sidebar, and the
/// symmetry argues for `d` doing the same — but the two keys are not
/// symmetrical. `c` creates, and a comment made by mistake is undone by `d`;
/// `d` destroys, and nothing undoes it. The file list shows files, so the
/// comment `d` would take from there is one the reviewer cannot see, on a diff
/// line they may never have opened. The sidebar's *other* tab does have a
/// comment of its own selected, and `d` deletes it — see
/// `d_from_the_comment_browser_deletes_behind_the_same_confirmation`.
#[test]
fn d_from_the_file_list_deletes_nothing() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, "needs a doc");
    let line = app.line_index();

    app.on_key(KeyCode::Left).expect("focus the file list");
    assert_eq!(app.focus(), Focus::Sidebar);
    app.on_key(KeyCode::Char('d')).expect("d");

    assert_eq!(
        app.mode(),
        Mode::Browse,
        "it opened a confirmation about a comment the file list does not show"
    );
    assert!(
        app.status().contains("not comments"),
        "and it said what it would need instead: {:?}",
        app.status()
    );
    assert_eq!(app.comments_for_line(line).len(), 1, "still there");
    assert_eq!(
        workspace.store().comments().expect("read").len(),
        1,
        "and still on disk"
    );

    // ...and pressing `y` next does not delete it either: there is no question
    // outstanding for `y` to be the answer to.
    app.on_key(KeyCode::Char('y')).expect("y");
    assert_eq!(
        workspace.store().comments().expect("read").len(),
        1,
        "a `d` that refused still armed the confirmation"
    );
}

/// `d` on a line with nothing on it says so and stays in `Browse`: there is no
/// question to ask, so asking one would be a state the reviewer has to escape
/// for no reason.
#[test]
fn d_with_nothing_to_delete_says_so() {
    let workspace = Fixture::new();
    let mut app = workspace.app();

    app.on_key(KeyCode::Char('d')).expect("d");

    assert_eq!(app.mode(), Mode::Browse);
    assert!(app.status().contains("no comments"), "{:?}", app.status());
}

// ---------------------------------------------------------------------------
// Collapsing a box
// ---------------------------------------------------------------------------

/// `s` is a toggle: the boxes on the selected line fold away and come back.
#[test]
fn s_collapses_and_expands_the_boxes_on_the_selected_line() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, "needs a doc");
    let id = app.comments()[0].id.clone();

    assert!(
        app.collapsed().is_empty(),
        "a comment is drawn open until the reviewer folds it"
    );
    app.on_key(KeyCode::Char('s')).expect("collapse");
    assert!(app.collapsed().contains(&id), "{:?}", app.collapsed());

    app.on_key(KeyCode::Char('s')).expect("expand");
    assert!(!app.collapsed().contains(&id), "{:?}", app.collapsed());
}

/// From the diff, `s` acts on the whole line: the reviewer is folding a *line*
/// away, and leaving half of its stack open would not do that. Mixed states
/// collapse first, so one press always gets a line out of the way.
#[test]
fn from_the_diff_s_folds_the_whole_line_together() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, "first finding");
    write_comment(&mut app, "second finding");
    let line = app.line_index();
    let ids: Vec<String> = app
        .comments_for_line(line)
        .iter()
        .map(|comment| comment.id.clone())
        .collect();
    assert_eq!(ids.len(), 2);

    // Fold just one of them, from inside the stack, so the line is mixed.
    app.on_key(KeyCode::Enter).expect("enter the stack");
    app.on_key(KeyCode::Char('s')).expect("collapse the first");
    app.on_key(KeyCode::Esc).expect("back to the diff");
    assert_eq!(app.collapsed().len(), 1);

    app.on_key(KeyCode::Char('s')).expect("fold the line");
    assert!(
        ids.iter().all(|id| app.collapsed().contains(id)),
        "a mixed line collapses the rest rather than expanding the one: {:?}",
        app.collapsed()
    );

    app.on_key(KeyCode::Char('s')).expect("unfold the line");
    assert!(
        ids.iter().all(|id| !app.collapsed().contains(id)),
        "an all-collapsed line expands together: {:?}",
        app.collapsed()
    );
}

/// From inside the stack, `s` folds the one box the cursor is on — the reason
/// there is a cursor in there at all.
#[test]
fn from_the_stack_s_collapses_only_the_selected_box() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, "first finding");
    write_comment(&mut app, "second finding");
    let first = app.comments_for_line(app.line_index())[0].id.clone();

    app.on_key(KeyCode::Enter).expect("enter the stack");
    app.on_key(KeyCode::Char('s')).expect("collapse the first");

    assert!(app.collapsed().contains(&first), "{:?}", app.collapsed());
    assert_eq!(app.collapsed().len(), 1, "the other box is untouched");

    app.on_key(KeyCode::Char('j')).expect("select the second");
    app.on_key(KeyCode::Char('s')).expect("collapse the second");
    assert_eq!(app.collapsed().len(), 2, "and now both are folded");
}

/// From the sidebar's **Comments** tab, `s` folds the comment the browser's
/// cursor is on — the same rule `d` follows there, and for the same reason: a
/// key pressed in the browser acts on what the browser is showing.
///
/// The browsed comment is deliberately anchored in the *other* file from the
/// one the diff cursor is in, so the rule this replaces — fold the selected
/// line's boxes — folds nothing at all here and cannot pass by coincidence.
#[test]
fn from_the_comments_tab_s_folds_the_browsed_comment() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    app.on_key(KeyCode::Char(']')).expect("next file");
    write_comment(&mut app, "on the second file");
    let id = app.comments()[0].id.clone();
    app.on_key(KeyCode::Char('['))
        .expect("back to the first file");
    assert!(
        app.comments_for_line(app.line_index()).is_empty(),
        "the cursor is on a line that has comments, so the line rule would pass by luck"
    );

    app.on_key(KeyCode::Tab).expect("comments tab");
    app.on_key(KeyCode::Left).expect("focus the sidebar");
    app.on_key(KeyCode::Char('s'))
        .expect("fold the browsed comment");

    assert!(
        app.collapsed().contains(&id),
        "`s` in the browser folded something other than the comment it is showing: {:?}",
        app.collapsed()
    );
    assert!(
        !app.status().contains("no comments"),
        "it refused and folded anyway: {:?}",
        app.status()
    );

    app.on_key(KeyCode::Char('s')).expect("unfold it again");
    assert!(
        app.collapsed().is_empty(),
        "it is a toggle in the browser too: {:?}",
        app.collapsed()
    );
}

/// ...and it is the *browsed* comment rather than the line's, with both on
/// screen: the cursor sits on the first comment's line while the browser is on
/// the second.
#[test]
fn from_the_comments_tab_s_folds_the_browsed_comment_not_the_selected_lines() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, "first finding");
    let first = app.comments()[0].id.clone();
    app.on_key(KeyCode::Char('j')).expect("next line");
    write_comment(&mut app, "second finding");
    let second = app.comments()[1].id.clone();
    assert_ne!(first, second);
    app.on_key(KeyCode::Char('k'))
        .expect("back onto the first comment's line");

    app.on_key(KeyCode::Tab).expect("comments tab");
    app.on_key(KeyCode::Left).expect("focus the sidebar");
    app.on_key(KeyCode::Down).expect("browse to the second");
    app.on_key(KeyCode::Char('s')).expect("fold it");

    assert!(
        app.collapsed().contains(&second),
        "the browsed comment is still open: {:?}",
        app.collapsed()
    );
    assert!(
        !app.collapsed().contains(&first),
        "the comment on the selected diff line was folded instead: {:?}",
        app.collapsed()
    );
}

/// The **Files** tab keeps the older rule, because a file row selects no
/// comment: `s` there folds the boxes on the diff line the reviewer left the
/// cursor on, which is the only comment the screen is showing them.
#[test]
fn from_the_files_tab_s_still_folds_the_selected_lines_boxes() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, "needs a doc");
    let id = app.comments()[0].id.clone();

    app.on_key(KeyCode::Left).expect("focus the file list");
    assert_eq!(app.sidebar_tab(), SidebarTab::Files);
    app.on_key(KeyCode::Char('s')).expect("fold");

    assert!(
        app.collapsed().contains(&id),
        "`s` from the file list stopped folding the selected line: {:?}",
        app.collapsed()
    );
}

/// `s` with an empty browser folds nothing and says why — and says it about the
/// review rather than about a line, because a line is not what the reviewer was
/// looking at.
#[test]
fn from_an_empty_comment_browser_s_says_so() {
    let workspace = Fixture::new();
    let mut app = workspace.app();

    app.on_key(KeyCode::Tab).expect("comments tab");
    app.on_key(KeyCode::Left).expect("focus the sidebar");
    app.on_key(KeyCode::Char('s')).expect("s");

    assert!(app.collapsed().is_empty(), "{:?}", app.collapsed());
    assert!(app.status().contains("no comments"), "{:?}", app.status());
    assert!(
        !app.status().contains("this line"),
        "the browser refused with a sentence about a line it is not showing: {:?}",
        app.status()
    );
}

/// Collapse is a *view* preference, held for this session only. It is not
/// review state: another reviewer opening the same `.review/` has their own
/// idea of which boxes are in their way, and an export written from a folded
/// screen must not be a folded document.
#[test]
fn collapse_state_never_reaches_disk() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, "needs a doc");

    let before = workspace_tree(workspace.root());
    assert!(!before.is_empty(), "the review wrote nothing to compare");

    app.on_key(KeyCode::Char('s')).expect("collapse");
    assert_eq!(app.collapsed().len(), 1, "nothing was collapsed");

    let after = workspace_tree(workspace.root());
    assert_eq!(
        after, before,
        "collapsing wrote to the workspace; it is a view preference, not review state"
    );
    for (path, _, bytes) in after
        .iter()
        .filter(|(path, ..)| path.starts_with(".review"))
    {
        assert!(
            !String::from_utf8_lossy(bytes).contains("collaps"),
            "{path} mentions collapsing"
        );
    }

    // ...and a reviewer who reopens the review finds every box open again.
    let reopened = workspace.app();
    assert!(
        reopened.collapsed().is_empty(),
        "collapse survived the process it was a preference of: {:?}",
        reopened.collapsed()
    );
}

/// A comment that is deleted is not a folded comment, so its id does not stay
/// in the fold set — where it would fold whatever later comment hashed to the
/// same id (the same body, on the same line) under a preference about a
/// comment the reviewer threw away.
#[test]
fn deleting_a_folded_comment_forgets_that_it_was_folded() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, "needs a doc");
    let id = app.comments()[0].id.clone();

    app.on_key(KeyCode::Char('s')).expect("fold it away");
    assert!(app.collapsed().contains(&id));

    app.on_key(KeyCode::Char('d')).expect("ask");
    app.on_key(KeyCode::Char('y')).expect("confirm");

    assert!(
        app.collapsed().is_empty(),
        "the deleted comment is still folded: {:?}",
        app.collapsed()
    );

    // Retyped, the same comment comes back open.
    write_comment(&mut app, "needs a doc");
    assert_eq!(
        app.comments()[0].id,
        id,
        "the id is derived, so it is the same"
    );
    assert!(
        app.collapsed().is_empty(),
        "a fresh comment inherited a fold: {:?}",
        app.collapsed()
    );
}

// ---------------------------------------------------------------------------
// Drawing a comment box, and which pane has the focus
// ---------------------------------------------------------------------------

/// A saved comment is drawn as a bordered box hanging off the line it is
/// anchored to — the whole point of this milestone, and the thing a reviewer
/// could not see at all before it.
///
/// Asserted on the *cells* rather than on the text: "blue and bordered" is the
/// requirement, and a test that only greps for the body passes against an
/// unstyled box. The rounded corners are what distinguish a comment box from
/// the panes' own plain borders.
#[test]
fn a_comment_renders_as_a_blue_bordered_box_under_its_line() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    select_line(&mut app, |line| line.text.contains("let x = 1;"));
    write_comment(&mut app, "needs a doc");

    let buffer = frame_at(&app, 100, 24);
    let text = buffer_text(&buffer);
    assert!(
        text.contains("needs a doc"),
        "the body is on screen:\n{text}"
    );
    // Asked inside the diff pane: the panes' own corners are rounded too, so a
    // `╭` at the edge of the frame is a frame rather than a box.
    let inside = text_in(&buffer, box_area());
    assert!(
        inside.contains('╭') && inside.contains('╰'),
        "the box has borders:\n{text}"
    );
    assert!(
        styled_blue_in(&buffer, box_area(), '╭'),
        "the border is blue, which is the requirement:\n{text}"
    );
    assert!(
        styled_blue_in(&buffer, box_area(), '╰'),
        "and so is its other end:\n{text}"
    );

    // ...and it hangs off *its own* line, in order: top border, body, bottom.
    let rows = rows_of(&buffer);
    let anchored = row_holding(&buffer, "let x = 1;");
    assert!(
        rows[anchored + 1].contains('╭'),
        "the box does not open directly under the line it is about:\n{text}"
    );
    assert!(
        rows[anchored + 2].contains("needs a doc") && rows[anchored + 2].contains('│'),
        "the body is not inside the box:\n{text}"
    );
    assert!(
        rows[anchored + 3].contains('╰'),
        "the box does not close under its body:\n{text}"
    );
}

/// The box is indented to the diff's gutter, so it reads as hanging off the
/// line rather than as another pane.
#[test]
fn a_comment_box_is_indented_to_the_diff_gutter() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, "needs a doc");

    let buffer = frame_at(&app, 100, 24);
    let (corner_x, _) = find_char_in(&buffer, box_area(), '╭').expect("a box top is on screen");
    // Counted in characters, not bytes: the panes' own borders are multi-byte,
    // so a byte offset is not a column.
    let sigil_row = rows_of(&buffer)[row_holding(&buffer, "+fn a() {")].clone();
    let sigil = sigil_row
        .char_indices()
        .position(|(offset, _)| sigil_row[offset..].starts_with("+fn a() {"))
        .expect("the added line carries its sigil");

    assert_eq!(
        usize::from(corner_x),
        sigil + 1,
        "the box does not start one column past the sigil, where the line's own \
         text starts:\n{}",
        buffer_text(&buffer)
    );
}

/// Border and title are blue; the body keeps the terminal's own foreground, so
/// the part being *read* is at full contrast.
#[test]
fn the_box_body_keeps_the_default_foreground() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, "needs a doc");

    let buffer = frame_at(&app, 100, 24);
    let body_row = u16::try_from(row_holding(&buffer, "needs a doc")).expect("a small row");

    assert_eq!(
        style_of_text(&buffer, body_row, "needs a doc").fg,
        Some(Color::Reset),
        "the comment body is recoloured, which is what makes it hard to read:\n{}",
        buffer_text(&buffer)
    );
    // The box's own left side, not the sidebar's border, which is the first
    // `│` on the row.
    assert_eq!(
        style_of_text(&buffer, body_row, "│ needs a doc").fg,
        Some(Color::Blue),
        "the box's side is not blue:\n{}",
        buffer_text(&buffer)
    );
}

/// Writes `reply` under every entry of the export and folds it back into the
/// store, which is the two halves of the LLM loop: the agent appends to the
/// document, and the next rewrite of it moves what the agent wrote into
/// `comments.json`.
///
/// Through the document rather than by editing the store, because that is the
/// only way a reply is ever created — there is no key for one — and a fixture
/// that invented one would be testing a state the product cannot reach.
fn reply_through_the_document(workspace: &Fixture, reply: &str) {
    let replied = insert_reply(&workspace.markdown(), reply);
    fs::write(
        workspace.root().join(".review/REVIEW-FEEDBACK.md"),
        &replied,
    )
    .expect("write the replied-to markdown");
    let review = session::build(workspace.root(), None, None).expect("build the review");
    session::write_markdown(&review).expect("fold the reply back into the store");
}

/// A reply is drawn inside the comment's own box, dimmed: it is part of that
/// conversation, and a reviewer scanning a screen of boxes has to be able to
/// tell their own words from the agent's answer without reading either.
///
/// Asserted on the **style**, not only on the text — the `reply:` prefix has
/// been on screen since the row model shipped, and a test that greps for it
/// passes against a reply drawn exactly like the body above it. The body is
/// checked in the same frame as the control: if everything were dimmed, nothing
/// would be.
#[test]
fn a_reply_renders_dimmed_inside_the_same_box() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, "needs a doc");
    reply_through_the_document(&workspace, "added one");
    drop(app);

    // Reopened, because the store is where the folded reply landed.
    let app = workspace.app();
    assert_eq!(
        app.comments()[0].reply.as_deref(),
        Some("added one"),
        "the fixture never got a reply into the store, so this proves nothing"
    );

    let buffer = frame_at(&app, 100, 24);
    let text = buffer_text(&buffer);
    let reply_row = u16::try_from(row_holding(&buffer, "reply: added one")).expect("a small row");
    let body_row = u16::try_from(row_holding(&buffer, "needs a doc")).expect("a small row");

    assert!(
        rows_of(&buffer)[usize::from(reply_row)].contains('│'),
        "the reply is not inside a box:\n{text}"
    );
    assert_eq!(
        reply_row,
        body_row + 1,
        "the reply is not under the body it answers:\n{text}"
    );

    let reply_style = style_of_text(&buffer, reply_row, "reply: added one");
    assert!(
        reply_style.add_modifier.contains(Modifier::DIM),
        "the reply is drawn exactly like the comment it answers:\n{text}"
    );
    assert_eq!(
        reply_style.fg,
        Some(Color::Reset),
        "the reply was recoloured rather than dimmed, which costs it the \
         contrast the body has:\n{text}"
    );
    assert!(
        !style_of_text(&buffer, body_row, "needs a doc")
            .add_modifier
            .contains(Modifier::DIM),
        "the comment's own body is dimmed too, so nothing tells the two apart:\n{text}"
    );
}

/// Focus is shown with a `▸` on the focused pane's title and a bold border —
/// never with colour, because blue already means "comment".
#[test]
fn the_focused_pane_is_marked() {
    let workspace = Fixture::new();
    let mut app = workspace.app();

    let diff_focused = buffer_text(&frame_at(&app, 100, 24));
    app.on_key(KeyCode::Left).expect("focus files");
    let files_focused = buffer_text(&frame_at(&app, 100, 24));

    assert_ne!(
        diff_focused, files_focused,
        "focus is invisible on screen:\n{files_focused}"
    );
    assert!(
        files_focused.contains("▸ Files"),
        "the focused pane's title is not marked:\n{files_focused}"
    );
    assert!(
        !files_focused.contains("▸ a.rs"),
        "the unfocused diff is marked too:\n{files_focused}"
    );
    assert!(
        diff_focused.contains("▸ a.rs") && !diff_focused.contains("▸ Files"),
        "the mark did not move with the focus:\n{diff_focused}"
    );
}

/// The same, in the borders: exactly one pane is bold, and it is the one the
/// next keystroke lands in.
#[test]
fn only_the_focused_panes_border_is_bold() {
    let workspace = Fixture::new();
    let mut app = workspace.app();

    // The bar is along the bottom, so both panes start at row 0 and row 1 is a
    // border cell of each: the sidebar's left edge at column 0, and the diff's
    // at column 30 — 30% of 100, less the divider column the sidebar gives up.
    let bold = |app: &App| {
        let buffer = frame_at(app, 100, 24);
        (
            buffer[(0, 1)].modifier.contains(Modifier::BOLD),
            buffer[(30, 1)].modifier.contains(Modifier::BOLD),
        )
    };

    assert_eq!(bold(&app), (false, true), "the diff has focus on launch");
    app.on_key(KeyCode::Left).expect("focus files");
    assert_eq!(bold(&app), (true, false), "the mark did not move");
    app.on_key(KeyCode::Right).expect("focus the diff");
    assert_eq!(bold(&app), (false, true), "and did not come back");
}

/// The sidebar's selection is `REVERSED` only while the sidebar has the focus;
/// unfocused it drops to a dim mark, so there is exactly one place on screen
/// the next keystroke will land.
#[test]
fn the_unfocused_sidebar_dims_its_selection() {
    let workspace = Fixture::new();
    let mut app = workspace.app();

    // Scanned inside the sidebar's own columns: the diff pane's title names
    // `a.rs` too, and it is the *list row* whose highlight this is about.
    let reversed = |app: &App| {
        let buffer = frame_at(app, 100, 24);
        (0..24).any(|y| {
            let row: String = (0..30).map(|x| buffer[(x, y)].symbol()).collect();
            row.contains("a.rs")
                && (0..30).any(|x| buffer[(x, y)].modifier.contains(Modifier::REVERSED))
        })
    };

    assert!(!reversed(&app), "the unfocused file list is still reversed");
    app.on_key(KeyCode::Left).expect("focus files");
    assert!(reversed(&app), "the focused file list lost its highlight");
}

/// Inside a stack the selected box is brighter and bold, so a reviewer can see
/// which of several comments `d` and `s` are aimed at.
#[test]
fn the_selected_box_in_the_stack_is_brighter_and_bold() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, "first finding");
    write_comment(&mut app, "second finding");

    let browsing = frame_at(&app, 100, 24);
    let first_corner = find_char_in(&browsing, box_area(), '╭').expect("a box top");
    assert_eq!(
        browsing[first_corner].style().fg,
        Some(Color::Blue),
        "an unselected box is not plain blue"
    );

    app.on_key(KeyCode::Enter).expect("enter the stack");
    let selected = frame_at(&app, 100, 24);
    let corner = &selected[first_corner];
    assert_eq!(
        corner.style().fg,
        Some(Color::LightBlue),
        "the selected box is not brighter:\n{}",
        buffer_text(&selected)
    );
    assert!(
        corner.modifier.contains(Modifier::BOLD),
        "the selected box is not bold:\n{}",
        buffer_text(&selected)
    );

    // ...and the box the cursor is *not* on stays plain, so "selected" means
    // one box rather than the whole stack.
    app.on_key(KeyCode::Char('j')).expect("select the second");
    let moved = frame_at(&app, 100, 24);
    assert_eq!(
        moved[first_corner].style().fg,
        Some(Color::Blue),
        "the highlight did not move off the first box:\n{}",
        buffer_text(&moved)
    );
}

/// A folded comment is one row: no borders, and the body still readable enough
/// to find it again.
#[test]
fn a_collapsed_box_is_a_single_row() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, "needs a doc");
    let id = app.comments()[0].id.clone();

    app.on_key(KeyCode::Char('s')).expect("fold it away");

    let buffer = frame_at(&app, 100, 24);
    let text = buffer_text(&buffer);
    let inside = text_in(&buffer, box_area());
    assert!(
        !inside.contains('╭') && !inside.contains('╰'),
        "a folded comment still draws a box:\n{text}"
    );
    let rows = rows_of(&buffer);
    let anchored = row_holding(&buffer, "fn a() {");
    assert!(
        rows[anchored + 1].contains(&id) && rows[anchored + 1].contains("needs a doc"),
        "the folded row does not say what it is folding:\n{text}"
    );
}

/// A comment that is no longer open is drawn grey and dim rather than blue, and
/// opens folded: it is still exactly where the reviewer left it, without
/// competing for attention with the comments that still need answering.
///
/// Driven through the store because nothing in the reviewer can produce a
/// non-`Open` comment yet — state transitions are milestone 2's work — and a
/// `.review/` written by that milestone, or by an agent, must render sensibly
/// today rather than whenever the keyboard catches up.
#[test]
fn an_outdated_comment_is_grey_and_folded() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, "this line moved");
    let id = app.comments()[0].id.clone();

    let mut stored = workspace.store().comments().expect("read comments");
    stored[0].state = CommentState::Outdated;
    workspace
        .store()
        .append_comment(&stored[0])
        .expect("store the outdated comment");

    let reopened = workspace.app();
    assert!(
        reopened.collapsed().contains(&id),
        "an outdated comment opens expanded: {:?}",
        reopened.collapsed()
    );
    let buffer = frame_at(&reopened, 100, 24);
    let text = buffer_text(&buffer);
    let row = u16::try_from(row_holding(&buffer, "this line moved")).expect("a small row");
    let style = style_of_text(&buffer, row, &id);
    assert_eq!(
        style.fg,
        Some(Color::Gray),
        "an outdated comment is drawn as loud as an open one:\n{text}"
    );
    assert!(
        style.add_modifier.contains(Modifier::DIM),
        "an outdated comment is not dimmed:\n{text}"
    );
    assert!(
        text.contains("outdated"),
        "the row does not say why it is grey:\n{text}"
    );

    drop(app);
}

/// The selected box is kept on screen in its own right, so stepping through a
/// stack in a short pane does not leave the cursor on a box below the fold.
#[test]
fn the_selected_box_stays_on_screen_in_a_short_pane() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    for body in ["first finding", "second finding", "third finding"] {
        write_comment(&mut app, body);
    }

    app.on_key(KeyCode::Enter).expect("enter the stack");
    app.on_key(KeyCode::Char('j')).expect("second");
    app.on_key(KeyCode::Char('j')).expect("third");

    // Eight rows: a status bar, two borders, and five rows of pane — far less
    // than the three boxes need.
    let text = buffer_text(&frame_at(&app, 100, 8));
    assert!(
        text.contains("third finding"),
        "the selected box is below the fold:\n{text}"
    );
}

/// Drawing must be total. A one-column pane is where ratatui layout code
/// classically panics, and a comment box subtracts a gutter, two borders and a
/// pad from whatever width it is given.
#[rstest]
#[case(1, 1)]
#[case(2, 5)]
#[case(20, 3)]
#[case(1, 40)]
#[case(5, 2)]
#[case(3, 3)]
#[case(9, 6)]
#[case(12, 24)]
fn drawing_never_panics_at_awkward_sizes(#[case] width: u16, #[case] height: u16) {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(
        &mut app,
        "needs a doc, and a body long enough to have to wrap somewhere",
    );
    write_comment(&mut app, "second finding");

    let _ = frame_at(&app, width, height);

    app.on_key(KeyCode::Enter).expect("enter the stack");
    let _ = frame_at(&app, width, height);

    app.on_key(KeyCode::Char('s'))
        .expect("fold the selected box");
    let _ = frame_at(&app, width, height);

    app.on_key(KeyCode::Left).expect("back to the diff");
    app.on_key(KeyCode::Left).expect("onto the sidebar");
    let _ = frame_at(&app, width, height);
}

// ---------------------------------------------------------------------------
// Never clipping content silently
// ---------------------------------------------------------------------------

/// A diff line too long for the pane says so. Neither pane wraps or scrolls
/// horizontally, and this repository contains 154-character lines: a review
/// tool that silently hides the code being judged is failing at its one job.
#[test]
fn a_long_diff_line_is_marked_rather_than_silently_clipped() {
    let workspace = Fixture::with_long_line(200);
    let app = workspace.app();

    let buffer = frame_at(&app, 60, 24);
    let text = buffer_text(&buffer);

    assert!(
        text.contains('…'),
        "a clipped line says so; silent truncation hides the code under review:\n{text}"
    );
    // ...and the marker sits against the pane's own right-hand border, so what
    // it reports is the edge of the pane rather than something dropped out of
    // the middle of the line.
    //
    // Cut to the diff pane's own columns: the file list clips its rows with the
    // same marker, and at sixty columns it is clipping `long.rs` too — so the
    // first `…` on this frame row belongs to the other pane.
    let y = u16::try_from(row_holding(&buffer, "xxx")).expect("a small row");
    let row = row_in(&buffer, areas(60, 24, Split::default()).diff, y);
    let after: String = row.chars().skip_while(|c| *c != '…').skip(1).collect();
    assert_eq!(
        after, "│",
        "the marker is not against the pane's edge: {row:?}"
    );
}

/// ...and it is *clipped*, not wrapped. The row model is built on one row per
/// diff line, and a reviewer counting lines against a file needs that
/// correspondence: a wrapped line would put the highlight and the line's own
/// number on different rows from the rest of it.
#[test]
fn a_long_diff_line_is_never_wrapped_onto_a_second_row() {
    let workspace = Fixture::with_long_line(200);
    let app = workspace.app();

    let buffer = frame_at(&app, 60, 24);
    let rows = rows_of(&buffer);
    let carrying: Vec<&String> = rows.iter().filter(|row| row.contains("xxx")).collect();

    assert_eq!(
        carrying.len(),
        1,
        "one diff line was drawn on {} rows:\n{}",
        carrying.len(),
        buffer_text(&buffer)
    );
}

/// A short line is left exactly as it was: the marker is a report of clipping,
/// not decoration.
#[test]
fn a_line_that_fits_is_not_marked() {
    let workspace = Fixture::new();
    let app = workspace.app();

    let text = buffer_text(&frame_at(&app, 100, 24));

    assert!(
        !text.contains('…'),
        "a pane with room to spare still claims it clipped something:\n{text}"
    );
    assert!(text.contains("    let x = 1;"), "{text}");
}

/// The comment bar follows the end of what is being typed. Past the bar's
/// width the reviewer used to be typing blind — the bar kept showing the
/// opening words while the cursor was 80 characters further on.
#[test]
fn the_comment_buffer_shows_the_tail_while_typing_past_the_width() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    app.on_key(KeyCode::Char('c')).expect("begin a comment");
    type_text(&mut app, "HEAD");
    type_text(&mut app, &"x".repeat(200));
    type_text(&mut app, "TAIL");

    let text = buffer_text(&frame_at(&app, 40, 24));

    assert!(
        text.contains("TAIL"),
        "what is being typed is not on screen:\n{text}"
    );
    assert!(
        !text.contains("HEAD"),
        "the bar is showing the start of a buffer whose end is where the cursor is:\n{text}"
    );
    assert_eq!(
        app.buffer().chars().count(),
        208,
        "the bar's window ate the buffer itself"
    );
}

/// A comment that fits is shown whole, from its first character: the tail is
/// what a long buffer falls back to, not what every buffer gets.
#[test]
fn a_short_comment_is_shown_from_its_beginning() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    app.on_key(KeyCode::Char('c')).expect("begin a comment");
    type_text(&mut app, "needs a doc");

    let text = buffer_text(&frame_at(&app, 40, 24));

    assert!(text.contains("needs a doc"), "{text}");
}

// ---------------------------------------------------------------------------
// Browsing comments in the sidebar
// ---------------------------------------------------------------------------

/// Walks the sidebar's comment browser to row `index` and presses `Enter`,
/// exactly the way a reviewer does — no test-only entry point into the jump.
fn jump_to_row(app: &mut App, index: usize) {
    app.on_key(KeyCode::Tab).expect("comments tab");
    app.on_key(KeyCode::Left).expect("focus the sidebar");
    for _ in 0..index {
        app.on_key(KeyCode::Down).expect("next row");
    }
    app.on_key(KeyCode::Enter).expect("jump");
}

#[test]
fn tab_switches_the_sidebar_between_files_and_comments() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    assert_eq!(app.sidebar_tab(), SidebarTab::Files, "files by default");

    app.on_key(KeyCode::Tab).expect("tab");
    assert_eq!(app.sidebar_tab(), SidebarTab::Comments);
    app.on_key(KeyCode::Tab).expect("tab back");
    assert_eq!(app.sidebar_tab(), SidebarTab::Files);
}

/// From any focus, and without disturbing anything else: `Tab` is about what
/// the left column *lists*, not about where the cursor is.
#[rstest]
#[case(&[])]
#[case(&[KeyCode::Left])]
#[case(&[KeyCode::Enter])]
fn tab_switches_the_tab_from_any_focus(#[case] approach: &[KeyCode]) {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, "needs a doc");
    for key in approach {
        app.on_key(*key).expect("get into position");
    }
    let (focus, file, line) = (app.focus(), app.file_index(), app.line_index());

    app.on_key(KeyCode::Tab).expect("tab");

    assert_eq!(app.sidebar_tab(), SidebarTab::Comments);
    assert_eq!(app.focus(), focus, "tab moved the cursor to another pane");
    assert_eq!((app.file_index(), app.line_index()), (file, line));
    assert_eq!(app.mode(), Mode::Browse);
}

#[test]
fn the_comment_browser_lists_every_comment_in_the_review() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, "first finding");
    app.on_key(KeyCode::Char(']')).expect("next file");
    write_comment(&mut app, "second finding");

    app.on_key(KeyCode::Tab).expect("comments tab");
    app.on_key(KeyCode::Left).expect("focus the sidebar");

    assert_eq!(
        app.browsed_comment().expect("a first row").body,
        "first finding",
        "the browser opens on the oldest comment"
    );
    app.on_key(KeyCode::Down).expect("next row");
    assert_eq!(
        app.browsed_comment().expect("a second row").body,
        "second finding",
        "the browser lists comments from every file, not only the open one"
    );
    app.on_key(KeyCode::Down).expect("past the end");
    assert_eq!(
        app.browsed_comment().expect("still the second").body,
        "second finding",
        "the cursor stops at the newest rather than wrapping"
    );
    app.on_key(KeyCode::Up).expect("back");
    assert_eq!(
        app.browsed_comment().expect("the first again").body,
        "first finding"
    );
    assert_eq!(
        app.file_index(),
        1,
        "walking the comment browser moved the file selection"
    );
}

/// The whole point of the browser: reading a comment and looking at the code it
/// is about are one keystroke apart.
#[test]
fn enter_on_a_comment_row_jumps_to_its_code() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    // Comment on the second file, then walk away to the first.
    app.on_key(KeyCode::Char(']')).expect("next file");
    app.on_key(KeyCode::Char('j')).expect("move down");
    let commented_file = app.file_index();
    let commented_line = app.line_index();
    write_comment(&mut app, "look at this");
    app.on_key(KeyCode::Char('['))
        .expect("back to the first file");
    assert_ne!(
        app.file_index(),
        commented_file,
        "the walk away did nothing"
    );

    jump_to_row(&mut app, 0);

    assert_eq!(app.file_index(), commented_file, "landed on the right file");
    assert_eq!(app.line_index(), commented_line, "and the right line");
    assert_eq!(
        app.focus(),
        Focus::Diff,
        "with the diff focused, ready to act"
    );
    assert_eq!(
        app.comments_for_line(app.line_index()).len(),
        1,
        "the comment is not on the line the jump landed on"
    );
    assert!(
        app.status().contains("b.rs"),
        "the jump does not say where it went: {:?}",
        app.status()
    );
}

/// The jump uses the same anchor key the save used, so it lands where the
/// comment says it is even when the two sides of the diff number that line
/// differently.
#[test]
fn a_jump_lands_on_the_line_the_comment_is_anchored_to() {
    let workspace = Fixture::renamed();
    let mut app = workspace.app_from("@--");
    let removed = select_line(&mut app, |line| {
        line.kind == LineKind::Removed && line.text.contains("let x = 1;")
    });
    let line = app.line_index();
    write_comment(&mut app, "why was this rewritten?");
    assert_eq!(
        removed.left,
        Some(2),
        "the fixture stopped pairing the rewrite"
    );
    assert_eq!(removed.right, Some(3), "{removed:?}");

    // Walk away, then come back through the browser.
    app.on_key(KeyCode::Char('k')).expect("up");
    app.on_key(KeyCode::Char('k')).expect("up");
    jump_to_row(&mut app, 0);

    assert_eq!(
        app.line_index(),
        line,
        "the jump used a different rule from the save: {:?}",
        app.selected_diff().expect("a diff").lines[app.line_index()]
    );
    assert_eq!(app.comments_for_line(app.line_index()).len(), 1);
}

/// A comment whose file has left the review's range is reported, not papered
/// over — and the reviewer is left exactly where they were.
#[test]
fn a_jump_to_a_file_outside_the_review_says_so() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, "on a file that moved away");
    store_variant(&workspace, "deadbee1", "gone.rs", 1);

    let mut reopened = workspace.app();
    let was = (reopened.file_index(), reopened.line_index());
    jump_to_row(&mut reopened, 1);

    assert!(
        reopened.status().contains("gone.rs") && reopened.status().contains("range"),
        "the jump did not say why it went nowhere: {:?}",
        reopened.status()
    );
    assert_eq!(
        (reopened.file_index(), reopened.line_index()),
        was,
        "a jump that could not be made moved the reviewer anyway"
    );
    drop(app);
}

/// A comment whose line has left the diff still opens its file: being in the
/// right file with a warning beats staying put.
#[test]
fn a_jump_to_a_missing_line_opens_the_file_anyway() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    app.on_key(KeyCode::Char(']')).expect("next file");
    write_comment(&mut app, "on the second file");
    store_variant(&workspace, "deadbee2", "a.rs", 99);

    let mut reopened = workspace.app();
    reopened.on_key(KeyCode::Char(']')).expect("open b.rs");
    jump_to_row(&mut reopened, 1);

    assert_eq!(
        reopened.selected_file().expect("a file").path,
        "a.rs",
        "the jump did not open the file the comment names"
    );
    assert_eq!(reopened.line_index(), 0, "and put the cursor at its top");
    assert_eq!(reopened.focus(), Focus::Diff);
    assert!(
        reopened.status().contains("99") && reopened.status().contains("not in this diff"),
        "the jump did not say what it could not find: {:?}",
        reopened.status()
    );
    drop(app);
}

/// Stores a copy of the review's first comment under a different id, file and
/// line — the two shapes a jump has to fail honestly on, neither of which the
/// keyboard can produce.
fn store_variant(workspace: &Fixture, id: &str, file: &str, line: u32) {
    let mut comment = workspace
        .store()
        .comments()
        .expect("read comments")
        .swap_remove(0);
    comment.id = id.to_owned();
    comment.anchor.file = file.to_owned();
    comment.anchor.line = line;
    comment.anchor.side = Side::Right;
    workspace
        .store()
        .append_comment(&comment)
        .expect("store the variant");
}

/// `d` from the browser deletes the comment the browser has selected, behind
/// the same confirmation as everywhere else.
#[test]
fn d_from_the_comment_browser_deletes_behind_the_same_confirmation() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, "first finding");
    app.on_key(KeyCode::Char('j')).expect("next line");
    write_comment(&mut app, "second finding");

    app.on_key(KeyCode::Tab).expect("comments tab");
    app.on_key(KeyCode::Left).expect("focus the sidebar");
    app.on_key(KeyCode::Down).expect("select the second");
    app.on_key(KeyCode::Char('d')).expect("ask");
    assert!(
        matches!(app.mode(), Mode::ConfirmDelete { .. }),
        "it deleted without asking: {:?}",
        app.mode()
    );
    app.on_key(KeyCode::Char('y')).expect("confirm");

    let left = workspace.store().comments().expect("read");
    assert_eq!(left.len(), 1, "{left:?}");
    assert_eq!(left[0].body, "first finding", "the browsed comment went");
    assert_eq!(
        app.browsed_comment().expect("a row").body,
        "first finding",
        "the browser's cursor was left past the end of the list"
    );
}

/// ...and `n` still cancels, from here as from anywhere.
#[test]
fn d_from_the_comment_browser_can_be_declined() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, "needs a doc");

    app.on_key(KeyCode::Tab).expect("comments tab");
    app.on_key(KeyCode::Left).expect("focus the sidebar");
    app.on_key(KeyCode::Char('d')).expect("ask");
    app.on_key(KeyCode::Char('n')).expect("decline");

    assert_eq!(app.mode(), Mode::Browse);
    assert_eq!(workspace.store().comments().expect("read").len(), 1);
}

/// The Files tab stays inert: it selects files, and a destructive key aimed
/// into it would be aimed at a comment the reviewer cannot see.
#[test]
fn d_from_the_files_tab_still_deletes_nothing() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, "needs a doc");

    app.on_key(KeyCode::Left).expect("focus the file list");
    assert_eq!(app.sidebar_tab(), SidebarTab::Files);
    app.on_key(KeyCode::Char('d')).expect("d");

    assert_eq!(app.mode(), Mode::Browse);
    assert!(app.status().contains("not comments"), "{:?}", app.status());
    assert_eq!(workspace.store().comments().expect("read").len(), 1);
}

/// `d` with an empty browser asks nothing and says why.
#[test]
fn d_from_an_empty_comment_browser_says_so() {
    let workspace = Fixture::new();
    let mut app = workspace.app();

    app.on_key(KeyCode::Tab).expect("comments tab");
    app.on_key(KeyCode::Left).expect("focus the sidebar");
    app.on_key(KeyCode::Char('d')).expect("d");

    assert_eq!(app.mode(), Mode::Browse);
    assert!(app.status().contains("no comments"), "{:?}", app.status());
}

#[test]
fn the_comment_browser_renders_path_line_and_state() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, "needs a doc");
    app.on_key(KeyCode::Tab).expect("comments tab");

    let buffer = frame_at(&app, 100, 24);
    let text = buffer_text(&buffer);

    assert!(
        text.contains("Comments (1)"),
        "the tab is unmistakable:\n{text}"
    );
    assert!(
        text.contains("a.rs:1"),
        "the file and line are named:\n{text}"
    );
    assert!(
        text.contains("needs a doc"),
        "the body is previewed:\n{text}"
    );
    assert!(text.contains("open"), "the state is shown:\n{text}");
    assert!(
        !text.contains("Files ("),
        "the file list is still on screen beside it:\n{text}"
    );
}

#[test]
fn an_empty_comment_browser_says_so() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    app.on_key(KeyCode::Tab).expect("comments tab");

    let text = buffer_text(&frame_at(&app, 100, 24));

    assert!(
        text.contains("no comments"),
        "an empty review does not explain itself:\n{text}"
    );
}

/// The browser's own selection is marked, and only while the sidebar has the
/// focus — the same rule the file list follows.
#[test]
fn the_browsed_row_is_highlighted_when_the_sidebar_has_focus() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, "needs a doc");
    app.on_key(KeyCode::Tab).expect("comments tab");

    let reversed = |app: &App| {
        let buffer = frame_at(app, 100, 24);
        (0..24).any(|y| (0..30).any(|x| buffer[(x, y)].modifier.contains(Modifier::REVERSED)))
    };

    assert!(!reversed(&app), "the unfocused browser is reversed");
    app.on_key(KeyCode::Left).expect("focus the sidebar");
    assert!(reversed(&app), "the focused browser has no selection");
}

/// The bar drops whole segments rather than cutting one in half, at every
/// width, and the pointer to the keymap is the last thing standing.
///
/// This replaces a test that asserted the opposite — that a status line too
/// long for the terminal ends in `…`. That was true when `app.status()` *was*
/// the bar, and it was the defect: half of `deleted comment at app.rs:42` is a
/// claim about a file that does not exist, and a status that owned the whole
/// row could evict the one in-app pointer to the keys. The bar is segments now
/// (see `rv::statusbar`), so a segment either fits or is dropped whole, and the
/// hint outlives every one of them.
#[test]
fn the_bar_drops_a_segment_whole_rather_than_cutting_a_word() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, "a finding");

    for width in [16u16, 24, 40, 60, 80, 100, 120] {
        let frame = frame_at(&app, width, 24);
        let bar = last_row(&frame);
        assert!(
            !bar.contains('…'),
            "the bar cut a segment in half at {width} columns: {bar:?}"
        );
        assert!(
            bar.contains("? help"),
            "the pointer to the keymap went first at {width} columns: {bar:?}"
        );
        assert!(
            (0..width).all(|x| bg_of(&frame, x, 23).is_some()),
            "the bar left part of the row bare at {width} columns: {bar:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// The documented keymap
// ---------------------------------------------------------------------------

/// Every key this reviewer binds while browsing, spelled the way README's
/// **Browsing** table spells the `Key` column of its rows.
///
/// All but the last are [`rv::app::App::on_key`]'s, read straight out of
/// `on_key_browse`; `Ctrl+C` is [`rv::app::App::on_key_event`]'s, answered
/// before the mode is consulted at all, and is in the same table because a
/// reviewer looking for how to get out does not know or care which function
/// answers them.
///
/// The two tests below hold this list and the table to each other in *both*
/// directions, so neither a binding that ships undocumented nor a row for a key
/// nobody wrote survives. Three waves of this milestone shipped keys the README
/// never mentioned — focus, the tab, the stack, `d`, `s` — which is the drift
/// this exists to stop. What each key actually does is pinned by
/// `rv/tests/app_cases.rs`'s `browse_keybindings` table and by the tests above;
/// this pair is only about whether a user can find out.
/// The arrow leads and the vim key follows in parentheses, here as in
/// [`BINDINGS`] and in the popup: rv is a tool a reviewer may open once a week,
/// and the arrows are the keys someone can find without being told.
const BROWSE_KEYS: &[&str] = &[
    "`↓` (`j`)",
    "`↑` (`k`)",
    "`←` (`h`)",
    "`→` (`l`)",
    "`]`",
    "`[`",
    "`Tab`",
    "`Enter`",
    "`Esc`",
    "`c`",
    "`d`",
    "`s`",
    "`<`",
    "`>`",
    "`?`",
    "`q`",
    "`Ctrl+C`",
];

/// The README, read from the workspace rather than from the process's working
/// directory — a test binary's cwd is not something to depend on.
fn readme() -> String {
    fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../README.md"))
        .expect("read README.md")
}

/// The `Key` column of the markdown table that follows `label`, one entry per
/// row, without the header row or its underline.
///
/// The table is taken as the run of `|` lines after the label, so the tables
/// under the other labels — `Esc` appears in two of them — cannot leak into the
/// answer.
fn table_keys(label: &str) -> Vec<String> {
    let readme = readme();
    let (_, body) = readme
        .split_once(label)
        .unwrap_or_else(|| panic!("the README has no {label} table"));
    body.lines()
        .skip_while(|line| !line.starts_with('|'))
        .take_while(|line| line.starts_with('|'))
        .filter_map(|line| line.split('|').nth(1))
        .map(|cell| cell.trim().to_owned())
        .filter(|cell| cell != "Key" && !cell.starts_with("---"))
        .collect()
}

/// The README under `heading`, up to the next heading of any level.
fn readme_section(heading: &str) -> String {
    let readme = readme();
    let (_, body) = readme
        .split_once(heading)
        .unwrap_or_else(|| panic!("the README has no {heading:?} section"));
    body.lines()
        .take_while(|line| !line.starts_with('#'))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn the_readme_documents_every_browse_binding() {
    let documented = table_keys("**Browsing**");
    for key in BROWSE_KEYS {
        assert!(
            documented.iter().any(|row| row == key),
            "the README's Browsing table has no row for {key}, so a reviewer \
             cannot find out that it exists: {documented:?}"
        );
    }
}

/// ...and no row for a key that is not one of them: a table that documents a
/// binding nobody wrote is worse than one that documents none, because a
/// reviewer will press it and read the result as a bug in the key rather than
/// in the page.
#[test]
fn the_readme_documents_no_binding_that_is_not_bound() {
    let documented = table_keys("**Browsing**");
    for row in &documented {
        assert!(
            BROWSE_KEYS.contains(&row.as_str()),
            "the README's Browsing table has a row for {row:?}, which is not one \
             of this reviewer's keys: {BROWSE_KEYS:?}"
        );
    }
}

/// The comment box is the thing a reviewer meets first and has the most
/// questions about, and every one of them below is answered somewhere in the
/// code rather than in the page: that a reply shares its comment's box and is
/// drawn dimmed, that folding is a preference of this session and reaches no
/// file, that a delete is permanent and wants a `y`, and that the markdown is
/// an export written by `rv render` — not a document kept continuously in step,
/// which is what a reader assumes of a file in their working tree until they
/// are told otherwise.
///
/// # Why two of these are phrases rather than words
///
/// A one-word probe passes on a mention, and a mention is not a claim. The
/// export cases are the ones where that difference bites: this section says
/// "an LLM reading the export" in its *folding* paragraph, so a page that had
/// been rewritten to promise `REVIEW-FEEDBACK.md` is kept continuously in step
/// with the store still contained the word `export` and still passed. That
/// mutant — the page asserting the exact opposite of the truth — survived the
/// wave that added these cases, which is the worst shape a documentation test
/// can have: it reports the drift it exists to catch as covered.
///
/// So the two export cases pin the sentence's *claim* — that the file **is** an
/// export, and that it is **not** kept in step — and are deliberately longer and
/// more brittle than the rest. A reworded README should fail them; a reworded
/// README is exactly when someone should be made to reread that paragraph and
/// decide whether the promise it makes is still true. `bordered` is split out of
/// `blue` for the smaller version of the same reason: the case was named for two
/// facts and checked one.
#[rstest]
#[case::under_its_line("beneath the line")]
#[case::the_box_is_blue("blue")]
#[case::the_box_is_bordered("bordered")]
#[case::a_reply_shares_the_box("reply")]
#[case::a_reply_is_dimmed("dimmed")]
#[case::folding_is_a_session_preference("session")]
#[case::deletion_is_permanent("permanent")]
#[case::deletion_is_confirmed("`y`")]
#[case::the_markdown_is_an_export("is an **export**")]
#[case::the_export_is_not_kept_in_step("not a document kept continuously in step")]
#[case::written_by_render("`rv render`")]
fn the_readme_explains_inline_comments(#[case] phrase: &str) {
    let section = readme_section("### Inline comments");
    assert!(
        section.contains(phrase),
        "the README's inline-comments section never mentions {phrase:?}:\n{section}"
    );
}

// ---------------------------------------------------------------------------
// Resizing the panes
// ---------------------------------------------------------------------------

/// Where each pane is at `width` x `height`, asked of the very function
/// [`rv::ui::draw`] paints from — so a test that reads a column out of the
/// buffer is reading the column the renderer wrote to, rather than one this
/// file computed for itself.
///
/// The one place these tests are allowed to talk about geometry: see
/// `rv/src/layout.rs` for why nothing else may compute a `Rect`.
fn areas(width: u16, height: u16, split: Split) -> rv::layout::Layout {
    layout(
        Rect::new(0, 0, width, height),
        split,
        Chrome {
            bar_rows: 1,
            help_open: false,
            toast: false,
        },
    )
}

#[test]
fn angle_brackets_resize_the_panes() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    let start = app.split().ratio();
    assert_eq!(
        start,
        Split::DEFAULT,
        "a reviewer opens on the default split"
    );

    app.on_key(KeyCode::Char('>')).expect(">");
    assert!(
        app.split().ratio() > start,
        "the sidebar grew: {}",
        app.split().ratio()
    );
    app.on_key(KeyCode::Char('<')).expect("<");
    assert_eq!(app.split().ratio(), start, "and shrank back");
}

#[test]
fn resizing_never_leaves_the_bounds_however_long_you_hold_it() {
    let workspace = Fixture::new();
    let mut app = workspace.app();

    for _ in 0..200 {
        app.on_key(KeyCode::Char('>')).expect(">");
    }
    assert!(
        app.split().ratio() <= Split::MAX_RATIO,
        "held past the right bound: {}",
        app.split().ratio()
    );
    for _ in 0..400 {
        app.on_key(KeyCode::Char('<')).expect("<");
    }
    assert!(
        app.split().ratio() >= Split::MIN_RATIO,
        "held past the left bound: {}",
        app.split().ratio()
    );
}

/// The accessor is not the point — the pane on screen is. A resize that moved
/// `App::split` without moving the divider would pass every test above.
#[test]
fn a_resized_pane_actually_renders_at_its_new_width() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    let before = frame_at(&app, 100, 24);

    for _ in 0..5 {
        app.on_key(KeyCode::Char('>')).expect(">");
    }
    let after = frame_at(&app, 100, 24);
    assert_ne!(before, after, "the frame does not reflect the resize");

    // ...and the divider is where `layout` says it is for the *new* split, not
    // the old one: the two panes' borders moved with it.
    let divider = areas(100, 24, app.split()).divider.x;
    assert!(
        divider > areas(100, 24, Split::default()).divider.x,
        "the divider did not move: {divider}"
    );
    assert_eq!(
        after[(divider - 1, 1)].symbol(),
        "│",
        "the sidebar's right border is not against the divider:\n{}",
        buffer_text(&after)
    );
    assert_eq!(
        after[(divider + 1, 1)].symbol(),
        "│",
        "the diff's left border is not against the divider:\n{}",
        buffer_text(&after)
    );
}

/// A split is a view preference, like folding: it never reaches `.review/`, and
/// a reviewer who reopens the review gets the default back.
#[test]
fn the_split_is_not_written_anywhere() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    let before = workspace_tree(workspace.root());
    assert!(!before.is_empty(), "the review wrote nothing to compare");

    for _ in 0..3 {
        app.on_key(KeyCode::Char('>')).expect(">");
    }
    assert_ne!(app.split().ratio(), Split::DEFAULT, "nothing was resized");
    assert_eq!(
        workspace_tree(workspace.root()),
        before,
        "resizing wrote to the workspace; it is a view preference, not review state"
    );

    let reopened = workspace.app();
    assert_eq!(
        reopened.split().ratio(),
        Split::DEFAULT,
        "the split survived the session it was a preference of"
    );
}

// ---------------------------------------------------------------------------
// The binding table and the `?` popup
// ---------------------------------------------------------------------------

/// The cell holding the key of the popup row that describes `what`.
///
/// Found by the description rather than by the key, because a single-character
/// key is a substring of half the screen: the row is located by the sentence
/// only it carries, and the key is then the last occurrence of `keys` in the
/// columns to its left.
fn cell_of_binding(buffer: &Buffer, keys: &str, what: &str) -> (u16, u16) {
    let rows = rows_of(buffer);
    let (y, row) = rows
        .iter()
        .enumerate()
        .find(|(_, row)| row.contains(what))
        .unwrap_or_else(|| panic!("{what:?} is not on screen:\n{}", buffer_text(buffer)));
    let at = row.find(what).expect("the row holds it");
    let before = &row[..at];
    let start = before
        .rfind(keys)
        .unwrap_or_else(|| panic!("{keys:?} is not left of {what:?} on row {y}: {row:?}"));
    let column = before[..start].chars().count();
    (
        u16::try_from(column).expect("a small column"),
        u16::try_from(y).expect("a small row"),
    )
}

/// Whether the cell at `at` is drawn dim — how the popup says a key does
/// nothing from where the cursor is.
fn is_dim(buffer: &Buffer, at: (u16, u16)) -> bool {
    buffer[at].modifier.contains(Modifier::DIM)
}

#[test]
fn question_mark_opens_the_help_and_esc_closes_it() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    assert!(
        !app.help_open(),
        "a reviewer opens on the review, not the manual"
    );

    app.on_key(KeyCode::Char('?')).expect("?");
    assert!(app.help_open());
    let frame = buffer_text(&frame_at(&app, 100, 24));
    assert!(
        frame.contains("comment"),
        "the popup lists what the keys do:\n{frame}"
    );

    app.on_key(KeyCode::Esc).expect("esc");
    assert!(!app.help_open());
    assert!(
        !buffer_text(&frame_at(&app, 100, 24)).contains("narrower sidebar"),
        "the popup is still on screen once it is closed"
    );
}

/// `?` is a toggle as well as an opener: the key that raised the manual is the
/// first one a reviewer presses again to get rid of it.
#[test]
fn question_mark_closes_the_help_it_opened() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    app.on_key(KeyCode::Char('?')).expect("?");
    app.on_key(KeyCode::Char('?')).expect("? again");
    assert!(!app.help_open());
}

#[test]
fn q_closes_the_help_rather_than_quitting() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    app.on_key(KeyCode::Char('?')).expect("?");

    let action = app.on_key(KeyCode::Char('q')).expect("q");
    assert_eq!(action, Action::Continue, "q in help closes the help");
    assert!(!app.help_open());
    assert_eq!(
        app.on_key(KeyCode::Char('q')).expect("q"),
        Action::Quit,
        "and quits once it is closed"
    );
}

/// While the manual is up every other key is inert — including the one that
/// destroys written work.
#[rstest]
#[case(KeyCode::Char('c'))]
#[case(KeyCode::Char('d'))]
#[case(KeyCode::Char('j'))]
#[case(KeyCode::Enter)]
#[case(KeyCode::Tab)]
#[case(KeyCode::Left)]
#[case(KeyCode::Char(']'))]
#[case(KeyCode::Char('s'))]
#[case(KeyCode::Char('>'))]
#[case(KeyCode::Char('t'))]
#[case(KeyCode::Char('o'))]
fn keys_are_inert_while_the_help_is_open(#[case] key: KeyCode) {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, "a finding");
    let before = workspace_tree(workspace.root());

    app.on_key(KeyCode::Char('?')).expect("?");
    let state = (
        app.mode(),
        app.focus(),
        app.file_index(),
        app.line_index(),
        app.sidebar_tab(),
        app.split().ratio(),
        app.collapsed().len(),
        app.tree_view(),
        app.sort(),
    );

    app.on_key(key).expect("key");

    assert_eq!(
        (
            app.mode(),
            app.focus(),
            app.file_index(),
            app.line_index(),
            app.sidebar_tab(),
            app.split().ratio(),
            app.collapsed().len(),
            app.tree_view(),
            app.sort(),
        ),
        state,
        "{key:?} did something while the help was open"
    );
    assert!(app.help_open(), "{key:?} closed the help");
    assert_eq!(
        workspace_tree(workspace.root()),
        before,
        "{key:?} wrote to the workspace from behind the help"
    );
}

#[test]
fn every_binding_the_handler_dispatches_appears_in_the_popup() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    app.on_key(KeyCode::Char('?')).expect("?");
    let frame = buffer_text(&frame_at(&app, 120, 40));

    assert!(!BINDINGS.is_empty(), "the binding table is empty");
    for binding in BINDINGS {
        assert!(
            frame.contains(binding.keys),
            "the popup does not list {}:\n{frame}",
            binding.keys
        );
        assert!(
            frame.contains(binding.what),
            "the popup lists {} without saying what it does:\n{frame}",
            binding.keys
        );
    }
}

/// 80x24 is what a reviewer over ssh actually has, and a keymap you must scroll
/// to read is a keymap you will not read. This is what forces the column
/// layout: sixteen bindings and their headings need twenty-one rows in one
/// list, and the popup has fourteen.
#[test]
fn the_whole_keymap_fits_at_80x24_without_scrolling() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    app.on_key(KeyCode::Char('?')).expect("?");
    let frame = buffer_text(&frame_at(&app, 80, 24));

    for binding in BINDINGS {
        assert!(
            frame.contains(binding.keys),
            "{} is off screen at 80x24:\n{frame}",
            binding.keys
        );
        assert!(
            frame.contains(binding.what),
            "{}'s description is off screen at 80x24:\n{frame}",
            binding.keys
        );
    }
    assert!(
        !frame.contains("more"),
        "something is hidden behind a scroll indicator:\n{frame}"
    );
}

/// `d` means nothing in the Files tab. A reviewer learning the tool should see
/// that the key exists and why it is inert here, not wonder whether they
/// misread the manual.
///
/// The control is in the same frame on purpose: if every row were dimmed,
/// nothing would be.
#[test]
fn a_binding_that_does_nothing_here_is_dimmed_rather_than_hidden() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    app.on_key(KeyCode::Left).expect("focus the sidebar");
    assert_eq!(app.sidebar_tab(), SidebarTab::Files);
    app.on_key(KeyCode::Char('?')).expect("?");

    let frame = frame_at(&app, 100, 30);
    assert!(
        buffer_text(&frame).contains("delete a comment"),
        "the binding was hidden rather than dimmed:\n{}",
        buffer_text(&frame)
    );
    assert!(
        is_dim(&frame, cell_of_binding(&frame, "d", "delete a comment")),
        "`d` is not shown as inactive in the file list:\n{}",
        buffer_text(&frame)
    );
    assert!(
        !is_dim(&frame, cell_of_binding(&frame, "q", "quit the review")),
        "every row is dimmed, so dimming says nothing:\n{}",
        buffer_text(&frame)
    );
}

/// ...and the same key is *not* dimmed where it does something, so the dimming
/// follows the cursor rather than being a property of the key.
#[test]
fn the_same_binding_is_live_where_it_acts_on_something() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, "a finding");
    app.on_key(KeyCode::Char('?')).expect("?");

    let frame = frame_at(&app, 100, 30);
    assert!(
        !is_dim(&frame, cell_of_binding(&frame, "d", "delete a comment")),
        "`d` is dimmed on a line that has a comment to delete:\n{}",
        buffer_text(&frame)
    );
}

#[rstest]
#[case(20, 6)]
#[case(1, 1)]
#[case(80, 1)]
#[case(2, 40)]
#[case(40, 3)]
#[case(12, 12)]
fn the_help_renders_in_a_pane_too_small_for_it(#[case] width: u16, #[case] height: u16) {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, "a finding");
    app.on_key(KeyCode::Char('?')).expect("?");

    let _ = frame_at(&app, width, height);
    // ...and scrolling a popup that cannot show its whole keymap is still just
    // drawing.
    for _ in 0..40 {
        app.on_key(KeyCode::Char('j')).expect("scroll");
    }
    let _ = frame_at(&app, width, height);
    for _ in 0..80 {
        app.on_key(KeyCode::Char('k')).expect("scroll back");
    }
    let _ = frame_at(&app, width, height);
}

/// The popup is drawn *over* the panes rather than beside them: what was
/// underneath is covered, which is what makes it readable.
#[test]
fn the_popup_covers_what_is_beneath_it() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    let beneath = frame_at(&app, 100, 24);
    let popup = layout(
        Rect::new(0, 0, 100, 24),
        Split::default(),
        Chrome {
            bar_rows: 1,
            help_open: true,
            toast: false,
        },
    )
    .popup
    .expect("the popup has a rect at 100x24");

    app.on_key(KeyCode::Char('?')).expect("?");
    let over = frame_at(&app, 100, 24);

    let changed = (popup.y..popup.bottom())
        .flat_map(|y| (popup.x..popup.right()).map(move |x| (x, y)))
        .filter(|at| beneath[*at].symbol() != over[*at].symbol())
        .count();
    assert!(
        changed > 0,
        "the popup left the panes beneath it showing through:\n{}",
        buffer_text(&over)
    );
    // The bar is outside the popup and keeps its own row.
    assert_eq!(last_row(&beneath), last_row(&over), "the popup ate the bar");
}

// ---------------------------------------------------------------------------
// Syntax colours inside the green and the red
// ---------------------------------------------------------------------------

/// The diff pane's rows at `width` x `height`, as `(frame row, text)` pairs
/// with the pane's own borders taken off.
fn diff_rows(buffer: &Buffer, area: Rect) -> Vec<(u16, String)> {
    ((area.y + 1)..area.bottom().saturating_sub(1))
        .map(|y| {
            let text = ((area.x + 1)..area.right().saturating_sub(1))
                .map(|x| buffer[(x, y)].symbol())
                .collect();
            (y, text)
        })
        .collect()
}

/// The frame row the diff pane draws its first line carrying `sigil` on.
///
/// The sigil is column 6 of the pane's inner area — a five-wide number field
/// and a space — so this cannot be fooled by a `+` inside a line's text.
fn row_of_sigil(buffer: &Buffer, area: Rect, sigil: char) -> u16 {
    diff_rows(buffer, area)
        .into_iter()
        .find(|(_, text)| text.chars().nth(6) == Some(sigil))
        .map(|(y, _)| y)
        .unwrap_or_else(|| {
            panic!(
                "no diff line carries the sigil {sigil:?}:\n{}",
                buffer_text(buffer)
            )
        })
}

/// The background the diff pane painted row `y` with, or `None` where the row
/// is left on the terminal's own ground.
fn diff_bg(buffer: &Buffer, area: Rect, y: u16) -> Option<Color> {
    match buffer[(area.x + 1, y)].style().bg {
        None | Some(Color::Reset) => None,
        colour => colour,
    }
}

/// Every distinct foreground the diff pane used on row `y`, ignoring the cells
/// that hold nothing.
fn distinct_foregrounds(buffer: &Buffer, area: Rect, y: u16) -> Vec<Color> {
    let mut seen: Vec<Color> = Vec::new();
    for x in (area.x + 1)..area.right().saturating_sub(1) {
        let cell = &buffer[(x, y)];
        if cell.symbol().trim().is_empty() {
            continue;
        }
        let fg = cell.style().fg.unwrap_or(Color::Reset);
        if !seen.contains(&fg) {
            seen.push(fg);
        }
    }
    seen
}

/// Every cell of the diff pane's interior that carries a glyph, as
/// `(column, row)`.
///
/// Blank cells are skipped because a blank cell has no foreground to judge:
/// what is being asked here is what colour the *code a reviewer is reading* was
/// sent in.
fn diff_pane_cells(buffer: &Buffer, area: Rect) -> Vec<(u16, u16)> {
    let mut cells = Vec::new();
    for y in (area.y + 1)..area.bottom().saturating_sub(1) {
        for x in (area.x + 1)..area.right().saturating_sub(1) {
            if !buffer[(x, y)].symbol().trim().is_empty() {
                cells.push((x, y));
            }
        }
    }
    cells
}

/// The foreground the diff pane drew the first `//` comment in.
fn colour_of_first_comment(buffer: &Buffer, area: Rect) -> Option<Color> {
    let (y, text) = diff_rows(buffer, area)
        .into_iter()
        .find(|(_, text)| text.contains("//"))
        .unwrap_or_else(|| panic!("no `//` comment is on screen:\n{}", buffer_text(buffer)));
    let at = text.find("//").expect("the row holds it");
    let column = area.x + 1 + u16::try_from(text[..at].chars().count()).expect("a small column");
    buffer[(column, y)].style().fg
}

/// The defect a user reported: comments render too white.
///
/// `Color::Gray` is ANSI index 7 — the terminal's *white* — which is what this
/// used to send, and index 8 (bright black) is the tone every scheme defines
/// for exactly this against its own background. The distinction is not
/// cosmetic: index 7 on a light scheme is near-invisible and on a dark one is
/// as loud as the code it annotates.
#[test]
fn a_comment_uses_the_terminals_muted_tone() {
    let workspace = Fixture::commented();
    let app = workspace.app();
    let frame = frame_at(&app, 100, 24);
    let area = areas(100, 24, Split::default()).diff;
    assert_eq!(
        colour_of_first_comment(&frame, area),
        Some(Color::DarkGray),
        "comments are index 8, the tone every scheme defines for exactly this:\n{}",
        buffer_text(&frame)
    );
}

/// Every capture maps to one of the 16 indexed ANSI colours, or to nothing at
/// all.
///
/// The indexed colours are a pass-through to the reviewer's own scheme: emit
/// index 4 and the terminal substitutes whatever *its* theme calls blue. That
/// is the whole of rv's theming design, which is why there is no theme option
/// — see `ui`'s module docs for which layer owns which colour.
///
/// Punctuation, variables and anything unrecognised are deliberately
/// **unstyled**: most of a line is one or the other, and a highlighter that
/// colours the majority of the text has stopped highlighting anything.
#[rstest]
#[case::keyword(Capture::Keyword, Color::Magenta)]
#[case::function(Capture::Function, Color::Blue)]
#[case::a_type(Capture::Type, Color::Cyan)]
#[case::string(Capture::String, Color::Green)]
#[case::number(Capture::Number, Color::Yellow)]
#[case::constant(Capture::Constant, Color::Yellow)]
#[case::comment(Capture::Comment, Color::DarkGray)]
#[case::punctuation(Capture::Punctuation, Color::Reset)]
#[case::variable(Capture::Variable, Color::Reset)]
#[case::other(Capture::Other, Color::Reset)]
fn every_capture_maps_to_an_indexed_colour(#[case] capture: Capture, #[case] expected: Color) {
    assert_eq!(ui::capture_colour(capture), expected);
}

/// ...and nothing the diff pane writes a glyph in dictates an exact colour.
///
/// An `Rgb` foreground overrides the reviewer's scheme instead of deferring to
/// it, which is how a tool ends up needing a theme option it should never have
/// needed. The boundary is asserted rather than remembered: the sweep covers
/// the code, the gutter sigils and a comment box's borders — everything with a
/// glyph on it — and the frame deliberately has a comment box in it so the
/// chrome is swept too.
///
/// The **background** is the bounded exception, and it is asserted here as one
/// so that this test cannot be read as forbidding it: the wash that marks a
/// line added or removed is a truecolour mix (see
/// `the_wash_is_the_palettes_own_green_and_red`) and cannot exist in 16
/// colours. Foreground and background never contend for the same channel, so a
/// syntax colour and a wash cannot collide.
#[test]
fn code_is_painted_only_in_indexed_colours() {
    let workspace = Fixture::commented();
    let mut app = workspace.app();
    write_comment(&mut app, "a finding, so the box is swept too");
    let frame = frame_at(&app, 100, 24);
    let area = areas(100, 24, Split::default()).diff;

    let cells = diff_pane_cells(&frame, area);
    assert!(cells.len() > 20, "the pane drew almost nothing to judge");
    for (column, row) in cells {
        let fg = frame[(column, row)].style().fg;
        assert!(
            !matches!(fg, Some(Color::Rgb(..))),
            "cell ({column},{row}) dictates an exact colour instead of using \
             the terminal's: {fg:?}\n{}",
            buffer_text(&frame)
        );
    }

    assert!(
        matches!(
            ui::line_background(LineKind::Added, false),
            Some(Color::Rgb(..))
        ),
        "the wash is no longer truecolour, so this test's exception has gone \
         stale and the rule above is broader than it says"
    );
}

/// Whether `target` sits on the ramp from `from` to the ink the diff washes
/// with — which is what "the diff and the sidebar share one green" means in
/// cells rather than in prose.
fn on_the_ramp(target: Color, from: gradient::Rgb) -> bool {
    (0..=1000).any(|step| {
        let gradient::Rgb(r, g, b) =
            gradient::oklab_mix(from, gradient::INK_DARK, step as f32 / 1000.0);
        Color::Rgb(r, g, b) == target
    })
}

#[test]
fn an_added_line_has_a_green_wash_and_coloured_code() {
    let workspace = Fixture::new();
    let app = workspace.app();
    let frame = frame_at(&app, 100, 24);
    let area = areas(100, 24, Split::default()).diff;
    let added = row_of_sigil(&frame, area, '+');

    let background = diff_bg(&frame, area, added);
    assert!(
        background.is_some(),
        "an added line carries no background tint:\n{}",
        buffer_text(&frame)
    );
    let foregrounds = distinct_foregrounds(&frame, area, added);
    assert!(
        foregrounds.len() > 1,
        "the code is one flat colour rather than syntax coloured: {foregrounds:?}\n{}",
        buffer_text(&frame)
    );
}

/// The wash is drawn from `gradient::ADDED` and `gradient::REMOVED` rather than
/// from a second green and a second red beside them — so the diff and the
/// sidebar's change bar cannot drift into two palettes.
#[test]
fn the_wash_is_the_palettes_own_green_and_red() {
    for (kind, hue) in [
        (LineKind::Added, gradient::ADDED),
        (LineKind::Removed, gradient::REMOVED),
    ] {
        for selected in [false, true] {
            let colour = ui::line_background(kind, selected)
                .unwrap_or_else(|| panic!("{kind:?} selected={selected} has no tint"));
            assert!(
                on_the_ramp(colour, hue),
                "{kind:?} selected={selected} is tinted {colour:?}, which is not \
                 {hue:?} taken toward the ink"
            );
        }
    }
    assert_eq!(
        ui::line_background(LineKind::Context, false),
        None,
        "a context line is tinted, so the tint no longer means added or removed"
    );
}

/// Reversing swaps the foreground and the background, which on a tinted line
/// turns the syntax colours into the wash and the wash into the text —
/// legible in neither direction. The selection is a *brighter* tint instead.
#[test]
fn the_selected_line_is_brighter_rather_than_reversed() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    let area = areas(100, 24, Split::default()).diff;

    let frame = frame_at(&app, 100, 24);
    let selected = area.y + 1;
    assert_eq!(app.line_index(), 0, "the reviewer opens on the first line");
    for x in (area.x + 1)..area.right() - 1 {
        assert!(
            !frame[(x, selected)].modifier.contains(Modifier::REVERSED),
            "the selected line is drawn with reversed video:\n{}",
            buffer_text(&frame)
        );
    }

    let bright = diff_bg(&frame, area, selected).expect("the selection is tinted");
    let neighbour = diff_bg(&frame, area, selected + 1).expect("its neighbour is tinted");
    assert_ne!(
        bright,
        neighbour,
        "the selected line is drawn exactly like the line under it:\n{}",
        buffer_text(&frame)
    );

    // ...and the brightness moves with the cursor rather than being a property
    // of the first row.
    app.on_key(KeyCode::Char('j')).expect("j");
    let moved = frame_at(&app, 100, 24);
    assert_eq!(
        diff_bg(&moved, area, selected + 1),
        Some(bright),
        "the highlight did not move onto the next line:\n{}",
        buffer_text(&moved)
    );
    assert_eq!(
        diff_bg(&moved, area, selected),
        Some(neighbour),
        "the highlight did not leave the line it was on:\n{}",
        buffer_text(&moved)
    );
}

/// A removed line takes its colours from the **base** blob.
///
/// The fixture is a rewrite that does not move: `rewrite.rs` line 2 on both
/// sides, a string literal on the base and a number on the head, at the same
/// columns. A lookup that ignored the side would paint the removed line's
/// string with the number's colour — and a renamed file could not catch that,
/// because a rename already encodes the side in the path.
#[test]
fn a_removed_line_takes_its_colours_from_the_base_blob() {
    let workspace = Fixture::rewritten();
    let app = workspace.app_from("@--");
    let area = areas(100, 24, Split::default()).diff;
    let frame = frame_at(&app, 100, 24);

    let removed = row_of_sigil(&frame, area, '-');
    let added = row_of_sigil(&frame, area, '+');
    let text_of = |y: u16| {
        diff_rows(&frame, area)
            .into_iter()
            .find(|(row, _)| *row == y)
            .map(|(_, text)| text)
            .expect("the row is in the pane")
    };
    assert!(
        text_of(removed).contains(REWRITE_BASE_LINE),
        "the removed half does not show the base blob's text:\n{}",
        buffer_text(&frame)
    );
    assert!(
        text_of(added).contains(REWRITE_HEAD_LINE),
        "the added half does not show the head blob's text:\n{}",
        buffer_text(&frame)
    );

    // Column 7 of the pane's inner area is where a line's own text starts: a
    // five-wide number, a space and the sigil.
    let literal = area.x + 1 + 7 + u16::try_from(REWRITE_LITERAL_COLUMN).expect("a small column");
    assert_eq!(
        frame[(literal, removed)].symbol(),
        "\"",
        "the base side's literal is not where this test looks for it:\n{}",
        buffer_text(&frame)
    );
    assert_eq!(
        frame[(literal, added)].symbol(),
        "1",
        "the head side's literal is not where this test looks for it:\n{}",
        buffer_text(&frame)
    );

    let base_colour = frame[(literal, removed)].style().fg;
    let head_colour = frame[(literal, added)].style().fg;
    assert_ne!(
        base_colour,
        head_colour,
        "the two sides colour that column the same way, so this proves nothing:\n{}",
        buffer_text(&frame)
    );
    assert_eq!(
        base_colour,
        Some(ui::capture_colour(rv_core::highlight::Capture::String)),
        "the removed line's literal is not coloured as the string the base blob \
         has there — its spans came from the head side:\n{}",
        buffer_text(&frame)
    );
    assert_eq!(
        head_colour,
        Some(ui::capture_colour(rv_core::highlight::Capture::Constant)),
        "the added line's literal is not coloured as the number the head blob \
         has there:\n{}",
        buffer_text(&frame)
    );
}

/// A file rv has no grammar for renders plain, and the pane says why rather
/// than leaving the reviewer to guess whether the colour is broken.
#[test]
fn a_file_with_no_grammar_renders_plain_and_says_so() {
    let workspace = Fixture::plain();
    let app = workspace.app();
    let frame = frame_at(&app, 100, 24);
    let area = areas(100, 24, Split::default()).diff;
    let text = buffer_text(&frame);

    assert!(
        text.contains("no highlighting"),
        "the title does not say why the code is plain:\n{text}"
    );
    let added = row_of_sigil(&frame, area, '+');
    assert_eq!(
        distinct_foregrounds(&frame, area, added).len(),
        2,
        "a file with no grammar was coloured anyway: {:?}\n{text}",
        distinct_foregrounds(&frame, area, added)
    );
}

/// ...and a file rv *does* have a grammar for is not labelled as plain.
#[test]
fn a_highlighted_file_is_not_labelled_as_plain() {
    let workspace = Fixture::new();
    let app = workspace.app();
    let text = buffer_text(&frame_at(&app, 100, 24));
    assert!(
        !text.contains("no highlighting"),
        "a Rust file is labelled as having no grammar:\n{text}"
    );
}

/// The wash is a band across the whole pane rather than a tint that stops
/// wherever the line's text happens to end: a ragged right edge reads as a
/// rendering fault rather than as a marked line.
#[test]
fn the_wash_reaches_the_edge_of_the_pane_and_no_further() {
    let workspace = Fixture::new();
    let app = workspace.app();
    let frame = frame_at(&app, 100, 24);
    let area = areas(100, 24, Split::default()).diff;
    let added = row_of_sigil(&frame, area, '+');
    let wash = diff_bg(&frame, area, added).expect("the line is tinted");

    assert_eq!(
        frame[(area.right() - 2, added)].style().bg,
        Some(wash),
        "the tint stops short of the pane's last column:\n{}",
        buffer_text(&frame)
    );
    assert_ne!(
        frame[(area.right() - 1, added)].style().bg,
        Some(wash),
        "the tint spilled onto the pane's own border:\n{}",
        buffer_text(&frame)
    );
}

/// Every key the README's **Browsing** table documents is a row of
/// [`BINDINGS`], which is what chains the page to the code rather than to
/// [`BROWSE_KEYS`]'s hand-kept list of spellings.
///
/// `Ctrl+C` is the exception, and the only one: [`rv::app::App::on_key_event`]
/// answers it before the mode is dispatched at all — it is the universal abort,
/// and it works from behind the `?` popup too, because an abort that first asks
/// you to press `Esc` is not an abort.
#[test]
fn every_documented_browse_key_is_a_row_of_the_binding_table() {
    for key in BROWSE_KEYS {
        if *key == "`Ctrl+C`" {
            continue;
        }
        let spelled = key.replace('`', "");
        assert!(
            BINDINGS.iter().any(|binding| binding.keys == spelled),
            "the README documents {key}, which is not a row of BINDINGS: {:?}",
            BINDINGS.iter().map(|b| b.keys).collect::<Vec<_>>()
        );
    }
}

/// `?` has to be findable from inside the binary, not only from the README.
///
/// As shipped, the popup was reachable only by guessing the key: the status bar
/// is the one surface every reviewer sees and it said nothing about `?`, and
/// the README's table said nothing about it either. This pins the pointer to
/// the manual in the bar itself, and `the_readme_mockup_draws_the_status_bar…`
/// in `app_cases.rs` chains the page's picture to the same constant, so the two
/// cannot drift apart again.
#[test]
fn the_status_bar_says_where_the_keymap_is() {
    let workspace = Fixture::new();
    let app = workspace.app();
    assert!(
        app.status().contains("? help"),
        "the bar a reviewer opens on does not mention the keymap: {:?}",
        app.status()
    );
    assert!(
        last_row(&frame_at(&app, 100, 24)).contains("? help"),
        "...and it is not on screen either"
    );
}

// ---------------------------------------------------------------------------
// The cursor walks rows, so a tall comment can be read
// ---------------------------------------------------------------------------

/// The diff pane's rectangle on a terminal `width` columns wide, sized so the
/// pane itself is `height` rows tall.
///
/// The bar takes the row under both panes, so the terminal has to be a row
/// taller than the pane asked for. The assertion is what keeps that arithmetic
/// honest rather than silently off by one if the chrome ever changes.
fn diff_pane(width: u16, height: u16) -> Rect {
    let area = areas(width, height + 1, Split::default()).diff;
    assert_eq!(
        area.height, height,
        "the diff pane is not {height} rows tall"
    );
    area
}

/// The rows of the diff pane's plan that are on screen at that size, as
/// indices into the plan.
///
/// Asked of [`rv::ui::visible`] — the very function [`rv::ui::draw`] windows
/// with — rather than recomputed here. That matters more in this section than
/// anywhere else in the file: the defect below *was* a window and a cursor
/// disagreeing, and a test with its own copy of the arithmetic would be
/// asserting about a third thing that neither of them uses.
fn visible_row_indices(app: &App, width: u16, height: u16) -> Range<usize> {
    ui::visible(app, diff_pane(width, height)).1
}

/// How many rows that plan holds in total.
fn row_count(app: &App, width: u16, height: u16) -> usize {
    ui::visible(app, diff_pane(width, height)).0.rows.len()
}

/// The defect, stated as a test: a comment taller than the diff pane must not
/// have rows that no cursor position can bring on screen.
///
/// It used to. The pane anchored its window on the row of the selected *diff
/// line* and `j` moved that selection to the next diff line, so a box between
/// two diff rows was stepped over rather than scrolled through: from the line
/// above you saw the box's top, from the line below its bottom, and the middle
/// was reachable from nowhere at all. What looked like scrolling jumping
/// through a comment was the pane never scrolling it.
#[test]
fn every_row_of_a_tall_comment_can_be_brought_on_screen() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, &"a very long finding. ".repeat(40));
    let height = 10;

    let mut seen: HashSet<usize> = HashSet::new();
    for _ in 0..80 {
        seen.extend(visible_row_indices(&app, 100, height));
        app.on_key(KeyCode::Char('j')).expect("j");
    }

    let total = row_count(&app, 100, height);
    assert!(
        total > usize::from(height),
        "the comment is not taller than the pane, so this proves nothing: \
         {total} rows in {height}"
    );
    let missed: Vec<usize> = (0..total).filter(|row| !seen.contains(row)).collect();
    assert!(
        missed.is_empty(),
        "rows unreachable at any cursor position: {missed:?}"
    );
}

/// `j` steps *into* the box rather than over it, and the selection everything
/// else depends on stays the line the box hangs from.
#[test]
fn j_walks_into_a_comment_box_rather_than_over_it() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, &"a long finding. ".repeat(20));
    let line = app.line_index();
    let row = app.cursor_row();

    app.on_key(KeyCode::Char('j')).expect("j");
    assert_eq!(
        app.line_index(),
        line,
        "the cursor left the line instead of walking into its comment"
    );
    assert!(
        app.cursor_row() > row,
        "the cursor did not move down a row: {} then {}",
        row,
        app.cursor_row()
    );
}

/// ...so `c` from inside a box comments on the line that box is about, which
/// is the only thing it could sensibly mean.
#[test]
fn commenting_from_inside_a_box_targets_the_line_the_box_belongs_to() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, &"a long finding. ".repeat(20));
    let line = app.line_index();

    app.on_key(KeyCode::Char('j')).expect("step into the box");
    write_comment(&mut app, "a second finding");
    assert_eq!(
        app.comments_for_line(line).len(),
        2,
        "the second comment did not land on the line the box belongs to"
    );
}

/// ...and the box is somewhere the cursor walks *through*, not into: past its
/// last row is the next diff line.
#[test]
fn stepping_past_the_last_row_of_a_box_lands_on_the_next_diff_line() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, "short");
    let line = app.line_index();

    for _ in 0..8 {
        app.on_key(KeyCode::Char('j')).expect("j");
    }
    assert!(
        app.line_index() > line,
        "the cursor never left the box: still on line {}",
        app.line_index()
    );
}

/// Folding the box the cursor is inside leaves the cursor on the line that box
/// belongs to, rather than on a row index that now means something else.
///
/// A fold is one of the three things that rebuild the plan under the cursor —
/// with a save and a delete — and a tall box collapsing to one row takes every
/// row after it with it. Left alone, the cursor would keep pointing at a row
/// past the end of the shortened plan: `line_index` would answer 0 while the
/// pane, which clamps its own anchor, scrolled to the bottom. The two would be
/// describing different places, which is the shape of the defect this whole
/// section exists to have fixed.
#[test]
fn folding_a_box_the_cursor_is_inside_keeps_the_cursor_on_its_line() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, &"a long finding. ".repeat(20));
    let line = app.line_index();
    for _ in 0..6 {
        app.on_key(KeyCode::Char('j')).expect("step into the box");
    }
    assert_eq!(
        app.line_index(),
        line,
        "the fixture's box is not tall enough for the cursor to be inside it"
    );

    app.on_key(KeyCode::Char('s')).expect("fold it");
    assert_eq!(
        app.line_index(),
        line,
        "the fold left the cursor on another line"
    );
    let plan = app.plan();
    assert_eq!(
        plan.line_of_row(app.cursor_row()),
        Some(line),
        "the cursor is on row {} of a plan that has {} rows",
        app.cursor_row(),
        plan.rows.len()
    );
    assert_eq!(
        plan.row_of_line(line),
        Some(app.cursor_row()),
        "the cursor did not land on the folded line's own row"
    );
}

// ---------------------------------------------------------------------------
// The file list as a counted tree
// ---------------------------------------------------------------------------

/// `t` flips the file list between whole paths and a directory tree, and the
/// pane says which it is showing.
///
/// A chain of single-child directories is one row: `docs/specs` and not `docs`
/// over `specs`. A 29-file review has perhaps 40 rows to spend, and a tree that
/// spent half of them on punctuation would be worse than the list it replaced.
#[test]
fn t_toggles_the_sidebar_between_a_list_and_a_tree() {
    let workspace = Fixture::nested();
    let mut app = workspace.app();

    let list = sidebar_text(&frame_at(&app, 100, 24), 100, 24, Split::default());
    assert!(
        list.contains("docs/specs/a.md"),
        "the flat list names whole paths:\n{list}"
    );
    assert!(
        sidebar_shape(&frame_at(&app, 100, 24)).contains("list"),
        "the pane does not say it is a list: {:?}",
        sidebar_shape(&frame_at(&app, 100, 24))
    );

    app.on_key(KeyCode::Char('t')).expect("t");
    let tree = sidebar_text(&frame_at(&app, 100, 24), 100, 24, Split::default());
    assert_ne!(list, tree, "the sidebar did not change shape");
    assert!(
        tree.contains("docs/specs"),
        "the single-child chain is one row:\n{tree}"
    );
    assert!(
        !tree.contains("docs/specs/a.md"),
        "a file under it is still named by its whole path:\n{tree}"
    );
    assert!(
        tree.contains("a.md") && tree.contains("b.md") && tree.contains("top.rs"),
        "the tree lost a file the flat list had:\n{tree}"
    );
    assert!(
        sidebar_shape(&frame_at(&app, 100, 24)).contains("tree"),
        "the pane does not say it is a tree: {:?}",
        sidebar_shape(&frame_at(&app, 100, 24))
    );

    app.on_key(KeyCode::Char('t')).expect("t again");
    assert_eq!(
        sidebar_text(&frame_at(&app, 100, 24), 100, 24, Split::default()),
        list,
        "t is a toggle, not a one-way door"
    );
}

/// `s` on a directory row folds it away — the project's one verb for *fold the
/// thing under the cursor*, which is already what it means for a comment box
/// and for a browsed comment.
#[test]
fn s_folds_a_directory_row_and_hides_the_files_under_it() {
    let workspace = Fixture::nested();
    let mut app = workspace.app();
    app.on_key(KeyCode::Char('t')).expect("the tree");
    app.on_key(KeyCode::Left).expect("focus the file list");
    // The review opens on the first file, which is under `docs/specs`; one
    // step up is the directory row itself.
    app.on_key(KeyCode::Up).expect("onto the directory row");

    app.on_key(KeyCode::Char('s')).expect("fold it");
    let folded = sidebar_text(&frame_at(&app, 100, 24), 100, 24, Split::default());
    assert!(
        folded.contains("docs/specs"),
        "the directory row itself is gone:\n{folded}"
    );
    assert!(
        !folded.contains("a.md") && !folded.contains("b.md"),
        "its files are still on screen:\n{folded}"
    );
    assert!(
        folded.contains("top.rs") && folded.contains("lib.rs"),
        "folding one directory took its siblings with it:\n{folded}"
    );

    app.on_key(KeyCode::Char('s')).expect("unfold it");
    assert!(
        sidebar_text(&frame_at(&app, 100, 24), 100, 24, Split::default()).contains("a.md"),
        "s did not put the directory back"
    );
}

/// A row that holds others says what the whole subtree costs, folded or not.
///
/// A folded row that hid its own weight would be a row you have to expand to
/// judge, which is the work folding it was meant to save.
#[test]
fn a_directory_row_carries_its_whole_subtrees_count() {
    let workspace = Fixture::nested();
    let mut app = workspace.app();
    app.on_key(KeyCode::Char('t')).expect("the tree");

    let frame = frame_at(&app, 100, 24);
    let row = sidebar_row_for(&frame, "docs/specs");
    let text: String = (0..100).map(|x| frame[(x, row)].symbol()).collect();
    assert!(
        text.contains("+15"),
        "a 10-line file and a 5-line file under one row add up to 15: {text:?}"
    );
}

/// Every row says what it costs to review.
#[test]
fn every_row_shows_what_it_costs_to_review() {
    let workspace = Fixture::mixed();
    let app = workspace.app_from("@--");

    let text = sidebar_text(&frame_at(&app, 100, 24), 100, 24, Split::default());
    assert!(text.contains("+40"), "additions are shown:\n{text}");
    assert!(text.contains("25"), "and so are removals:\n{text}");
}

/// A row too narrow for both gives up its counts and keeps its path.
///
/// The path is the row's identity and the counts are context. The change bar
/// has already gone by this point — see
/// `the_bar_is_dropped_before_the_counts_are` — so the order in which the pane
/// gives things up is bar, counts, path, each more the row's identity than the
/// last.
#[test]
fn a_narrow_sidebar_drops_the_counts_before_the_path() {
    let workspace = Fixture::mixed();
    let mut app = workspace.app_from("@--");
    for _ in 0..30 {
        app.on_key(KeyCode::Char('<')).expect("squeeze the sidebar");
    }

    let split = app.split();
    let text = sidebar_text(&frame_at(&app, 60, 24), 60, 24, split);
    assert!(
        text.contains("added"),
        "the path went before the counts did:\n{text}"
    );
    assert!(
        !text.contains("+40"),
        "the counts survived a column that cannot hold both:\n{text}"
    );
}

/// `o` cycles the order, and the pane names it — a list whose order you cannot
/// see is a list you cannot trust.
#[test]
fn o_cycles_the_order_and_the_sidebar_says_which() {
    let workspace = Fixture::nested();
    let mut app = workspace.app();
    let shape = |app: &App| sidebar_shape(&frame_at(app, 100, 24));

    assert!(shape(&app).contains("natural"), "{:?}", shape(&app));
    app.on_key(KeyCode::Char('o')).expect("o");
    assert!(shape(&app).contains("added"), "{:?}", shape(&app));
    app.on_key(KeyCode::Char('o')).expect("o");
    assert!(shape(&app).contains("removed"), "{:?}", shape(&app));
    app.on_key(KeyCode::Char('o')).expect("o");
    assert!(
        shape(&app).contains("natural"),
        "it does not cycle: {:?}",
        shape(&app)
    );
}

/// ...and the rows actually move when it does.
#[test]
fn sorting_by_additions_puts_the_biggest_file_first() {
    let workspace = Fixture::nested();
    let mut app = workspace.app();

    let natural = sidebar_filled(&frame_at(&app, 100, 24), 100, 24, Split::default());
    assert!(
        natural[0].contains("a.md"),
        "the natural order is path order: {natural:?}"
    );

    app.on_key(KeyCode::Char('o')).expect("order by additions");
    let by_size = sidebar_filled(&frame_at(&app, 100, 24), 100, 24, Split::default());
    assert!(
        by_size[0].contains("top.rs"),
        "the 50-line file is not first: {by_size:?}"
    );
    let mut sorted = by_size.clone();
    sorted.sort();
    let mut was = natural.clone();
    was.sort();
    assert_eq!(sorted, was, "an order that loses a file is worse than none");
}

/// The counts carry the colour: the additions in the palette's green, the
/// removals in its red, as a foreground on the terminal's own ground.
#[test]
fn the_sidebar_colours_the_counts_by_the_shape_of_the_change() {
    let workspace = Fixture::mixed();
    let app = workspace.app_from("@--");
    let frame = frame_at(&app, 100, 24);

    let added = sidebar_row_for(&frame, "added.rs");
    let removed = sidebar_row_for(&frame, "removed.rs");
    assert_eq!(
        style_of_text(&frame, added, "+40").fg,
        Some(colour(gradient::ADDED)),
        "the additions are not the palette's green:\n{}",
        sidebar_text(&frame, 100, 24, Split::default())
    );
    assert_eq!(
        style_of_text(&frame, added, "-0").fg,
        Some(colour(gradient::REMOVED)),
        "the removals are not the palette's red"
    );
    assert_eq!(
        style_of_text(&frame, removed, "-25").fg,
        Some(colour(gradient::REMOVED)),
        "and the other row disagrees with the first"
    );
}

/// **No row is washed.** Spec §7 rules it out after two rounds of looking at
/// the running tool: a full-row wash reads as a selection and competes with the
/// real one, and even a text-width wash paints over the indentation and the
/// fold marks, which in tree mode *are* the structure. The only full-row
/// background in this pane is the selection.
#[test]
fn no_row_of_the_file_list_is_painted_over() {
    let workspace = Fixture::mixed();
    let mut app = workspace.app_from("@--");

    for focused in [false, true] {
        if focused {
            app.on_key(KeyCode::Left).expect("focus the file list");
        }
        let frame = frame_at(&app, 100, 24);
        let area = inner(areas(100, 24, Split::default()).sidebar);
        for y in area.y..area.bottom() {
            for x in area.x..area.right() {
                assert_eq!(
                    bg_of(&frame, x, y),
                    None,
                    "({x},{y}) is painted over with the file list {}:\n{}",
                    if focused { "focused" } else { "unfocused" },
                    sidebar_text(&frame, 100, 24, Split::default())
                );
            }
        }
    }
}

/// The proportion survives as a small bar beside the counts, on a row with the
/// columns to spare — a mark on the row rather than the row itself.
#[test]
fn a_row_with_room_to_spare_draws_its_proportion_as_a_bar() {
    let workspace = Fixture::mixed();
    let mut app = workspace.app_from("@--");
    for _ in 0..12 {
        app.on_key(KeyCode::Char('>')).expect("widen the sidebar");
    }

    let split = app.split();
    let frame = frame_at(&app, 120, 24);
    let area = inner(areas(120, 24, split).sidebar);
    let added = sidebar_row_for_in(&frame, area, "added.rs");
    let removed = sidebar_row_for_in(&frame, area, "removed.rs");

    let bar_of = |row: u16| -> Vec<Option<Color>> {
        (area.x..area.right())
            .filter(|x| frame[(*x, row)].symbol() == "\u{2588}")
            .map(|x| frame[(x, row)].style().fg)
            .collect()
    };
    let green = bar_of(added);
    let red = bar_of(removed);
    assert!(
        !green.is_empty(),
        "no bar on a row with room for one:\n{}",
        text_in(&frame, area)
    );
    assert!(
        green
            .iter()
            .all(|ink| *ink == Some(colour(gradient::ADDED))),
        "a file that is nothing but additions has a bar that is not all green: {green:?}"
    );
    assert!(
        red.iter()
            .all(|ink| *ink == Some(colour(gradient::REMOVED))),
        "a file that is nothing but removals has a bar that is not all red: {red:?}"
    );
}

/// ...and it is the first thing given up, ahead of the counts, which are given
/// up ahead of the path. Each is more the row's identity than the last.
#[test]
fn the_bar_is_dropped_before_the_counts_are() {
    let workspace = Fixture::nested();
    let mut app = workspace.app();

    // At the default split these paths leave no room for a bar beside them.
    let text = sidebar_text(&frame_at(&app, 100, 24), 100, 24, Split::default());
    assert!(
        !text.contains('\u{2588}'),
        "the bar was drawn by clipping the names:\n{text}"
    );
    assert!(
        text.contains("+10"),
        "and it took the counts with it:\n{text}"
    );

    // Squeezed further, the counts go too and the names stay.
    for _ in 0..30 {
        app.on_key(KeyCode::Char('<')).expect("squeeze the sidebar");
    }
    let split = app.split();
    let text = sidebar_text(&frame_at(&app, 60, 24), 60, 24, split);
    assert!(
        !text.contains("+10") && !text.contains('\u{2588}'),
        "the counts outlived the path:\n{text}"
    );
    assert!(text.contains("top.rs"), "the path went first:\n{text}");
}

/// A change with no shape says nothing: no counts, no bar, and none of the
/// palette's colours. A gradient over zero changed lines would be inventing a
/// ratio.
#[test]
fn a_pure_rename_is_left_neutral() {
    let workspace = Fixture::pure_rename();
    let app = workspace.app_from("@--");
    let frame = frame_at(&app, 100, 24);
    let area = inner(areas(100, 24, Split::default()).sidebar);

    let row = sidebar_row_for(&frame, "b.rs");
    let text = row_in(&frame, area, row);
    assert!(
        !text.contains('+') && !text.contains('\u{2588}'),
        "a rename that changed no line was counted anyway: {text:?}"
    );
    for x in area.x..area.right() {
        let ink = frame[(x, row)].style().fg;
        assert!(
            ink != Some(colour(gradient::ADDED)) && ink != Some(colour(gradient::REMOVED)),
            "column {x} of a rename that changed no line carries a change colour"
        );
    }
}

/// The colours are computed once, when the review is opened, and never move.
#[test]
fn the_colours_do_not_move_as_you_browse() {
    let workspace = Fixture::nested();
    let mut app = workspace.app();
    let before = sidebar_inks(&frame_at(&app, 100, 24));
    assert!(
        before.iter().any(Option::is_some),
        "the file list drew no colour at all, so this proves nothing"
    );

    for _ in 0..3 {
        app.on_key(KeyCode::Char(']')).expect("next file");
    }
    assert_eq!(
        sidebar_inks(&frame_at(&app, 100, 24)),
        before,
        "the colours were recomputed as files were opened"
    );
}

/// The shape, the order and the folds are this session's, like every other view
/// preference in this reviewer.
#[test]
fn the_shape_and_the_order_never_reach_disk() {
    let workspace = Fixture::nested();
    let mut app = workspace.app();
    let before = workspace_tree(workspace.root());

    app.on_key(KeyCode::Char('t')).expect("the tree");
    app.on_key(KeyCode::Char('o')).expect("order by additions");
    app.on_key(KeyCode::Left).expect("focus the file list");
    app.on_key(KeyCode::Char('s')).expect("fold something");
    app.on_key(KeyCode::Char('o')).expect("order by removals");

    assert_eq!(
        workspace_tree(workspace.root()),
        before,
        "how one reviewer likes their file list is not review state"
    );
}

/// Walking onto a directory row moves the cursor and leaves the diff alone: the
/// reviewer chose the file they are reading, and a folder is a thing to fold
/// rather than a file to open.
#[test]
fn the_cursor_can_rest_on_a_directory_without_changing_the_diff() {
    let workspace = Fixture::nested();
    let mut app = workspace.app();
    app.on_key(KeyCode::Char('t')).expect("the tree");
    app.on_key(KeyCode::Left).expect("focus the file list");

    let file = app.file_index();
    app.on_key(KeyCode::Up).expect("onto the directory row");
    assert_eq!(app.file_index(), file, "a directory row selected a file");
    assert!(
        buffer_text(&frame_at(&app, 100, 24)).contains("a.md"),
        "the diff pane stopped showing the file that is selected"
    );

    app.on_key(KeyCode::Down).expect("back onto the file");
    assert_eq!(app.file_index(), file, "and coming back moved it");
}

/// The file list's cursor is on the row that holds the selected file, whatever
/// order the rows are in and however the file came to be selected.
///
/// The two are different numbers the moment an order moves a row: under
/// `added` the review's first file is the *third* row here, and `]` from it
/// lands on the fourth. A cursor that stayed at the row number would highlight
/// a file nobody selected — and `s`, which acts on the row, would be aimed at
/// it.
#[test]
fn the_file_lists_cursor_follows_the_file_that_is_selected() {
    let workspace = Fixture::nested();
    let mut app = workspace.app();
    // Natural order is path order: a.md (10), b.md (5), lib.rs (30), top.rs
    // (50). By additions it is top.rs, lib.rs, a.md, b.md.
    assert_eq!((app.file_index(), app.sidebar_row()), (0, 0));

    app.on_key(KeyCode::Char('o')).expect("order by additions");
    assert_eq!(app.file_index(), 0, "reordering moved the selection");
    assert_eq!(
        app.sidebar_row(),
        2,
        "the cursor stayed at a row number instead of following the file"
    );

    app.on_key(KeyCode::Char(']')).expect("next file");
    assert_eq!(app.file_index(), 1, "] did not move to the next file");
    assert_eq!(
        app.sidebar_row(),
        3,
        "the 5-line file is the last row under this order"
    );

    // ...and the pane highlights that row rather than the file's own index.
    app.on_key(KeyCode::Left).expect("focus the file list");
    let frame = frame_at(&app, 100, 24);
    let area = inner(areas(100, 24, Split::default()).sidebar);
    let highlighted = (area.y..area.bottom())
        .find(|y| frame[(area.x, *y)].modifier.contains(Modifier::REVERSED))
        .expect("the focused file list highlights a row");
    assert!(
        row_in(&frame, area, highlighted).contains("b.md"),
        "the highlight is on {:?}",
        row_in(&frame, area, highlighted)
    );
}

/// `t` and `o` are preferences about the file list, so from the comment browser
/// they refuse and say where the list they are about is.
#[test]
fn the_view_keys_say_they_are_about_the_file_list() {
    let workspace = Fixture::nested();
    let mut app = workspace.app();
    app.on_key(KeyCode::Tab).expect("the comments tab");
    let tree = app.tree_view();
    let sort = app.sort();

    app.on_key(KeyCode::Char('t')).expect("t");
    assert_eq!(app.tree_view(), tree, "t reshaped a list nobody can see");
    assert!(
        app.status().contains("file list"),
        "t refused without saying why: {:?}",
        app.status()
    );

    app.on_key(KeyCode::Char('o')).expect("o");
    assert_eq!(app.sort(), sort, "o reordered a list nobody can see");
    assert!(
        app.status().contains("file list"),
        "o refused without saying why: {:?}",
        app.status()
    );
}

// ---------------------------------------------------------------------------
// The bar is the status bar
// ---------------------------------------------------------------------------

/// The bar along the bottom is drawn from `rv::statusbar`, so it says what mode
/// the keyboard is in, which file is selected and where in the list, what is
/// under review, how many comments are open, and where the keymap is.
#[test]
fn the_bar_is_drawn_from_the_status_bars_segments() {
    let workspace = Fixture::new();
    let app = workspace.app();

    let bar = last_row(&frame_at(&app, 100, 24));
    assert!(
        bar.contains("BROWSE"),
        "the mode is not on the bar: {bar:?}"
    );
    assert!(bar.contains("a.rs"), "nor the selected file: {bar:?}");
    assert!(bar.contains("1/2"), "nor how far through the list: {bar:?}");
    assert!(bar.contains("trunk()"), "nor what is in scope: {bar:?}");
    assert!(bar.contains("0 open"), "nor the comment count: {bar:?}");
    assert!(bar.contains("? help"), "nor where the keymap is: {bar:?}");
}

/// The defect the `?` popup was a workaround for, pinned in the frame: a status
/// message is one segment among six now, so it cannot take the keymap hint's
/// place.
#[test]
fn a_status_message_can_no_longer_evict_the_keymap_hint() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, "a finding");
    assert!(
        app.status().contains("comment saved"),
        "the fixture did not produce a status message: {:?}",
        app.status()
    );

    let bar = last_row(&frame_at(&app, 120, 24));
    assert!(
        bar.contains("comment saved"),
        "the message is not on the bar at all: {bar:?}"
    );
    assert!(
        bar.contains("? help"),
        "the message took the hint's place: {bar:?}"
    );
    assert!(bar.contains("BROWSE"), "...and the mode's with it: {bar:?}");
}

/// A confirmation keeps the whole row. It is a modal question whose answer
/// destroys written work, and a question that could be dropped for want of room
/// is a question the reviewer answers blind.
#[test]
fn a_confirmation_is_never_dropped_for_want_of_room() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, "a finding");
    app.on_key(KeyCode::Char('d')).expect("d");

    let bar = last_row(&frame_at(&app, 24, 24));
    assert!(
        bar.contains("delete"),
        "the question was dropped like a status message: {bar:?}"
    );
    assert!(
        bar.contains('…'),
        "...and what did not fit was not marked: {bar:?}"
    );
}

/// `RV_ASCII` is read once, by the app, and the renderer draws from what the
/// app read rather than asking the environment per frame.
///
/// "Once" is a property of the code's shape rather than of a frame — there is
/// no way to observe a syscall from out here — so what is pinned is the chain:
/// the app owns the answer, and the bar on screen agrees with it.
#[test]
fn the_powerline_glyphs_are_decided_by_the_app_at_startup() {
    let workspace = Fixture::new();
    let app = workspace.app();

    assert_eq!(
        app.ascii(),
        rv::statusbar::ascii_from(std::env::var_os(rv::statusbar::RV_ASCII).as_deref()),
        "the app did not read the switch the status bar defines"
    );
    let bar = last_row(&frame_at(&app, 100, 24));
    assert_eq!(
        bar.contains('\u{e0b0}'),
        !app.ascii(),
        "the bar drew glyphs the app did not ask for: {bar:?}"
    );
}

// ---------------------------------------------------------------------------
// Focus colours the border
// ---------------------------------------------------------------------------

/// The focused pane's border is the magenta accent and the other pane's is not.
#[test]
fn the_focused_pane_border_is_the_accent_and_the_other_is_not() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    let rects = areas(100, 24, Split::default());
    let corner = |frame: &Buffer, area: Rect| frame[(area.x, area.y)].style().fg;
    let accent = Some(colour(gradient::FOCUS));

    let frame = frame_at(&app, 100, 24);
    assert_eq!(
        corner(&frame, rects.diff),
        accent,
        "the diff has focus at launch and its border is not the accent"
    );
    assert_ne!(
        corner(&frame, rects.sidebar),
        accent,
        "the sidebar does not have focus and its border is the accent anyway"
    );

    app.on_key(KeyCode::Left).expect("focus the sidebar");
    let frame = frame_at(&app, 100, 24);
    assert_eq!(
        corner(&frame, rects.sidebar),
        accent,
        "the accent did not move"
    );
    assert_ne!(corner(&frame, rects.diff), accent, "...and did not let go");
}

/// The accent is none of the colours that already mean something.
#[test]
fn the_accent_is_none_of_the_colours_that_already_mean_something() {
    for taken in [
        gradient::ADDED,
        gradient::REMOVED,
        gradient::COMMENT,
        gradient::ALERT,
    ] {
        assert_ne!(
            gradient::FOCUS,
            taken,
            "the focus accent must be unambiguous"
        );
    }
}

/// The `▸` on the focused pane's title survives the colour.
///
/// Redundant on purpose: a sixteen-colour terminal, or a reader who does not
/// separate magenta from red, still needs to know where the keys go.
#[test]
fn the_title_marker_survives_the_colour() {
    let workspace = Fixture::new();
    let app = workspace.app();
    assert!(buffer_text(&frame_at(&app, 100, 24)).contains('▸'));
}

/// The panes' borders are rounded.
#[test]
fn the_panes_borders_are_rounded() {
    let workspace = Fixture::new();
    let app = workspace.app();
    let frame = frame_at(&app, 100, 24);
    let rects = areas(100, 24, Split::default());

    for area in [rects.sidebar, rects.diff] {
        assert_eq!(
            frame[(area.x, area.y)].symbol(),
            "╭",
            "a pane's top-left corner is square"
        );
        assert_eq!(
            frame[(area.x, area.bottom() - 1)].symbol(),
            "╰",
            "a pane's bottom-left corner is square"
        );
    }
}

// ---------------------------------------------------------------------------
// The mouse
// ---------------------------------------------------------------------------

/// A left-button press at `(column, row)`, which is what a click sends first
/// and the only half of one `rv` acts on: a click is a *choice*, and the choice
/// is made where the button went down.
fn click(column: u16, row: u16) -> MouseEvent {
    mouse(MouseEventKind::Down(MouseButton::Left), column, row)
}

/// The same event, under the name a drag starts with. Spelled twice on purpose:
/// a press on the divider begins a resize and a press anywhere else is a click,
/// and reading `press(divider, 6)` beside `click(60, 6)` is what says so.
fn press(column: u16, row: u16) -> MouseEvent {
    click(column, row)
}

fn drag(column: u16, row: u16) -> MouseEvent {
    mouse(MouseEventKind::Drag(MouseButton::Left), column, row)
}

fn release(column: u16, row: u16) -> MouseEvent {
    mouse(MouseEventKind::Up(MouseButton::Left), column, row)
}

fn scroll_down(column: u16, row: u16) -> MouseEvent {
    mouse(MouseEventKind::ScrollDown, column, row)
}

fn scroll_up(column: u16, row: u16) -> MouseEvent {
    mouse(MouseEventKind::ScrollUp, column, row)
}

fn mouse(kind: MouseEventKind, column: u16, row: u16) -> MouseEvent {
    MouseEvent {
        kind,
        column,
        row,
        modifiers: KeyModifiers::NONE,
    }
}

/// Paints a frame at `width` x `height` — which is how the app comes to know
/// the geometry the pointer is about to be over — and answers the frame row of
/// the diff pane's `row`-th content row.
///
/// The frame is not incidental. A click resolves against the layout that was
/// *painted*, so a test that clicked without drawing would be asking about a
/// screen the reviewer never saw. Assumes [`Mode::Browse`], which is the only
/// mode with a one-row bar.
fn diff_pane_row(app: &App, width: u16, height: u16, row: u16) -> u16 {
    let _ = frame_at(app, width, height);
    inner(areas(width, height, app.split()).diff).y + row
}

/// The same for the sidebar.
fn sidebar_pane_row(app: &App, width: u16, height: u16, row: u16) -> u16 {
    let _ = frame_at(app, width, height);
    inner(areas(width, height, app.split()).sidebar).y + row
}

/// The same for the one column between the panes, which is the resize handle.
fn divider_column(app: &App, width: u16, height: u16) -> u16 {
    let _ = frame_at(app, width, height);
    areas(width, height, app.split()).divider.x
}

/// Clicking a diff line selects it and hands the keys to the diff.
#[test]
fn clicking_a_diff_line_selects_it_and_focuses_the_diff() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    app.on_key(KeyCode::Left).expect("focus the sidebar");
    assert_eq!(app.focus(), Focus::Sidebar);

    let row = diff_pane_row(&app, 100, 24, 2);
    app.on_mouse(click(60, row)).expect("click in the diff");

    assert_eq!(app.focus(), Focus::Diff, "the click moved the focus");
    assert_eq!(
        app.line_index(),
        2,
        "and selected the line under the pointer"
    );
}

/// A click below the last row of the plan selects nothing at all.
///
/// Slop that points at a row nothing was drawn on is not slop: the reviewer
/// clicked empty space, and a clamp onto the last line would be the tool
/// choosing a line they did not.
#[test]
fn clicking_below_the_last_row_of_the_diff_selects_nothing() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    app.on_key(KeyCode::Left).expect("focus the sidebar");
    let before = (app.focus(), app.line_index());

    let row = diff_pane_row(&app, 100, 24, 12);
    app.on_mouse(click(60, row)).expect("click on empty space");

    assert_eq!(
        (app.focus(), app.line_index()),
        before,
        "a click on a row nothing was painted on moved something"
    );
}

/// Clicking a comment box steps into that line's stack, on that comment.
#[test]
fn clicking_a_comment_box_focuses_the_stack_and_selects_that_comment() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, "a finding");

    let frame = frame_at(&app, 100, 24);
    let (_, row) = find_char_in(&frame, box_area(), '╭').expect("a comment box is drawn");
    app.on_mouse(click(60, row)).expect("click the box");

    assert_eq!(app.focus(), Focus::Stack);
    assert_eq!(
        app.selected_comment().expect("a selected comment").body,
        "a finding"
    );
}

/// Clicking a file row selects that file and hands the keys to the file list.
#[test]
fn clicking_a_file_row_selects_that_file_and_focuses_the_sidebar() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    assert_eq!(app.focus(), Focus::Diff);

    let row = sidebar_pane_row(&app, 100, 24, 1);
    app.on_mouse(click(3, row)).expect("click the second file");

    assert_eq!(app.focus(), Focus::Sidebar);
    assert_eq!(app.selected_file().expect("a file").path, "b.rs");
    assert_eq!(app.sidebar_row(), 1);
}

/// Clicking a directory row folds it, which is what `s` does to the row under
/// the cursor — one verb, reached two ways.
#[test]
fn clicking_a_directory_row_folds_it() {
    let workspace = Fixture::nested();
    let mut app = workspace.app();
    app.on_key(KeyCode::Char('t')).expect("the tree");
    let folded = app
        .sidebar_nodes()
        .iter()
        .position(|node| node.label == "docs/specs")
        .expect("a directory row");

    let row = sidebar_pane_row(&app, 100, 24, u16::try_from(folded).expect("a small row"));
    app.on_mouse(click(3, row)).expect("click the directory");

    let labels: Vec<String> = app
        .sidebar_nodes()
        .iter()
        .map(|node| node.label.clone())
        .collect();
    assert!(
        labels.iter().any(|label| label == "docs/specs"),
        "the directory row itself is gone: {labels:?}"
    );
    assert!(
        !labels.iter().any(|label| label.ends_with("a.md")),
        "its children are still listed: {labels:?}"
    );
}

/// Dragging the divider resizes the panes and moves nothing else.
#[test]
fn dragging_the_divider_resizes_and_changes_nothing_else() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    let before = (app.file_index(), app.line_index(), app.focus());

    let divider = divider_column(&app, 100, 24);
    app.on_mouse(press(divider, 6)).expect("press the divider");
    app.on_mouse(drag(divider + 10, 6)).expect("drag");
    app.on_mouse(release(divider + 10, 6)).expect("release");

    assert!(
        app.split().ratio() > Split::DEFAULT,
        "the split did not follow the pointer: {}",
        app.split().ratio()
    );
    assert_eq!(
        (app.file_index(), app.line_index(), app.focus()),
        before,
        "the resize moved something other than the divider"
    );
}

/// The pointer stops dragging the divider when the button comes up.
#[test]
fn the_divider_stops_following_the_pointer_at_the_release() {
    let workspace = Fixture::new();
    let mut app = workspace.app();

    let divider = divider_column(&app, 100, 24);
    app.on_mouse(press(divider, 6)).expect("press");
    app.on_mouse(drag(divider + 10, 6)).expect("drag");
    app.on_mouse(release(divider + 10, 6)).expect("release");
    let settled = app.split().ratio();

    app.on_mouse(drag(divider + 25, 6)).expect("move on");
    assert_eq!(
        app.split().ratio(),
        settled,
        "the divider kept following a pointer that had let go of it"
    );
}

/// The wheel moves the view and leaves the selection where it was.
///
/// Scrolling is looking; clicking is choosing. A wheel nudge that moved the
/// selection would silently re-aim the next `c` or `d` at another line.
#[test]
fn scrolling_moves_the_view_without_moving_the_selection() {
    let workspace = Fixture::mixed();
    let mut app = workspace.app_from("@--");
    let selected = app.line_index();

    let row = diff_pane_row(&app, 100, 24, 3);
    let before = visible_row_indices(&app, 100, 23);
    app.on_mouse(scroll_down(60, row)).expect("scroll");
    let after = visible_row_indices(&app, 100, 23);

    assert_eq!(
        app.line_index(),
        selected,
        "scrolling is looking, not choosing — cursor row {}, file {:?}, \
         plan {} rows, window {before:?} then {after:?}",
        app.cursor_row(),
        app.selected_file().map(|file| file.path.clone()),
        row_count(&app, 100, 23),
    );
    assert!(
        after.start > before.start,
        "the view did not move: {before:?} then {after:?}"
    );

    app.on_mouse(scroll_up(60, row)).expect("scroll back");
    assert_eq!(
        visible_row_indices(&app, 100, 23),
        before,
        "the wheel does not come back"
    );
}

/// The same for the file list: the wheel looks ahead down a list too long for
/// the pane without moving which file is selected.
#[test]
fn scrolling_the_sidebar_looks_ahead_without_moving_the_selection() {
    let workspace = Fixture::nested();
    let mut app = workspace.app();
    app.on_key(KeyCode::Char('t'))
        .expect("the tree, which is taller");

    let before = sidebar_rows(&frame_at(&app, 60, 8), 60, 8, Split::default());
    let selected = (app.sidebar_row(), app.file_index());

    let row = sidebar_pane_row(&app, 60, 8, 1);
    app.on_mouse(scroll_down(3, row))
        .expect("scroll the file list");

    let after = sidebar_rows(&frame_at(&app, 60, 8), 60, 8, Split::default());
    assert_ne!(before, after, "the file list did not scroll:\n{before:?}");
    assert_eq!(
        (app.sidebar_row(), app.file_index()),
        selected,
        "scrolling the list moved the selection"
    );
}

/// No gesture destroys review state. There is no click target for `d`, and
/// dragging a comment does nothing: the confirmation exists because deletion is
/// unrecoverable, and a mis-click is exactly the accident it guards against.
#[test]
fn no_gesture_deletes_anything() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, "a finding");
    assert_eq!(app.comments().len(), 1);

    let diff = diff_pane_row(&app, 100, 24, 1);
    let sidebar = sidebar_pane_row(&app, 100, 24, 0);
    let divider = divider_column(&app, 100, 24);
    let before = workspace_tree(workspace.root());

    for event in [
        click(60, diff),
        click(3, sidebar),
        scroll_up(60, diff),
        scroll_down(60, diff),
        press(divider, 6),
        drag(divider + 6, 6),
        release(divider + 6, 6),
        press(60, diff),
        drag(60, diff + 2),
        release(60, diff + 2),
    ] {
        app.on_mouse(event).expect("gesture");
    }

    assert_eq!(app.comments().len(), 1, "a gesture removed a comment");
    assert_eq!(
        workspace_tree(workspace.root()),
        before,
        "the mouse reached disk"
    );
}

/// A click lands on the line the frame actually painted, scrolled or not.
///
/// The scroll is the point: with the view moved off the top of the plan, a
/// hit test that forgot the window's offset still resolves to *a* line, and the
/// only way to tell is to read what was drawn on the row that was clicked.
#[test]
fn a_click_lands_on_the_line_the_frame_actually_painted() {
    let workspace = Fixture::mixed();
    let mut app = workspace.app_from("@--");

    let row = diff_pane_row(&app, 100, 24, 4);
    for _ in 0..3 {
        app.on_mouse(scroll_down(60, row)).expect("scroll");
    }

    let frame = frame_at(&app, 100, 24);
    let painted = row_in(&frame, inner(areas(100, 24, app.split()).diff), row);
    app.on_mouse(click(60, row)).expect("click");

    let selected = app.selected_diff().expect("a diff").lines[app.line_index()]
        .text
        .clone();
    assert!(
        painted.trim_end().ends_with(&selected),
        "clicked a row painted {painted:?} and selected {selected:?}"
    );
}

/// The bar is not a click target.
#[test]
fn clicking_the_bar_does_nothing() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    let before = (app.focus(), app.line_index(), app.file_index());
    let _ = frame_at(&app, 100, 24);

    app.on_mouse(click(40, 23)).expect("click the bar");

    assert_eq!(
        (app.focus(), app.line_index(), app.file_index()),
        before,
        "the status bar answered a click"
    );
}

/// The mouse is inert while a comment is being typed.
///
/// A click that moved the selection under a half-typed comment would save that
/// comment against a line the reviewer never chose — the same silent re-aiming
/// the wheel is kept away from, with a body attached.
#[test]
fn the_mouse_is_inert_while_a_comment_is_being_typed() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    let row = diff_pane_row(&app, 100, 24, 2);
    let sidebar = sidebar_pane_row(&app, 100, 24, 1);
    let divider = divider_column(&app, 100, 24);

    app.on_key(KeyCode::Char('c')).expect("open the box");
    let before = (
        app.focus(),
        app.line_index(),
        app.file_index(),
        app.split().ratio(),
    );

    for event in [
        click(60, row),
        click(3, sidebar),
        scroll_down(60, row),
        press(divider, 6),
        drag(divider + 10, 6),
        release(divider + 10, 6),
    ] {
        app.on_mouse(event).expect("gesture");
    }

    assert_eq!(app.mode(), Mode::Comment, "a gesture left the comment box");
    assert_eq!(
        (
            app.focus(),
            app.line_index(),
            app.file_index(),
            app.split().ratio()
        ),
        before,
        "a gesture moved something under a half-typed comment"
    );
}

/// While the `?` popup is up the pointer moves nothing under it, and the wheel
/// scrolls the keymap exactly as `j` and `k` do.
#[test]
fn the_mouse_is_inert_while_the_help_is_open() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    let row = diff_pane_row(&app, 100, 24, 2);
    let sidebar = sidebar_pane_row(&app, 100, 24, 1);

    app.on_key(KeyCode::Char('?')).expect("?");
    let before = (app.focus(), app.line_index(), app.file_index());

    app.on_mouse(click(50, 12)).expect("click inside the popup");
    app.on_mouse(click(60, row)).expect("click behind it");
    app.on_mouse(click(3, sidebar)).expect("click beside it");
    assert!(app.help_open(), "a click closed the keymap");
    assert_eq!(
        (app.focus(), app.line_index(), app.file_index()),
        before,
        "a click reached through the keymap"
    );

    app.on_mouse(scroll_down(50, 12))
        .expect("scroll the keymap");
    assert_eq!(app.help_scroll(), 1, "the wheel scrolls the keymap");
    app.on_mouse(scroll_up(50, 12)).expect("scroll back");
    assert_eq!(app.help_scroll(), 0);
}

// ---------------------------------------------------------------------------
// Alerts that float and fade
// ---------------------------------------------------------------------------

/// Where the toast floats at `width` x `height`, asked of the same [`layout`]
/// that painted it.
fn toast_area(width: u16, height: u16) -> Rect {
    layout(
        Rect::new(0, 0, width, height),
        Split::default(),
        Chrome {
            bar_rows: 1,
            help_open: false,
            toast: true,
        },
    )
    .toast
    .expect("a toast has a rectangle at this size")
}

/// The colour the toast's border is drawn in.
fn toast_border_colour(buffer: &Buffer) -> Color {
    let area = toast_area(buffer.area.width, buffer.area.height);
    buffer[(area.x, area.y)]
        .style()
        .fg
        .expect("the toast's border carries a colour")
}

/// How light a colour is, for comparing one step of the fade against the next.
fn luma(colour: Color) -> f32 {
    match colour {
        Color::Rgb(red, green, blue) => {
            0.2126 * f32::from(red) + 0.7152 * f32::from(green) + 0.0722 * f32::from(blue)
        }
        other => panic!("{other:?} is not a colour with a lightness"),
    }
}

/// An alert shows up, stays a few seconds, and leaves on its own.
#[test]
fn an_alert_appears_then_leaves_on_its_own() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    let t0 = Instant::now();

    app.alert("src/old.rs is no longer in this range", t0);
    assert_eq!(app.alerts().len(), 1);
    assert!(
        buffer_text(&frame_at_time(&app, 100, 24, t0)).contains("no longer in this range"),
        "the toast is not on screen:\n{}",
        buffer_text(&frame_at_time(&app, 100, 24, t0))
    );

    app.expire_alerts(t0 + Duration::from_secs(2));
    assert_eq!(app.alerts().len(), 1, "still up at two seconds");

    app.expire_alerts(t0 + Duration::from_secs(6));
    assert!(app.alerts().is_empty(), "gone by six");
    assert!(
        !buffer_text(&frame_at_time(&app, 100, 24, t0 + Duration::from_secs(6)))
            .contains("no longer in this range")
    );
}

/// The toast takes no key and steals no focus: it is a notification, not a
/// dialog.
#[test]
fn the_toast_takes_no_key_and_steals_no_focus() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    let focus = app.focus();
    let line = app.line_index();

    app.alert("something went wrong", Instant::now());
    app.on_key(KeyCode::Char('j')).expect("j");

    assert_eq!(app.focus(), focus, "the toast took the focus");
    assert_eq!(app.line_index(), line + 1, "and j still moved the line");
    assert_eq!(app.alerts().len(), 1, "the key dismissed it");
}

/// The toast fades down over its last second rather than blinking out.
#[test]
fn the_border_dims_as_the_deadline_approaches() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    let t0 = Instant::now();
    app.alert("careful", t0);

    let bright = toast_border_colour(&frame_at_time(&app, 100, 24, t0));
    let faded = toast_border_colour(&frame_at_time(
        &app,
        100,
        24,
        t0 + Duration::from_millis(4600),
    ));

    assert_ne!(bright, faded, "the toast vanishes rather than fading");
    assert!(
        luma(faded) < luma(bright),
        "it faded up, not down: {bright:?} then {faded:?}"
    );
}

/// The event loop is told how long it may block for.
#[test]
fn the_event_loop_is_told_when_to_wake_up() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    let t0 = Instant::now();

    assert_eq!(
        app.next_deadline(t0),
        None,
        "an idle rv waits for a key, forever"
    );

    app.alert("careful", t0);
    let wait = app
        .next_deadline(t0)
        .expect("a live alert gives the loop a timeout");
    assert!(
        wait <= Duration::from_secs(5),
        "the timeout outlives the alert: {wait:?}"
    );

    app.expire_alerts(t0 + Duration::from_secs(6));
    assert_eq!(
        app.next_deadline(t0 + Duration::from_secs(6)),
        None,
        "an expired alert still asks the loop to wake up"
    );
}

/// Two alerts at once are both readable.
///
/// **Not one panel each.** `rv::layout::layout` gives the toast three rows —
/// two borders and one message — and no rectangle in this reviewer is computed
/// anywhere but there, so several alerts share the panel rather than stacking
/// down the screen. The claim the plan's version of this test makes is that
/// none of them is lost, and that is what is asserted.
#[test]
fn several_alerts_share_the_panel_and_none_is_lost() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    let t0 = Instant::now();

    app.alert("first", t0);
    app.alert("second", t0);

    let text = buffer_text(&frame_at_time(&app, 100, 24, t0));
    assert!(
        text.contains("first"),
        "the first alert is not on screen:\n{text}"
    );
    assert!(
        text.contains("second"),
        "the second alert is not on screen:\n{text}"
    );
}

/// A jump to a comment whose file has left the range is an alert, not only a
/// status: nothing moved, and a line in the bar is the easiest thing on screen
/// to miss.
#[test]
fn a_jump_to_a_file_that_left_the_range_raises_an_alert() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, "a finding");

    // A range whose only change touches no file at all, so the comment above is
    // anchored outside it.
    let mut later = workspace.app_from("@-");
    assert!(
        later.files().is_empty(),
        "the later range still holds the commented file: {:?}",
        later.files()
    );
    assert_eq!(later.comments().len(), 1);
    assert!(later.alerts().is_empty(), "{:?}", later.alerts().len());

    later.on_key(KeyCode::Left).expect("the sidebar");
    later.on_key(KeyCode::Tab).expect("the comment browser");
    later.on_key(KeyCode::Enter).expect("jump");

    assert!(
        later.status().contains("not in this review's range"),
        "the bar says nothing about it: {:?}",
        later.status()
    );
    assert_eq!(
        later.alerts().len(),
        1,
        "a stale anchor is a status and nothing else"
    );
    assert!(
        later.alerts()[0].message.contains("a.rs"),
        "the alert does not name the file: {:?}",
        later.alerts()[0].message
    );
}

/// The same failure twice is one toast.
///
/// A panel reading `x · x` says nothing the first `x` did not, and a reviewer
/// who pressed `Enter` twice on a comment that cannot be jumped to has been told
/// once already.
#[test]
fn the_same_failure_twice_is_one_toast() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, "a finding");
    let mut later = workspace.app_from("@-");

    later.on_key(KeyCode::Left).expect("the sidebar");
    later.on_key(KeyCode::Tab).expect("the comment browser");
    for _ in 0..3 {
        later.on_key(KeyCode::Enter).expect("jump");
    }

    assert_eq!(
        later.alerts().len(),
        1,
        "one failure, three tellings: {:?}",
        later.alerts()
    );
    let t0 = Instant::now();
    later.alert("a stale finding", t0);
    later.alert("a stale finding", t0);
    assert_eq!(
        later.alerts().len(),
        2,
        "a second, different alert is its own"
    );
}

/// An alert raised where no clock is in reach is stamped by the first pass of
/// the event loop, and ages from there.
///
/// This is what keeps `App` clock-free without leaving an alert immortal: a key
/// press knows what went wrong and nothing about the time, so the time is
/// applied by whoever has it.
#[test]
fn an_alert_raised_before_the_clock_is_known_is_stamped_by_the_first_pass() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, "a finding");
    let mut later = workspace.app_from("@-");

    later.on_key(KeyCode::Left).expect("the sidebar");
    later.on_key(KeyCode::Tab).expect("the comment browser");
    later.on_key(KeyCode::Enter).expect("jump");
    assert_eq!(later.alerts().len(), 1);
    assert_eq!(
        later.alerts()[0].raised,
        None,
        "a key press stamped an alert with a time it could not have"
    );
    assert_eq!(
        later.next_deadline(Instant::now()),
        Some(Duration::ZERO),
        "an unstamped alert does not ask the loop to come straight back"
    );

    let t0 = Instant::now();
    later.expire_alerts(t0);
    assert_eq!(later.alerts()[0].raised, Some(t0), "the pass stamped it");

    later.expire_alerts(t0 + Duration::from_secs(2));
    assert_eq!(
        later.alerts().len(),
        1,
        "it aged from the stamp, not before"
    );
    later.expire_alerts(t0 + Duration::from_secs(6));
    assert!(later.alerts().is_empty());
}

/// Alerts are session-only, like every other preference in this reviewer.
#[test]
fn alerts_are_never_written_anywhere() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    let before = workspace_tree(workspace.root());

    let t0 = Instant::now();
    app.alert("something went wrong", t0);
    let _ = frame_at_time(&app, 100, 24, t0);
    app.expire_alerts(t0 + Duration::from_secs(6));

    assert_eq!(
        workspace_tree(workspace.root()),
        before,
        "an alert reached disk"
    );
}
