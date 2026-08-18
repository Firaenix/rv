//! Saving a comment.

use std::fs;

use crossterm::event::KeyCode;
use rv::session;

use crate::support::*;

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
