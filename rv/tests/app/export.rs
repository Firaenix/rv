//! Rendering the review document.
//!
//! Export is no longer a keystroke — it is the `rv render` CLI command, which
//! calls [`session::write_markdown`]. These tests drive that same writer
//! directly, which is the path the command runs.

use std::fs;

use rv::session;

use crate::support::*;

/// A delete deliberately leaves a previously rendered document alone, so it
/// keeps claiming a comment that is gone until something rewrites it. A fresh
/// render is that something.
#[test]
fn a_render_refreshes_a_document_a_delete_left_stale() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, "worth exporting");
    write_comment(&mut app, "and then withdrawn");

    let review = session::read(workspace.root(), None, None).expect("read");
    session::write_markdown(&review).expect("render both");

    app.on_key(crossterm::event::KeyCode::Char('c'))
        .expect("comment leader");
    app.on_key(crossterm::event::KeyCode::Char('d'))
        .expect("ask to delete");
    app.on_key(crossterm::event::KeyCode::Char('y'))
        .expect("confirm");
    assert!(
        workspace.markdown().contains("and then withdrawn"),
        "the delete rewrote the document, so this test proves nothing:\n{}",
        workspace.markdown()
    );

    let review = session::read(workspace.root(), None, None).expect("read");
    session::write_markdown(&review).expect("render again");

    let document = workspace.markdown();
    assert!(
        !document.contains("and then withdrawn"),
        "the render left the deleted comment in the document:\n{document}"
    );
    assert!(
        document.contains("worth exporting"),
        "the render dropped the comment that is still there:\n{document}"
    );
}

/// A render writes the store and only the store: the document is a view, so an
/// edit made to the file is overwritten, never ingested — the reply channel is
/// `rv reply`, and nothing reads this document back at all.
#[test]
fn rendering_overwrites_the_document_without_reading_it_back() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, "please explain");

    let review = session::read(workspace.root(), None, None).expect("read");
    session::write_markdown(&review).expect("first render");

    let replied = insert_reply(&workspace.markdown(), "because of the deadline");
    fs::write(
        workspace.root().join(".review/REVIEW-FEEDBACK.md"),
        &replied,
    )
    .expect("write the reply into the document");

    let review = session::read(workspace.root(), None, None).expect("read");
    session::write_markdown(&review).expect("second render");

    assert!(
        !workspace.markdown().contains("because of the deadline"),
        "the render read the document back, which is the round trip this \
         amendment deleted:\n{}",
        workspace.markdown()
    );
    let stored = workspace.store().comments().expect("read the comments");
    assert_eq!(
        stored[0].reply, None,
        "the render wrote into the store, which only `rv reply` may do"
    );
}

/// A review nobody has commented on still has a session to describe, so a
/// render writes rather than refusing.
#[test]
fn rendering_a_review_with_no_comments_still_writes() {
    let workspace = Fixture::new();
    let review = session::read(workspace.root(), None, None).expect("read");
    session::write_markdown(&review).expect("render");

    assert!(
        workspace.root().join(".review/REVIEW-FEEDBACK.md").exists(),
        "an empty review rendered nothing at all"
    );
}
