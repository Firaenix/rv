mod fixture;

use std::fs;

use fixture::Fixture;
use rv_core::vcs::Annotation;
use rv_core::vcs::Repository;

fn line_descriptions(annotation: &Annotation) -> Vec<&str> {
    (1..=annotation.line_count())
        .map(|line| {
            annotation
                .origin_of_line(line)
                .expect("every line has an origin")
                .description_first_line
                .as_str()
        })
        .collect()
}

#[test]
fn each_line_names_the_change_that_last_touched_it() {
    let workspace = Fixture::new();
    workspace.write("a.rs", "one\ntwo\nthree\nfour\n");
    workspace.commit("add a.rs\n\nwith a body that is not shown");
    workspace.write("a.rs", "one\nTWO\nthree\nfour\n");
    workspace.commit("shout two");
    workspace.write("a.rs", "one\nTWO\nthree\nFOUR\nfive\n");
    workspace.commit("shout four, add five");

    let repo = Repository::open(workspace.root()).expect("open workspace");
    let head = repo.resolve(Some("@-")).expect("resolve @-");
    let annotation = repo
        .annotate(&head, "a.rs")
        .expect("annotate a.rs")
        .expect("a.rs is text");

    assert_eq!(
        line_descriptions(&annotation),
        [
            "add a.rs",
            "shout two",
            "add a.rs",
            "shout four, add five",
            "shout four, add five",
        ]
    );
    assert_eq!(annotation.origins.len(), 3, "origins are de-duplicated");
    assert_eq!(annotation.origin_of_line(0), None);
    assert_eq!(annotation.origin_of_line(6), None);

    let first_line = annotation.origin_of_line(1).expect("line 1");
    let first_change = repo.change_of(&first_line.commit_id).expect("read change");
    assert_eq!(first_line.change_id, first_change.change_id);
    assert_eq!(first_line.author, "rv-test");
    assert!(first_line.timestamp_secs > 0, "{first_line:?}");
}

#[test]
fn a_file_added_in_the_last_change_is_all_that_change() {
    let workspace = Fixture::new();
    workspace.write("a.rs", "fn a() {}\n");
    workspace.commit("add a.rs");
    workspace.write("b.rs", "fn b() {}\nfn c() {}\n");
    workspace.commit("add b.rs");

    let repo = Repository::open(workspace.root()).expect("open workspace");
    let head = repo.resolve(Some("@-")).expect("resolve @-");
    let annotation = repo
        .annotate(&head, "b.rs")
        .expect("annotate b.rs")
        .expect("b.rs is text");

    assert_eq!(line_descriptions(&annotation), ["add b.rs", "add b.rs"]);
    assert_eq!(annotation.origins.len(), 1);
    assert_eq!(annotation.origins[0].commit_id, head);
}

#[test]
fn a_missing_or_binary_file_has_no_annotation() {
    let workspace = Fixture::new();
    fs::write(workspace.root().join("logo.bin"), [0, 159, 146, 150]).expect("write logo.bin");
    workspace.commit("add a logo");

    let repo = Repository::open(workspace.root()).expect("open workspace");
    let head = repo.resolve(Some("@-")).expect("resolve @-");
    assert_eq!(repo.annotate(&head, "absent.rs").expect("annotate"), None);
    assert_eq!(repo.annotate(&head, "logo.bin").expect("annotate"), None);
}

#[test]
fn an_empty_file_has_no_lines() {
    let workspace = Fixture::new();
    workspace.write("empty.rs", "");
    workspace.commit("add an empty file");

    let repo = Repository::open(workspace.root()).expect("open workspace");
    let head = repo.resolve(Some("@-")).expect("resolve @-");
    let annotation = repo
        .annotate(&head, "empty.rs")
        .expect("annotate")
        .expect("an empty file is text");
    assert_eq!(annotation.line_count(), 0);
}
