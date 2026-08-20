//! Upsert semantics, conservation and ordering of the comment array.
//!
//! Split from [`super`] for the 400-line rule; the fixtures, strategies,
//! `Op` model and sequence oracles live there.
//! Split from [`super`] for the 400-line rule; the fixtures, strategies,
//! `Op` model and sequence oracles live there.

use std::fs;

use proptest::prelude::*;
use rv_core::store::Comment;
use rv_core::store::CommentState;
use rv_core::store::Store;

use super::*;
// ---------------------------------------------------------------------------
// session.toml's comments: upsert semantics, conservation, ordering
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
    /// reopened handle cannot make: a `write_review` that buffered its bytes
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
            prop_assert_eq!(
                store.read_review().expect("read review"),
                expected_review(&ops).unwrap_or_default()
            );
        }

        let reopened = Store::open(repo.path()).expect("reopen store");
        prop_assert_eq!(reopened.comments().expect("read comments"), expected_comments(&ops));
        prop_assert_eq!(fs::read(reopened.markdown_path()).ok(), expected_markdown(&ops));
        prop_assert_eq!(
            reopened.read_review().expect("read review"),
            expected_review(&ops).unwrap_or_default()
        );

        let ran_ensure_excluded = ops.iter().any(|op| matches!(op, Op::EnsureExcluded));
        prop_assert_eq!(
            reopened.ensure_excluded().expect("ensure_excluded on the fresh handle"),
            !ran_ensure_excluded,
            "a fresh handle must see the exclusion the sequence did (or did not) record"
        );
    }

    /// Hostile text survives `session.toml` byte-identically. TOML has to
    /// carry quotes, backslashes, raw newlines, NUL and other C0 controls,
    /// astral-plane scalars and a BOM through the `toml` crate and back
    /// without normalizing, trimming or re-encoding anything.
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
                context_start: 1,
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
