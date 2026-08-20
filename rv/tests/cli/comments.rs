//! The agent's comment operations: saving, settling, replying, and reading back.

use super::support::*;

/// `rv comment` is the reviewer agent's entry point: the anchor and the id are
/// handled, so nothing writes `.review/` files by hand.
///
/// It goes through the same functions the TUI's `c` does — the project has
/// already shipped one bug from two places deciding which side a thing is on —
/// so a comment added here is indistinguishable from one typed in the pane.
#[test]
fn rv_comment_saves_an_anchored_comment_without_touching_the_export() {
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
    let stored = stored_comments(&workspace);
    let comment = &stored[0];
    assert_eq!(comment.anchor.file, "a.rs");
    assert_eq!(comment.anchor.line, 2);
    assert_eq!(comment.state, rv_core::store::CommentState::Open);
    assert!(
        !comment.anchor.context.is_empty(),
        "the anchor quotes nothing: {comment:?}"
    );

    // The markdown is a view rendered on request: saving writes no export, and
    // a polling worker reads `rv status` / `rv comments` instead.
    assert!(
        !workspace.root().join(".review/REVIEW-FEEDBACK.md").exists(),
        "saving refreshed the export, which nothing reads back"
    );

    // The worker's answer goes through the CLI, not through the document.
    let id = comment.id.clone();
    let replied = workspace.rv(&["reply", &id, "-m", "renamed it to `total`"]);
    assert!(replied.status.success(), "{}", streams(&replied));
    assert!(
        stored_comments(&workspace)
            .iter()
            .any(|comment| comment.reply.as_deref() == Some("renamed it to `total`")),
        "the reply never reached the store"
    );
}

/// The comments as `session.toml` holds them — one file, read through the
/// store rather than by parsing whichever file happens to back it.
fn stored_comments(workspace: &Fixture) -> Vec<rv_core::store::Comment> {
    rv_core::store::Store::open(workspace.root())
        .expect("open the store")
        .comments()
        .expect("read the comments")
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

    let stored = stored_comments(&workspace);
    assert_eq!(stored[0].state, rv_core::store::CommentState::Resolved);
    assert_eq!(
        stored[0].settled_by,
        Some(rv_core::store::SettledBy::Agent),
        "who settled it went unrecorded — which is the one thing that must not"
    );

    // The worker's poll stops seeing it as work.
    let status = workspace.rv(&["status", "--json"]);
    let report: serde_json::Value =
        serde_json::from_slice(&status.stdout).expect("status --json is valid json");
    assert_eq!(report["comments"]["open"], 0, "{}", streams(&status));
    assert_eq!(report["comments"]["resolved"], 1, "{}", streams(&status));

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

    let stored = stored_comments(&workspace);
    assert_eq!(
        stored[0].state,
        rv_core::store::CommentState::Abandoned,
        "abandoning stored some other state"
    );

    let unknown = workspace.rv(&["resolve", "ffffffff"]);
    assert!(!unknown.status.success());
    assert!(
        String::from_utf8_lossy(&unknown.stderr).contains("no comment ffffffff"),
        "{}",
        streams(&unknown)
    );
}

/// `rv comments --json` is the read channel: everything the store and a load
/// can say, on the same in-range view the TUI and `rv status` read — one
/// review, three readers, one answer.
#[test]
fn rv_comments_json_is_the_read_channel() {
    let workspace = Fixture::new();
    workspace.write("a.rs", "fn a() {\n    let x = 1;\n}\n");
    workspace.commit("first change");
    let saved = workspace.rv(&["comment", "a.rs", "--line", "2", "-m", "needs a name"]);
    let id = String::from_utf8_lossy(&saved.stdout)
        .split_whitespace()
        .nth(1)
        .expect("an id")
        .to_owned();
    workspace.rv(&[
        "comment",
        "a.rs",
        "--line",
        "1",
        "-m",
        "and this one settles",
    ]);

    let listed = workspace.rv(&["comments", "--json"]);
    assert!(listed.status.success(), "{}", streams(&listed));
    let comments: serde_json::Value =
        serde_json::from_slice(&listed.stdout).expect("comments --json is valid json");
    let all = comments.as_array().expect("an array");
    assert_eq!(all.len(), 2, "{comments}");
    let first = all
        .iter()
        .find(|comment| comment["id"] == id.as_str())
        .expect("the saved comment is listed");
    assert_eq!(first["state"], "open");
    assert_eq!(first["outdated"], false);
    assert_eq!(first["body"], "needs a name");
    assert_eq!(first["reply"], serde_json::Value::Null);
    assert_eq!(first["anchor"]["file"], "a.rs");
    assert_eq!(first["anchor"]["side"], "right");
    assert_eq!(first["anchor"]["line"], 2);
    assert!(
        first["anchor"]["context_start"].is_number(),
        "the excerpt does not say where it starts: {first}"
    );
    assert!(
        first["anchor"]["context"]
            .as_array()
            .is_some_and(|context| !context.is_empty()),
        "the anchor quotes nothing: {first}"
    );

    // `--state open` is the worker's first question, without `jq`.
    let other = all
        .iter()
        .find(|comment| comment["id"] != id.as_str())
        .expect("two comments")["id"]
        .as_str()
        .expect("an id")
        .to_owned();
    workspace.rv(&["resolve", &other]);
    let open = workspace.rv(&["comments", "--json", "--state", "open"]);
    let open: serde_json::Value = serde_json::from_slice(&open.stdout).expect("filtered json");
    let open = open.as_array().expect("an array");
    assert_eq!(open.len(), 1, "{open:?}");
    assert_eq!(open[0]["id"], id.as_str());
}

/// `rv reply` is the answer channel: unknown ids are errors, a second reply
/// replaces the first, and resolving afterwards keeps the reply intact.
#[test]
fn rv_reply_stores_replaces_and_survives_settling() {
    let workspace = Fixture::new();
    workspace.write("a.rs", "fn a() {\n    let x = 1;\n}\n");
    workspace.commit("first change");
    let saved = workspace.rv(&["comment", "a.rs", "--line", "2", "-m", "needs a name"]);
    let id = String::from_utf8_lossy(&saved.stdout)
        .split_whitespace()
        .nth(1)
        .expect("an id")
        .to_owned();

    // A typoed id is an error, not a silently dropped answer — which is the
    // markdown failure mode this command exists to delete.
    let unknown = workspace.rv(&["reply", "ffffffff", "-m", "lost work"]);
    assert!(!unknown.status.success(), "{}", streams(&unknown));
    assert!(
        String::from_utf8_lossy(&unknown.stderr).contains("ffffffff"),
        "{}",
        streams(&unknown)
    );
    let stored = stored_comments(&workspace);
    assert!(
        stored.iter().all(|comment| comment.reply.is_none()),
        "the failed reply stored something: {stored:?}"
    );

    workspace.rv(&["reply", &id, "-m", "first answer"]);
    workspace.rv(&["reply", &id, "-m", "better answer"]);
    workspace.rv(&["resolve", &id]);

    let listed = workspace.rv(&["comments", "--json"]);
    let comments: serde_json::Value = serde_json::from_slice(&listed.stdout).expect("valid json");
    assert_eq!(comments[0]["reply"], "better answer", "{comments}");
    assert_eq!(comments[0]["state"], "resolved");
    assert_eq!(comments[0]["settled_by"], "agent");
}

/// Saving, settling and replying leave the export's bytes untouched: the file
/// is a view produced on request, and a file nothing reads back cannot be
/// dangerously stale.
#[test]
fn saving_settling_and_replying_leave_the_export_untouched() {
    let workspace = Fixture::new();
    workspace.write("a.rs", "fn a() {\n    let x = 1;\n}\n");
    workspace.commit("first change");
    workspace.rv(&["render", "--out", ".review/REVIEW-FEEDBACK.md"]);
    let before = std::fs::read(workspace.root().join(".review/REVIEW-FEEDBACK.md"))
        .expect("read the export");

    let saved = workspace.rv(&["comment", "a.rs", "--line", "2", "-m", "needs a name"]);
    let id = String::from_utf8_lossy(&saved.stdout)
        .split_whitespace()
        .nth(1)
        .expect("an id")
        .to_owned();
    workspace.rv(&["reply", &id, "-m", "done"]);
    workspace.rv(&["resolve", &id]);

    let after = std::fs::read(workspace.root().join(".review/REVIEW-FEEDBACK.md"))
        .expect("read the export again");
    assert_eq!(before, after, "a side effect rewrote the export");

    // An explicit render carries the current review.
    let rendered = workspace.rv(&["render"]);
    let document = String::from_utf8_lossy(&rendered.stdout);
    assert!(
        document.contains("needs a name") && document.contains("done"),
        "the view is not the current review:\n{document}"
    );
}

/// `-m -` reads the body from stdin, so backticks, quotes, `$` and newlines
/// arrive byte-exact instead of one shell-quoting mistake from mangled.
#[test]
fn a_body_from_stdin_round_trips_byte_identically() {
    let workspace = Fixture::new();
    workspace.write("a.rs", "fn a() {\n    let x = 1;\n}\n");
    workspace.commit("first change");

    let body = "`content_hash` is computed from the $untrimmed line — \"so\"\nre-indenting breaks 'every' anchor.";
    let saved = workspace.rv_with_stdin(&["comment", "a.rs", "--line", "2", "-m", "-"], body);
    assert!(saved.status.success(), "{}", streams(&saved));

    let listed = workspace.rv(&["comments", "--json"]);
    let comments: serde_json::Value = serde_json::from_slice(&listed.stdout).expect("valid json");
    assert_eq!(comments[0]["body"], body, "{comments}");

    // An empty stdin body is refused exactly as an empty `-m` argument is.
    let empty = workspace.rv_with_stdin(&["comment", "a.rs", "--line", "1", "-m", "-"], "  \n");
    assert!(!empty.status.success(), "{}", streams(&empty));
}
