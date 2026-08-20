//! The session record: which range it points at, and when the review began.

use super::support::*;

/// Opening a review over another range re-points the record, and that is
/// deliberate: a reviewer asking for a narrower range is asking for it.
///
/// The comments do not move and are not mislabelled — each carries its own
/// change, commit and anchor, and the reviewer sees only the ones the open range
/// can reach. What used to be wrong was `rv status` doing this *without being
/// asked*, which `status_writes_nothing` above is the guard for.
#[test]
fn opening_another_range_re_points_the_record_and_keeps_the_comments() {
    let workspace = Fixture::new();
    rv::session::build(workspace.root(), None, None).expect("open the default range");

    let comment = serde_json::json!([{
        "id": "deadbee1",
        "change_id": "z".repeat(32),
        "commit_id": "a".repeat(40),
        "anchor": {
            "file": "a.rs",
            "side": "Right",
            "line": 1,
            "content_hash": "0".repeat(64),
            "context": ["fn a() {"],
        },
        "body": "made against the default range",
        "state": "open",
        "reply": null,
    }]);
    std::fs::write(
        workspace.root().join(".review/comments.json"),
        serde_json::to_vec_pretty(&comment).expect("serialize"),
    )
    .expect("write comments.json");

    rv::session::build(workspace.root(), Some("@-"), None).expect("open a narrower range");

    let session = std::fs::read_to_string(workspace.root().join(".review/session.toml"))
        .expect("read session.toml");
    assert!(
        session.contains("revset = \"@-..@\""),
        "the record does not describe the range that was asked for:\n{session}"
    );
    // The comment is still there, and it came through the v1.0.0 migration on
    // the way: it was written as `comments.json` and is now in `session.toml`.
    let store = rv_core::store::Store::open(workspace.root()).expect("open the store");
    let comments = store.comments().expect("read the comments");
    assert!(
        comments
            .iter()
            .any(|comment| comment.body == "made against the default range"),
        "re-pointing the record deleted a comment: {comments:?}"
    );
    assert!(
        !workspace.root().join(".review/comments.json").exists(),
        "the absorbed legacy file is still on disk"
    );
}

/// Re-opening the *same* range keeps `started_at`: it says when the review began,
/// and re-stamping it on every command would make it say when the reviewer last
/// ran one — which moved the timestamp in the header of an existing export.
#[test]
fn re_opening_the_same_range_keeps_when_the_review_began() {
    let workspace = Fixture::new();
    rv::session::build(workspace.root(), None, None).expect("open");
    let first = std::fs::read_to_string(workspace.root().join(".review/session.toml"))
        .expect("read session.toml");

    rv::session::build(workspace.root(), None, None).expect("open again");
    let again = std::fs::read_to_string(workspace.root().join(".review/session.toml"))
        .expect("read session.toml");

    let started = |toml: &str| {
        toml.lines()
            .find(|line| line.starts_with("started_at"))
            .expect("a started_at")
            .to_owned()
    };
    assert_eq!(started(&first), started(&again), "the clock was restarted");
}
