//! Corruption is reported, never papered over.
//!
//! Split from [`super`] for the 400-line rule; the fixtures, strategies,
//! `Op` model and sequence oracles live there.
//! Split from [`super`] for the 400-line rule; the fixtures, strategies,
//! `Op` model and sequence oracles live there.

use std::fs;
use std::path::Path;

use proptest::prelude::*;
use rv_core::store::Comment;
use rv_core::store::Store;

use super::*;
// ---------------------------------------------------------------------------
// corruption is reported, never papered over
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(config(24))]

    /// `comments()` returns an empty vector only for a *missing* file. A
    /// `session.toml` truncated anywhere short of its end — the shape a torn
    /// write or a truncated copy would leave — must be reported as an error
    /// whenever the truncation actually breaks the syntax, never silently read
    /// as "no comments", which would quietly discard a reviewer's work.
    ///
    /// TOML, unlike a JSON array, has valid *prefixes*: cutting a file after a
    /// complete `[[comments]]` table leaves a parseable document holding fewer
    /// comments. That case is not silent emptiness and is not a lie about the
    /// bytes on disk, so the claim is the one that matters — a truncation is
    /// either reported or reads as a strict prefix of the review, and never as
    /// nothing at all while a populated file sits there.
    #[test]
    fn a_truncated_session_toml_is_never_read_as_silent_emptiness(
        sequence in distinct_comments(1..4),
        cut in any::<prop::sample::Index>(),
    ) {
        let repo = repo_root();
        let store = Store::open(repo.path()).expect("open store");
        for comment in &sequence {
            store.append_comment(comment).expect("append comment");
        }
        let expected = upsert_reduce(&sequence);

        let path = repo.path().join(".review/session.toml");
        let good = fs::read_to_string(&path).expect("read session.toml");
        let chars: Vec<char> = good.chars().collect();
        // `Index::index(len)` is in `0..len`, so this is always a strict prefix.
        let keep = cut.index(chars.len());
        let truncated: String = chars[..keep].iter().collect();
        fs::write(&path, &truncated).expect("write truncated session.toml");

        match store.comments() {
            // Which error matters: a caller telling "corrupt" from
            // "unreadable" needs the parse failure to arrive as
            // InvalidSession, not as Io.
            Err(error) => prop_assert!(
                matches!(error, rv_core::store::Error::InvalidSession { .. }),
                "a parse failure must be reported as InvalidSession, got {error:?}"
            ),
            // A truncation that still parses is a prefix of the review's
            // *comments*, never a shorter list with the wrong ones or an
            // empty one. The comments themselves may differ from what was
            // stored: cutting inside a `[[comments]]` table can drop a
            // `#[serde(default)]` field and leave the rest readable, so the
            // last entry comes back with a defaulted `settled_by` or
            // `context_start`. That is a partially-read comment, which is
            // visible; it is not a comment silently gone.
            Ok(read) => prop_assert!(
                read.len() <= expected.len()
                    && read
                        .iter()
                        .zip(&expected)
                        .all(|(read, stored)| read.id == stored.id),
                "a parseable truncation must be a prefix of the review, got {:?} of {:?} \
                 (kept {keep} of {} chars)",
                ids_of(&read),
                ids_of(&expected),
                chars.len()
            ),
        }
    }

    /// The complementary half: a *missing* `session.toml` is not an error —
    /// a `.review/` no command has recorded yet has nothing to read — and the
    /// module's documented crash residue is genuinely harmless.
    ///
    /// A crash between `write_atomic`'s fsync and its rename strands a
    /// `.rv-store-*.tmp` sibling, and a `.review/` copied from an earlier
    /// version may still hold a `snapshots/` directory current versions never
    /// write. The doc calls both harmless; this generates exactly that tree
    /// and holds it to that word: `comments()` is empty (not an error, and not
    /// the orphans resurrected), and a subsequent append works normally —
    /// including when it reuses an orphan's id, whose file the store neither
    /// adopts nor rewrites, because nothing reads it.
    #[test]
    fn crash_residue_is_harmless_and_a_missing_session_toml_reads_as_empty(
        document in hostile_text(24),
        orphans in prop::collection::vec((prop::sample::select(id_pool(5)), hostile_text(16)), 1..4),
        stray_temp in any::<bool>(),
        reuse in any::<bool>(),
        newcomer in comment(prop::sample::select(id_pool(5)), 12),
    ) {
        let repo = repo_root();
        let store = Store::open(repo.path()).expect("open store");
        store.write_markdown(&document).expect("write markdown");

        // The tree an earlier version's crash left behind: legacy orphans in a
        // directory current versions never create.
        let snapshots = repo.path().join(".review/snapshots");
        fs::create_dir_all(&snapshots).expect("create the legacy dir");
        for (id, body) in &orphans {
            fs::write(snapshots.join(id), body).expect("plant an orphaned snapshot");
        }
        if stray_temp {
            fs::write(
                repo.path().join(".review").join(format!("{TEMP_PREFIX}orphan.tmp")),
                b"half a session.toml",
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

        // The one copy of the context is the stored anchor's: the legacy orphan
        // is neither adopted nor replaced, because nothing reads it.
        if reuse {
            let orphan = fs::read_to_string(snapshots.join(&newcomer.id))
                .expect("the legacy orphan is still on disk");
            // The generator may plant one id twice; the file holds the last body.
            let planted = orphans
                .iter()
                .rev()
                .find(|(id, _)| *id == newcomer.id)
                .map(|(_, body)| body.clone())
                .expect("the reused id was planted");
            prop_assert_eq!(orphan, planted, "the orphan's bytes are not rv's to rewrite");
        }
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
fn a_session_toml_that_cannot_be_read_is_an_error_not_silent_emptiness() {
    let repo = repo_root();
    let store = Store::open(repo.path()).expect("open store");
    let path = repo.path().join(".review/session.toml");
    fs::create_dir(&path).expect("plant a directory at session.toml");

    let result = store.comments();

    assert!(
        result.is_err(),
        "an unreadable session.toml read as {:?} instead of failing",
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
/// `session.toml` that is a perfectly good file the process simply may not
/// open — a review directory copied between accounts, a restrictive umask, a
/// sandbox. Distinct from the directory case because it is an `EACCES` at
/// `open` rather than an `EISDIR` at `read`, and a handler that special-cases
/// one kind may still swallow the other.
#[cfg(unix)]
#[test]
fn a_session_toml_the_process_may_not_open_is_an_error_not_silent_emptiness() {
    use std::os::unix::fs::PermissionsExt;

    let repo = repo_root();
    let store = Store::open(repo.path()).expect("open store");
    store
        .append_comment(&fixed_comment("id0"))
        .expect("append comment");
    let path = repo.path().join(".review/session.toml");

    if !permission_bits_bite(repo.path()) {
        // Running as root, or on a filesystem that ignores the mode bits.
        return;
    }
    fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).expect("chmod session.toml");

    let result = store.comments();

    // Restore before asserting, so a failure still leaves a removable tempdir.
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).expect("restore session.toml");
    assert!(
        result.is_err(),
        "an unreadable session.toml read as {:?} instead of failing — \
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
