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
//! * `split('\n')` as the inverse of the `join("\n")` the snapshot file uses,
//! * whole-directory byte snapshots as a conservation oracle ("nothing lost,
//!   nothing invented") for operations that must not touch other files,
//! * permutation invariance, idempotence and last-write-wins as algebraic laws,
//! * and, for the module's headline crash-safety claims, *forced* failures
//!   rather than a wait for a real crash: a snapshot write made to fail
//!   (`comments.json` is the authority on which comments exist, so a comment
//!   whose snapshot could not be written must not appear in it), the same
//!   ordering read backwards on the delete path (a `comments.json` rewrite made
//!   to fail, after which the snapshot must still be there), and a file handle
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
use rstest::rstest;
use rv_core::model::Anchor;
use rv_core::model::ChangeRef;
use rv_core::model::Side;
use rv_core::store::Comment;
use rv_core::store::CommentState;
use rv_core::store::Session;
use rv_core::store::Store;

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

/// The names of the files in `.review/snapshots/`.
fn snapshot_ids(root: &Path) -> Vec<String> {
    let mut ids: Vec<String> = fs::read_dir(root.join(".review/snapshots"))
        .expect("read snapshots dir")
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    ids.sort();
    ids
}

/// Conservation over `.review/snapshots/`, in both directions: every comment in
/// `expected` has a snapshot holding exactly its anchor's context lines, and no
/// other snapshot file exists — no comment without one, no orphan left by a
/// removal, and nothing invented.
///
/// The content oracle is `split('\n')`, the genuine inverse of the `join("\n")`
/// the store writes, which only works because context lines never contain `\n`
/// (they come from splitting file text, and [`hostile_line`] keeps the
/// generated ones that way). The empty-context case is separate because `join`
/// maps `[]` to `""` while `split` maps `""` back to `[""]`; that asymmetry is
/// inherent to the format, not a bug.
fn snapshots_match(
    root: &Path,
    expected: &[Comment],
) -> Result<(), proptest::test_runner::TestCaseError> {
    let mut expected_ids = ids_of(expected);
    expected_ids.sort();
    prop_assert_eq!(
        snapshot_ids(root),
        expected_ids,
        "exactly the comments that exist may have snapshots"
    );
    for comment in expected {
        let path = root.join(".review/snapshots").join(&comment.id);
        let snapshot = fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("read snapshot {}: {error}", path.display()));
        if comment.anchor.context.is_empty() {
            prop_assert_eq!(
                snapshot.as_str(),
                "",
                "no context lines means an empty snapshot"
            );
        } else {
            prop_assert_eq!(
                snapshot.split('\n').collect::<Vec<_>>(),
                comment
                    .anchor
                    .context
                    .iter()
                    .map(String::as_str)
                    .collect::<Vec<_>>()
            );
        }
    }
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

// ---------------------------------------------------------------------------
// strategies
// ---------------------------------------------------------------------------

/// Characters aimed at every serialization layer the store crosses: JSON
/// metacharacters, TOML metacharacters, C0 controls, the Unicode line and
/// paragraph separators, a BOM, and multi-byte / astral-plane scalars — salted
/// with fully arbitrary `char`s so the set is not just the ones I thought of.
fn hostile_char() -> impl Strategy<Value = char> {
    prop_oneof![
        3 => any::<char>(),
        7 => prop::sample::select(vec![
            '"', '\\', '/', '\n', '\r', '\t', '\0', '\u{1}', '\u{1b}', '\u{7f}',
            '{', '}', '[', ']', ':', ',', '\'', '=', '#', '.', ' ',
            'é', 'ß', '中', '🙂', '\u{2028}', '\u{feff}',
        ]),
    ]
}

fn hostile_text(max: usize) -> impl Strategy<Value = String> {
    prop::collection::vec(hostile_char(), 0..max).prop_map(|chars| chars.into_iter().collect())
}

/// Hostile text with the line terminators removed. Anchor context lines come
/// from splitting file text into lines, so they never contain `\n`; the
/// snapshot file's `join("\n")` is only invertible for such lines, which is
/// what [`each_appended_comment_snapshots_its_context_lines`] relies on.
fn hostile_line(max: usize) -> impl Strategy<Value = String> {
    hostile_text(max).prop_map(|text| text.replace(['\n', '\r'], "~"))
}

/// Comment ids double as single filesystem path components — the snapshot
/// lives at `.review/snapshots/<id>` — so `rv` mints them itself. Feeding in
/// `../`, `/`, or `""` violates a precondition rather than exposing a broken
/// promise, so every strategy here stays path-safe and the id space is kept
/// small on purpose, to make upsert collisions common.
///
/// The ids also vary in *length* and are deliberately laid out so that some are
/// prefixes of others (`id` of everything, `id0` of `id00`, `id1` of `id10`).
/// An id space of equal-length ids cannot tell `existing.id == comment.id`
/// apart from `existing.id.starts_with(&comment.id)` (or `contains`), which is
/// the same class of defect as keying the upsert on the wrong field: identity
/// matched by something looser than equality. The prefix pairs make the two
/// disagree, so the upsert properties can see the difference.
const ID_POOL: &[&str] = &["id", "id0", "id00", "id1", "id10", "id2"];

/// The `index`-th distinct id: [`ID_POOL`] while it lasts, then `id6`, `id7`, …
/// which stay distinct from every pool entry. Used where a strategy needs *n*
/// pairwise-different ids rather than collisions.
fn distinct_id(index: usize) -> String {
    ID_POOL
        .get(index)
        .map(|id| (*id).to_owned())
        .unwrap_or_else(|| format!("id{}", index + ID_POOL.len()))
}

fn id_pool(count: usize) -> Vec<String> {
    ID_POOL
        .iter()
        .take(count)
        .map(|id| (*id).to_owned())
        .collect()
}

fn hex(max: usize) -> impl Strategy<Value = String> {
    prop::collection::vec(
        prop::sample::select(vec!['0', '3', '9', 'a', 'd', 'f']),
        1..max,
    )
    .prop_map(|chars| chars.into_iter().collect())
}

fn side() -> impl Strategy<Value = Side> {
    prop_oneof![Just(Side::Left), Just(Side::Right)]
}

fn comment_state() -> impl Strategy<Value = CommentState> {
    prop_oneof![
        Just(CommentState::Open),
        Just(CommentState::AwaitingVerification),
        Just(CommentState::Resolved),
        Just(CommentState::Outdated),
    ]
}

fn anchor(text_max: usize) -> impl Strategy<Value = Anchor> {
    (
        hostile_text(text_max),
        side(),
        any::<u32>(),
        hex(12),
        prop::collection::vec(hostile_line(text_max), 0..4),
    )
        .prop_map(|(file, side, line, content_hash, context)| Anchor {
            file,
            side,
            line,
            content_hash,
            context,
        })
}

fn comment(id: impl Strategy<Value = String>, text_max: usize) -> impl Strategy<Value = Comment> {
    (
        id,
        hex(12),
        hex(12),
        anchor(text_max),
        hostile_text(text_max),
        comment_state(),
        prop::option::of(hostile_text(text_max)),
    )
        .prop_map(
            |(id, change_id, commit_id, anchor, body, state, reply)| Comment {
                id,
                change_id,
                commit_id,
                anchor,
                body,
                state,
                reply,
                settled_by: None,
            },
        )
}

/// A sequence of comments drawn from a small id pool, so runs contain both
/// fresh inserts and same-id updates — and, because [`ID_POOL`] is
/// prefix-structured, both same-id updates and *near*-id non-updates.
fn comment_sequence(len: std::ops::Range<usize>) -> impl Strategy<Value = Vec<Comment>> {
    prop::collection::vec(comment(prop::sample::select(id_pool(5)), 8), len)
}

/// Comments whose ids are distinct by construction, so no upsert collapsing
/// happens and every appended comment must survive. The ids still come from
/// [`ID_POOL`], so "distinct" here means distinct under `==` while remaining
/// entangled under `starts_with`.
fn distinct_comments(len: std::ops::Range<usize>) -> impl Strategy<Value = Vec<Comment>> {
    prop::collection::vec(comment(Just(String::new()), 8), len).prop_map(|mut comments| {
        for (index, comment) in comments.iter_mut().enumerate() {
            comment.id = distinct_id(index);
        }
        comments
    })
}

/// Distinct ids as above, but `change_id` drawn from a two-element pool so
/// collisions on it are the common case rather than a `hex(12)` lottery. Any
/// identity keyed on `change_id` instead of `id` collapses these; identity
/// keyed on `id` leaves every one of them standing.
fn distinct_comments_sharing_change_ids(
    len: std::ops::Range<usize>,
) -> impl Strategy<Value = Vec<Comment>> {
    prop::collection::vec(
        (
            comment(Just(String::new()), 8),
            prop::sample::select(vec!["chg0".to_owned(), "chg1".to_owned()]),
        ),
        len,
    )
    .prop_map(|pairs| {
        pairs
            .into_iter()
            .enumerate()
            .map(|(index, (mut comment, change_id))| {
                comment.id = distinct_id(index);
                comment.change_id = change_id;
                comment
            })
            .collect()
    })
}

fn change_ref(text_max: usize) -> impl Strategy<Value = ChangeRef> {
    (hex(12), hex(12), hostile_text(text_max)).prop_map(|(change_id, commit_id, description)| {
        ChangeRef {
            change_id,
            commit_id,
            description,
        }
    })
}

fn session(text_max: usize, max_changes: usize) -> impl Strategy<Value = Session> {
    (
        hostile_text(text_max),
        hex(12),
        hex(12),
        prop::collection::vec(change_ref(text_max), 0..max_changes),
        hostile_text(text_max),
    )
        .prop_map(
            |(revset, base_commit, head_commit, changes, started_at)| Session {
                revset,
                base_commit,
                head_commit,
                changes,
                started_at,
            },
        )
}

/// Plausible `.git/info/exclude` contents: other tools' patterns mixed with
/// near-misses of `/.review/` that must not be mistaken for it.
fn exclude_seed() -> impl Strategy<Value = String> {
    let line = prop_oneof![
        Just("/.review/".to_owned()),
        Just("#/.review/".to_owned()),
        Just("# /.review/".to_owned()),
        Just("/.review".to_owned()),
        Just(".review/".to_owned()),
        Just("/.review/ ".to_owned()),
        Just("  /.review/".to_owned()),
        Just("x/.review/y".to_owned()),
        Just("!/.review/".to_owned()),
        Just("/.review/*".to_owned()),
        Just(String::new()),
        Just("target/".to_owned()),
        Just("*.log".to_owned()),
        Just("\t/.review/".to_owned()),
    ];
    (prop::collection::vec(line, 0..5), any::<bool>()).prop_map(|(lines, trailing_newline)| {
        let mut seed = lines.join("\n");
        if trailing_newline && !seed.is_empty() {
            seed.push('\n');
        }
        seed
    })
}

/// One store operation, so a property can drive an arbitrary interleaving of
/// everything the module writes. Every mutating method on `Store` has a
/// variant here — `append_comment`, `remove_comment`, `write_session`,
/// `write_markdown`, `ensure_excluded` — which is what lets the properties
/// below say "every write the module makes" and mean it.
#[derive(Clone, Debug)]
enum Op {
    Append(Comment),
    Remove(String),
    WriteSession(Session),
    WriteMarkdown(String),
    EnsureExcluded,
}

/// Ids for [`Op::Remove`] to aim at. Mostly the same pool [`Op::Append`] draws
/// from, so a removal usually finds its target; sometimes an id that pool never
/// mints (`id10` is outside `id_pool(4)`, and `gone` is in no pool at all), so
/// the unknown-id path — a no-op that must neither fail nor disturb anything —
/// gets exercised in the same sequences.
///
/// [`ID_POOL`]'s prefix structure does double duty on the delete path: removing
/// `id0` must not take `id00` with it, and removing `id` must not empty the
/// store, so `existing.id != id` is distinguishable from
/// `!existing.id.starts_with(id)`.
fn removable_id() -> impl Strategy<Value = String> {
    prop_oneof![
        8 => prop::sample::select(id_pool(4)),
        1 => Just("id10".to_owned()),
        1 => Just("gone".to_owned()),
    ]
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        4 => comment(prop::sample::select(id_pool(4)), 8).prop_map(Op::Append),
        3 => removable_id().prop_map(Op::Remove),
        2 => session(8, 2).prop_map(Op::WriteSession),
        2 => hostile_text(20).prop_map(Op::WriteMarkdown),
        2 => Just(Op::EnsureExcluded),
    ]
}

/// Appends and removals only, in a ratio that makes long append/remove/re-append
/// histories over a handful of ids the common case rather than a rarity.
fn append_or_remove() -> impl Strategy<Value = Op> {
    prop_oneof![
        3 => comment(prop::sample::select(id_pool(4)), 8).prop_map(Op::Append),
        2 => removable_id().prop_map(Op::Remove),
    ]
}

fn apply(store: &Store, op: &Op) -> Result<(), rv_core::store::Error> {
    match op {
        Op::Append(comment) => store.append_comment(comment),
        Op::Remove(id) => store.remove_comment(id).map(|_| ()),
        Op::WriteSession(session) => store.write_session(session),
        Op::WriteMarkdown(document) => store.write_markdown(document),
        Op::EnsureExcluded => store.ensure_excluded().map(|_| ()),
    }
}

// --- oracles computed from an op *sequence*, never read back from disk -------
//
// The point of these three is that they are a model of what the sequence
// implies, evaluated without touching the store. A property whose expectation
// is a second read through the same code path can only catch a cache; one whose
// expectation comes from the sequence catches an operation that wrote the
// wrong bytes, or wrote them into somebody else's file.

/// What `comments.json` must hold after the sequence: every `Append` upserted
/// by id — last write wins, first-insertion order preserved — and every
/// `Remove` dropping that id's entry, or, when the id is not there, doing
/// nothing whatsoever, exactly as `remove_comment` documents.
///
/// Deliberately computed with a different shape than the store's in-place
/// `iter_mut().find()` and `retain` — a hash map for the values plus a separate
/// vector for the order — so it is a recomputation rather than a restatement.
/// The order vector is what makes the removal clause say something: it records
/// *where* a surviving comment sits, so a delete that shuffles the survivors,
/// or one that resurrects a removed id at its old position on re-append, both
/// diverge from it.
fn expected_comments(ops: &[Op]) -> Vec<Comment> {
    let mut order: Vec<String> = Vec::new();
    let mut latest: HashMap<String, Comment> = HashMap::new();
    for op in ops {
        match op {
            Op::Append(comment) => {
                if latest.insert(comment.id.clone(), comment.clone()).is_none() {
                    order.push(comment.id.clone());
                }
            }
            Op::Remove(id) => {
                if latest.remove(id).is_some() {
                    order.retain(|surviving| surviving != id);
                }
            }
            Op::WriteSession(_) | Op::WriteMarkdown(_) | Op::EnsureExcluded => {}
        }
    }
    order
        .into_iter()
        .map(|id| latest.remove(&id).expect("id was recorded"))
        .collect()
}

/// Whether the sequence ever wrote `comments.json` at all — which is exactly
/// "did it append?", since an append always writes the file and a removal only
/// ever rewrites one that an append already created. A sequence that appends
/// and then removes everything leaves the file behind holding `[]`, so the
/// file's *existence* cannot be predicted from [`expected_comments`] being
/// non-empty.
fn wrote_comments_file(ops: &[Op]) -> bool {
    ops.iter().any(|op| matches!(op, Op::Append(_)))
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

/// What `session.toml` must round-trip to: the last session written.
fn expected_session(ops: &[Op]) -> Option<Session> {
    ops.iter().rev().find_map(|op| match op {
        Op::WriteSession(session) => Some(session.clone()),
        _ => None,
    })
}

// ---------------------------------------------------------------------------
// comments.json: upsert semantics, conservation, ordering
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(config(24))]

    /// `comments()` is exactly the upsert-by-id reduction of the append
    /// sequence: last write wins per id, first-insertion order preserved.
    /// This is the whole contract of `append_comment` in one equation, checked
    /// against an independently computed reduction rather than against another
    /// call into the store.
    #[test]
    fn comments_equal_the_upsert_by_id_reduction_of_the_append_sequence(
        sequence in comment_sequence(0..8),
    ) {
        let repo = repo_root();
        let store = Store::open(repo.path()).expect("open store");
        for comment in &sequence {
            store.append_comment(comment).expect("append comment");
        }

        let stored = store.comments().expect("read comments");
        prop_assert_eq!(stored, upsert_reduce(&sequence));
    }

    /// Nothing lost, nothing invented: the set of ids on disk is precisely the
    /// set of ids appended, each appearing exactly once, no matter how many
    /// times it was written.
    #[test]
    fn every_appended_id_appears_exactly_once_and_no_others_appear(
        sequence in comment_sequence(1..8),
    ) {
        let repo = repo_root();
        let store = Store::open(repo.path()).expect("open store");
        for comment in &sequence {
            store.append_comment(comment).expect("append comment");
        }

        let stored = store.comments().expect("read comments");
        for comment in &sequence {
            let hits = stored.iter().filter(|c| c.id == comment.id).count();
            prop_assert_eq!(hits, 1, "id {:?} should appear once, appeared {}", comment.id, hits);
        }
        for stored_comment in &stored {
            prop_assert!(
                sequence.iter().any(|c| c.id == stored_comment.id),
                "invented an id never appended: {:?}",
                stored_comment.id
            );
        }
    }

    /// Re-appending an existing id is an update, not an insert: the length is
    /// unchanged, the entry keeps its slot, every other entry is untouched,
    /// and the entry now holds the new content.
    #[test]
    fn reappending_an_existing_id_changes_neither_length_nor_position(
        initial in distinct_comments(1..6),
        target in any::<prop::sample::Index>(),
        mut replacement in comment(Just(String::new()), 12),
    ) {
        let repo = repo_root();
        let store = Store::open(repo.path()).expect("open store");
        for comment in &initial {
            store.append_comment(comment).expect("append comment");
        }
        let before = store.comments().expect("read comments");

        let slot = target.index(before.len());
        replacement.id = before[slot].id.clone();
        store.append_comment(&replacement).expect("append replacement");

        let after = store.comments().expect("read comments");
        prop_assert_eq!(after.len(), before.len(), "an update must not add an entry");
        prop_assert_eq!(ids_of(&after), ids_of(&before), "positions must not move");
        prop_assert_eq!(&after[slot], &replacement, "the updated slot holds the new content");
        for (index, (old, new)) in before.iter().zip(after.iter()).enumerate() {
            if index != slot {
                prop_assert_eq!(old, new, "entry {} was collateral damage", index);
            }
        }
    }

    /// Idempotence: appending the very same comment again is a no-op on the
    /// whole `.review/` tree, byte for byte — not merely "the comment is still
    /// there".
    #[test]
    fn appending_an_identical_comment_twice_is_byte_identical(
        sequence in distinct_comments(1..5),
        target in any::<prop::sample::Index>(),
    ) {
        let repo = repo_root();
        let store = Store::open(repo.path()).expect("open store");
        for comment in &sequence {
            store.append_comment(comment).expect("append comment");
        }
        let before = dir_snapshot(&repo.path().join(".review"));

        let again = &sequence[target.index(sequence.len())];
        store.append_comment(again).expect("append the same comment again");

        prop_assert_eq!(dir_snapshot(&repo.path().join(".review")), before);
    }
}

proptest! {
    #![proptest_config(config(16))]

    /// Order permutes the *sequence* on disk but never the *set*: two stores
    /// fed two permutations of the same distinct-id comments hold the same
    /// comments, so each is the other's oracle. Append order is allowed to
    /// decide where a comment sits (that is
    /// [`comments_equal_the_upsert_by_id_reduction_of_the_append_sequence`]'s
    /// business) — it must never decide *which* comments survive. An upsert
    /// keyed on anything but `id`, or any capacity limit, would make the
    /// surviving set depend on the order the reviewer happened to work in.
    ///
    /// The comments' `change_id`s are drawn from a two-element pool rather than
    /// from `hex(12)`, which is what makes that first clause bite: a
    /// `change_id`-keyed identity only diverges between the two orders when two
    /// comments *share* a `change_id`, and independent `hex(12)` draws collide
    /// in well under 1% of cases. With a two-element pool a collision is the
    /// norm, and each store then keeps a different survivor at a different
    /// position. The permutation is forced away from the identity for the same
    /// reason — an identity permutation compares a store against itself.
    #[test]
    fn the_set_of_comments_never_depends_on_append_order(
        (canonical, shuffled) in distinct_comments_sharing_change_ids(1..6)
            .prop_flat_map(|comments| (Just(comments.clone()), Just(comments).prop_shuffle()))
            .prop_map(|(canonical, mut shuffled)| {
                if shuffled == canonical && shuffled.len() > 1 {
                    shuffled.rotate_left(1);
                }
                (canonical, shuffled)
            }),
    ) {
        let repo_a = repo_root();
        let repo_b = repo_root();
        let store_a = Store::open(repo_a.path()).expect("open store a");
        let store_b = Store::open(repo_b.path()).expect("open store b");
        for comment in &canonical {
            store_a.append_comment(comment).expect("append into a");
        }
        for comment in &shuffled {
            store_b.append_comment(comment).expect("append into b");
        }

        let a = sorted_by_id(store_a.comments().expect("read a"));
        let b = sorted_by_id(store_b.comments().expect("read b"));

        prop_assert_eq!(&a, &b, "the set of comments must not depend on append order");
        prop_assert_eq!(ids_of(&a), ids_of(&sorted_by_id(canonical)),
            "and it must be every comment appended, none dropped");
    }

    /// Write-through, generalized from the single-comment hand-written case:
    /// after an arbitrary interleaving of every write the module performs, a
    /// *freshly opened* `Store` over the same root sees exactly what the op
    /// sequence implies — and so does the handle that did the writing.
    ///
    /// The expectation is computed from the sequence ([`expected_comments`] and
    /// friends), not read back out of the store. A read-back oracle turns this
    /// into a self-consistency check: it can only fail if someone adds an
    /// in-memory cache, because any damage an op does mid-sequence is baked
    /// into both sides of the comparison. Comparing against the sequence pins
    /// what was written as well as that both handles agree about it.
    ///
    /// "Everything" includes the one write whose result is not a file the store
    /// reads back: `ensure_excluded` is asked, on the fresh handle, whether the
    /// line still needs adding, and its answer must be "no" exactly when the
    /// sequence already excluded the directory. That check runs last, since it
    /// is the one assertion here that writes.
    ///
    /// The writing handle is asked about all three readable files *while it is
    /// still alive*, inside the block, which is the half of the claim the
    /// reopened handle cannot make: a `write_session` that buffered its bytes
    /// and flushed them on `Drop` would satisfy every assertion below the block
    /// and still not be write-through. Only an assertion made before the handle
    /// is dropped can tell the two apart.
    #[test]
    fn everything_written_is_visible_to_a_freshly_opened_store(
        ops in prop::collection::vec(op(), 1..7),
    ) {
        let repo = repo_root();
        {
            let store = Store::open(repo.path()).expect("open store");
            for op in &ops {
                apply(&store, op).expect("apply op");
            }
            prop_assert_eq!(store.comments().expect("read comments"), expected_comments(&ops));
            prop_assert_eq!(fs::read(store.markdown_path()).ok(), expected_markdown(&ops));
            prop_assert_eq!(store.read_session().ok(), expected_session(&ops));
        }

        let reopened = Store::open(repo.path()).expect("reopen store");
        prop_assert_eq!(reopened.comments().expect("read comments"), expected_comments(&ops));
        prop_assert_eq!(fs::read(reopened.markdown_path()).ok(), expected_markdown(&ops));
        prop_assert_eq!(reopened.read_session().ok(), expected_session(&ops));

        let ran_ensure_excluded = ops.iter().any(|op| matches!(op, Op::EnsureExcluded));
        prop_assert_eq!(
            reopened.ensure_excluded().expect("ensure_excluded on the fresh handle"),
            !ran_ensure_excluded,
            "a fresh handle must see the exclusion the sequence did (or did not) record"
        );
    }

    /// Hostile text survives `comments.json` byte-identically. JSON has to
    /// carry quotes, backslashes, raw newlines, NUL and other C0 controls,
    /// astral-plane scalars and a BOM through `serde_json` and back without
    /// normalizing, trimming or re-encoding anything.
    #[test]
    fn hostile_comment_text_roundtrips_byte_identically(
        body in hostile_text(48),
        reply in prop::option::of(hostile_text(48)),
        context in prop::collection::vec(hostile_text(24), 0..4),
        file in hostile_text(24),
    ) {
        let repo = repo_root();
        let store = Store::open(repo.path()).expect("open store");
        let mut comment = Comment {
            id: "id0".to_owned(),
            change_id: "abc123".to_owned(),
            commit_id: "def456".to_owned(),
            anchor: Anchor {
                file,
                side: Side::Right,
                line: 7,
                content_hash: "deadbeef".to_owned(),
                context,
            },
            body,
            state: CommentState::Open,
            reply,
            settled_by: None,
        };
        // The context is written to a snapshot path too, but ids — not
        // context — name that file, so hostile context is safe here.
        store.append_comment(&comment).expect("append comment");

        let read_back = Store::open(repo.path())
            .expect("reopen store")
            .comments()
            .expect("read comments");
        prop_assert_eq!(read_back.len(), 1);
        prop_assert_eq!(read_back[0].body.as_bytes(), comment.body.as_bytes());
        prop_assert_eq!(&read_back[0].reply, &comment.reply);
        prop_assert_eq!(&read_back[0].anchor.context, &comment.anchor.context);
        prop_assert_eq!(&read_back[0], &comment);

        // And the same holds for a body swapped in later, so this covers the
        // update path's serialization too, not just the insert path.
        comment.body = format!("{}{}", comment.body, comment.body);
        store.append_comment(&comment).expect("append updated comment");
        prop_assert_eq!(store.comments().expect("read comments"), vec![comment]);
    }
}

// ---------------------------------------------------------------------------
// snapshots and the write ordering that makes a crash safe
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(config(20))]

    /// Every comment in `comments.json` has a snapshot holding its anchor's
    /// context lines, and no snapshot exists for an id never appended — the
    /// conservation law [`snapshots_match`] states, over an append-only
    /// sequence.
    #[test]
    fn each_appended_comment_snapshots_its_context_lines(
        sequence in comment_sequence(1..6),
    ) {
        let repo = repo_root();
        let store = Store::open(repo.path()).expect("open store");
        for comment in &sequence {
            store.append_comment(comment).expect("append comment");
        }

        snapshots_match(repo.path(), &store.comments().expect("read comments"))?;
    }

    /// Appends and removals interleaved over a handful of ids: after *every*
    /// step, `comments()` is exactly what the model says, and at the end the
    /// snapshots directory holds exactly the surviving comments' context and
    /// nothing else.
    ///
    /// Three separate claims about `remove_comment` ride on this, none of them
    /// reachable from an append-only sequence:
    ///
    /// * the entry goes, and only that entry — the model's order vector pins
    ///   the survivors' positions as well as their identity, and [`ID_POOL`]'s
    ///   prefix structure means a delete matching on `starts_with` instead of
    ///   `==` takes bystanders with it;
    /// * the snapshot goes with it, so a removal that forgets the second write
    ///   leaves an orphan that [`snapshots_match`] sees;
    /// * and the returned `bool` is the truth about whether anything was there,
    ///   which is the store's answer to "did this id exist?" and is checked
    ///   against the model *before* the call, including for the ids the append
    ///   pool never mints.
    ///
    /// Checking after every step rather than only at the end is what makes a
    /// shrunk counterexample point at the operation that broke it instead of at
    /// the whole history.
    #[test]
    fn appends_and_removals_leave_exactly_the_model_comments_and_snapshots(
        ops in prop::collection::vec(append_or_remove(), 1..10),
    ) {
        let repo = repo_root();
        let store = Store::open(repo.path()).expect("open store");

        let mut so_far: Vec<Op> = Vec::new();
        for op in &ops {
            let before = expected_comments(&so_far);
            so_far.push(op.clone());
            match op {
                Op::Remove(id) => {
                    let present = before.iter().any(|comment| comment.id == *id);
                    let removed = store.remove_comment(id).expect("remove comment");
                    prop_assert_eq!(removed, present,
                        "remove_comment({:?}) reported {:?} with {:?} on disk",
                        id, removed, ids_of(&before));
                }
                _ => apply(&store, op).expect("apply op"),
            }
            prop_assert_eq!(store.comments().expect("read comments"),
                expected_comments(&so_far), "after {:?}", op);
        }

        snapshots_match(repo.path(), &expected_comments(&ops))?;
        prop_assert!(
            stray_temp_files(repo.path()).is_empty(),
            "stray temp files: {:?}", stray_temp_files(repo.path())
        );
    }

    /// The module's headline crash-safety claim, tested by forcing the failure
    /// rather than waiting for a crash: the snapshot is written *before*
    /// `comments.json`, so if the snapshot write cannot succeed the comment
    /// must not be recorded at all. A directory planted at the snapshot's path
    /// makes the rename fail exactly where an interrupted write would have
    /// stopped.
    ///
    /// The reverse ordering — `comments.json` first — would leave the store
    /// claiming a comment whose snapshot never existed, which the doc says can
    /// never happen.
    #[test]
    fn a_comment_whose_snapshot_cannot_be_written_is_never_recorded(
        existing in distinct_comments(0..4),
        doomed in comment(Just("doomed".to_owned()), 12),
    ) {
        let repo = repo_root();
        let store = Store::open(repo.path()).expect("open store");
        for comment in &existing {
            store.append_comment(comment).expect("append comment");
        }
        let before = store.comments().expect("read comments");
        let before_bytes = fs::read(repo.path().join(".review/comments.json")).ok();

        // A directory cannot be replaced by a rename of a regular file, so
        // write_atomic's persist step fails here.
        fs::create_dir(repo.path().join(".review/snapshots").join(&doomed.id))
            .expect("plant a directory where the snapshot file would go");

        let result = store.append_comment(&doomed);
        prop_assert!(result.is_err(), "the snapshot write cannot have succeeded");

        let after = store.comments().expect("read comments");
        prop_assert!(
            !after.iter().any(|c| c.id == doomed.id),
            "comments.json recorded a comment whose snapshot was never written"
        );
        prop_assert_eq!(after, before, "a failed append must not disturb prior comments");
        prop_assert_eq!(fs::read(repo.path().join(".review/comments.json")).ok(), before_bytes);
        prop_assert!(
            stray_temp_files(repo.path()).is_empty(),
            "a failed write must still clean up its temp file"
        );
    }
}

// ---------------------------------------------------------------------------
// atomicity residue
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(config(16))]

    /// After any successful sequence of operations, none of `write_atomic`'s
    /// temp files remain anywhere under the repo root — not in `.review/`, not
    /// in `.review/snapshots/`, not in `.git/info/`. Generalizes the
    /// hand-written three-append case to arbitrary interleavings of every
    /// write the module makes.
    #[test]
    fn no_operation_sequence_leaves_temp_files_behind(
        ops in prop::collection::vec(op(), 1..8),
    ) {
        let repo = repo_root();
        let store = Store::open(repo.path()).expect("open store");
        for op in &ops {
            apply(&store, op).expect("apply op");
        }

        let stray = stray_temp_files(repo.path());
        prop_assert!(stray.is_empty(), "stray temp files: {:?}", stray);
    }

    /// A destination file always holds exactly the newest content and nothing
    /// else: last write wins, and a shorter document must not leave a tail of
    /// the longer one it replaced. Run over `REVIEW-FEEDBACK.md`, the one file
    /// another program reads while `rv` runs.
    #[test]
    fn markdown_is_last_write_wins_with_no_residual_bytes(
        documents in prop::collection::vec(hostile_text(64), 1..4),
    ) {
        let repo = repo_root();
        let store = Store::open(repo.path()).expect("open store");
        for document in &documents {
            store.write_markdown(document).expect("write markdown");
        }

        let expected = documents.last().expect("at least one document");
        let on_disk = fs::read(store.markdown_path()).expect("read markdown");
        prop_assert_eq!(on_disk.len(), expected.len(), "residual bytes from an earlier write");
        prop_assert_eq!(on_disk, expected.as_bytes());
    }
}

// ---------------------------------------------------------------------------
// session.toml
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(config(24))]

    /// `read_session` after `write_session` reproduces the session exactly,
    /// for arbitrary field values: empty strings, unicode, no changes, several
    /// changes, and change lists with duplicate ids (the store stores what it
    /// is given and does not dedup a stack).
    #[test]
    fn session_toml_roundtrips_arbitrary_sessions(session in session(16, 4)) {
        let repo = repo_root();
        let store = Store::open(repo.path()).expect("open store");
        store.write_session(&session).expect("write session");

        let read_back = Store::open(repo.path())
            .expect("reopen store")
            .read_session()
            .expect("read session");
        prop_assert_eq!(read_back, session);
    }

    /// Duplicate change ids in one session survive as duplicates, in position:
    /// `changes` is a list, not a set, and the store must not collapse it.
    ///
    /// The general round-trip property above cannot see a dedup pass, because
    /// its `change_id`s are independent `hex(12)` draws that practically never
    /// collide. Here they collide by construction, in three shapes a
    /// `dedup`/`unique_by` would each treat differently: exact clones, entries
    /// sharing a `change_id` but differing in `description` (the real case — a
    /// stack whose changes were described differently at two points in time),
    /// and a duplicate separated from its twin by an unrelated entry, which
    /// distinguishes an adjacent-only `Vec::dedup` from a global one.
    #[test]
    fn duplicate_change_ids_in_a_session_are_preserved(
        change in change_ref(12),
        other in change_ref(12),
        description in hostile_text(12),
        copies in 1usize..5,
    ) {
        let repo = repo_root();
        let store = Store::open(repo.path()).expect("open store");
        let mut twin = change.clone();
        twin.description = description;
        let mut changes = vec![change.clone(); copies];
        changes.push(other);
        changes.push(twin);
        changes.push(change.clone());
        let expected_len = changes.len();

        let session = Session {
            revset: "trunk()..@".to_owned(),
            base_commit: "abc123".to_owned(),
            head_commit: "def456".to_owned(),
            changes: changes.clone(),
            started_at: "epoch:1755460770".to_owned(),
        };
        store.write_session(&session).expect("write session");

        let read_back = store.read_session().expect("read session");
        prop_assert_eq!(read_back.changes.len(), expected_len,
            "a change list must not be deduped, adjacently or globally");
        prop_assert_eq!(&read_back.changes, &changes, "and its order must be preserved");
        prop_assert_eq!(read_back, session);
    }

    /// `write_session` is a wholesale replacement: the last session written is
    /// the only one readable, with no residue from a longer predecessor.
    #[test]
    fn session_writes_are_last_write_wins(sessions in prop::collection::vec(session(12, 3), 1..4)) {
        let repo = repo_root();
        let store = Store::open(repo.path()).expect("open store");
        for session in &sessions {
            store.write_session(session).expect("write session");
        }

        let expected = sessions.last().expect("at least one session").clone();
        prop_assert_eq!(store.read_session().expect("read session"), expected);
    }
}

// ---------------------------------------------------------------------------
// ensure_excluded
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(config(32))]

    /// `ensure_excluded` over arbitrary pre-existing exclude-file content:
    /// idempotent (at most one call ever adds the line), non-destructive
    /// (every pre-existing line survives, in order, and the pre-existing bytes
    /// remain a prefix), and exact in its matching (a commented-out or
    /// substring form is *not* the line, so it must not suppress the append).
    #[test]
    fn ensure_excluded_is_idempotent_and_never_corrupts_the_exclude_file(
        seed in exclude_seed(),
        file_exists in any::<bool>(),
        calls in 1usize..4,
    ) {
        let repo = repo_root();
        let exclude = repo.path().join(".git/info/exclude");
        if file_exists {
            fs::write(&exclude, &seed).expect("seed exclude file");
        }
        // A missing file is contractually the same as an empty one.
        let before = if file_exists { seed.clone() } else { String::new() };
        let already = before.lines().filter(|line| *line == EXCLUDE_LINE).count();

        let store = Store::open(repo.path()).expect("open store");
        let results: Vec<bool> = (0..calls)
            .map(|_| store.ensure_excluded().expect("ensure_excluded"))
            .collect();
        let after = fs::read_to_string(&exclude).expect("read exclude file");

        let added = results.iter().filter(|added| **added).count();
        if already > 0 {
            prop_assert_eq!(added, 0, "the line was already present");
            prop_assert_eq!(&after, &before, "an already-excluded repo must not be rewritten");
        } else {
            prop_assert_eq!(added, 1, "exactly one call may add the line, got {:?}", results);
            prop_assert!(results[0], "and it must be the first call");
            prop_assert!(after.ends_with('\n'), "the appended line must be terminated");
            prop_assert!(after.starts_with(&before), "pre-existing bytes must survive verbatim");
            let mut expected: Vec<&str> = before.lines().collect();
            expected.push(EXCLUDE_LINE);
            prop_assert_eq!(after.lines().collect::<Vec<_>>(), expected,
                "other tools' lines must be preserved in order, with ours appended last");
        }
        prop_assert_eq!(
            after.lines().filter(|line| *line == EXCLUDE_LINE).count(),
            already.max(1),
            "exclude file:\n{}", after
        );
    }
}

// ---------------------------------------------------------------------------
// isolation between the files the store owns
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(config(16))]

    /// After an arbitrary interleaving of every operation the module performs,
    /// each of the files it touches — `comments.json`, `session.toml`,
    /// `REVIEW-FEEDBACK.md`, `.git/info/exclude` and the per-comment snapshots
    /// — holds exactly what *its own* operations wrote, and the tree contains
    /// nothing else.
    ///
    /// This is the isolation law stated positively, with an oracle computed
    /// from the op sequence rather than read back from disk. That distinction
    /// is the whole point: a property that snapshots the tree after the ops and
    /// compares it with itself bakes any mid-sequence damage into both sides,
    /// so `append_comment` clobbering `.git/info/exclude`, or `ensure_excluded`
    /// deleting `comments.json`, sails straight through it. Here every file's
    /// expectation is derived from the operations that own it, so an operation
    /// writing outside its own file changes a value nothing in the sequence
    /// asked to change, and the mismatch is immediate.
    ///
    /// `.git/info/exclude` gets the strongest form of the claim, because it is
    /// the one file that lives outside `.review/` and belongs to git: it must
    /// be byte-identical to its seed unless an `EnsureExcluded` op ran, and
    /// even then only by the one line, appended after everything already there.
    #[test]
    fn every_file_holds_exactly_what_its_own_operations_wrote(
        seed in exclude_seed(),
        ops in prop::collection::vec(op(), 1..7),
    ) {
        let repo = repo_root();
        let exclude_path = repo.path().join(".git/info/exclude");
        fs::write(&exclude_path, &seed).expect("seed exclude file");

        let store = Store::open(repo.path()).expect("open store");
        for op in &ops {
            apply(&store, op).expect("apply op");
        }

        let expected = expected_comments(&ops);
        prop_assert_eq!(store.comments().expect("read comments"), expected.clone());
        prop_assert_eq!(fs::read(store.markdown_path()).ok(), expected_markdown(&ops));
        prop_assert_eq!(store.read_session().ok(), expected_session(&ops));

        let after = fs::read_to_string(&exclude_path).expect("read exclude file");
        let ran_ensure_excluded = ops.iter().any(|op| matches!(op, Op::EnsureExcluded));
        let already_excluded = seed.lines().any(|line| line == EXCLUDE_LINE);
        if !ran_ensure_excluded || already_excluded {
            prop_assert_eq!(&after, &seed,
                "an operation that does not own .git/info/exclude rewrote it");
        } else {
            let mut expected_lines: Vec<&str> = seed.lines().collect();
            expected_lines.push(EXCLUDE_LINE);
            prop_assert_eq!(after.lines().collect::<Vec<_>>(), expected_lines,
                "ensure_excluded may add its line and change nothing else");
            prop_assert!(after.starts_with(&seed) || seed.is_empty(),
                "pre-existing exclude bytes must survive verbatim");
        }

        // Conservation over the whole tree: exactly the files these operations
        // are entitled to create exist, and nothing they never mentioned does.
        let mut entitled: BTreeSet<PathBuf> = BTreeSet::new();
        entitled.insert(PathBuf::from(".git/info/exclude"));
        if wrote_comments_file(&ops) {
            entitled.insert(PathBuf::from(".review/comments.json"));
        }
        if expected_session(&ops).is_some() {
            entitled.insert(PathBuf::from(".review/session.toml"));
        }
        if expected_markdown(&ops).is_some() {
            entitled.insert(PathBuf::from(".review/REVIEW-FEEDBACK.md"));
        }
        for comment in &expected {
            entitled.insert(Path::new(".review/snapshots").join(&comment.id));
        }
        prop_assert_eq!(relative_files(repo.path()), entitled);
        // And the snapshots hold their own comment's context, not merely exist.
        snapshots_match(repo.path(), &expected)?;
    }

    /// Two live `Store` handles over one root are interchangeable. Writes
    /// interleaved between them leave exactly what the *merged* sequence
    /// implies, and both handles read that same state back.
    ///
    /// `Store` is documented to hold no cached state, which is what makes this
    /// true; the property is the test of that claim. A per-handle cache — the
    /// obvious "optimization" for a `comments()` that re-reads the file on
    /// every append — is invisible to every single-handle property in this
    /// file, because one handle's cache is always coherent with a file only it
    /// writes. With two handles it is not: the stale one's next append rebuilds
    /// `comments.json` from its own snapshot of the past and silently deletes
    /// whatever the other handle wrote in the meantime.
    ///
    /// The appends are routed to alternating handles on purpose. Staleness only
    /// shows up in an `A`, `B`, `A` sandwich — the third write is the one that
    /// resurrects a list from before the second — and random routing produces
    /// that sandwich often enough to catch a cache only two runs in three.
    /// Alternating makes every third append a witness. Everything else about
    /// the plan, including where the non-append operations go, is generated.
    #[test]
    fn two_store_handles_over_one_root_agree_on_the_interleaved_history(
        plan in prop::collection::vec((op(), any::<bool>()), 3..9).prop_map(|mut plan| {
            let mut appends = 0usize;
            for (op, to_first) in plan.iter_mut() {
                if matches!(op, Op::Append(_)) {
                    *to_first = appends.is_multiple_of(2);
                    appends += 1;
                }
            }
            // And never let the whole plan land on one handle, which would just
            // be the single-handle case again.
            if plan.iter().all(|(_, to_first)| *to_first)
                || plan.iter().all(|(_, to_first)| !*to_first)
            {
                plan[1].1 = !plan[0].1;
            }
            plan
        }),
    ) {
        let repo = repo_root();
        let first = Store::open(repo.path()).expect("open first handle");
        let second = Store::open(repo.path()).expect("open second handle");

        for (op, to_first) in &plan {
            apply(if *to_first { &first } else { &second }, op).expect("apply op");
        }

        let ops: Vec<Op> = plan.into_iter().map(|(op, _)| op).collect();
        prop_assert_eq!(first.comments().expect("read via the first handle"),
            expected_comments(&ops));
        prop_assert_eq!(second.comments().expect("read via the second handle"),
            expected_comments(&ops));
        prop_assert_eq!(fs::read(first.markdown_path()).ok(), expected_markdown(&ops));
        prop_assert_eq!(fs::read(second.markdown_path()).ok(), expected_markdown(&ops));
        prop_assert_eq!(first.read_session().ok(), expected_session(&ops));
        prop_assert_eq!(second.read_session().ok(), expected_session(&ops));
    }

    /// Opening a store is never destructive: `open` only has to create
    /// `.review/snapshots`, so re-opening over a populated store must leave
    /// every byte under the repo root exactly as it was.
    #[test]
    fn opening_a_store_never_destroys_existing_state(
        ops in prop::collection::vec(op(), 1..6),
        reopens in 1usize..4,
    ) {
        let repo = repo_root();
        {
            let store = Store::open(repo.path()).expect("open store");
            for op in &ops {
                apply(&store, op).expect("apply op");
            }
        }
        let before = dir_snapshot(repo.path());

        for _ in 0..reopens {
            Store::open(repo.path()).expect("reopen store");
        }

        prop_assert_eq!(dir_snapshot(repo.path()), before);
    }

    /// Each write touches only its own file. `comments.json`, `session.toml`
    /// and `REVIEW-FEEDBACK.md` are independent: rewriting any one of them
    /// leaves the other two byte-identical — including the one write that
    /// *deletes* rather than adds, which has a second file of its own to
    /// remove and so has two chances to reach past `comments.json`.
    ///
    /// That second chance is the snapshot unlink, and no comparison of sibling
    /// files can see it: a removal that took the whole snapshots directory with
    /// it leaves `session.toml` and `REVIEW-FEEDBACK.md` byte-identical. So the
    /// removal is followed by [`snapshots_match`], which holds the delete to the
    /// one file it owns.
    #[test]
    fn writing_one_file_never_disturbs_the_others(
        comments in distinct_comments(1..4),
        first_session in session(12, 2),
        second_session in session(12, 2),
        document in hostile_text(32),
        extra in comment(Just("extra".to_owned()), 12),
    ) {
        let repo = repo_root();
        let store = Store::open(repo.path()).expect("open store");
        for comment in &comments {
            store.append_comment(comment).expect("append comment");
        }
        store.write_session(&first_session).expect("write session");
        store.write_markdown(&document).expect("write markdown");

        let comments_path = repo.path().join(".review/comments.json");
        let session_path = repo.path().join(".review/session.toml");
        let markdown_path = store.markdown_path();
        let comments_bytes = fs::read(&comments_path).expect("read comments.json");
        let markdown_bytes = fs::read(&markdown_path).expect("read markdown");

        store.write_session(&second_session).expect("rewrite session");
        prop_assert_eq!(fs::read(&comments_path).expect("read comments.json"), comments_bytes.clone(),
            "write_session touched comments.json");
        prop_assert_eq!(fs::read(&markdown_path).expect("read markdown"), markdown_bytes.clone(),
            "write_session touched REVIEW-FEEDBACK.md");

        let session_bytes = fs::read(&session_path).expect("read session.toml");
        store.append_comment(&extra).expect("append extra comment");
        prop_assert_eq!(fs::read(&session_path).expect("read session.toml"), session_bytes.clone(),
            "append_comment touched session.toml");
        prop_assert_eq!(fs::read(&markdown_path).expect("read markdown"), markdown_bytes,
            "append_comment touched REVIEW-FEEDBACK.md");

        let comments_bytes = fs::read(&comments_path).expect("read comments.json");
        store.write_markdown(&document).expect("rewrite markdown");
        prop_assert_eq!(fs::read(&comments_path).expect("read comments.json"), comments_bytes,
            "write_markdown touched comments.json");
        prop_assert_eq!(fs::read(&session_path).expect("read session.toml"), session_bytes.clone(),
            "write_markdown touched session.toml");
        prop_assert_eq!(store.comments().expect("read comments").len(), comments.len() + 1);

        let markdown_bytes = fs::read(&markdown_path).expect("read markdown");
        prop_assert!(store.remove_comment(&extra.id).expect("remove the extra comment"));
        prop_assert_eq!(fs::read(&session_path).expect("read session.toml"), session_bytes,
            "remove_comment touched session.toml");
        prop_assert_eq!(fs::read(&markdown_path).expect("read markdown"), markdown_bytes,
            "remove_comment touched REVIEW-FEEDBACK.md");
        prop_assert_eq!(store.comments().expect("read comments").len(), comments.len());
        snapshots_match(repo.path(), &store.comments().expect("read comments"))?;
    }
}

// ---------------------------------------------------------------------------
// corruption is reported, never papered over
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(config(24))]

    /// `comments()` returns an empty vector only for a *missing* file. A
    /// `comments.json` truncated anywhere short of its end — the shape a torn
    /// write or a truncated copy would leave — must be reported as an error,
    /// never silently read as "no comments", which would quietly discard a
    /// reviewer's work.
    #[test]
    fn a_truncated_comments_json_is_an_error_not_silent_emptiness(
        sequence in distinct_comments(1..4),
        cut in any::<prop::sample::Index>(),
    ) {
        let repo = repo_root();
        let store = Store::open(repo.path()).expect("open store");
        for comment in &sequence {
            store.append_comment(comment).expect("append comment");
        }

        let path = repo.path().join(".review/comments.json");
        let good = fs::read_to_string(&path).expect("read comments.json");
        let chars: Vec<char> = good.chars().collect();
        // `Index::index(len)` is in `0..len`, so this is always a strict
        // prefix, and a strict prefix of a JSON array is never a valid one.
        let keep = cut.index(chars.len());
        let truncated: String = chars[..keep].iter().collect();
        fs::write(&path, &truncated).expect("write truncated comments.json");

        let result = store.comments();
        prop_assert!(
            result.is_err(),
            "truncated comments.json read as {:?} instead of failing (kept {} of {} chars)",
            result.as_ref().map(Vec::len),
            keep,
            chars.len()
        );
        // Which error matters: a caller telling "corrupt" from "unreadable"
        // needs the parse failure to arrive as InvalidComments, not as Io.
        prop_assert!(
            matches!(result, Err(rv_core::store::Error::InvalidComments { .. })),
            "a parse failure must be reported as InvalidComments, got {:?}",
            result.err()
        );
    }

    /// The complementary half: a *missing* `comments.json` is not an error —
    /// a session with no comments has nothing to read — and the module's
    /// documented crash residue is genuinely harmless.
    ///
    /// `append_comment` writes the snapshot first and `comments.json` last, so
    /// a crash between the two strands an orphaned snapshot with no matching
    /// entry, and a crash between `write_atomic`'s fsync and its rename strands
    /// a `.rv-store-*.tmp` sibling. The doc calls both harmless; this generates
    /// exactly that post-crash tree and holds it to that word: `comments()` is
    /// empty (not an error, and not the orphans resurrected), and a subsequent
    /// append works normally — including when it reuses an orphan's id, where
    /// the stale snapshot must be replaced by the new comment's context rather
    /// than left in place.
    #[test]
    fn an_orphaned_snapshot_is_harmless_and_a_missing_comments_json_reads_as_empty(
        session in session(12, 2),
        document in hostile_text(24),
        orphans in prop::collection::vec((prop::sample::select(id_pool(5)), hostile_text(16)), 1..4),
        stray_temp in any::<bool>(),
        reuse in any::<bool>(),
        newcomer in comment(prop::sample::select(id_pool(5)), 12),
    ) {
        let repo = repo_root();
        let store = Store::open(repo.path()).expect("open store");
        store.write_session(&session).expect("write session");
        store.write_markdown(&document).expect("write markdown");

        // The tree a crash between the two writes leaves behind.
        let snapshots = repo.path().join(".review/snapshots");
        for (id, body) in &orphans {
            fs::write(snapshots.join(id), body).expect("plant an orphaned snapshot");
        }
        if stray_temp {
            fs::write(
                repo.path().join(".review").join(format!("{TEMP_PREFIX}orphan.tmp")),
                b"half a comments.json",
            ).expect("plant a stray temp file");
        }

        prop_assert_eq!(store.comments().expect("read comments"), Vec::<Comment>::new());

        // Appending after the crash behaves as if the orphans were not there.
        let mut newcomer = newcomer;
        if reuse {
            newcomer.id = orphans[0].0.clone();
        }
        store.append_comment(&newcomer).expect("append after a crash");
        prop_assert_eq!(store.comments().expect("read comments"), vec![newcomer.clone()]);

        let written = fs::read_to_string(snapshots.join(&newcomer.id)).expect("read new snapshot");
        prop_assert_eq!(written, newcomer.anchor.context.join("\n"),
            "a stale orphaned snapshot must be replaced, not kept");
    }
}

/// `comments()` has three cases to tell apart and only one of them means "no
/// comments": *missing* (empty, no error), *corrupt* (an error), and *present
/// but unreadable* (an error too). The third is the one that collapses first,
/// because `Err(_) => Ok(Vec::new())` is one keystroke away from the `NotFound`
/// arm that genuinely does mean "nothing saved yet" — and the consequence is a
/// reviewer's saved work reported as an empty review, silently, with the file
/// sitting right there on disk. The two properties above cover missing and
/// corrupt; this covers unreadable, which is an *IO* failure and so reaches a
/// different arm than a parse failure does.
///
/// A directory planted at the path is the portable way to produce one: reading
/// it fails with a kind that is not `NotFound` on every platform, no `chmod`
/// and no privileges involved.
#[test]
fn a_comments_json_that_cannot_be_read_is_an_error_not_silent_emptiness() {
    let repo = repo_root();
    let store = Store::open(repo.path()).expect("open store");
    let path = repo.path().join(".review/comments.json");
    fs::create_dir(&path).expect("plant a directory at comments.json");

    let result = store.comments();

    assert!(
        result.is_err(),
        "an unreadable comments.json read as {:?} instead of failing",
        result.as_ref().map(Vec::len)
    );
    assert!(
        matches!(result, Err(rv_core::store::Error::Io { .. })),
        "an IO failure must be reported as Io, not as a parse error: {:?}",
        result.err()
    );
}

/// True when this process actually cannot read a mode-`0o000` file. It can when
/// it runs as root, and on a filesystem that does not enforce permission bits —
/// in either case the permission test below would be testing nothing, so it
/// skips rather than fails.
#[cfg(unix)]
fn permission_bits_bite(probe_dir: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;

    let probe = probe_dir.join(".permission-probe");
    fs::write(&probe, b"probe").expect("write the probe file");
    fs::set_permissions(&probe, fs::Permissions::from_mode(0o000)).expect("chmod the probe file");
    let enforced = fs::read(&probe).is_err();
    fs::set_permissions(&probe, fs::Permissions::from_mode(0o600)).expect("restore the probe file");
    fs::remove_file(&probe).expect("remove the probe file");
    enforced
}

/// The same claim as above at the shape it takes in the field: a
/// `comments.json` that is a perfectly good file the process simply may not
/// open — a review directory copied between accounts, a restrictive umask, a
/// sandbox. Distinct from the directory case because it is an `EACCES` at
/// `open` rather than an `EISDIR` at `read`, and a handler that special-cases
/// one kind may still swallow the other.
#[cfg(unix)]
#[test]
fn a_comments_json_the_process_may_not_open_is_an_error_not_silent_emptiness() {
    use std::os::unix::fs::PermissionsExt;

    let repo = repo_root();
    let store = Store::open(repo.path()).expect("open store");
    store
        .append_comment(&fixed_comment("id0"))
        .expect("append comment");
    let path = repo.path().join(".review/comments.json");

    if !permission_bits_bite(repo.path()) {
        // Running as root, or on a filesystem that ignores the mode bits.
        return;
    }
    fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).expect("chmod comments.json");

    let result = store.comments();

    // Restore before asserting, so a failure still leaves a removable tempdir.
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).expect("restore comments.json");
    assert!(
        result.is_err(),
        "an unreadable comments.json read as {:?} instead of failing — \
         a saved review reported as empty",
        result.as_ref().map(Vec::len)
    );
    assert!(
        matches!(result, Err(rv_core::store::Error::Io { .. })),
        "an IO failure must be reported as Io, not as a parse error: {:?}",
        result.err()
    );
    // And the data really was there to be lost.
    assert_eq!(store.comments().expect("read comments").len(), 1);
}

// ---------------------------------------------------------------------------
// removal: the snapshot delete, and the states it has to tolerate
// ---------------------------------------------------------------------------

/// `remove_comment` deletes the `comments.json` entry first and the snapshot
/// second, and treats a snapshot that is *already* gone as success rather than
/// as an IO error: the delete is a step towards "no such comment", and finding
/// that step already taken is not a failure.
///
/// The state is reachable without any crash at all. `.review/` is a scratch
/// directory deliberately kept out of version control, so nothing restores it:
/// a stray `rm`, a partial copy between machines, a half-restored backup or a
/// cleanup script all leave `comments.json` — the authority on which comments
/// exist — still listing a comment whose snapshot is not there. Without the
/// `NotFound` arm that reviewer's next delete fails, and fails *after* the
/// entry has already been rewritten out, so the caller is told the removal
/// failed while the store has in fact performed it.
///
/// The two cases reach the arm by different routes — the snapshot file unlinked
/// on its own, and the whole snapshots directory gone — because a handler that
/// happens to tolerate one need not tolerate the other.
#[rstest]
#[case::the_snapshot_file_was_deleted(".review/snapshots/id0", true)]
#[case::the_whole_snapshots_directory_was_deleted(".review/snapshots", false)]
fn removing_a_comment_whose_snapshot_is_already_gone_still_succeeds(
    #[case] pruned: &str,
    #[case] survivor_keeps_its_snapshot: bool,
) {
    let repo = repo_root();
    let store = Store::open(repo.path()).expect("open store");
    store
        .append_comment(&fixed_comment("id0"))
        .expect("append id0");
    store
        .append_comment(&fixed_comment("id1"))
        .expect("append id1");

    // Out of band: nothing the store did, exactly as a stray `rm` leaves it.
    let pruned = repo.path().join(pruned);
    if pruned.is_dir() {
        fs::remove_dir_all(&pruned).expect("prune the snapshots directory");
    } else {
        fs::remove_file(&pruned).expect("prune the snapshot file");
    }

    let removed = store
        .remove_comment("id0")
        .expect("a removal whose snapshot is already gone must still succeed");

    assert!(
        removed,
        "the comment was in comments.json, so the removal did remove one"
    );
    assert_eq!(
        ids_of(&store.comments().expect("read comments")),
        vec!["id1".to_owned()],
        "the entry is gone and the other comment is untouched"
    );
    assert!(!repo.path().join(".review/snapshots/id0").exists());
    assert_eq!(
        repo.path().join(".review/snapshots/id1").exists(),
        survivor_keeps_its_snapshot,
        "the removal must not reach past the snapshot it owns"
    );
    assert!(stray_temp_files(repo.path()).is_empty());
    // And the retry an interrupted delete issues is the ordinary no-op.
    assert!(
        !store
            .remove_comment("id0")
            .expect("a second removal of the same id must succeed too")
    );
}

/// The residue a crash *inside* `remove_comment` leaves, held to the same word
/// the module uses for the append path's residue: inert.
///
/// The delete rewrites `comments.json` first and unlinks the snapshot second,
/// so the window between the two is a live store whose snapshots directory
/// holds a file no entry names. `an_orphaned_snapshot_is_harmless_and_a_missing_comments_json_reads_as_empty`
/// covers the *append* path's version of that state, where `comments.json` does
/// not exist at all; this is the delete path's version, where it exists and
/// lists other comments, which is a different code path through `comments()`.
///
/// What the store must make of it: the removed comment stays removed rather
/// than being resurrected from the snapshot that outlived it, the surviving
/// comment is untouched, the retry an interrupted delete issues reports `false`
/// and succeeds, and re-adding the same id overwrites the stale snapshot with
/// the new comment's context instead of adopting it.
#[test]
fn a_crash_between_a_removals_two_writes_leaves_an_inert_orphan() {
    let repo = repo_root();
    let store = Store::open(repo.path()).expect("open store");
    store
        .append_comment(&fixed_comment("id0"))
        .expect("append id0");
    store
        .append_comment(&fixed_comment("id1"))
        .expect("append id1");
    let snapshots = repo.path().join(".review/snapshots");

    // Exactly the state a crash between the two writes leaves: the entry
    // rewritten out of comments.json, the snapshot never unlinked.
    store.remove_comment("id0").expect("remove id0");
    fs::write(snapshots.join("id0"), "the context id0 had").expect("plant the orphan");

    assert_eq!(
        ids_of(&store.comments().expect("read comments")),
        vec!["id1".to_owned()],
        "an orphaned snapshot must not resurrect the comment it belonged to"
    );
    assert!(
        !store
            .remove_comment("id0")
            .expect("the retry an interrupted delete issues must succeed"),
        "there is no entry left to remove, so the retry removed nothing"
    );
    assert_eq!(
        fs::read_to_string(snapshots.join("id1")).expect("read the survivor's snapshot"),
        fixed_comment("id1").anchor.context.join("\n"),
        "the surviving comment's snapshot is untouched"
    );

    // And the id can be reused: the stale snapshot is replaced, not adopted.
    let mut newcomer = fixed_comment("id0");
    newcomer.anchor.context = vec!["fn b() {}".to_owned(), "// and a second line".to_owned()];
    store
        .append_comment(&newcomer)
        .expect("re-add the id the orphan belongs to");

    assert_eq!(
        ids_of(&store.comments().expect("read comments")),
        vec!["id1".to_owned(), "id0".to_owned()],
        "a re-added comment is a new entry at the end, not a revival of its old slot"
    );
    assert_eq!(
        fs::read_to_string(snapshots.join("id0")).expect("read the reused snapshot"),
        "fn b() {}\n// and a second line",
        "a stale orphaned snapshot must be replaced, not kept"
    );
    assert!(stray_temp_files(repo.path()).is_empty());
}

/// True when this process actually cannot create a file in a mode-`0o500`
/// directory. It can when it runs as root, and on a filesystem that does not
/// enforce permission bits — in either case the test below would be testing
/// nothing, so it skips rather than fails. The file-mode twin of
/// [`permission_bits_bite`], and separate from it because the two ask about
/// different bits: reading a file versus creating one in a directory.
#[cfg(unix)]
fn directory_permission_bits_bite(probe_dir: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;

    let probe = probe_dir.join(".permission-probe-dir");
    fs::create_dir(&probe).expect("create the probe directory");
    fs::set_permissions(&probe, fs::Permissions::from_mode(0o500))
        .expect("chmod the probe directory");
    let enforced = fs::write(probe.join("probe"), b"probe").is_err();
    fs::set_permissions(&probe, fs::Permissions::from_mode(0o700))
        .expect("restore the probe directory");
    fs::remove_dir_all(&probe).expect("remove the probe directory");
    enforced
}

/// The removal half of the module's crash-safety ordering, forced rather than
/// waited for, and the mirror of
/// [`a_comment_whose_snapshot_cannot_be_written_is_never_recorded`]:
/// `remove_comment` rewrites `comments.json` *before* deleting the snapshot, so
/// when that rewrite cannot happen the snapshot must still be on disk and the
/// comment must still be listed — no half-deleted comment, no live entry
/// pointing at a snapshot that has been unlinked out from under it.
///
/// `write_atomic` creates its temp file in the destination's own directory, so
/// a `.review/` this process may not write to is a rewrite that cannot even
/// start, while leaving `.review/snapshots/` writable and `comments.json`
/// readable — which is exactly the shape that tells the two orderings apart. A
/// removal that deleted the snapshot first would leave `comments.json` claiming
/// a comment whose context is gone, and the whole-directory comparison sees it.
#[cfg(unix)]
#[test]
fn a_removal_whose_comments_json_cannot_be_written_deletes_nothing() {
    use std::os::unix::fs::PermissionsExt;

    let repo = repo_root();
    let store = Store::open(repo.path()).expect("open store");
    store
        .append_comment(&fixed_comment("id0"))
        .expect("append id0");
    store
        .append_comment(&fixed_comment("id1"))
        .expect("append id1");
    let before = dir_snapshot(&repo.path().join(".review"));

    if !directory_permission_bits_bite(repo.path()) {
        // Running as root, or on a filesystem that ignores the mode bits.
        return;
    }
    let review = repo.path().join(".review");
    fs::set_permissions(&review, fs::Permissions::from_mode(0o500)).expect("chmod .review");

    let result = store.remove_comment("id0");

    // Restore before asserting, so a failure still leaves a removable tempdir.
    fs::set_permissions(&review, fs::Permissions::from_mode(0o700)).expect("restore .review");
    assert!(
        result.is_err(),
        "the comments.json rewrite cannot have succeeded: {result:?}"
    );
    assert_eq!(
        dir_snapshot(&repo.path().join(".review")),
        before,
        "a removal that could not rewrite comments.json still changed .review/ — \
         a snapshot deleted before its entry leaves a comment with no context"
    );
    assert!(stray_temp_files(repo.path()).is_empty());
}

// ---------------------------------------------------------------------------
// atomicity: what a reader sees across a write
// ---------------------------------------------------------------------------

/// The module's headline claim — "a reader can never observe a half-written
/// file" — made observable without a race.
///
/// `write_atomic` renames a fresh temp file over the destination, which
/// repoints the destination's *directory entry* at a new inode and leaves the
/// old one complete and unmodified for as long as anybody holds it open. A
/// plain `fs::write` instead truncates and rewrites the existing inode in
/// place, which is precisely the window in which a concurrent reader sees a
/// prefix of the new bytes, or an empty file, or a mix of both.
///
/// So the test opens the file first and reads it last: under a rename the held
/// handle yields the document that was there when it was opened, whole; under
/// an in-place rewrite it yields the newest one, or a truncated splice of the
/// two. That difference is *deterministic*, which is why this is a plain test
/// and not the threaded writer-and-reader probe the claim seems to demand — a
/// probe racing two real threads can only ever fail to notice, never notice
/// something that is not there, so it would be a weaker guard at a higher cost
/// in wall time and flakiness.
#[cfg(unix)]
#[rstest]
#[case::a_shorter_replacement("the original document\n", "short\n")]
#[case::a_longer_replacement("short\n", "a considerably longer replacement document\n")]
#[case::an_empty_replacement("the original document\n", "")]
fn a_reader_holding_the_file_open_never_sees_a_later_write(
    #[case] original: &str,
    #[case] replacement: &str,
) {
    use std::io::Read as _;

    let repo = repo_root();
    let store = Store::open(repo.path()).expect("open store");
    store.write_markdown(original).expect("write markdown");

    let mut held = fs::File::open(store.markdown_path()).expect("open markdown before the rewrite");
    store.write_markdown(replacement).expect("rewrite markdown");

    let mut seen = String::new();
    held.read_to_string(&mut seen)
        .expect("read the handle opened before the rewrite");
    assert_eq!(
        seen, original,
        "a reader that opened the file before the write observed the write"
    );
    // And the write did land, so the assertion above is not satisfied by a
    // no-op.
    assert_eq!(
        fs::read_to_string(store.markdown_path()).expect("read markdown"),
        replacement
    );
    assert!(stray_temp_files(repo.path()).is_empty());
}

/// The same, for the file that matters most: `comments.json` is rewritten
/// wholesale on every single saved comment, so every append is a chance for a
/// reader to catch it mid-flight.
#[cfg(unix)]
#[test]
fn a_reader_holding_comments_json_open_never_sees_a_later_append() {
    use std::io::Read as _;

    let repo = repo_root();
    let store = Store::open(repo.path()).expect("open store");
    store
        .append_comment(&fixed_comment("id0"))
        .expect("append the first comment");
    let path = repo.path().join(".review/comments.json");
    let original = fs::read_to_string(&path).expect("read comments.json");

    let mut held = fs::File::open(&path).expect("open comments.json before the append");
    for id in ["id1", "id10", "id2"] {
        store
            .append_comment(&fixed_comment(id))
            .expect("append another comment");
    }

    let mut seen = String::new();
    held.read_to_string(&mut seen)
        .expect("read the handle opened before the appends");
    assert_eq!(
        seen, original,
        "a reader that opened comments.json before the appends observed them"
    );
    assert_eq!(store.comments().expect("read comments").len(), 4);
}

// ---------------------------------------------------------------------------
// parameterized case tables
// ---------------------------------------------------------------------------

/// `markdown_path()` names the one file another program reads *while* `rv` is
/// running, so where it points is contract, not an implementation detail: it is
/// `REVIEW-FEEDBACK.md` inside `.review/`, and in particular it is not at the
/// repo root, where it would land inside the change under review and defeat the
/// whole point of [`Store::ensure_excluded`].
#[rstest]
#[case::repo_root_is_the_tempdir("")]
#[case::repo_root_is_nested("outer/inner")]
fn markdown_is_written_inside_the_review_directory(#[case] relative_root: &str) {
    let tempdir = tempfile::tempdir().expect("create temp dir");
    let root = tempdir.path().join(relative_root);
    fs::create_dir_all(root.join(".git/info")).expect("create .git/info");
    let store = Store::open(&root).expect("open store");

    assert_eq!(
        store.markdown_path(),
        root.join(".review").join("REVIEW-FEEDBACK.md")
    );

    store.write_markdown("a document").expect("write markdown");
    assert_eq!(
        fs::read_to_string(root.join(".review/REVIEW-FEEDBACK.md")).expect("read markdown"),
        "a document"
    );
    assert!(
        !root.join("REVIEW-FEEDBACK.md").exists(),
        "the feedback document must not land at the repo root"
    );
}

/// `.git/info/exclude` contents that must and must not count as
/// already-excluded, with the exact bytes the store is allowed to leave
/// behind. `None` means the file does not exist yet.
#[rstest]
// created from nothing
#[case(None, true, "/.review/\n")]
#[case(Some(""), true, "/.review/\n")]
// appended after another tool's patterns
#[case(Some("target/\n"), true, "target/\n/.review/\n")]
#[case(Some("target/\n*.log\n"), true, "target/\n*.log\n/.review/\n")]
// no trailing newline: a separator is added rather than gluing the line on
#[case(Some("target/"), true, "target/\n/.review/\n")]
#[case(Some("a\nb"), true, "a\nb\n/.review/\n")]
// CRLF content is not rewritten to LF
#[case(Some("target/\r\n"), true, "target/\r\n/.review/\n")]
// blank lines are content too and must survive
#[case(Some("a\n\n"), true, "a\n\n/.review/\n")]
// already present: a byte-for-byte no-op, wherever it sits
#[case(Some("/.review/\n"), false, "/.review/\n")]
#[case(
    Some("target/\n/.review/\n*.log\n"),
    false,
    "target/\n/.review/\n*.log\n"
)]
#[case(Some("/.review/"), false, "/.review/")]
#[case(Some("/.review/\r\n"), false, "/.review/\r\n")]
// duplicates already in the file are left alone rather than "fixed"
#[case(Some("/.review/\n/.review/\n"), false, "/.review/\n/.review/\n")]
// look-alikes that must NOT suppress the append
#[case(Some("#/.review/\n"), true, "#/.review/\n/.review/\n")]
#[case(Some("# /.review/\n"), true, "# /.review/\n/.review/\n")]
#[case(Some("/.review\n"), true, "/.review\n/.review/\n")]
#[case(Some(".review/\n"), true, ".review/\n/.review/\n")]
#[case(Some("/.review/*\n"), true, "/.review/*\n/.review/\n")]
#[case(Some("x/.review/y\n"), true, "x/.review/y\n/.review/\n")]
#[case(Some("/.review/ \n"), true, "/.review/ \n/.review/\n")]
#[case(Some("  /.review/\n"), true, "  /.review/\n/.review/\n")]
#[case(Some("\t/.review/\n"), true, "\t/.review/\n/.review/\n")]
#[case(Some("!/.review/\n"), true, "!/.review/\n/.review/\n")]
fn ensure_excluded_exclude_file_cases(
    #[case] existing: Option<&str>,
    #[case] expect_added: bool,
    #[case] expected: &str,
) {
    let repo = repo_root();
    let exclude = repo.path().join(".git/info/exclude");
    if let Some(contents) = existing {
        fs::write(&exclude, contents).expect("seed exclude file");
    }
    let store = Store::open(repo.path()).expect("open store");

    let added = store.ensure_excluded().expect("ensure_excluded");

    assert_eq!(added, expect_added, "return value for {existing:?}");
    let after = fs::read_to_string(&exclude).expect("read exclude file");
    assert_eq!(after, expected, "exclude file contents for {existing:?}");
    // A second call is always a no-op from here.
    assert!(!store.ensure_excluded().expect("second ensure_excluded"));
    assert_eq!(
        fs::read_to_string(&exclude).expect("read exclude file"),
        expected
    );
}

/// `ensure_excluded` creates `.git/info/` when it is missing, not just the
/// `exclude` file inside it.
#[rstest]
#[case::no_git_dir_at_all(&[])]
#[case::git_dir_only(&[".git"])]
#[case::git_info_exists(&[".git", ".git/info"])]
fn ensure_excluded_creates_missing_parents(#[case] preexisting: &[&str]) {
    let tempdir = tempfile::tempdir().expect("create temp dir");
    for dir in preexisting {
        fs::create_dir_all(tempdir.path().join(dir)).expect("create dir");
    }
    let store = Store::open(tempdir.path()).expect("open store");

    assert!(store.ensure_excluded().expect("ensure_excluded"));

    let exclude = tempdir.path().join(".git/info/exclude");
    assert_eq!(
        fs::read_to_string(&exclude).expect("read exclude file"),
        "/.review/\n"
    );
    assert!(stray_temp_files(tempdir.path()).is_empty());
}

/// Session fields are opaque strings — `description` in particular is a commit
/// description, which really can start with a newline or contain quotes — so
/// each of these must survive `session.toml` untouched. TOML's multi-line
/// string forms trim a newline immediately after the opening delimiter and
/// cannot carry some control characters, so a serializer that reaches for them
/// carelessly loses bytes exactly here.
#[rstest]
#[case::empty("")]
#[case::leading_newline("\nafter a leading newline")]
#[case::trailing_newline("before a trailing newline\n")]
#[case::only_newlines("\n\n\n")]
#[case::crlf("first\r\nsecond")]
#[case::lone_carriage_return("first\rsecond")]
#[case::interior_newlines("subject\n\nbody line one\nbody line two")]
#[case::basic_quotes("he said \"maybe\"")]
#[case::triple_basic_quotes("guard \"\"\" against")]
#[case::literal_quotes("it's fine")]
#[case::triple_literal_quotes("guard ''' against")]
#[case::backslashes("C:\\path\\to\\nowhere")]
#[case::escape_lookalike("literally \\n not a newline")]
#[case::tabs("\tindented\tby\ttabs")]
#[case::trailing_space("trailing space ")]
#[case::nul("before\0after")]
#[case::escape_char("\u{1b}[31mred\u{1b}[0m")]
#[case::delete_char("before\u{7f}after")]
#[case::bom("\u{feff}leading bom")]
#[case::line_separator("before\u{2028}after")]
#[case::unicode("é ß 中 🙂")]
#[case::toml_syntax("[table]\nkey = \"value\" # comment")]
fn session_fields_survive_toml_hostile_strings(#[case] text: &str) {
    let repo = repo_root();
    let store = Store::open(repo.path()).expect("open store");
    let session = Session {
        revset: text.to_owned(),
        base_commit: "abc123def456".to_owned(),
        head_commit: "def456abc123".to_owned(),
        changes: vec![ChangeRef {
            change_id: "nowwnlnmvkwo".to_owned(),
            commit_id: "def456abc123".to_owned(),
            description: text.to_owned(),
        }],
        started_at: text.to_owned(),
    };

    store.write_session(&session).expect("write session");

    let read_back = Store::open(repo.path())
        .expect("reopen store")
        .read_session()
        .expect("read session");
    assert_eq!(read_back.revset, session.revset, "revset");
    assert_eq!(read_back.started_at, session.started_at, "started_at");
    assert_eq!(
        read_back.changes[0].description, session.changes[0].description,
        "change description"
    );
    assert_eq!(read_back, session);
}

/// `CommentState` serializes in kebab-case on disk — the vocabulary the
/// markdown export shares — and reads back as the same variant.
#[rstest]
#[case(CommentState::Open, "open")]
#[case(CommentState::AwaitingVerification, "awaiting-verification")]
#[case(CommentState::Resolved, "resolved")]
#[case(CommentState::Outdated, "outdated")]
fn comment_state_wire_format(#[case] state: CommentState, #[case] expected: &str) {
    let repo = repo_root();
    let store = Store::open(repo.path()).expect("open store");
    let comment = Comment {
        id: "id0".to_owned(),
        change_id: "nowwnlnmvkwo".to_owned(),
        commit_id: "abc123def456".to_owned(),
        anchor: Anchor {
            file: "src/lib.rs".to_owned(),
            side: Side::Right,
            line: 1,
            content_hash: "deadbeef".to_owned(),
            context: vec!["fn a() {}".to_owned()],
        },
        body: "why".to_owned(),
        state,
        reply: None,
        settled_by: None,
    };

    store.append_comment(&comment).expect("append comment");

    let raw =
        fs::read_to_string(repo.path().join(".review/comments.json")).expect("read comments.json");
    assert!(
        raw.contains(&format!("\"state\": \"{expected}\"")),
        "comments.json should spell {state:?} in kebab-case:\n{raw}"
    );
    // Pretty-printed for a human poking around .review/, per the module doc.
    assert!(
        raw.contains("\n  {"),
        "comments.json should be pretty-printed:\n{raw}"
    );
    assert_eq!(
        store.comments().expect("read comments")[0].state,
        state,
        "state must read back as the same variant"
    );
}
