//! `session.toml` round trips, the exclude file, and isolation between the
//! files the store owns.
//!
//! Split from [`super`] for the 400-line rule; the fixtures, strategies,
//! `Op` model and sequence oracles live there.
//! Split from [`super`] for the 400-line rule; the fixtures, strategies,
//! `Op` model and sequence oracles live there.

use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

use rv_core::store::Store;

use super::*;
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
    /// each of the files it touches — `session.toml`, `REVIEW-FEEDBACK.md` and
    /// `.git/info/exclude` — holds exactly what *its own* operations wrote,
    /// and the tree contains nothing else.
    ///
    /// This is the isolation law stated positively, with an oracle computed
    /// from the op sequence rather than read back from disk. That distinction
    /// is the whole point: a property that snapshots the tree after the ops and
    /// compares it with itself bakes any mid-sequence damage into both sides,
    /// so `append_comment` clobbering `.git/info/exclude`, or `ensure_excluded`
    /// deleting `session.toml`, sails straight through it. Here every file's
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

        prop_assert_eq!(store.comments().expect("read comments"), expected_comments(&ops));
        prop_assert_eq!(fs::read(store.markdown_path()).ok(), expected_markdown(&ops));
        prop_assert_eq!(
            store.read_review().expect("read review"),
            expected_review(&ops).unwrap_or_default()
        );

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
        if wrote_the_review_file(&ops) {
            entitled.insert(PathBuf::from(".review/session.toml"));
        }
        if expected_markdown(&ops).is_some() {
            entitled.insert(PathBuf::from(".review/REVIEW-FEEDBACK.md"));
        }
        prop_assert_eq!(relative_files(repo.path()), entitled);
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
    /// `session.toml` from its own snapshot of the past and silently deletes
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
        prop_assert_eq!(
            first.read_review().expect("read review"),
            expected_review(&ops).unwrap_or_default()
        );
        prop_assert_eq!(
            second.read_review().expect("read review"),
            expected_review(&ops).unwrap_or_default()
        );
    }

    /// Opening a store is never destructive: `open` creates `.review/` and
    /// migrates a v1.0.0 `comments.json` if one is there, so re-opening over a
    /// store already in the current format must leave every byte under the
    /// repo root exactly as it was.
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

    /// The two files left are independent. `session.toml` is the review and
    /// `REVIEW-FEEDBACK.md` is a view of it, and neither write may reach the
    /// other: a save must not refresh the export (the export is produced on
    /// request and nothing reads it back), and rendering must not disturb the
    /// review it is rendering.
    ///
    /// The comment writes go *through* `session.toml` now, so the claim about
    /// them is about the array rather than about the file's bytes: an append
    /// and a removal leave the scope fields exactly as the last `write_review`
    /// set them.
    #[test]
    fn writing_one_file_never_disturbs_the_other(
        comments in distinct_comments(1..4),
        review in session(12, 2),
        document in hostile_text(32),
        extra in comment(Just("extra".to_owned()), 12),
    ) {
        let repo = repo_root();
        let store = Store::open(repo.path()).expect("open store");
        let mut review = review;
        review.comments = comments.clone();
        store.write_review(&review).expect("write review");
        store.write_markdown(&document).expect("write markdown");

        let session_path = repo.path().join(".review/session.toml");
        let markdown_path = store.markdown_path();
        let markdown_bytes = fs::read(&markdown_path).expect("read markdown");

        store.append_comment(&extra).expect("append extra comment");
        prop_assert_eq!(fs::read(&markdown_path).expect("read markdown"), markdown_bytes.clone(),
            "append_comment refreshed the export, which nothing reads back");
        let scope = store.read_review().expect("read review");
        prop_assert_eq!(&scope.revset, &review.revset, "append_comment rewrote the scope");
        prop_assert_eq!(&scope.changes, &review.changes, "append_comment rewrote the changes");
        prop_assert_eq!(scope.comments.len(), comments.len() + 1);

        let session_bytes = fs::read(&session_path).expect("read session.toml");
        store.write_markdown(&document).expect("rewrite markdown");
        prop_assert_eq!(fs::read(&session_path).expect("read session.toml"), session_bytes,
            "write_markdown touched the review it renders");

        prop_assert!(store.remove_comment(&extra.id).expect("remove the extra comment"));
        prop_assert_eq!(fs::read(&markdown_path).expect("read markdown"), markdown_bytes,
            "remove_comment refreshed the export");
        let scope = store.read_review().expect("read review");
        prop_assert_eq!(&scope.revset, &review.revset, "remove_comment rewrote the scope");
        prop_assert_eq!(scope.comments, comments);
        only_one_file(repo.path())?;
    }
}
