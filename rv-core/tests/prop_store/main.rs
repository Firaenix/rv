//! Property-based and parameterized tests for the `.review/` on-disk store
//! (spec §10), complementing the hand-written cases in `tests/store.rs`.
//!
//! The hand-written tests pin single concrete examples of each behaviour. The
//! properties here go after the *laws* the module's doc comment claims, with
//! oracles that are independent of the implementation wherever possible:
//!
//! * an in-memory upsert-and-delete reduction recomputed with a different data
//!   structure (`HashMap` + insertion-order `Vec`) as the oracle for
//!   `comments()`,
//! * for the interleaving properties, an oracle derived from the operation
//!   *sequence* rather than read back off disk — a read-back oracle bakes any
//!   damage an operation did mid-sequence into both sides of the comparison,
//!   so it can only ever catch a cache,
//! * whole-directory byte snapshots as a conservation oracle ("nothing lost,
//!   nothing invented") for operations that must not touch other files,
//! * permutation invariance, idempotence and last-write-wins as algebraic laws,
//! * and, for the module's headline crash-safety claims, *forced* failures
//!   rather than a wait for a real crash: a `session.toml` rewrite made to
//!   fail, after which the review must be exactly as it was, and a file handle
//!   opened before a write and read after it (a rename leaves the old inode
//!   whole, so the holder sees the complete previous document — a plain
//!   in-place rewrite does not).
//!
//! Two shapes recur because both are places identity gets matched by something
//! looser than it should be: comment ids of *varying length that are prefixes
//! of one another*, so `==` is distinguishable from `starts_with`, and
//! `change_id`s drawn from a pool small enough that collisions are the norm
//! rather than a `hex(12)` lottery.
//!
//! Like `tests/store.rs`, these need no jj repository — only a tempdir shaped
//! like a repo root, so `.git/info/exclude` lands where `ensure_excluded`
//! looks for it.

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::path::PathBuf;

use proptest::prelude::*;
use rv_core::model::Anchor;
use rv_core::model::ChangeRef;
use rv_core::model::Side;
use rv_core::store::Comment;
use rv_core::store::CommentState;
use rv_core::store::Session;

/// The line `Store::ensure_excluded` is documented to append.
const EXCLUDE_LINE: &str = "/.review/";

/// The prefix `write_atomic`'s temp files carry, per the module doc, so a
/// leftover is recognizable as `rv`'s.
const TEMP_PREFIX: &str = ".rv-store-";

/// Case counts are kept low deliberately — every case does real filesystem
/// work, including an `fsync` per write — but `max_shrink_iters` defaults to a
/// multiple of the case count, which would leave counterexamples full of noise
/// exactly when they matter. Shrinking only runs after a failure, so raising
/// it costs nothing on the green path and buys a small, readable
/// counterexample when a property does bite.
fn config(cases: u32) -> ProptestConfig {
    ProptestConfig {
        cases,
        max_shrink_iters: 8192,
        ..ProptestConfig::default()
    }
}

// ---------------------------------------------------------------------------
// fixtures
// ---------------------------------------------------------------------------

/// A fresh temp directory laid out like a repo root — same fixture shape as
/// `tests/store.rs`, kept local because integration test targets do not share
/// helpers.
fn repo_root() -> tempfile::TempDir {
    let tempdir = tempfile::tempdir().expect("create temp dir");
    fs::create_dir_all(tempdir.path().join(".git/info")).expect("create .git/info");
    tempdir
}

/// Every regular file under `dir`, keyed by path. Used as a conservation
/// oracle: if an operation is supposed to leave other files alone, comparing
/// two of these detects any byte changed anywhere, including in files the test
/// did not think to name.
fn dir_snapshot(dir: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut out = BTreeMap::new();
    let mut stack = vec![dir.to_owned()];
    while let Some(current) = stack.pop() {
        let Ok(entries) = fs::read_dir(&current) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if let Ok(bytes) = fs::read(&path) {
                out.insert(path, bytes);
            }
        }
    }
    out
}

/// Every regular file under `dir`, as a set of paths relative to it. The
/// companion to [`dir_snapshot`]: where that one asks "did any byte change?",
/// this one asks "does any file exist that nothing was entitled to create?".
fn relative_files(dir: &Path) -> BTreeSet<PathBuf> {
    dir_snapshot(dir)
        .into_keys()
        .map(|path| {
            path.strip_prefix(dir)
                .expect("dir_snapshot only yields paths under dir")
                .to_owned()
        })
        .collect()
}

/// Any of `write_atomic`'s temp files still lying around under `dir`,
/// recursively. On the happy path the rename consumes every one of them.
fn stray_temp_files(dir: &Path) -> Vec<PathBuf> {
    dir_snapshot(dir)
        .into_keys()
        .filter(|path| {
            path.file_name()
                .map(|name| name.to_string_lossy().starts_with(TEMP_PREFIX))
                .unwrap_or(false)
        })
        .collect()
}

/// The one-file law (storage spec §2): whatever sequence of saves,
/// settlements and deletions ran, the only thing under `.review/` is
/// `session.toml` — plus `REVIEW-FEEDBACK.md` where an export was asked for.
/// No `comments.json`, no `snapshots/`, and no second copy of an excerpt.
fn only_one_file(root: &Path) -> Result<(), proptest::test_runner::TestCaseError> {
    let unexpected: Vec<PathBuf> = relative_files(&root.join(".review"))
        .into_iter()
        .filter(|path| path != Path::new("session.toml"))
        .filter(|path| path != Path::new("REVIEW-FEEDBACK.md"))
        .collect();
    prop_assert!(
        unexpected.is_empty(),
        "rv wrote something beside the one file it maintains: {unexpected:?}"
    );
    Ok(())
}

/// The oracle for `Store::comments()` over a sequence of appends and nothing
/// else: last write wins per id, first-insertion order preserved. A thin
/// specialization of [`expected_comments`], the model that also knows about
/// removals, so the two can never drift apart.
fn upsert_reduce(sequence: &[Comment]) -> Vec<Comment> {
    let appends: Vec<Op> = sequence.iter().cloned().map(Op::Append).collect();
    expected_comments(&appends)
}

/// A concrete, boring comment, for the table-driven cases where the content is
/// beside the point and only the file's fate matters.
fn fixed_comment(id: &str) -> Comment {
    Comment {
        id: id.to_owned(),
        change_id: "nowwnlnmvkwo".to_owned(),
        commit_id: "abc123def456".to_owned(),
        anchor: Anchor {
            file: "src/lib.rs".to_owned(),
            side: Side::Right,
            line: 1,
            content_hash: "deadbeef".to_owned(),
            context: vec!["fn a() {}".to_owned()],
            context_start: 1,
        },
        body: "why".to_owned(),
        state: CommentState::Open,
        reply: None,
        settled_by: None,
    }
}

fn ids_of(comments: &[Comment]) -> Vec<String> {
    comments.iter().map(|c| c.id.clone()).collect()
}

fn sorted_by_id(mut comments: Vec<Comment>) -> Vec<Comment> {
    comments.sort_by(|a, b| a.id.cmp(&b.id));
    comments
}

mod strategies;

use strategies::*;

// --- oracles computed from an op *sequence*, never read back from disk -------
//
// The point of these three is that they are a model of what the sequence
// implies, evaluated without touching the store. A property whose expectation
// is a second read through the same code path can only catch a cache; one whose
// expectation comes from the sequence catches an operation that wrote the
// wrong bytes, or wrote them into somebody else's file.

/// The whole of `session.toml` after the sequence.
///
/// `Append` upserts by id — last write wins, first-insertion order preserved —
/// and `Remove` drops that id's entry, or, when the id is not there, does
/// nothing whatsoever, exactly as `remove_comment` documents. `WriteReview`
/// replaces the file wholesale, comments included: it is the one write that
/// says "this is the review now", which is why `session::build` reads the
/// stored comments before it calls it.
///
/// The comment reduction is deliberately computed with a different shape than
/// the store's in-place `iter_mut().find()` and `retain` — a hash map for the
/// values plus a separate vector for the order — so it is a recomputation
/// rather than a restatement. The order vector is what makes the removal
/// clause say something: it records *where* a surviving comment sits, so a
/// delete that shuffles the survivors, or one that resurrects a removed id at
/// its old position on re-append, both diverge from it.
fn expected_review(ops: &[Op]) -> Option<Session> {
    let mut written = false;
    let mut scope = Session::default();
    let mut order: Vec<String> = Vec::new();
    let mut latest: HashMap<String, Comment> = HashMap::new();
    for op in ops {
        match op {
            Op::Append(comment) => {
                written = true;
                if latest.insert(comment.id.clone(), comment.clone()).is_none() {
                    order.push(comment.id.clone());
                }
            }
            Op::Remove(id) => {
                // An unknown id writes nothing at all — `remove_comment`
                // returns before the rewrite — so it cannot bring the file
                // into existence.
                if latest.remove(id).is_some() {
                    written = true;
                    order.retain(|surviving| surviving != id);
                }
            }
            Op::WriteReview(review) => {
                written = true;
                scope = review.clone();
                order = review
                    .comments
                    .iter()
                    .map(|comment| comment.id.clone())
                    .collect();
                latest = review
                    .comments
                    .iter()
                    .map(|comment| (comment.id.clone(), comment.clone()))
                    .collect();
            }
            Op::WriteMarkdown(_) | Op::EnsureExcluded => {}
        }
    }
    written.then(|| Session {
        comments: order
            .into_iter()
            .map(|id| latest.remove(&id).expect("id was recorded"))
            .collect(),
        ..scope
    })
}

/// The comment array of [`expected_review`], which is what `comments()`
/// returns — empty when the sequence never wrote the file at all.
fn expected_comments(ops: &[Op]) -> Vec<Comment> {
    expected_review(ops)
        .map(|review| review.comments)
        .unwrap_or_default()
}

/// Whether the sequence ever wrote `session.toml`, which is what decides
/// whether the file exists at all. A sequence that appends and then removes
/// everything leaves the file behind holding an empty array, so the file's
/// *existence* cannot be predicted from [`expected_comments`] being non-empty.
fn wrote_the_review_file(ops: &[Op]) -> bool {
    expected_review(ops).is_some()
}

/// What `REVIEW-FEEDBACK.md` must hold: the last document written, or nothing
/// at all if the sequence never wrote one.
fn expected_markdown(ops: &[Op]) -> Option<Vec<u8>> {
    ops.iter()
        .rev()
        .find_map(|op| match op {
            Op::WriteMarkdown(document) => Some(document.clone()),
            _ => None,
        })
        .map(String::into_bytes)
}

mod faults;
mod isolation;
mod laws;
mod onefile;
mod readers;
mod removal;
mod tables;
