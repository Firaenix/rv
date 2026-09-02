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

    // Same head, so the same review key: both opens share one directory and
    // the narrower range re-points that one record.
    let reviews = rv_core::store::Store::list_reviews(workspace.root());
    let [(_, session)] = reviews.as_slice() else {
        panic!("two opens of one head made two reviews: {reviews:?}");
    };
    assert_eq!(
        session.revset, "@-..@",
        "the record does not describe the range that was asked for"
    );
    // The comment is still there, and it came through the v1.0.0 migration on
    // the way: it was written as `comments.json` and is now in `session.toml`.
    let comments = workspace.store().comments().expect("read the comments");
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
    let started = || {
        let (_, session) = rv_core::store::Store::list_reviews(workspace.root())
            .into_iter()
            .next()
            .expect("a stored review");
        session.started_at
    };
    let first = started();

    rv::session::build(workspace.root(), None, None).expect("open again");
    assert_eq!(first, started(), "the clock was restarted");
}

/// Two named heads are two reviews: each bookmark gets its own directory
/// under `.review/reviews/`, comments land in their own review, and jumping
/// back to the first finds it exactly as it was left — the multi-branch
/// workflow the keyed store exists for.
#[test]
fn two_named_heads_are_two_reviews_that_do_not_clobber() {
    let workspace = Fixture::new();
    workspace.write("a.rs", "fn a() {}\n");
    workspace.jj(&["describe", "-m", "first change"]);
    workspace.jj(&["new"]);
    workspace.jj(&["bookmark", "create", "feature-a", "-r", "@-"]);
    workspace.write("c.rs", "fn c() {}\n");
    workspace.jj(&["describe", "-m", "second change"]);
    workspace.jj(&["bookmark", "create", "feature-b", "-r", "@"]);
    workspace.jj(&["new"]);

    let first = rv::session::build(workspace.root(), None, Some("feature-a")).expect("open a");
    rv::session::add_comment(&first, "a.rs", rv_core::model::Side::Right, 1, "note on a")
        .expect("comment on a");
    let second = rv::session::build(workspace.root(), None, Some("feature-b")).expect("open b");
    rv::session::add_comment(&second, "c.rs", rv_core::model::Side::Right, 1, "note on b")
        .expect("comment on b");

    let reviews = rv_core::store::Store::list_reviews(workspace.root());
    let keys: Vec<&str> = reviews.iter().map(|(key, _)| key.as_str()).collect();
    assert_eq!(
        keys,
        ["feature-a", "feature-b"],
        "each named head stores its own review"
    );

    let again =
        rv::session::build(workspace.root(), Some("@---"), Some("feature-a")).expect("reopen a");
    let bodies: Vec<&str> = again
        .session
        .comments
        .iter()
        .map(|comment| comment.body.as_str())
        .collect();
    assert_eq!(
        bodies,
        ["note on a"],
        "jumping back to feature-a must find its own comment and only its own"
    );
}
