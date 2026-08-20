//! Removal, and the legacy tree it has to tolerate.
//!
//! Split from [`super`] for the 400-line rule; the fixtures, strategies,
//! `Op` model and sequence oracles live there.
//! Split from [`super`] for the 400-line rule; the fixtures, strategies,
//! `Op` model and sequence oracles live there.

use std::fs;
use std::path::Path;

use rstest::rstest;
use rv_core::store::Store;

use super::*;
// ---------------------------------------------------------------------------
// removal, and the legacy tree it has to tolerate
// ---------------------------------------------------------------------------

/// Deleting is idempotent, and it reaches nothing but the one entry.
///
/// A `.review/` from an earlier version may still hold a `snapshots/`
/// directory — bytes `anchor.context` already carries, which nothing ever read
/// back (storage spec §11). The store no longer writes or deletes one, and the
/// point of this case is that it does not *reach* for one either: a removal
/// leaves whatever is in there exactly as it found it.
#[rstest]
#[case::a_legacy_directory_survives_beside_the_store(true)]
#[case::no_legacy_directory_exists(false)]
fn removing_a_comment_is_idempotent_and_leaves_the_legacy_tree_alone(#[case] plant: bool) {
    let repo = repo_root();
    let store = Store::open(repo.path()).expect("open store");
    store
        .append_comment(&fixed_comment("id0"))
        .expect("append id0");
    store
        .append_comment(&fixed_comment("id1"))
        .expect("append id1");
    if plant {
        let snapshots = repo.path().join(".review/snapshots");
        fs::create_dir_all(&snapshots).expect("create the legacy dir");
        fs::write(snapshots.join("id0"), "legacy").expect("plant id0's leftover");
        fs::write(snapshots.join("id1"), "legacy").expect("plant id1's leftover");
    }

    let removed = store.remove_comment("id0").expect("remove id0");

    assert!(removed, "the comment was in session.toml");
    assert_eq!(
        ids_of(&store.comments().expect("read comments")),
        vec!["id1".to_owned()],
        "the entry is gone and the other comment is untouched"
    );
    assert_eq!(
        repo.path().join(".review/snapshots/id0").exists(),
        plant,
        "a user's legacy files are not rv's to delete"
    );
    assert!(stray_temp_files(repo.path()).is_empty());
    assert!(
        !store
            .remove_comment("id0")
            .expect("a second removal of the same id must succeed too")
    );
}

/// A legacy orphan — a `snapshots/<id>` file naming no comment — is inert.
///
/// Earlier versions could strand one; the store must neither resurrect the
/// comment it belonged to nor adopt its bytes when the id is reused. The one
/// copy of a comment's context is `anchor.context` in `session.toml`.
#[test]
fn a_legacy_orphaned_snapshot_is_inert() {
    let repo = repo_root();
    let store = Store::open(repo.path()).expect("open store");
    store
        .append_comment(&fixed_comment("id1"))
        .expect("append id1");
    let snapshots = repo.path().join(".review/snapshots");
    fs::create_dir_all(&snapshots).expect("create the legacy dir");
    fs::write(snapshots.join("id0"), "the context id0 had").expect("plant the orphan");

    assert_eq!(
        ids_of(&store.comments().expect("read comments")),
        vec!["id1".to_owned()],
        "an orphaned snapshot must not resurrect the comment it belonged to"
    );
    assert!(
        !store
            .remove_comment("id0")
            .expect("removing the id the orphan names must succeed"),
        "there is no entry to remove"
    );

    // And the id can be reused without adopting the orphan's bytes.
    let mut newcomer = fixed_comment("id0");
    newcomer.anchor.context = vec!["fn b() {}".to_owned()];
    store.append_comment(&newcomer).expect("re-add the id");
    let stored = store.comments().expect("read comments");
    assert_eq!(
        stored.last().expect("the newcomer").anchor.context,
        vec!["fn b() {}".to_owned()],
        "the stored context is the newcomer's, not the orphan's"
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

/// The removal half of the module's crash-safety story, forced rather than
/// waited for: `remove_comment` rewrites `session.toml` through
/// `write_atomic`, so when that rewrite cannot happen nothing at all changes —
/// no half-deleted comment, and no `.review/` left in a state between the two.
///
/// `write_atomic` creates its temp file in the destination's own directory, so
/// a `.review/` this process may not write to is a rewrite that cannot even
/// start, while leaving the file itself readable. The whole-directory
/// comparison is what makes the claim total: it sees a change to any byte of
/// any file, not only to the one the test thought to name.
#[cfg(unix)]
#[test]
fn a_removal_whose_session_toml_cannot_be_written_deletes_nothing() {
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
        "the session.toml rewrite cannot have succeeded: {result:?}"
    );
    assert_eq!(
        dir_snapshot(&repo.path().join(".review")),
        before,
        "a removal that could not rewrite session.toml still changed .review/"
    );
    assert!(stray_temp_files(repo.path()).is_empty());
    assert_eq!(
        ids_of(&store.comments().expect("read comments")),
        vec!["id0".to_owned(), "id1".to_owned()],
        "and the comment it could not remove is still there"
    );
}
