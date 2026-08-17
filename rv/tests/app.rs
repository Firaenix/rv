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

use crossterm::event::KeyCode;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use rv::app::App;
use rv::app::Mode;
use rv::session;
use rv::ui;
use rv_core::anchor;
use rv_core::diff::DiffLine;
use rv_core::diff::LineKind;
use rv_core::model::ChangeKind;
use rv_core::model::Side;
use rv_core::store::CommentState;
use rv_core::store::Store;
use tempfile::TempDir;

/// The file every fixture reviews. Two indented lines so that `j` has
/// somewhere to go and the diff pane has something recognizable to render.
const SOURCE: &str = "fn a() {\n    let x = 1;\n}\n";

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
    /// [`SOURCE`] as `a.rs`.
    fn new() -> Self {
        let fixture = Self {
            tempdir: tempfile::tempdir().expect("create temp dir"),
        };
        fixture.jj(&["git", "init", "--colocate"]);
        fixture.write("a.rs", SOURCE);
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

#[test]
fn frame_renders_file_list_and_diff() {
    let workspace = Fixture::new();
    let app = workspace.app();

    let rendered = render(&app);

    assert!(rendered.contains("a.rs"), "{rendered}");
    assert!(rendered.contains("let x = 1;"), "{rendered}");
}
