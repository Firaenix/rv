//! Exporting the review with `e`.

use std::fs;

use crossterm::event::KeyCode;

use crate::support::*;

/// A delete deliberately leaves the export alone, so the document keeps
/// claiming a comment that is gone until something rewrites it. `e` is that
/// something, without quitting the reviewer.
#[test]
fn e_refreshes_an_export_a_delete_left_stale() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, "worth exporting");
    write_comment(&mut app, "and then withdrawn");
    app.on_key(KeyCode::Char('e')).expect("export both");

    app.on_key(KeyCode::Char('d')).expect("ask to delete");
    app.on_key(KeyCode::Char('y')).expect("confirm");
    assert!(
        workspace.markdown().contains("and then withdrawn"),
        "the delete rewrote the export, so this test proves nothing:\n{}",
        workspace.markdown()
    );

    app.on_key(KeyCode::Char('e')).expect("export");

    let document = workspace.markdown();
    assert!(
        !document.contains("and then withdrawn"),
        "`e` left the deleted comment in the document:\n{document}"
    );
    assert!(
        document.contains("worth exporting"),
        "`e` dropped the comment that is still there:\n{document}"
    );
}

/// The status line names what was written, because a key that writes a file
/// silently is a key a reviewer presses twice.
#[test]
fn the_status_line_names_the_file() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, "a remark");

    app.on_key(KeyCode::Char('e')).expect("export");

    assert!(
        app.status().contains("REVIEW-FEEDBACK.md"),
        "the status line does not say what was written: {:?}",
        app.status()
    );
}

/// `e` renders the store and only the store: the document is a view, so an
/// edit made to the file is overwritten, not ingested — the reply channel is
/// `rv reply`, and a pre-amendment reply is rescued at *load*, not at export.
#[test]
fn exporting_overwrites_the_document_without_reading_it_back() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, "please explain");
    app.on_key(KeyCode::Char('e')).expect("first export");

    let replied = insert_reply(&workspace.markdown(), "because of the deadline");
    fs::write(
        workspace.root().join(".review/REVIEW-FEEDBACK.md"),
        &replied,
    )
    .expect("write the reply into the document");

    app.on_key(KeyCode::Char('e')).expect("second export");

    assert!(
        !workspace.markdown().contains("because of the deadline"),
        "the export read the document back, which is the round trip this \
         amendment deleted:\n{}",
        workspace.markdown()
    );
    let stored = workspace.store().comments().expect("read the comments");
    assert_eq!(
        stored[0].reply, None,
        "the export wrote into the store, which only `rv reply` may do"
    );
}

/// A review nobody has commented on still has a session to describe, so the key
/// writes rather than refusing.
#[test]
fn e_exports_a_review_with_no_comments() {
    let workspace = Fixture::new();
    let mut app = workspace.app();

    app.on_key(KeyCode::Char('e')).expect("export");

    assert!(
        workspace.root().join(".review/REVIEW-FEEDBACK.md").exists(),
        "an empty review exported nothing at all"
    );
}
