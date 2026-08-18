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
use rstest::rstest;
use rv::app::Action;
use rv::app::App;
use rv::app::Focus;
use rv::app::Mode;
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
/// line they may never have opened. Until the sidebar has a comment of its own
/// selected, a destructive key aimed into it is wrong.
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
