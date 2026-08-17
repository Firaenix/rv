//! Tests for the `.review/` on-disk store (spec §10).
//!
//! `Store` talks only to the filesystem, so unlike the jj-lib tests elsewhere
//! in this crate these do not need a real jj repository. Each test builds a
//! bare `<tempdir>/.git/info/` layout by hand instead of paying for a real
//! `jj git init --colocate` via the [`fixture`] module — there is nothing
//! here for jj itself to do.

use std::fs;

use rv_core::model::Anchor;
use rv_core::model::ChangeRef;
use rv_core::model::Side;
use rv_core::store::Comment;
use rv_core::store::CommentState;
use rv_core::store::Session;
use rv_core::store::Store;

/// A fresh temp directory laid out like a repo root: just enough of `.git/`
/// for `Store::ensure_excluded` to find (or create) `info/exclude` at the
/// expected relative path.
fn repo_root() -> tempfile::TempDir {
    let tempdir = tempfile::tempdir().expect("create temp dir");
    fs::create_dir_all(tempdir.path().join(".git/info")).expect("create .git/info");
    tempdir
}

fn sample_anchor() -> Anchor {
    Anchor {
        file: "src/lib.rs".to_owned(),
        side: Side::Right,
        line: 3,
        content_hash: "deadbeef".to_owned(),
        context: vec![
            "fn a() {".to_owned(),
            "    let x = 1;".to_owned(),
            "}".to_owned(),
        ],
    }
}

fn sample_comment(id: &str) -> Comment {
    Comment {
        id: id.to_owned(),
        change_id: "nowwnlnmvkwo".to_owned(),
        commit_id: "abc123def456".to_owned(),
        anchor: sample_anchor(),
        body: "why does this exist".to_owned(),
        state: CommentState::Open,
        reply: None,
    }
}

/// First call appends the line and reports that it did; the second call is a
/// no-op and reports that. Either way the file ends up with exactly one
/// `/.review/` line, never a duplicate.
#[test]
fn ensure_excluded_adds_review_exactly_once() {
    let repo = repo_root();
    let store = Store::open(repo.path()).expect("open store");

    let first = store.ensure_excluded().expect("first ensure_excluded");
    let second = store.ensure_excluded().expect("second ensure_excluded");

    assert!(first, "first call should add the line");
    assert!(!second, "second call should find it already present");

    let exclude =
        fs::read_to_string(repo.path().join(".git/info/exclude")).expect("read exclude file");
    let occurrences = exclude.lines().filter(|line| *line == "/.review/").count();
    assert_eq!(occurrences, 1, "exclude file:\n{exclude}");
}

/// `append_comment` is write-through: a second, independently opened `Store`
/// over the same root must see the comment immediately, with no separate
/// flush or close step.
#[test]
fn appended_comments_persist_immediately() {
    let repo = repo_root();
    let store = Store::open(repo.path()).expect("open store");
    let comment = sample_comment("c1");

    store.append_comment(&comment).expect("append comment");

    let reopened = Store::open(repo.path()).expect("reopen store");
    let comments = reopened.comments().expect("read comments");

    assert_eq!(comments, vec![comment]);
}

/// Appending a comment whose id matches an existing one updates it in place
/// rather than adding a duplicate entry, and leaves the other comment's
/// position untouched.
#[test]
fn same_id_updates() {
    let repo = repo_root();
    let store = Store::open(repo.path()).expect("open store");
    let mut first = sample_comment("c1");
    let second = sample_comment("c2");
    store.append_comment(&first).expect("append first");
    store.append_comment(&second).expect("append second");

    first.state = CommentState::Resolved;
    first.reply = Some("fixed in the next commit".to_owned());
    store.append_comment(&first).expect("append updated first");

    let comments = store.comments().expect("read comments");

    assert_eq!(comments.len(), 2, "update must not add a third entry");
    assert_eq!(comments[0], first, "c1 stays in its original slot, updated");
    assert_eq!(comments[1], second, "c2 is untouched");
}

/// Every appended comment gets a snapshot file named after its id, holding
/// its anchor's context lines verbatim.
#[test]
fn snapshot_file_written() {
    let repo = repo_root();
    let store = Store::open(repo.path()).expect("open store");
    let comment = sample_comment("c1");

    store.append_comment(&comment).expect("append comment");

    let snapshot_path = repo.path().join(".review/snapshots/c1");
    let snapshot = fs::read_to_string(&snapshot_path).expect("read snapshot file");
    assert_eq!(snapshot, comment.anchor.context.join("\n"));
}

/// `write_session` followed by `read_session` (even from a freshly opened
/// `Store`) reproduces the exact `Session` that was written.
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
    };

    store.write_session(&session).expect("write session");

    let reopened = Store::open(repo.path()).expect("reopen store");
    let read_back = reopened.read_session().expect("read session");

    assert_eq!(read_back, session);
}
