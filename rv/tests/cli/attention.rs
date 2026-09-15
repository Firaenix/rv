//! The attention channel beside the comment one: flags and reviewed ticks.

use super::support::*;

#[test]
fn rv_flag_points_at_a_line_lists_it_and_never_gates_check() {
    let workspace = Fixture::new();
    workspace.write("a.rs", "fn a() {\n    let x = 1;\n}\n");
    workspace.commit("first change");

    let output = workspace.rv(&["flag", "a.rs", "--line", "2", "-m", "start here"]);
    assert!(output.status.success(), "{}", streams(&output));

    let listed = workspace.rv(&["flags"]);
    let said = String::from_utf8_lossy(&listed.stdout);
    assert!(
        said.contains("open") && said.contains("a.rs:2") && said.contains("start here"),
        "the listing does not describe the flag: {said}"
    );

    // A flag asks for a look, not a change: the worker's poll stays green.
    let check = workspace.rv(&["status", "--check"]);
    assert!(check.status.success(), "a flag gated --check");

    let flags = workspace.store().flags().expect("read the flags");
    let id = flags[0].id.clone();
    let acked = workspace.rv(&["ack", &id]);
    assert!(acked.status.success(), "{}", streams(&acked));
    assert!(workspace.store().flags().expect("read")[0].acknowledged);

    let json = workspace.rv(&["flags", "--json", "--open"]);
    let said = String::from_utf8_lossy(&json.stdout);
    assert_eq!(
        said.trim(),
        "[]",
        "an acknowledged flag is still open: {said}"
    );

    let removed = workspace.rv(&["unflag", &id]);
    assert!(removed.status.success(), "{}", streams(&removed));
    assert!(workspace.store().flags().expect("read").is_empty());
}

#[test]
fn rv_review_ticks_a_file_and_reports_when_it_changes() {
    let workspace = Fixture::new();
    workspace.write("a.rs", "fn a() {\n    let x = 1;\n}\n");
    workspace.commit("first change");

    let output = workspace.rv(&["review", "a.rs"]);
    assert!(output.status.success(), "{}", streams(&output));

    let status = workspace.rv(&["status"]);
    let said = String::from_utf8_lossy(&status.stdout);
    assert!(said.contains("reviewed  1 of 1 files"), "{said}");

    let listed = workspace.rv(&["reviewed"]);
    let said = String::from_utf8_lossy(&listed.stdout);
    assert!(said.contains("a.rs") && !said.contains("changed"), "{said}");

    workspace.write("a.rs", "fn a() {\n    let x = 2;\n}\n");
    workspace.commit("second change");
    let listed = workspace.rv(&["reviewed", "--json"]);
    let said = String::from_utf8_lossy(&listed.stdout);
    assert!(said.contains("\"changed\": true"), "{said}");

    let cleared = workspace.rv(&["unreview", "a.rs"]);
    assert!(cleared.status.success(), "{}", streams(&cleared));
    let again = workspace.rv(&["unreview", "a.rs"]);
    assert!(!again.status.success(), "clearing a missing tick succeeded");
}
