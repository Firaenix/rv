//! Saving a comment.

use std::fs;

use crossterm::event::KeyCode;
use rv::app::Mode;
use rv_core::anchor;
use rv_core::diff::DiffSource;
use rv_core::diff::LineKind;
use rv_core::model::ChangeKind;
use rv_core::model::Side;
use rv_core::store::CommentState;

use crate::support::*;

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

    // The markdown is a view rendered on request — saving must not write it.
    assert!(
        !workspace.root().join(".review/REVIEW-FEEDBACK.md").exists(),
        "saving a comment refreshed the export, which nothing reads back"
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
fn a_reply_left_in_an_old_export_is_rescued_on_load() {
    let workspace = Fixture::new();
    let mut app = workspace.app();

    write_comment(&mut app, "first");
    app.on_key(KeyCode::Char('e')).expect("export");

    // What a pre-amendment agent did to the document: append a reply under the
    // entry it just addressed, leaving every marker alone. The CLI is the
    // reply channel now, so this is the migration case (CLI-loop spec §5).
    let replied = insert_reply(&workspace.markdown(), "fixed in the next change");
    fs::write(
        workspace.root().join(".review/REVIEW-FEEDBACK.md"),
        &replied,
    )
    .expect("write the replied-to markdown");

    // The next load rescues it, once: opening the review folds the reply into
    // the store — and only into a comment that has no stored reply.
    drop(app);
    let app = workspace.app();
    let comments = workspace.store().comments().expect("read comments.json");
    let first = comments
        .iter()
        .find(|comment| comment.body == "first")
        .expect("the first comment is still stored");
    assert_eq!(first.reply.as_deref(), Some("fixed in the next change"));
    // A reply is not a state transition.
    assert_eq!(first.state, CommentState::Open);
    // The export itself is not modified by the rescue: it goes stale
    // harmlessly until the next explicit render.
    assert!(
        workspace
            .markdown()
            .contains("**Reply:** fixed in the next change"),
        "the rescue rewrote the export it was reading:\n{}",
        workspace.markdown()
    );
    drop(app);
}
