//! `rv status`: the range it reports, what it derives, and what it refuses to write.

use super::support::*;

#[test]
fn status_json_reports_range_and_zero_comments() {
    let workspace = Fixture::new();
    workspace.write("a.rs", "fn a() {}\n");
    workspace.commit("first change");
    workspace.write("b.rs", "fn b() {}\n");
    // `describe` without a following `jj new`, so the working copy *is* the
    // second change and the stack is exactly two changes deep.
    workspace.jj(&["describe", "-m", "second change"]);

    let output = workspace.rv(&["status", "--json"]);
    assert!(output.status.success(), "{}", streams(&output));

    let report: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("status --json emits json");

    assert_eq!(report["revset"], "trunk()..@", "{}", streams(&output));
    assert_eq!(report["comments"]["open"], 0, "{}", streams(&output));
    assert_eq!(report["comments"]["resolved"], 0, "{}", streams(&output));

    let changes = report["changes"].as_array().expect("changes is an array");
    assert_eq!(changes.len(), 2, "{}", streams(&output));
    let descriptions: Vec<&str> = changes
        .iter()
        .map(|change| change["description"].as_str().expect("description string"))
        .collect();
    assert_eq!(descriptions, ["second change", "first change"]);

    let files = report["files"].as_array().expect("files is an array");
    let paths: Vec<&str> = files
        .iter()
        .map(|file| file["path"].as_str().expect("path string"))
        .collect();
    assert!(paths.contains(&"a.rs"), "{paths:?}");
    assert!(
        files
            .iter()
            .all(|file| file["binary"] == false && file["kind"].is_string()),
        "{files:?}"
    );

    for endpoint in ["base", "head"] {
        let commit = report[endpoint].as_str().expect("endpoint is a string");
        assert!(
            !commit.is_empty() && commit.chars().all(|char| char.is_ascii_hexdigit()),
            "{endpoint} {commit} is not a hex commit id"
        );
    }
}

#[test]
fn empty_range_fails_naming_endpoints() {
    let workspace = Fixture::new();
    workspace.write("a.rs", "fn a() {}\n");
    workspace.commit("first change");

    let output = workspace.rv(&["--from", "@", "--to", "@", "status"]);
    assert!(!output.status.success(), "{}", streams(&output));

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("empty"), "{}", streams(&output));
    assert!(stderr.contains("@..@"), "{}", streams(&output));
}

/// `rv status` derives `outdated` like every other load, so the command and the
/// TUI never disagree about the same review.
///
/// This reported `1 open, 0 outdated` for a comment about a line that no longer
/// existed, which is the number a script would have acted on. On this repository
/// it claimed twenty-two open comments where fourteen were stale.
#[test]
fn status_reports_a_stale_comment_as_outdated() {
    let workspace = Fixture::new();
    workspace.write("a.rs", "fn a() {\n    let x = 1;\n}\n");
    workspace.commit("first change");

    // A comment written against a line, by hand — the TUI is not what is under
    // test here — and then the line removed under it. The hash no longer
    // matches anything, and the file is short of the anchor's number, so
    // neither the content tier nor the line-number fallback can place it.
    let head = workspace.rv(&["status", "--json"]).stdout.clone();
    let head: serde_json::Value =
        serde_json::from_slice(&head).expect("status --json is valid json");
    let head_commit = head["head"].as_str().expect("a head commit");
    let comment = serde_json::json!([{
        "id": "deadbee1",
        "change_id": "z".repeat(32),
        "commit_id": head_commit,
        "anchor": {
            "file": "a.rs",
            "side": "Right",
            "line": 2,
            "content_hash": "0".repeat(64),
            "context": ["    let x = 1;"],
        },
        "body": "about a line that is about to change",
        "state": "open",
        "reply": null,
    }]);
    // Written as v1.0.0 wrote it: the next `rv` run absorbs it into
    // `session.toml` on the way, which is the migration doing its job here
    // rather than being the point of the case.
    std::fs::write(
        workspace.root().join(".review/comments.json"),
        serde_json::to_vec_pretty(&comment).expect("serialize"),
    )
    .expect("write the v1.0.0 comments.json");

    // And the line removed from it, so the number fallback cannot hold the
    // anchor either: the file no longer has a line 2.
    workspace.write("a.rs", "fn a() {}\n");
    workspace.commit("the commented line is gone");

    let output = workspace.rv(&["status", "--json"]);
    let report: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("status --json is valid json");
    assert_eq!(
        report["comments"]["outdated"],
        1,
        "a comment whose hash cannot be found is not reported outdated: {}",
        streams(&output)
    );
    assert_eq!(
        report["comments"]["open"],
        0,
        "and it is not also counted as open: {}",
        streams(&output)
    );
}

/// `rv status` is a query: it reports the range and rewrites nothing.
///
/// It used to go through the same writer the TUI does, so a command that reads
/// like a pure question rewrote `session.toml` and its `started_at` on every run
/// — moving the timestamp in the header of an already-rendered export.
#[test]
fn status_writes_nothing() {
    let workspace = Fixture::new();
    // A review opened first, so there is a session record to leave alone.
    rv::session::build(workspace.root(), None, None).expect("open the review");
    workspace.rv(&["render"]);
    let before = tree(workspace.root());

    let output = workspace.rv(&["status", "--json"]);
    assert!(output.status.success(), "{}", streams(&output));

    assert_eq!(
        tree(workspace.root()),
        before,
        "`rv status` touched the workspace"
    );
}

/// A `.review/` committed before `Store::ensure_excluded` ever ran — or forced
/// in since — makes every comment the reviewer writes part of the change under
/// review. The exclude file cannot undo that, so `rv status` says so.
#[test]
fn status_warns_when_the_review_directory_is_tracked() {
    let workspace = Fixture::new();
    workspace.write("a.rs", "fn a() {}\n");
    // Snapshotted into the working-copy commit before rv has run once, which is
    // exactly how a real `.review/` gets committed.
    workspace.write(".review/comments.json", "[]\n");
    workspace.jj(&["describe", "-m", "first change"]);

    let output = workspace.rv(&["status", "--json"]);
    assert!(output.status.success(), "{}", streams(&output));
    let report: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("status --json is valid json");
    assert_eq!(
        report["review_tracked"],
        true,
        "a script cannot tell the review state is in the change: {}",
        streams(&output)
    );

    let text = workspace.rv(&["status"]);
    let stdout = String::from_utf8_lossy(&text.stdout);
    assert!(
        stdout.contains("warning") && stdout.contains(".review/ is tracked"),
        "the text form said nothing: {}",
        streams(&text)
    );

    // The gate stays a question about open comments: there are none, so a CI
    // run over this workspace still passes.
    let check = workspace.rv(&["status", "--check"]);
    assert!(
        check.status.success(),
        "a tracked .review/ started failing --check: {}",
        streams(&check)
    );
}

#[test]
fn status_is_silent_about_an_excluded_review_directory() {
    let workspace = Fixture::new();
    workspace.write("a.rs", "fn a() {}\n");
    workspace.jj(&["describe", "-m", "first change"]);
    // rv's own first run creates `.review/` and excludes it, so jj never
    // snapshots it.
    workspace.rv(&["render"]);

    let output = workspace.rv(&["status", "--json"]);
    let report: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("status --json is valid json");
    assert_eq!(
        report["review_tracked"],
        false,
        "an excluded .review/ was reported as tracked: {}",
        streams(&output)
    );

    let text = workspace.rv(&["status"]);
    let stdout = String::from_utf8_lossy(&text.stdout);
    assert!(
        !stdout.contains(".review/ is tracked"),
        "a clean workspace was warned at: {}",
        streams(&text)
    );
}

/// `rv status --check` is the worker's poll and a CI gate: exit 1 while any
/// comment is open, 0 once none is, nothing printed — unless `--json` asks for
/// the report too.
#[test]
fn rv_status_check_answers_in_the_exit_code() {
    let workspace = Fixture::new();
    workspace.write("a.rs", "fn a() {\n    let x = 1;\n}\n");
    workspace.commit("first change");

    let clean = workspace.rv(&["status", "--check"]);
    assert!(clean.status.success(), "{}", streams(&clean));
    assert!(clean.stdout.is_empty(), "{}", streams(&clean));

    let saved = workspace.rv(&["comment", "a.rs", "--line", "2", "-m", "work"]);
    let id = String::from_utf8_lossy(&saved.stdout)
        .split_whitespace()
        .nth(1)
        .expect("an id")
        .to_owned();

    let open = workspace.rv(&["status", "--check"]);
    assert_eq!(open.status.code(), Some(1), "{}", streams(&open));
    assert!(open.stdout.is_empty(), "{}", streams(&open));

    // `--check --json` prints the report *and* sets the code.
    let both = workspace.rv(&["status", "--check", "--json"]);
    assert_eq!(both.status.code(), Some(1), "{}", streams(&both));
    assert!(!both.stdout.is_empty(), "{}", streams(&both));

    workspace.rv(&["resolve", &id]);
    let settled = workspace.rv(&["status", "--check"]);
    assert!(settled.status.success(), "{}", streams(&settled));
}
