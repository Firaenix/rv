//! One file, whatever the sequence, and the residue an atomic write leaves.
//!
//! Split from [`super`] for the 400-line rule; the fixtures, strategies,
//! `Op` model and sequence oracles live there.
//! Split from [`super`] for the 400-line rule; the fixtures, strategies,
//! `Op` model and sequence oracles live there.

use std::fs;

use rv_core::store::Session;
use rv_core::store::Store;

use super::*;
// ---------------------------------------------------------------------------
// one file, whatever the sequence
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(config(20))]

    /// Saving never writes a second file. The excerpt lives in
    /// `anchor.context` inside `session.toml`, and the byte-for-byte duplicate
    /// earlier versions wrote under `.review/snapshots/` protected nothing
    /// (storage spec §1), so conservation here is an *absence* oracle.
    #[test]
    fn appending_writes_nothing_beside_session_toml(
        sequence in comment_sequence(1..6),
    ) {
        let repo = repo_root();
        let store = Store::open(repo.path()).expect("open store");
        for comment in &sequence {
            store.append_comment(comment).expect("append comment");
        }

        only_one_file(repo.path())?;
    }

    /// Appends and removals interleaved over a handful of ids: after *every*
    /// step, `comments()` is exactly what the model says, and at the end
    /// `.review/` still holds the one file.
    ///
    /// Two separate claims about `remove_comment` ride on this, neither
    /// reachable from an append-only sequence:
    ///
    /// * the entry goes, and only that entry — the model's order vector pins
    ///   the survivors' positions as well as their identity, and [`ID_POOL`]'s
    ///   prefix structure means a delete matching on `starts_with` instead of
    ///   `==` takes bystanders with it;
    /// * and the returned `bool` is the truth about whether anything was there,
    ///   which is the store's answer to "did this id exist?" and is checked
    ///   against the model *before* the call, including for the ids the append
    ///   pool never mints.
    ///
    /// Checking after every step rather than only at the end is what makes a
    /// shrunk counterexample point at the operation that broke it instead of at
    /// the whole history.
    #[test]
    fn appends_and_removals_leave_exactly_the_model_comments(
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

        only_one_file(repo.path())?;
        prop_assert!(
            stray_temp_files(repo.path()).is_empty(),
            "stray temp files: {:?}", stray_temp_files(repo.path())
        );
    }

}

// ---------------------------------------------------------------------------
// atomicity residue
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(config(16))]

    /// After any successful sequence of operations, none of `write_atomic`'s
    /// temp files remain anywhere under the repo root — not in `.review/`,
    /// not in `.git/info/`. Generalizes the hand-written three-append case to
    /// arbitrary interleavings of every write the module makes.
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

    /// `read_review` after `write_review` reproduces the whole file exactly,
    /// for arbitrary field values: empty strings, unicode, no changes, several
    /// changes, change lists with duplicate ids (the store stores what it is
    /// given and does not dedup a stack), and the comments beside them.
    #[test]
    fn session_toml_roundtrips_arbitrary_sessions(session in session(16, 4)) {
        let repo = repo_root();
        let store = Store::open(repo.path()).expect("open store");
        store.write_review(&session).expect("write review");

        let read_back = Store::open(repo.path())
            .expect("reopen store")
            .read_review()
            .expect("read review");
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
            comments: Vec::new(),
        };
        store.write_review(&session).expect("write review");

        let read_back = store.read_review().expect("read review");
        prop_assert_eq!(read_back.changes.len(), expected_len,
            "a change list must not be deduped, adjacently or globally");
        prop_assert_eq!(&read_back.changes, &changes, "and its order must be preserved");
        prop_assert_eq!(read_back, session);
    }

    /// `write_review` is a wholesale replacement: the last review written is
    /// the only one readable, with no residue from a longer predecessor.
    #[test]
    fn session_writes_are_last_write_wins(sessions in prop::collection::vec(session(12, 3), 1..4)) {
        let repo = repo_root();
        let store = Store::open(repo.path()).expect("open store");
        for session in &sessions {
            store.write_review(session).expect("write review");
        }

        let expected = sessions.last().expect("at least one session").clone();
        prop_assert_eq!(store.read_review().expect("read review"), expected);
    }
}
