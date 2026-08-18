//! End-to-end tests for the `rv` binary: they run the real executable against a
//! throwaway jj workspace and inspect what it wrote to disk and to its streams.
//!
//! The fixture below is a copy of `rv-core/tests/fixture.rs`. It is duplicated
//! rather than shared because a `tests/` helper cannot be imported across crates
//! without publishing it from a library, and publishing a test scaffold from
//! `rv-core` would put fixture code in the shipped API.
//!
//! Only tests may drive the `jj` CLI — production code speaks to jj-lib in
//! process. Every invocation is made hermetic with `JJ_CONFIG=/dev/null` plus a
//! fixed author, so the developer's own jj config cannot change what the tests
//! see.

use std::fs;
use std::path::Path;
use std::process::Command;
use std::process::Output;

use tempfile::TempDir;

struct Fixture {
    tempdir: TempDir,
}

impl Fixture {
    /// Creates a colocated jj workspace in a fresh temporary directory.
    fn new() -> Self {
        let fixture = Self {
            tempdir: tempfile::tempdir().expect("create temp dir"),
        };
        fixture.jj(&["git", "init", "--colocate"]);
        fixture
    }

    /// The workspace root.
    fn root(&self) -> &Path {
        self.tempdir.path()
    }

    /// Runs `jj` in the workspace and returns its stdout, panicking on failure.
    fn jj(&self, args: &[&str]) -> String {
        let output = Command::new("jj")
            .args(args)
            .current_dir(self.root())
            .env("JJ_CONFIG", "/dev/null")
            .env("JJ_USER", "rv-test")
            .env("JJ_EMAIL", "rv-test@localhost")
            .output()
            .expect("run jj");
        assert!(
            output.status.success(),
            "jj {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).expect("jj stdout is utf-8")
    }

    /// Writes a file in the working copy, creating parent directories.
    fn write(&self, rel: &str, contents: &str) {
        let path = self.root().join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parent dir");
        }
        fs::write(&path, contents).expect("write file");
    }

    /// Describes the working-copy change and opens an empty one on top of it.
    fn commit(&self, message: &str) {
        self.jj(&["describe", "-m", message]);
        self.jj(&["new"]);
    }

    /// Runs the `rv` binary with the workspace as its current directory, so the
    /// default `--repo` is exercised too.
    fn rv(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_rv"))
            .args(args)
            .current_dir(self.root())
            .output()
            .expect("run rv")
    }
}

/// Renders `output`'s streams for an assertion message.
fn streams(output: &Output) -> String {
    format!(
        "status {}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    )
}

#[test]
fn render_writes_markdown_and_excludes() {
    let workspace = Fixture::new();
    workspace.write("a.rs", "fn a() {}\n");
    workspace.commit("first change");

    let output = workspace.rv(&["render"]);
    assert!(output.status.success(), "{}", streams(&output));

    let markdown = fs::read_to_string(workspace.root().join(".review/REVIEW-FEEDBACK.md"))
        .expect("read rendered markdown");
    assert!(
        markdown.starts_with("<!-- rv:v1 -->"),
        "markdown does not open with the version marker:\n{markdown}"
    );
    assert!(
        markdown.contains("**For LLMs:**"),
        "markdown is missing the LLM protocol block:\n{markdown}"
    );

    let exclude = fs::read_to_string(workspace.root().join(".git/info/exclude"))
        .expect("read .git/info/exclude");
    assert!(
        exclude.lines().any(|line| line == "/.review/"),
        "exclude file does not list /.review/:\n{exclude}"
    );
}

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

    // A comment written against a line, by hand — the TUI is not what is under
    // test here — and then the line rewritten under it.
    let head = workspace
        .rv(&["status", "--json"])
        .stdout
        .clone();
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
    std::fs::write(
        workspace.root().join(".review/comments.json"),
        serde_json::to_vec_pretty(&comment).expect("serialize"),
    )
    .expect("write comments.json");

    let output = workspace.rv(&["status", "--json"]);
    let report: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("status --json is valid json");
    assert_eq!(
        report["comments"]["outdated"], 1,
        "a comment whose hash cannot be found is not reported outdated: {}",
        streams(&output)
    );
    assert_eq!(
        report["comments"]["open"], 0,
        "and it is not also counted as open: {}",
        streams(&output)
    );
}
