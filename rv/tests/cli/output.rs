//! What the other commands print: the export, the diff, the help, the degraded note.

use super::support::*;
use std::fs;

#[test]
fn render_prints_the_view_and_out_writes_it() {
    let workspace = Fixture::new();
    workspace.write("a.rs", "fn a() {}\n");
    workspace.commit("first change");

    // The default is stdout: the markdown is a projection for reading, and
    // nothing reads it back, so where it lands is the caller's business.
    let output = workspace.rv(&["render"]);
    assert!(output.status.success(), "{}", streams(&output));
    let markdown = String::from_utf8_lossy(&output.stdout);
    assert!(
        markdown.starts_with("<!-- rv:v1 -->"),
        "the view does not open with the version marker:\n{markdown}"
    );
    assert!(
        markdown.contains("rendered view") && markdown.contains("rv comments --json"),
        "the view does not name the CLI as the real interface:\n{markdown}"
    );
    assert!(
        !workspace.root().join(".review/REVIEW-FEEDBACK.md").exists(),
        "a bare render wrote a file nobody asked for"
    );

    // `--out` is the artefact-on-request form.
    let output = workspace.rv(&["render", "--out", ".review/REVIEW-FEEDBACK.md"]);
    assert!(output.status.success(), "{}", streams(&output));
    let written = fs::read_to_string(workspace.root().join(".review/REVIEW-FEEDBACK.md"))
        .expect("read rendered markdown");
    assert!(written.starts_with("<!-- rv:v1 -->"), "{written}");

    let exclude = fs::read_to_string(workspace.root().join(".git/info/exclude"))
        .expect("read .git/info/exclude");
    assert!(
        exclude.lines().any(|line| line == "/.review/"),
        "exclude file does not list /.review/:\n{exclude}"
    );
}

/// A repo with no remote has no `trunk()`, and the export says so instead of
/// presenting the whole history as a branch review.
///
/// `trunk()` is a union of the usual remote bookmarks *and the repository root*,
/// so it degrades silently. The export used to come out headed `trunk()..@` over
/// an all-zero base with every file marked added, and a model handed that document
/// cannot tell a whole-repo dump from a real review — nor can a reviewer tell why
/// everything is a `+`.
#[test]
fn a_degraded_trunk_is_named_rather_than_implied() {
    let workspace = Fixture::new();

    let status = workspace.rv(&["status"]);
    let text = String::from_utf8_lossy(&status.stdout);
    assert!(
        text.contains("resolved to the repository root"),
        "`rv status` presents the whole history as a branch review: {}",
        streams(&status)
    );

    let json = workspace.rv(&["status", "--json"]);
    let report: serde_json::Value =
        serde_json::from_slice(&json.stdout).expect("status --json is valid json");
    assert_eq!(
        report["degraded_base"],
        true,
        "a script cannot tell the difference: {}",
        streams(&json)
    );

    let rendered = workspace.rv(&["render"]);
    let document = String::from_utf8_lossy(&rendered.stdout);
    assert!(
        document.contains("resolved to the repository root"),
        "the view does not name the degradation:\n{document}"
    );
}

/// `--no-difft` is a capability a reviewer has, not a hook the tests reach
/// through.
///
/// The engine used to be selectable only by a constructor named after a fallback,
/// whose one caller was a test file — so the thing a user with no `difft` sees was
/// unreachable from the command line. The flag is now in `--help`, which is where
/// a documented capability lives.
#[test]
fn the_fallback_engine_is_a_documented_flag() {
    let workspace = Fixture::new();

    let help = workspace.rv(&["--help"]);
    let text = String::from_utf8_lossy(&help.stdout);
    assert!(
        text.contains("--no-difft"),
        "the flag is not documented: {}",
        streams(&help)
    );

    // And it is accepted rather than merely listed.
    let output = workspace.rv(&["--no-difft", "status"]);
    assert!(output.status.success(), "{}", streams(&output));
}

/// `rv diff --json` issues coordinates in rv's own vocabulary, and a line it
/// reports as `right: n` is a line `rv comment --line n` accepts — the tool
/// that validates the anchor is the tool that issued the numbers.
#[test]
fn rv_diff_json_issues_the_coordinates_comment_accepts() {
    let workspace = Fixture::new();
    workspace.write("a.rs", "fn a() {\n    let x = 1;\n}\n");
    workspace.commit("first change");

    let output = workspace.rv(&["diff", "--json"]);
    assert!(output.status.success(), "{}", streams(&output));
    let files: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("diff --json is valid json");
    let file = &files.as_array().expect("an array")[0];
    assert_eq!(file["file"], "a.rs");
    assert!(
        file["engine"] == "difftastic" || file["engine"] == "fallback",
        "the engine is stated: {file}"
    );
    assert_eq!(file["binary"], false);

    let line = file["lines"]
        .as_array()
        .expect("lines")
        .iter()
        .find(|line| line["kind"] == "added" && line["right"].is_number())
        .expect("an added line with a head-side number")
        .clone();
    let number = line["right"].as_u64().expect("a number").to_string();

    let saved = workspace.rv(&[
        "comment",
        "a.rs",
        "--line",
        &number,
        "-m",
        "on rv's own number",
    ]);
    assert!(
        saved.status.success(),
        "rv refused a coordinate it issued itself: {}",
        streams(&saved)
    );

    // One file by name, and a file outside the range is an error.
    let one = workspace.rv(&["diff", "a.rs", "--json"]);
    assert!(one.status.success(), "{}", streams(&one));
    let missing = workspace.rv(&["diff", "nope.rs", "--json"]);
    assert!(!missing.status.success(), "{}", streams(&missing));
}

/// The worker's whole loop, CLI only, with no read of the markdown anywhere:
/// check → comments → reply → resolve → check.
#[test]
fn the_worker_loop_runs_without_the_markdown() {
    let workspace = Fixture::new();
    workspace.write("a.rs", "fn a() {\n    let x = 1;\n}\n");
    workspace.commit("first change");
    workspace.rv(&["comment", "a.rs", "--line", "2", "-m", "needs a name"]);

    assert_eq!(
        workspace.rv(&["status", "--check"]).status.code(),
        Some(1),
        "there is work"
    );
    let listed = workspace.rv(&["comments", "--json", "--state", "open"]);
    let open: serde_json::Value = serde_json::from_slice(&listed.stdout).expect("valid json");
    let id = open[0]["id"].as_str().expect("an id").to_owned();

    // …fix the code…
    workspace.rv(&["reply", &id, "-m", "renamed; the tests pin it"]);
    workspace.rv(&["resolve", &id]);

    assert!(
        workspace.rv(&["status", "--check"]).status.success(),
        "the loop did not converge"
    );
    assert!(
        !workspace.root().join(".review/REVIEW-FEEDBACK.md").exists(),
        "something in the loop touched the markdown"
    );
}
