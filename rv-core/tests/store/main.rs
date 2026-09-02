//! Tests for the `.review/` on-disk store (spec §10).
//!
//! `Store` talks only to the filesystem, so unlike the jj-lib tests elsewhere
//! in this crate these do not need a real jj repository. Each test builds a
//! bare `<tempdir>/.git/info/` layout by hand instead of paying for a real
//! `jj git init --colocate` via the [`fixture`] module — there is nothing
//! here for jj itself to do.
//!
//! The v1.0.0 `comments.json` migration and the `session.toml` round trip it
//! lands in are in [`migration`], which shares the fixtures below.

use std::fs;

use rv_core::model::Anchor;
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
        context_start: 1,
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
        settled_by: None,
    }
}

mod migration;

/// First call appends the line and reports that it did; the second call is a
/// no-op and reports that. Either way the file ends up with exactly one
/// `/.review/` line, never a duplicate.
#[test]
fn ensure_excluded_adds_review_exactly_once() {
    let repo = repo_root();
    let store = Store::open(repo.path(), "main").expect("open store");

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
    let store = Store::open(repo.path(), "main").expect("open store");
    let comment = sample_comment("c1");

    store.append_comment(&comment).expect("append comment");

    let reopened = Store::open(repo.path(), "main").expect("reopen store");
    let comments = reopened.comments().expect("read comments");

    assert_eq!(comments, vec![comment]);
}

/// Appending a comment whose id matches an existing one updates it in place
/// rather than adding a duplicate entry, and leaves the other comment's
/// position untouched.
#[test]
fn same_id_updates() {
    let repo = repo_root();
    let store = Store::open(repo.path(), "main").expect("open store");
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

/// Comment identity is `id`, not `change_id`. Every comment made during one
/// review session against the same change carries that change's id, so a
/// reviewer leaving several notes on one change is the *normal* case, not an
/// edge case: two comments with distinct ids must both survive, in insertion
/// order, even though their `change_id` is identical. Upserting by
/// `change_id` instead would let the second comment silently replace the
/// first, losing review work with no error.
#[test]
fn distinct_ids_with_one_change_id_all_persist() {
    let repo = repo_root();
    let store = Store::open(repo.path(), "main").expect("open store");

    let mut first = sample_comment("c1");
    first.body = "first note on this change".to_owned();
    first.anchor.line = 3;

    let mut second = sample_comment("c2");
    second.body = "second note on the same change".to_owned();
    second.anchor.line = 9;

    assert_eq!(
        first.change_id, second.change_id,
        "the whole point: one change, two comments"
    );
    assert_ne!(first.id, second.id, "distinct comment identities");

    store.append_comment(&first).expect("append first");
    store.append_comment(&second).expect("append second");

    let comments = store.comments().expect("read comments");

    assert_eq!(
        comments.len(),
        2,
        "both comments on the same change must persist, got: {comments:#?}"
    );
    assert_eq!(comments[0], first, "first comment, in insertion order");
    assert_eq!(comments[1], second, "second comment, in insertion order");
}

/// A save writes `session.toml` and nothing else: one file is the whole
/// store, so there is no second copy of an excerpt and no cross-file ordering
/// rule (storage spec §2).
#[test]
fn a_save_writes_only_session_toml() {
    let repo = repo_root();
    let store = Store::open(repo.path(), "main").expect("open store");
    let comment = sample_comment("c1");

    store.append_comment(&comment).expect("append comment");

    let mut written: Vec<String> = fs::read_dir(repo.path().join(".review/reviews/main"))
        .expect("read the review's directory")
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    written.sort();
    assert_eq!(
        written,
        ["session.toml"],
        "a save wrote something beside the one file rv maintains"
    );
    let stored = store.comments().expect("read back")[0].clone();
    assert_eq!(
        stored.anchor.context, comment.anchor.context,
        "the one copy of the context is the stored anchor's"
    );
}

/// Every write in the store goes through a temp-file-plus-rename helper so
/// that a destination file is never observed half-written. On the happy
/// path — no crash, no kill — that temp file is renamed away, so after a
/// batch of successful appends (inserts and an update) `.review/` should have
/// none of the helper's temp files left behind.
#[test]
fn append_comment_leaves_no_stray_temp_files() {
    let repo = repo_root();
    let store = Store::open(repo.path(), "main").expect("open store");

    store
        .append_comment(&sample_comment("c1"))
        .expect("append c1");
    store
        .append_comment(&sample_comment("c2"))
        .expect("append c2");
    let mut updated = sample_comment("c1");
    updated.state = CommentState::Resolved;
    store.append_comment(&updated).expect("append updated c1");

    assert_no_stray_temp_files(&repo.path().join(".review"));
}

/// Simulates a crash between "temp file written" and "temp file renamed
/// into place": a stray, half-written temp file is dropped next to an
/// already-good `session.toml`, without ever renaming it on top. This is
/// exactly the write `write_atomic` would have been doing at the moment of
/// an interruption. Because reads only ever look at `session.toml` itself
/// — never at sibling temp files — the last good state must still be
/// exactly what `comments()` returns: the interrupted write can strand a
/// stray file, but it can never corrupt the file readers actually consult.
#[test]
fn interrupted_write_never_disturbs_the_last_good_session() {
    let repo = repo_root();
    let store = Store::open(repo.path(), "main").expect("open store");
    let good = sample_comment("c1");
    store.append_comment(&good).expect("append good comment");

    let stray_temp = repo.path().join(".review/.rv-store-crash-simulated.tmp");
    fs::write(
        &stray_temp,
        b"not valid toml: this is what a half-written\n",
    )
    .expect("seed stray temp file simulating an interrupted write");

    let comments = store
        .comments()
        .expect("comments() must still read the last good file");
    assert_eq!(comments, vec![good]);
}

/// Proves the write is a wholesale replacement, not an in-place patch: after
/// shrinking a comment's body (so the freshly serialized `session.toml` is
/// shorter than what was on disk before), the file on disk is byte-for-byte
/// exactly the new serialization — no trailing bytes surviving from the
/// longer previous version, which a non-atomic in-place overwrite (write
/// new bytes over old without truncating) could otherwise leave behind.
#[test]
fn append_comment_shrinking_body_leaves_no_residual_bytes() {
    let repo = repo_root();
    let store = Store::open(repo.path(), "main").expect("open store");

    let mut long = sample_comment("c1");
    long.body = "x".repeat(500);
    store
        .append_comment(&long)
        .expect("append long-bodied comment");

    let mut short = sample_comment("c1");
    short.body = "y".to_owned();
    store.append_comment(&short).expect("append shrunk comment");

    let on_disk = fs::read_to_string(repo.path().join(".review/reviews/main/session.toml"))
        .expect("read session.toml");
    let expected = toml::to_string_pretty(&Session {
        comments: vec![short],
        ..Session::default()
    })
    .expect("serialize expected");
    assert_eq!(on_disk, expected);
}

/// Deleting a comment removes its entry from `session.toml` and nothing else.
/// The removal is write-through like every other store write, so a freshly
/// opened `Store` over the same root sees the shortened list; and it is
/// surgical, so the other comment stays exactly as it was.
#[test]
fn removing_a_comment_drops_only_that_comment() {
    let repo = repo_root();
    let store = Store::open(repo.path(), "main").expect("open store");
    let survivor = sample_comment("c2");
    store
        .append_comment(&sample_comment("c1"))
        .expect("append c1");
    store.append_comment(&survivor).expect("append c2");

    let removed = store.remove_comment("c1").expect("remove c1");

    assert!(removed, "remove_comment reports it removed something");
    let left = Store::open(repo.path(), "main")
        .expect("reopen store")
        .comments()
        .expect("read comments");
    assert_eq!(
        left,
        vec![survivor],
        "only the other comment survives, whole, and a fresh Store sees it"
    );
}

/// Deleting an id that is not there is a no-op, not a failure. The reviewer
/// can only reach delete through a comment they can see, but a retry after an
/// interrupted delete re-issues an id that is already gone, and that retry
/// must succeed rather than surface an error over work already done.
#[test]
fn removing_an_unknown_id_is_not_an_error() {
    let repo = repo_root();
    let store = Store::open(repo.path(), "main").expect("open store");
    store
        .append_comment(&sample_comment("c1"))
        .expect("append c1");

    let removed = store.remove_comment("nosuchid").expect("remove unknown id");

    assert!(!removed, "nothing was removed");
    assert_eq!(
        store.comments().expect("read comments").len(),
        1,
        "nothing was lost"
    );
}

/// Removal writes `session.toml` through the same temp-file-plus-rename
/// helper as every other store write, so on the happy path it must leave no
/// temp file behind in `.review/`.
#[test]
fn remove_comment_leaves_no_stray_temp_files() {
    let repo = repo_root();
    let store = Store::open(repo.path(), "main").expect("open store");
    store
        .append_comment(&sample_comment("c1"))
        .expect("append c1");
    store
        .append_comment(&sample_comment("c2"))
        .expect("append c2");

    store.remove_comment("c1").expect("remove c1");

    assert_no_stray_temp_files(&repo.path().join(".review"));
}

/// A directory listing helper for [`append_comment_leaves_no_stray_temp_files`]:
/// none of `write_atomic`'s temp files (recognizable by the module's
/// `.rv-store-` prefix) should remain in `dir`.
fn assert_no_stray_temp_files(dir: &std::path::Path) {
    let stray: Vec<String> = fs::read_dir(dir)
        .expect("read dir")
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.contains(".rv-store-"))
        .collect();
    assert!(
        stray.is_empty(),
        "stray temp files left in {}: {stray:?}",
        dir.display()
    );
}
