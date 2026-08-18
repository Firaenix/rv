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

use std::fs;
use std::path::Path;
use std::process::Command;
use std::time::SystemTime;

use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyModifiers;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::style::Color;
use ratatui::style::Modifier;
use rstest::rstest;
use rv::app::Action;
use rv::app::App;
use rv::app::Focus;
use rv::app::Mode;
use rv::app::SidebarTab;
use rv::session;
use rv::ui;
use rv_core::anchor;
use rv_core::diff::DiffLine;
use rv_core::diff::DiffSource;
use rv_core::diff::LineKind;
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

/// One frame of the reviewer, as a 100x24 `TestBackend` renders it.
fn render(app: &App) -> String {
    let mut terminal = Terminal::new(TestBackend::new(100, 24)).expect("build a test terminal");
    terminal
        .draw(|frame| ui::draw(frame, app))
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
    let mut terminal =
        Terminal::new(TestBackend::new(width, height)).expect("build a test terminal");
    terminal
        .draw(|frame| ui::draw(frame, app))
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

/// Where `needle` first appears in the frame, scanning rows top to bottom.
fn find_char(buffer: &Buffer, needle: char) -> Option<(u16, u16)> {
    let wanted = needle.to_string();
    (0..buffer.area.height)
        .flat_map(|y| (0..buffer.area.width).map(move |x| (x, y)))
        .find(|(x, y)| buffer[(*x, *y)].symbol() == wanted)
}

/// Whether the first cell holding `needle` is drawn in blue — the colour this
/// reviewer reserves for comments.
fn styled_blue(buffer: &Buffer, needle: char) -> bool {
    find_char(buffer, needle).is_some_and(|(x, y)| buffer[(x, y)].style().fg == Some(Color::Blue))
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
    assert!(
        text.contains('╭') && text.contains('╰'),
        "the box has borders:\n{text}"
    );
    assert!(
        styled_blue(&buffer, '╭'),
        "the border is blue, which is the requirement:\n{text}"
    );
    assert!(
        styled_blue(&buffer, '╰'),
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
    let (corner_x, _) = find_char(&buffer, '╭').expect("a box top is on screen");
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
    let first_corner = find_char(&browsing, '╭').expect("a box top");
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
    assert!(
        !text.contains('╭') && !text.contains('╰'),
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
    let row = rows_of(&buffer)[row_holding(&buffer, "xxx")].clone();
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

/// The status line is content too: a terminal too narrow for it says so rather
/// than dropping the end of a sentence about a deletion.
#[test]
fn a_status_line_too_long_for_the_terminal_is_marked() {
    let workspace = Fixture::new();
    let app = workspace.app();

    // The help text is 68 columns, which a 40-column terminal cannot show.
    // The bar is the *last* row of the frame — see `rv::layout`.
    let narrow = last_row(&frame_at(&app, 40, 24));
    assert!(
        narrow.ends_with('…'),
        "the status line was cut silently: {narrow:?}"
    );

    let wide = last_row(&frame_at(&app, 100, 24));
    assert!(
        !wide.contains('…') && wide.contains("q quit"),
        "a status line with room to spare was marked anyway: {wide:?}"
    );
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
const BROWSE_KEYS: &[&str] = &[
    "`j` / `↓`",
    "`k` / `↑`",
    "`←`",
    "`→`",
    "`]`",
    "`[`",
    "`Tab`",
    "`Enter`",
    "`Esc`",
    "`c`",
    "`d`",
    "`s`",
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
