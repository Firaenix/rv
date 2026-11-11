//! `rv browse` and `--tree`: the subcommands reach the tree review it opens.

use super::support::*;

fn browsed() -> Fixture {
    let workspace = Fixture::new();
    workspace.write("a.rs", "fn a() {\n    let x = 1;\n}\n");
    workspace.write("src/b.rs", "fn b() {}\n");
    workspace.commit("first change");
    rv::session::browse(workspace.root(), None, None).expect("browse the tree");
    workspace
}

fn json(workspace: &Fixture, args: &[&str]) -> serde_json::Value {
    let output = workspace.rv(args);
    assert!(output.status.success(), "{}", streams(&output));
    serde_json::from_slice(&output.stdout).expect("valid json")
}

#[test]
fn a_tree_note_is_seen_by_comments_json_and_status_check() {
    let workspace = browsed();
    let saved = workspace.rv(&["--tree", "comment", "a.rs", "--line", "2", "-m", "why one?"]);
    assert!(saved.status.success(), "{}", streams(&saved));

    let comments = json(&workspace, &["--tree", "comments", "--json"]);
    let [comment] = comments.as_array().expect("an array").as_slice() else {
        panic!("expected the one note: {comments}");
    };
    assert_eq!(comment["body"], "why one?");
    assert_eq!(comment["anchor"]["line"], 2);
    assert_eq!(comment["anchor"]["side"], "right");

    let check = workspace.rv(&["status", "--tree", "--check"]);
    assert_eq!(check.status.code(), Some(1), "{}", streams(&check));
    let status = json(&workspace, &["--tree", "status", "--json"]);
    assert_eq!(status["revset"], "tree(@)", "{status}");
    assert_eq!(status["comments"]["open"], 1, "{status}");
}

#[test]
fn the_range_review_does_not_see_a_tree_note() {
    let workspace = browsed();
    let saved = workspace.rv(&[
        "--tree",
        "comment",
        "a.rs",
        "--line",
        "1",
        "-m",
        "tree only",
    ]);
    assert!(saved.status.success(), "{}", streams(&saved));

    let range = json(&workspace, &["comments", "--json"]);
    assert_eq!(range, serde_json::json!([]), "{range}");
    let check = workspace.rv(&["status", "--check"]);
    assert!(check.status.success(), "{}", streams(&check));
}

#[test]
fn a_tree_status_lists_every_file_unchanged() {
    let workspace = browsed();
    let status = json(&workspace, &["--tree", "status", "--json"]);
    let files: Vec<(&str, &str)> = status["files"]
        .as_array()
        .expect("files")
        .iter()
        .map(|file| {
            (
                file["path"].as_str().expect("path"),
                file["kind"].as_str().expect("kind"),
            )
        })
        .collect();
    assert_eq!(files, [("a.rs", "unchanged"), ("src/b.rs", "unchanged")]);
    assert_eq!(status["base"], status["head"], "{status}");
}

#[test]
fn a_tree_diff_is_the_whole_file_as_context() {
    let workspace = browsed();
    let diff = json(&workspace, &["--tree", "diff", "a.rs", "--json"]);
    let lines = diff[0]["lines"].as_array().expect("lines");
    assert_eq!(lines.len(), 3, "{diff}");
    assert!(
        lines
            .iter()
            .all(|line| line["kind"] == "context" && line["left"] == line["right"]),
        "{diff}"
    );
    assert_eq!(diff[0]["engine"], "whole-file", "{diff}");
}

#[test]
fn tree_and_from_cannot_be_combined() {
    let workspace = browsed();
    let output = workspace.rv(&["--tree", "--from", "@-", "status"]);
    assert!(!output.status.success(), "{}", streams(&output));
}

#[test]
fn browse_help_names_the_tree_flag() {
    let workspace = Fixture::new();
    let output = workspace.rv(&["browse", "--help"]);
    assert!(output.status.success(), "{}", streams(&output));
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("--tree"),
        "{}",
        streams(&output)
    );
}
