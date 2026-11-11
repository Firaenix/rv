//! `rv brief`: the handoff bundle, for a range review and a tree review.

use super::support::*;

fn numbered(count: u32) -> String {
    (1..=count).map(|n| format!("line {n}\n")).collect()
}

fn succeed(workspace: &Fixture, args: &[&str]) -> String {
    let output = workspace.rv(args);
    assert!(output.status.success(), "{}", streams(&output));
    String::from_utf8(output.stdout).expect("utf-8 stdout")
}

fn json(workspace: &Fixture, args: &[&str]) -> serde_json::Value {
    serde_json::from_str(&succeed(workspace, args)).expect("valid json")
}

fn only_note(brief: &serde_json::Value) -> &serde_json::Value {
    let [note] = brief["notes"].as_array().expect("notes").as_slice() else {
        panic!("expected one note: {brief}");
    };
    note
}

#[test]
fn excerpt_after_an_edit_above_the_anchor_shows_the_new_lines_at_the_moved_line() {
    let workspace = Fixture::new();
    workspace.write("a.txt", &numbered(10));
    workspace.commit("ten lines");
    succeed(
        &workspace,
        &["comment", "a.txt", "--line", "5", "-m", "why five?"],
    );

    workspace.write("a.txt", &format!("added 1\nadded 2\n{}", numbered(10)));
    workspace.commit("two lines above");

    let brief = json(&workspace, &["brief", "--json", "--context", "1"]);
    let note = only_note(&brief);
    assert_eq!(note["line"], 7, "{brief}");
    assert_eq!(note["confidence"], "moved", "{brief}");
    assert_eq!(note["excerpt"]["source"], "head", "{brief}");
    assert_eq!(note["excerpt"]["start_line"], 6, "{brief}");
    assert_eq!(
        note["excerpt"]["lines"],
        serde_json::json!(["line 4", "line 5", "line 6"]),
        "{brief}"
    );

    let document = succeed(&workspace, &["brief", "--context", "1"]);
    assert!(
        document.contains("### `a.txt:7`")
            && document.contains("Lines 6–8 at head; the note is on line 7.")
            && document.contains("  line 4\n  line 5\n  line 6\n"),
        "{document}"
    );
}

#[test]
fn brief_json_has_session_notes_and_protocol() {
    let workspace = Fixture::new();
    workspace.write("a.rs", "fn a() {\n    let x = 1;\n}\n");
    workspace.commit("first change");
    succeed(
        &workspace,
        &["comment", "a.rs", "--line", "2", "-m", "name it"],
    );
    let id = workspace.store().comments().expect("comments")[0]
        .id
        .clone();
    succeed(&workspace, &["reply", &id, "-m", "renamed"]);

    let brief = json(&workspace, &["brief", "--json"]);
    let session = &brief["session"];
    assert_eq!(session["kind"], "range", "{brief}");
    assert_eq!(session["revset"], "trunk()..@", "{brief}");
    for key in ["base", "head", "workspace", "started_at", "changes"] {
        assert!(!session[key].is_null(), "session lacks {key}: {brief}");
    }
    let note = only_note(&brief);
    assert_eq!(note["id"], id.as_str());
    assert_eq!(note["file"], "a.rs");
    assert_eq!(note["line"], 2);
    assert_eq!(note["side"], "right");
    assert_eq!(note["confidence"], "exact");
    assert_eq!(note["state"], "open");
    assert_eq!(note["body"], "name it");
    assert_eq!(note["replies"], serde_json::json!(["renamed"]));
    assert_eq!(note["excerpt"]["start_line"], 1);
    assert_eq!(note["excerpt"]["lines"].as_array().map(Vec::len), Some(3));
    assert_eq!(brief["protocol"]["reply"], "rv reply <id> -m <answer>");
    assert_eq!(brief["protocol"]["resolve"], "rv resolve <id>");
}

#[test]
fn an_outdated_anchor_is_reported_with_its_stored_context() {
    let workspace = Fixture::new();
    workspace.write("a.rs", "fn a() {\n    let x = 1;\n}\n");
    workspace.commit("first change");
    succeed(
        &workspace,
        &["comment", "a.rs", "--line", "2", "-m", "why one?"],
    );

    workspace.write("a.rs", "fn a() {}\n");
    workspace.commit("the commented line is gone");

    let brief = json(&workspace, &["brief", "--json"]);
    let note = only_note(&brief);
    assert_eq!(note["confidence"], "outdated", "{brief}");
    assert_eq!(note["state"], "outdated", "{brief}");
    assert_eq!(note["excerpt"]["source"], "stored", "{brief}");
    assert_eq!(
        note["excerpt"]["lines"],
        serde_json::json!(["fn a() {", "    let x = 1;", "}"]),
        "{brief}"
    );

    let document = succeed(&workspace, &["brief"]);
    assert!(
        document.contains("Outdated: the code it was written against is gone")
            && !document.contains("  fn a() {}\n"),
        "{document}"
    );
}

#[test]
fn settled_notes_are_left_out_of_the_brief() {
    let workspace = Fixture::new();
    workspace.write("a.rs", "fn a() {}\n");
    workspace.commit("first change");
    succeed(
        &workspace,
        &["comment", "a.rs", "--line", "1", "-m", "done"],
    );
    let id = workspace.store().comments().expect("comments")[0]
        .id
        .clone();
    succeed(&workspace, &["resolve", &id]);

    let brief = json(&workspace, &["brief", "--json"]);
    assert_eq!(brief["notes"], serde_json::json!([]), "{brief}");
    assert!(succeed(&workspace, &["brief"]).contains("No note is waiting on an answer."));
}

#[test]
fn a_tree_brief_names_the_tree_flag_in_its_commands() {
    let workspace = Fixture::new();
    workspace.write("src/b.rs", "fn b() {}\n");
    workspace.commit("first change");
    rv::session::browse(workspace.root(), None, None).expect("browse the tree");
    succeed(
        &workspace,
        &[
            "--tree",
            "comment",
            "src/b.rs",
            "--line",
            "1",
            "-m",
            "tree note",
        ],
    );

    let brief = json(&workspace, &["--tree", "brief", "--json"]);
    assert_eq!(brief["session"]["kind"], "tree", "{brief}");
    assert_eq!(only_note(&brief)["body"], "tree note", "{brief}");
    assert_eq!(
        brief["protocol"]["reply"],
        "rv --tree reply <id> -m <answer>"
    );

    let document = succeed(&workspace, &["brief", "--tree"]);
    assert!(
        document.contains("Tree review of every file")
            && document.contains("`rv --tree resolve <id>`"),
        "{document}"
    );
    assert!(
        json(&workspace, &["brief", "--json"])["notes"]
            .as_array()
            .is_some_and(Vec::is_empty),
        "the range brief must not carry the tree note"
    );
}

#[test]
fn brief_out_writes_the_file_and_leaves_the_review_untouched() {
    let workspace = Fixture::new();
    workspace.write("a.rs", "fn a() {}\n");
    workspace.commit("first change");
    succeed(
        &workspace,
        &["comment", "a.rs", "--line", "1", "-m", "look"],
    );
    let before = tree(&workspace.root().join(".review"));

    let said = succeed(&workspace, &["brief", "--out", "brief.md"]);
    assert!(said.contains("wrote brief.md"), "{said}");
    let written = std::fs::read_to_string(workspace.root().join("brief.md")).expect("brief.md");
    assert!(
        written.starts_with("# Review brief: `trunk()..@`") && written.contains("## How to answer"),
        "{written}"
    );
    assert_eq!(
        before,
        tree(&workspace.root().join(".review")),
        "a brief rewrote .review/"
    );
}
