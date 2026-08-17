mod fixture;

use fixture::Fixture;
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
            !change.commit_id.is_empty()
                && change.commit_id.chars().all(|c| c.is_ascii_hexdigit()),
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
