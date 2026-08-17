//! Property-based and parameterized tests for the `.review/` on-disk store
//! (spec §10), complementing the hand-written cases in `tests/store.rs`.
//!
//! The hand-written tests pin single concrete examples of each behaviour. The
//! properties here go after the *laws* the module's doc comment claims, with
//! oracles that are independent of the implementation wherever possible:
//!
//! * an in-memory upsert reduction recomputed with a different data structure
//!   (`HashMap` + insertion-order `Vec`) as the oracle for `comments()`,
//! * `split('\n')` as the inverse of the `join("\n")` the snapshot file uses,
//! * whole-directory byte snapshots as a conservation oracle ("nothing lost,
//!   nothing invented") for operations that must not touch other files,
//! * permutation invariance, idempotence and last-write-wins as algebraic laws,
//! * and, for the module's headline crash-safety claim, a *forced* failure of
//!   the snapshot write: `comments.json` is the authority on which comments
//!   exist, so a comment whose snapshot could not be written must not appear
//!   in it.
//!
//! Like `tests/store.rs`, these need no jj repository — only a tempdir shaped
//! like a repo root, so `.git/info/exclude` lands where `ensure_excluded`
//! looks for it.

use std::collections::BTreeMap;
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

/// The oracle for `Store::comments()`: last write wins per id, first-insertion
/// order preserved. Deliberately computed with a different shape than the
/// store's in-place `iter_mut().find()` — a hash map for the values plus a
/// separate vector for the order — so it is a recomputation rather than a
/// restatement.
fn upsert_reduce(sequence: &[Comment]) -> Vec<Comment> {
    let mut order: Vec<String> = Vec::new();
    let mut latest: HashMap<String, Comment> = HashMap::new();
    for comment in sequence {
        if latest.insert(comment.id.clone(), comment.clone()).is_none() {
            order.push(comment.id.clone());
        }
    }
    order
        .into_iter()
        .map(|id| latest.remove(&id).expect("id was recorded"))
        .collect()
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
fn id_pool(count: usize) -> Vec<String> {
    (0..count).map(|i| format!("id{i}")).collect()
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
            },
        )
}

/// A sequence of comments drawn from a small id pool, so runs contain both
/// fresh inserts and same-id updates.
fn comment_sequence(len: std::ops::Range<usize>) -> impl Strategy<Value = Vec<Comment>> {
    prop::collection::vec(comment(prop::sample::select(id_pool(4)), 8), len)
}

/// Comments whose ids are distinct by construction, so no upsert collapsing
/// happens and every appended comment must survive.
fn distinct_comments(len: std::ops::Range<usize>) -> impl Strategy<Value = Vec<Comment>> {
    prop::collection::vec(comment(Just(String::new()), 8), len).prop_map(|mut comments| {
        for (index, comment) in comments.iter_mut().enumerate() {
            comment.id = format!("id{index}");
        }
        comments
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
/// everything the module writes.
#[derive(Clone, Debug)]
enum Op {
    Append(Comment),
    WriteSession(Session),
    WriteMarkdown(String),
    EnsureExcluded,
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        4 => comment(prop::sample::select(id_pool(3)), 8).prop_map(Op::Append),
        2 => session(8, 2).prop_map(Op::WriteSession),
        2 => hostile_text(20).prop_map(Op::WriteMarkdown),
        1 => Just(Op::EnsureExcluded),
    ]
}

fn apply(store: &Store, op: &Op) -> Result<(), rv_core::store::Error> {
    match op {
        Op::Append(comment) => store.append_comment(comment),
        Op::WriteSession(session) => store.write_session(session),
        Op::WriteMarkdown(document) => store.write_markdown(document),
        Op::EnsureExcluded => store.ensure_excluded().map(|_| ()),
    }
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
    #[test]
    fn the_set_of_comments_never_depends_on_append_order(
        (canonical, shuffled) in distinct_comments(1..6)
            .prop_flat_map(|comments| (Just(comments.clone()), Just(comments).prop_shuffle())),
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
    /// *freshly opened* `Store` over the same root sees exactly what the first
    /// one does. There is no in-memory state a second `Store` cannot see.
    #[test]
    fn everything_written_is_visible_to_a_freshly_opened_store(
        ops in prop::collection::vec(op(), 1..7),
    ) {
        let repo = repo_root();
        let seen_by_writer;
        let markdown_by_writer;
        {
            let store = Store::open(repo.path()).expect("open store");
            for op in &ops {
                apply(&store, op).expect("apply op");
            }
            seen_by_writer = store.comments().expect("read comments");
            markdown_by_writer = fs::read(store.markdown_path()).ok();
        }

        let reopened = Store::open(repo.path()).expect("reopen store");
        prop_assert_eq!(reopened.comments().expect("read comments"), seen_by_writer);
        prop_assert_eq!(fs::read(reopened.markdown_path()).ok(), markdown_by_writer);
        prop_assert_eq!(reopened.markdown_path(), repo.path().join(".review/REVIEW-FEEDBACK.md"));
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
    /// context lines, and no snapshot exists for an id never appended.
    ///
    /// The oracle is `split('\n')`, the genuine inverse of the `join("\n")`
    /// the store uses — which only works because context lines never contain
    /// `\n` (they come from splitting file text). The empty-context case is
    /// checked separately, since `join` maps `[]` to `""` while `split` maps
    /// `""` back to `[""]`; that asymmetry is inherent to the format, not a
    /// bug.
    #[test]
    fn each_appended_comment_snapshots_its_context_lines(
        sequence in comment_sequence(1..6),
    ) {
        let repo = repo_root();
        let store = Store::open(repo.path()).expect("open store");
        for comment in &sequence {
            store.append_comment(comment).expect("append comment");
        }

        let stored = store.comments().expect("read comments");
        for comment in &stored {
            let path = repo.path().join(".review/snapshots").join(&comment.id);
            let snapshot = fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("read snapshot {}: {error}", path.display()));
            if comment.anchor.context.is_empty() {
                prop_assert_eq!(snapshot.as_str(), "",
                    "no context lines means an empty snapshot");
            } else {
                prop_assert_eq!(
                    snapshot.split('\n').collect::<Vec<_>>(),
                    comment.anchor.context.iter().map(String::as_str).collect::<Vec<_>>()
                );
            }
        }

        // Conservation in both directions: exactly the ids in comments.json
        // have snapshots — no comment without one, no orphan on the happy path.
        let mut expected = ids_of(&stored);
        expected.sort();
        prop_assert_eq!(snapshot_ids(repo.path()), expected);
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

    /// Duplicate change ids in one session survive as duplicates: `changes` is
    /// a list, not a set, and the store must not collapse it.
    #[test]
    fn duplicate_change_ids_in_a_session_are_preserved(
        change in change_ref(12),
        copies in 1usize..5,
    ) {
        let repo = repo_root();
        let store = Store::open(repo.path()).expect("open store");
        let session = Session {
            revset: "trunk()..@".to_owned(),
            base_commit: "abc123".to_owned(),
            head_commit: "def456".to_owned(),
            changes: vec![change.clone(); copies],
            started_at: "epoch:1755460770".to_owned(),
        };
        store.write_session(&session).expect("write session");

        let read_back = store.read_session().expect("read session");
        prop_assert_eq!(read_back.changes.len(), copies);
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
    /// leaves the other two byte-identical.
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
        let session_bytes = fs::read(&session_path).expect("read session.toml");
        let markdown_bytes = fs::read(&markdown_path).expect("read markdown");

        store.write_session(&second_session).expect("rewrite session");
        prop_assert_eq!(fs::read(&comments_path).expect("read comments.json"), comments_bytes.clone(),
            "write_session touched comments.json");
        prop_assert_eq!(fs::read(&markdown_path).expect("read markdown"), markdown_bytes.clone(),
            "write_session touched REVIEW-FEEDBACK.md");
        let _ = session_bytes;

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
        prop_assert_eq!(fs::read(&session_path).expect("read session.toml"), session_bytes,
            "write_markdown touched session.toml");
        prop_assert_eq!(store.comments().expect("read comments").len(), comments.len() + 1);
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
    }

    /// The complementary half: a *missing* `comments.json` is not an error —
    /// a session with no comments has nothing to read. Holds no matter what
    /// else exists under `.review/`.
    #[test]
    fn a_missing_comments_json_reads_as_empty(
        session in session(12, 2),
        document in hostile_text(24),
    ) {
        let repo = repo_root();
        let store = Store::open(repo.path()).expect("open store");
        store.write_session(&session).expect("write session");
        store.write_markdown(&document).expect("write markdown");

        prop_assert_eq!(store.comments().expect("read comments"), Vec::<Comment>::new());
    }
}

// ---------------------------------------------------------------------------
// parameterized case tables
// ---------------------------------------------------------------------------

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
