//! Parameterized case tables: where the markdown lands, the exclude file's
//! shapes, TOML-hostile session fields, and the comment-state wire format.
//!
//! Split from [`super`] for the 400-line rule; the fixtures, strategies,
//! `Op` model and sequence oracles live there.

use std::fs;

use rstest::rstest;
use rv_core::store::Comment;
use rv_core::store::CommentState;
use rv_core::store::Session;
use rv_core::store::Store;

use super::*;
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
        comments: vec![fixed_comment("id0")],
    };

    store.write_review(&session).expect("write review");

    let read_back = Store::open(repo.path())
        .expect("reopen store")
        .read_review()
        .expect("read review");
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
            context_start: 1,
        },
        body: "why".to_owned(),
        state,
        reply: None,
        settled_by: None,
    };

    store.append_comment(&comment).expect("append comment");

    let raw =
        fs::read_to_string(repo.path().join(".review/session.toml")).expect("read session.toml");
    assert!(
        raw.contains(&format!("state = \"{expected}\"")),
        "session.toml should spell {state:?} in kebab-case:\n{raw}"
    );
    // One `[[comments]]` table per comment, for a human poking around
    // `.review/`, per the module doc.
    assert!(
        raw.contains("[[comments]]"),
        "a comment should be its own array-of-tables entry:\n{raw}"
    );
    assert_eq!(
        store.comments().expect("read comments")[0].state,
        state,
        "state must read back as the same variant"
    );
}
