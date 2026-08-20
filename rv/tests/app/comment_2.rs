//! Saving a comment.

use crossterm::event::KeyCode;
use rv::session;

use crate::support::*;

/// A comment stored under an id this version of `rv` would never derive still
/// resolves, and a new comment beside it neither disturbs nor duplicates it.
///
/// This is the compatibility question the `comment_id` seed change raises —
/// adding the anchor's side changed every id the function produces, so a
/// `.review/` written by the previous build carries ids that no longer match
/// what its own location and body would hash to today. Nothing recomputes an
/// id to find a comment: the store is keyed by the id it stored. So a review
/// in progress keeps working across the change. Rather than assert that from
/// reading the code, this drives it: `0badc0de` is not a digest of anything
/// here.
#[test]
fn a_comment_stored_under_a_foreign_id_keeps_working() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, "written by the previous build");

    // Re-key the stored comment to an id this build's seed cannot produce.
    const LEGACY: &str = "0badc0de";
    let store = workspace.store();
    let mut review = store.read_review().expect("read the review");
    assert_eq!(review.comments.len(), 1, "{:?}", review.comments);
    let derived = review.comments[0].id.clone();
    assert_ne!(derived, LEGACY);
    review.comments[0].id = LEGACY.to_owned();
    store.write_review(&review).expect("re-key the comment");

    // The export is a projection of the store, so it carries the legacy marker.
    let review = session::build(workspace.root(), None, None).expect("build the review");
    session::write_markdown(&review).expect("rewrite the export");
    assert!(
        workspace.markdown().contains(LEGACY),
        "the export does not carry the legacy id:\n{}",
        workspace.markdown()
    );

    // Answering it goes through the store, and a second comment saves beside
    // it with this build's id scheme.
    let review = session::read(workspace.root(), None, None).expect("read the review");
    session::reply(&review, LEGACY, "still addressable").expect("store the reply");
    drop(app);
    let mut app = workspace.app();
    app.on_key(KeyCode::Char('j')).expect("move down a line");
    write_comment(&mut app, "written by this build");

    let comments = workspace.store().comments().expect("read the comments");
    assert_eq!(comments.len(), 2, "{comments:?}");
    let legacy = comments
        .iter()
        .find(|comment| comment.id == LEGACY)
        .unwrap_or_else(|| panic!("the legacy comment was lost or re-keyed: {comments:?}"));
    assert_eq!(legacy.body, "written by the previous build");
    assert_eq!(legacy.reply.as_deref(), Some("still addressable"));

    // An explicit render — the only writer left — carries the reply.
    session::write_markdown(&session::read(workspace.root(), None, None).expect("read"))
        .expect("render the export");
    assert!(
        workspace
            .markdown()
            .contains("**Reply:** still addressable"),
        "the render dropped the reply to the legacy comment:\n{}",
        workspace.markdown()
    );
}
