//! The v1.0.0 `comments.json` migration (storage spec §6), and the
//! `session.toml` round trip it lands in.

use std::fs;

use rv_core::model::ChangeRef;
use rv_core::store::Comment;
use rv_core::store::CommentState;
use rv_core::store::Session;
use rv_core::store::Store;

use super::repo_root;
use super::sample_anchor;
use super::sample_comment;

/// `write_review` followed by `read_review` (even from a freshly opened
/// `Store`) reproduces the exact `Session` that was written, comments and all.
#[test]
fn session_toml_roundtrip() {
    let repo = repo_root();
    let store = Store::open(repo.path()).expect("open store");
    let session = Session {
        revset: "trunk()..@".to_owned(),
        base_commit: "abc123def456".to_owned(),
        head_commit: "def456abc123".to_owned(),
        changes: vec![ChangeRef {
            change_id: "nowwnlnmvkwo".to_owned(),
            commit_id: "def456abc123".to_owned(),
            description: "do the thing".to_owned(),
        }],
        started_at: "epoch:1755460770".to_owned(),
        comments: vec![sample_comment("c1"), sample_comment("c2")],
    };

    store.write_review(&session).expect("write review");

    let reopened = Store::open(repo.path()).expect("reopen store");
    let read_back = reopened.read_review().expect("read review");

    assert_eq!(read_back, session);
}

/// A `.review/` that no command has recorded yet reads as an empty review
/// rather than an error: `Store::open` creates the directory and writes
/// nothing, and a query against a fresh repository is a legitimate question.
#[test]
fn a_store_with_no_session_file_reads_as_an_empty_review() {
    let repo = repo_root();
    let store = Store::open(repo.path()).expect("open store");

    assert_eq!(
        store.read_review().expect("read review"),
        Session::default()
    );
    assert!(store.comments().expect("read comments").is_empty());
}

/// The v1.0.0 migration (storage spec §6): a `.review/` written by the
/// shipped release has its comments in a sibling `comments.json`, and opening
/// it must bring every one of them through into `session.toml` — an existing
/// review silently emptied by an upgrade is the worst outcome this store has.
#[test]
fn a_v1_review_migrates_every_comment_into_session_toml() {
    let repo = repo_root();
    write_legacy_review(&repo, &["a1b2c3d4", "e5f6a7b8", "9900aabb"]);

    let store = Store::open(repo.path()).expect("open the v1.0.0 review");

    let comments = store.comments().expect("read comments");
    assert_eq!(
        comments
            .iter()
            .map(|comment| comment.id.as_str())
            .collect::<Vec<_>>(),
        ["a1b2c3d4", "e5f6a7b8", "9900aabb"],
        "every v1.0.0 comment must survive the migration, in order"
    );
    assert_eq!(
        comments[1].body, "why does this exist",
        "a migrated comment keeps its body, not just its id"
    );
    assert_eq!(
        comments[0].anchor.context,
        sample_anchor().context,
        "and its stored excerpt"
    );

    let stored = store.read_review().expect("read review");
    assert_eq!(stored.comments, comments, "session.toml is now the store");
    assert_eq!(
        stored.revset, "trunk()..@",
        "the scope the v1.0.0 session recorded is not lost to the fold"
    );
    assert!(
        !repo.path().join(".review/comments.json").exists(),
        "the absorbed comments.json must be gone, or the next open re-folds it forever"
    );
}

/// The interrupted migration, from both sides of the one rename it makes.
///
/// Killed before it: `comments.json` is untouched, so the next open migrates.
/// Killed after it but before the unlink: both files hold the comments, and
/// the next open must fold idempotently rather than duplicate them. The state
/// the user must never be left in — neither file — is unreachable, because
/// the unlink only ever runs after the rename has landed.
#[test]
fn an_interrupted_migration_loses_no_comment_from_either_side() {
    let repo = repo_root();
    write_legacy_review(&repo, &["a1b2c3d4", "e5f6a7b8"]);
    let legacy_bytes =
        fs::read(repo.path().join(".review/comments.json")).expect("read the legacy file");

    // Killed before the rename: nothing has been written, so both files are
    // as v1.0.0 left them and the comments are still only in the JSON.
    assert!(
        !fs::read_to_string(repo.path().join(".review/session.toml"))
            .expect("read the v1.0.0 session")
            .contains("[[comments]]"),
        "the fixture must start with the comments only in comments.json"
    );

    Store::open(repo.path()).expect("first open migrates");

    // Killed between the rename and the unlink: session.toml holds the
    // comments and comments.json is still there. Put the file back to
    // reproduce exactly that, and open again.
    fs::write(repo.path().join(".review/comments.json"), &legacy_bytes)
        .expect("restore the file an interrupted unlink would have left");

    let store = Store::open(repo.path()).expect("second open re-folds");

    let comments = store.comments().expect("read comments");
    assert_eq!(
        comments
            .iter()
            .map(|comment| comment.id.as_str())
            .collect::<Vec<_>>(),
        ["a1b2c3d4", "e5f6a7b8"],
        "re-folding must be idempotent, not duplicate every comment"
    );
    assert!(
        !repo.path().join(".review/comments.json").exists(),
        "the retry must finish the unlink the interruption skipped"
    );
}

/// `session.toml` wins over a legacy twin. Every write since the migration
/// went to `session.toml`, so a `comments.json` a half-finished migration left
/// behind must not roll a reply — or a resolution — back.
#[test]
fn a_stored_comment_is_not_overwritten_by_its_legacy_twin() {
    let repo = repo_root();
    write_legacy_review(&repo, &["a1b2c3d4"]);
    let legacy_bytes =
        fs::read(repo.path().join(".review/comments.json")).expect("read the legacy file");

    let store = Store::open(repo.path()).expect("migrate");
    let mut answered = store.comments().expect("read comments")[0].clone();
    answered.reply = Some("fixed in the next change".to_owned());
    answered.state = CommentState::Resolved;
    store.append_comment(&answered).expect("store the reply");

    fs::write(repo.path().join(".review/comments.json"), &legacy_bytes)
        .expect("restore the stale legacy file");
    let reopened = Store::open(repo.path()).expect("reopen over the stale legacy file");

    let comments = reopened.comments().expect("read comments");
    assert_eq!(comments.len(), 1, "the twin must not be added beside it");
    assert_eq!(
        comments[0].reply.as_deref(),
        Some("fixed in the next change"),
        "the stale legacy file rolled a stored reply back"
    );
    assert_eq!(comments[0].state, CommentState::Resolved);
}

/// An unparseable `comments.json` is an error, not a shrug. It is the user's
/// review, and stepping quietly over a file this tool wrote is how a reviewer
/// loses a day's comments without being told.
#[test]
fn an_unreadable_legacy_file_is_reported_rather_than_skipped() {
    let repo = repo_root();
    fs::create_dir_all(repo.path().join(".review")).expect("create .review");
    fs::write(
        repo.path().join(".review/comments.json"),
        b"[{\"id\": truncated mid-write",
    )
    .expect("write a damaged legacy file");

    let opened = Store::open(repo.path());

    assert!(
        matches!(opened, Err(rv_core::store::Error::InvalidComments { .. })),
        "a damaged comments.json must be named, not silently discarded: {:?}",
        opened.err()
    );
    assert!(
        repo.path().join(".review/comments.json").exists(),
        "and it must still be on disk for the user to repair"
    );
}

/// A `.review/` exactly as v1.0.0 left it: a `session.toml` holding only the
/// scope, and the comments in `comments.json` beside it.
fn write_legacy_review(repo: &tempfile::TempDir, ids: &[&str]) {
    let review = repo.path().join(".review");
    fs::create_dir_all(&review).expect("create .review");
    fs::write(
        review.join("session.toml"),
        "revset = \"trunk()..@\"\n\
         base_commit = \"abc123def456\"\n\
         head_commit = \"def456abc123\"\n\
         started_at = \"epoch:1755460770\"\n\
         changes = []\n",
    )
    .expect("write the v1.0.0 session.toml");
    let comments: Vec<Comment> = ids.iter().map(|id| sample_comment(id)).collect();
    fs::write(
        review.join("comments.json"),
        serde_json::to_string_pretty(&comments).expect("serialize"),
    )
    .expect("write the v1.0.0 comments.json");
}
