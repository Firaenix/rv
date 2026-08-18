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
    workspace.write("a.rs", "fn a() {\n    let x = 1;\n}\n");
    workspace.commit("first change");

    // A comment written against a line, by hand — the TUI is not what is under
    // test here — and then the line rewritten under it.
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
    std::fs::write(
        workspace.root().join(".review/comments.json"),
        serde_json::to_vec_pretty(&comment).expect("serialize"),
    )
    .expect("write comments.json");

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

/// Opening a review over another range re-points the record, and that is
/// deliberate: a reviewer asking for a narrower range is asking for it.
///
/// The comments do not move and are not mislabelled — each carries its own
/// change, commit and anchor, and the reviewer sees only the ones the open range
/// can reach. What used to be wrong was `rv status` doing this *without being
/// asked*, which `status_writes_nothing` above is the guard for.
#[test]
fn opening_another_range_re_points_the_record_and_keeps_the_comments() {
    let workspace = Fixture::new();
    rv::session::build(workspace.root(), None, None).expect("open the default range");

    let comment = serde_json::json!([{
        "id": "deadbee1",
        "change_id": "z".repeat(32),
        "commit_id": "a".repeat(40),
        "anchor": {
            "file": "a.rs",
            "side": "Right",
            "line": 1,
            "content_hash": "0".repeat(64),
            "context": ["fn a() {"],
        },
        "body": "made against the default range",
        "state": "open",
        "reply": null,
    }]);
    std::fs::write(
        workspace.root().join(".review/comments.json"),
        serde_json::to_vec_pretty(&comment).expect("serialize"),
    )
    .expect("write comments.json");

    rv::session::build(workspace.root(), Some("@-"), None).expect("open a narrower range");

    let session = std::fs::read_to_string(workspace.root().join(".review/session.toml"))
        .expect("read session.toml");
    assert!(
        session.contains("revset = \"@-..@\""),
        "the record does not describe the range that was asked for:\n{session}"
    );
    let stored = std::fs::read_to_string(workspace.root().join(".review/comments.json"))
        .expect("read comments.json");
    assert!(
        stored.contains("made against the default range"),
        "re-pointing the record deleted a comment"
    );
}

/// Re-opening the *same* range keeps `started_at`: it says when the review began,
/// and re-stamping it on every command would make it say when the reviewer last
/// ran one — which moved the timestamp in the header of an existing export.
#[test]
fn re_opening_the_same_range_keeps_when_the_review_began() {
    let workspace = Fixture::new();
    rv::session::build(workspace.root(), None, None).expect("open");
    let first = std::fs::read_to_string(workspace.root().join(".review/session.toml"))
        .expect("read session.toml");

    rv::session::build(workspace.root(), None, None).expect("open again");
    let again = std::fs::read_to_string(workspace.root().join(".review/session.toml"))
        .expect("read session.toml");

    let started = |toml: &str| {
        toml.lines()
            .find(|line| line.starts_with("started_at"))
            .expect("a started_at")
            .to_owned()
    };
    assert_eq!(started(&first), started(&again), "the clock was restarted");
}

/// Every file in the workspace, so a test can say "this wrote nothing at all".
fn tree(root: &std::path::Path) -> Vec<(String, std::time::SystemTime, Vec<u8>)> {
    let mut files = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
            } else if let Ok(metadata) = std::fs::metadata(&path) {
                files.push((
                    path.strip_prefix(root)
                        .expect("under the root")
                        .display()
                        .to_string(),
                    metadata.modified().expect("an mtime"),
                    std::fs::read(&path).unwrap_or_default(),
                ));
            }
        }
    }
    files.sort();
    files
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

    workspace.rv(&["render"]);
    let document = std::fs::read_to_string(workspace.root().join(".review/REVIEW-FEEDBACK.md"))
        .expect("read the export");
    assert!(
        document.contains("resolved to the repository root"),
        "the export does not name the degradation:\n{document}"
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

/// `rv comment` is the reviewer agent's entry point: the anchor, the id and the
/// export are all handled, so nothing writes `.review/` files by hand.
///
/// It goes through the same functions the TUI's `c` does — the project has
/// already shipped one bug from two places deciding which side a thing is on —
/// so a comment added here is indistinguishable from one typed in the pane.
#[test]
fn rv_comment_saves_an_anchored_comment_and_refreshes_the_export() {
    let workspace = Fixture::new();
    workspace.write("a.rs", "fn a() {\n    let x = 1;\n}\n");
    workspace.commit("first change");

    let output = workspace.rv(&[
        "comment",
        "a.rs",
        "--line",
        "2",
        "-m",
        "this line needs a name",
    ]);
    assert!(output.status.success(), "{}", streams(&output));
    let said = String::from_utf8_lossy(&output.stdout);
    assert!(
        said.contains("a.rs:2") && said.contains("right"),
        "the confirmation does not say where the comment landed: {said}"
    );

    // Anchored like the TUI would anchor it: side, hash and context all present.
    let stored = std::fs::read_to_string(workspace.root().join(".review/comments.json"))
        .expect("read comments.json");
    let comments: serde_json::Value = serde_json::from_str(&stored).expect("valid json");
    let comment = &comments[0];
    assert_eq!(comment["anchor"]["file"], "a.rs");
    assert_eq!(comment["anchor"]["line"], 2);
    assert_eq!(comment["state"], "open");
    assert!(
        comment["anchor"]["context"]
            .as_array()
            .is_some_and(|context| !context.is_empty()),
        "the anchor quotes nothing: {comment}"
    );

    // And the export already carries it, so a worker agent polling the document
    // sees it without a separate render step.
    let document = std::fs::read_to_string(workspace.root().join(".review/REVIEW-FEEDBACK.md"))
        .expect("read the export");
    assert!(
        document.contains("this line needs a name"),
        "the export was not refreshed:\n{document}"
    );

    // The round trip: a reply appended to the document reaches the store on the
    // next render, which is the worker agent's half of the loop.
    let replied = document.replace(
        "**Comment:** this line needs a name",
        "**Comment:** this line needs a name\n**Reply:** renamed it to `total`",
    );
    std::fs::write(workspace.root().join(".review/REVIEW-FEEDBACK.md"), replied)
        .expect("write the reply");
    workspace.rv(&["render"]);
    let stored = std::fs::read_to_string(workspace.root().join(".review/comments.json"))
        .expect("read comments.json");
    assert!(
        stored.contains("renamed it to `total`"),
        "the reply never reached the store:\n{stored}"
    );
}

/// The refusals name what went wrong, because the caller is a program: a
/// reviewer agent that mistypes a path must hear so now, not discover a missing
/// comment three rounds later.
#[test]
fn rv_comment_refuses_with_reasons_a_program_can_act_on() {
    let workspace = Fixture::new();
    workspace.write("a.rs", "fn a() {\n    let x = 1;\n}\n");
    workspace.commit("first change");

    let missing = workspace.rv(&["comment", "nope.rs", "--line", "1", "-m", "x"]);
    assert!(!missing.status.success());
    assert!(
        String::from_utf8_lossy(&missing.stderr).contains("not in this review's range"),
        "{}",
        streams(&missing)
    );

    let past_the_end = workspace.rv(&["comment", "a.rs", "--line", "999", "-m", "x"]);
    assert!(!past_the_end.status.success());
    assert!(
        String::from_utf8_lossy(&past_the_end.stderr).contains("has lines 1..="),
        "{}",
        streams(&past_the_end)
    );

    let empty = workspace.rv(&["comment", "a.rs", "--line", "1", "-m", "   "]);
    assert!(!empty.status.success());
    assert!(
        String::from_utf8_lossy(&empty.stderr).contains("empty comment"),
        "{}",
        streams(&empty)
    );
}

/// The worker's tick-off: `rv resolve <id>` records that it was addressed and
/// **who says so**, and the same command re-applied is the undo.
#[test]
fn rv_resolve_settles_a_comment_and_records_the_agent() {
    let workspace = Fixture::new();
    workspace.write("a.rs", "fn a() {\n    let x = 1;\n}\n");
    workspace.commit("first change");
    let saved = workspace.rv(&["comment", "a.rs", "--line", "2", "-m", "needs a name"]);
    let id = String::from_utf8_lossy(&saved.stdout)
        .split_whitespace()
        .nth(1)
        .expect("the confirmation names the id")
        .to_owned();

    let output = workspace.rv(&["resolve", &id]);
    assert!(output.status.success(), "{}", streams(&output));

    let stored = std::fs::read_to_string(workspace.root().join(".review/comments.json"))
        .expect("read comments.json");
    let comments: serde_json::Value = serde_json::from_str(&stored).expect("valid json");
    assert_eq!(comments[0]["state"], "resolved");
    assert_eq!(
        comments[0]["settled_by"], "agent",
        "who settled it went unrecorded — which is the one thing that must not"
    );

    // The export moved it out of Open, so a polling worker stops seeing it as
    // work.
    let document = std::fs::read_to_string(workspace.root().join(".review/REVIEW-FEEDBACK.md"))
        .expect("read the export");
    assert!(
        document.contains("## Open (0)") && document.contains("## Resolved (1)"),
        "the export still lists it as open:\n{document}"
    );

    // And re-applying is the undo.
    let again = workspace.rv(&["resolve", &id]);
    assert!(
        String::from_utf8_lossy(&again.stdout).contains("reopened"),
        "{}",
        streams(&again)
    );
}

/// Abandoned is not resolved: dropped-unfixed and fixed are different
/// conclusions, and the store keeps them apart.
#[test]
fn rv_abandon_is_a_distinct_state() {
    let workspace = Fixture::new();
    workspace.write("a.rs", "fn a() {\n    let x = 1;\n}\n");
    workspace.commit("first change");
    let saved = workspace.rv(&["comment", "a.rs", "--line", "1", "-m", "out of scope"]);
    let id = String::from_utf8_lossy(&saved.stdout)
        .split_whitespace()
        .nth(1)
        .expect("an id")
        .to_owned();

    workspace.rv(&["abandon", &id]);

    let stored = std::fs::read_to_string(workspace.root().join(".review/comments.json"))
        .expect("read comments.json");
    assert!(
        stored.contains("\"abandoned\""),
        "abandoning stored some other state:\n{stored}"
    );

    let unknown = workspace.rv(&["resolve", "ffffffff"]);
    assert!(!unknown.status.success());
    assert!(
        String::from_utf8_lossy(&unknown.stderr).contains("no comment ffffffff"),
        "{}",
        streams(&unknown)
    );
}
