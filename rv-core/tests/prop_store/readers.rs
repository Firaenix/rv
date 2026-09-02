//! What a reader holding a file open sees across a write.
//!
//! Split from [`super`] for the 400-line rule; the fixtures, strategies,
//! `Op` model and sequence oracles live there.
//! Split from [`super`] for the 400-line rule; the fixtures, strategies,
//! `Op` model and sequence oracles live there.

use std::fs;

use rstest::rstest;
use rv_core::store::Store;

use super::*;
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
    let store = Store::open(repo.path(), "main").expect("open store");
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

/// The same, for the file that matters most: `session.toml` is rewritten
/// wholesale on every single saved comment, so every append is a chance for a
/// reader to catch it mid-flight.
#[cfg(unix)]
#[test]
fn a_reader_holding_session_toml_open_never_sees_a_later_append() {
    use std::io::Read as _;

    let repo = repo_root();
    let store = Store::open(repo.path(), "main").expect("open store");
    store
        .append_comment(&fixed_comment("id0"))
        .expect("append the first comment");
    let path = repo.path().join(".review/reviews/main/session.toml");
    let original = fs::read_to_string(&path).expect("read session.toml");

    let mut held = fs::File::open(&path).expect("open session.toml before the append");
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
        "a reader that opened session.toml before the appends observed them"
    );
    assert_eq!(store.comments().expect("read comments").len(), 4);
}
