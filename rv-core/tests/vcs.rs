mod fixture;

use std::fs;

use fixture::Fixture;
use rv_core::model::ChangeKind;
use rv_core::vcs::Error;
use rv_core::vcs::Repository;

#[test]
fn stack_lists_changes_newest_first_with_reverse_hex_ids() {
    let workspace = Fixture::new();
    workspace.write("a.rs", "fn a() {}\n");
    workspace.commit("first change");
    workspace.write("b.rs", "fn b() {}\n");
    workspace.commit("second change");

    let repo = Repository::open(workspace.root()).expect("open workspace");
    let stack = repo.stack(None, None).expect("enumerate stack");

    let descriptions: Vec<&str> = stack.iter().map(|c| c.description.as_str()).collect();
    assert_eq!(descriptions, ["", "second change", "first change"]);

    for change in &stack {
        assert!(
            !change.change_id.is_empty()
                && change.change_id.chars().all(|c| ('k'..='z').contains(&c)),
            "change id {} is not reverse hex (k-z)",
            change.change_id
        );
        assert!(
            !change.commit_id.is_empty() && change.commit_id.chars().all(|c| c.is_ascii_hexdigit()),
            "commit id {} is not plain hex",
            change.commit_id
        );
    }
}

#[test]
fn empty_range_is_an_error_naming_endpoints() {
    let workspace = Fixture::new();
    workspace.write("a.rs", "fn a() {}\n");
    workspace.commit("first change");

    let repo = Repository::open(workspace.root()).expect("open workspace");
    let error = repo
        .stack(Some("@"), Some("@"))
        .expect_err("@..@ contains no changes");

    let message = error.to_string();
    assert!(message.contains("empty"), "{message}");
    assert!(message.contains("@..@"), "{message}");
    assert!(matches!(error, Error::EmptyRange { .. }), "{error:?}");
}

#[test]
fn files_reports_added_paths_and_reads_blobs() {
    let workspace = Fixture::new();
    workspace.write("a.rs", "fn a() {}\n");
    workspace.commit("first change");
    // Rename and extend: against the default base — `trunk()`, which degrades to
    // `root()` here — a.rs never existed, so the head-side b.rs is simply added.
    fs::rename(workspace.root().join("a.rs"), workspace.root().join("b.rs")).expect("rename a.rs");
    workspace.write("b.rs", "fn a() {}\nfn b() {}\n");
    workspace.commit("rename and extend");

    let repo = Repository::open(workspace.root()).expect("open workspace");
    let (base, head) = repo.endpoints(None, None).expect("resolve endpoints");
    let files = repo.files(&base, &head).expect("enumerate files");

    assert_eq!(files.len(), 1, "{files:?}");
    assert_eq!(files[0].path, "b.rs");
    assert_eq!(files[0].kind, ChangeKind::Added);
    assert_eq!(files[0].source_path, None);
    assert!(!files[0].binary, "{:?}", files[0]);

    assert_eq!(
        repo.read_blob(&head, "b.rs").expect("read b.rs at head"),
        Some(b"fn a() {}\nfn b() {}\n".to_vec())
    );
    // Absence is not an error: the file is simply not in the base tree.
    assert_eq!(
        repo.read_blob(&base, "b.rs").expect("read b.rs at base"),
        None
    );
}

#[test]
fn rename_between_two_changes_is_reported_with_its_source() {
    let workspace = Fixture::new();
    workspace.write("a.rs", "fn a() {}\n");
    workspace.commit("first change");

    // Pin the change that holds a.rs as the base before the rename exists. The
    // handle is loaded at one operation, so a fresh one is needed afterwards.
    let base = {
        let repo = Repository::open(workspace.root()).expect("open workspace");
        repo.endpoints(None, Some("@-")).expect("resolve @-").1
    };

    fs::rename(workspace.root().join("a.rs"), workspace.root().join("b.rs")).expect("rename a.rs");
    workspace.commit("rename a.rs to b.rs");

    let repo = Repository::open(workspace.root()).expect("reopen workspace");
    let (_, head) = repo.endpoints(None, None).expect("resolve endpoints");
    let files = repo.files(&base, &head).expect("enumerate files");

    assert_eq!(files.len(), 1, "{files:?}");
    assert_eq!(files[0].path, "b.rs");
    assert_eq!(files[0].kind, ChangeKind::Renamed);
    assert_eq!(files[0].source_path.as_deref(), Some("a.rs"));
    // The base-side content is reachable under the base-side path.
    assert_eq!(
        repo.read_blob(&base, "a.rs").expect("read a.rs at base"),
        Some(b"fn a() {}\n".to_vec())
    );
}

#[test]
fn binary_files_are_flagged_not_decoded() {
    let workspace = Fixture::new();
    // A leading NUL byte, then bytes that are not valid UTF-8 on their own.
    fs::write(workspace.root().join("logo.bin"), [0, 159, 146, 150]).expect("write logo.bin");
    workspace.write("a.rs", "fn a() {}\n");
    workspace.commit("add a logo");

    let repo = Repository::open(workspace.root()).expect("open workspace");
    let (base, head) = repo.endpoints(None, None).expect("resolve endpoints");
    let files = repo.files(&base, &head).expect("enumerate files");

    let paths: Vec<&str> = files.iter().map(|file| file.path.as_str()).collect();
    assert_eq!(paths, ["a.rs", "logo.bin"], "expected paths sorted");
    assert!(!files[0].binary, "a.rs is text: {:?}", files[0]);
    assert!(files[1].binary, "logo.bin is binary: {:?}", files[1]);

    // Flagged, not decoded: the bytes come back exactly as written.
    assert_eq!(
        repo.read_blob(&head, "logo.bin").expect("read logo.bin"),
        Some(vec![0, 159, 146, 150])
    );
}

/// `.review/` is a *directory*, and a tree lookup for one comes back as a
/// `Tree` entry rather than a `File` — the check that answers "is this
/// tracked?" must not filter to files the way `read_blob` does, or a committed
/// `.review/` would report clean forever.
#[test]
fn tracks_sees_a_committed_directory_and_not_an_absent_one() {
    let workspace = Fixture::new();
    workspace.write(".review/comments.json", "[]\n");
    workspace.write("a.rs", "fn a() {}\n");
    workspace.commit("commit the review notes by mistake");

    let repo = Repository::open(workspace.root()).expect("open workspace");

    assert!(
        repo.tracks(".review").expect("look up .review"),
        "the directory itself is tracked"
    );
    assert!(
        repo.tracks(".review/comments.json")
            .expect("look up the comments file"),
        "so is the file under it"
    );
    assert!(
        !repo.tracks(".notes").expect("look up an absent path"),
        "a path no commit carries is untracked"
    );
}

#[test]
fn tracks_reports_an_excluded_directory_as_untracked() {
    let workspace = Fixture::new();
    workspace.write("a.rs", "fn a() {}\n");
    workspace.commit("first change");
    // Written after the commit and excluded the way `Store::ensure_excluded`
    // does it, so jj's next snapshot leaves it out of the working-copy commit.
    fs::write(workspace.root().join(".git/info/exclude"), "/.review/\n")
        .expect("write the exclude file");
    workspace.write(".review/comments.json", "[]\n");
    workspace.jj(&["status"]);

    let repo = Repository::open(workspace.root()).expect("open workspace");
    assert!(
        !repo.tracks(".review").expect("look up .review"),
        "an excluded directory never reaches the tree"
    );
}
