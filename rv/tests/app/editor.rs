//! `v`: the selected file in `$EDITOR`.
//!
//! `$EDITOR` is read from the process environment, and `set_var` is unsafe in
//! edition 2024 precisely because it races any concurrent `getenv` — which a
//! cargo test binary running on many threads guarantees. So the two halves of
//! this file are run in a **child** of this binary, given the environment on
//! its command line, the way `tests/statusbar.rs` pins `RV_ASCII`.
//!
//! What the child does is the real thing end to end: it presses `v`, runs the
//! editor the key resolved, and then keeps driving the reviewer afterwards.

use std::ffi::OsStr;
use std::process::Command;

use crossterm::event::KeyCode;
use rv::app::Action;
use rv::app::Focus;
use rv::app::Mode;

use crate::support::*;

/// Tells a re-executed copy of this test binary which half it is running, and
/// stops it re-executing itself again.
const CHILD: &str = "RV_EDITOR_CHILD";

/// A command that exists everywhere, does nothing, and exits zero — an editor
/// that opens the file and closes it again.
const HARMLESS: &str = "true";

/// Runs `test` — this module's, named without its `editor::` prefix — in a
/// child of this binary with `$EDITOR` set, or removed entirely when `editor`
/// is `None`.
fn in_child(test: &str, editor: Option<&str>) {
    let mut child = Command::new(std::env::current_exe().expect("this test binary"));
    child
        .args(["--exact", "--test-threads=1", &format!("editor::{test}")])
        .env(CHILD, "1");
    match editor {
        Some(editor) => child.env("EDITOR", editor),
        None => child.env_remove("EDITOR"),
    };

    let output = child.output().expect("re-run this test binary");
    let log = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "the child failed:\n{log}{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        log.contains("1 passed"),
        "the child filtered the case away instead of running it:\n{log}"
    );
}

/// Whether this process is the child, and so should run the body rather than
/// spawn one.
fn is_child() -> bool {
    std::env::var_os(CHILD).is_some()
}

/// The reviewer is still the reviewer: browse mode, the diff focused, keys
/// answered, and a comment still savable. This is what "the TUI is usable
/// afterwards" means in a test with no terminal in it.
fn still_usable(workspace: &Fixture, app: &mut rv::app::App) {
    assert_eq!(app.mode(), Mode::Browse);
    assert_eq!(app.focus(), Focus::Diff);
    assert_eq!(
        app.on_key(KeyCode::Down).expect("j still moves"),
        Action::Continue
    );
    assert_eq!(app.line_index(), 1, "the cursor stopped moving");

    app.on_key(KeyCode::Char('c')).expect("c");
    for character in "after the editor".chars() {
        app.on_key(KeyCode::Char(character)).expect("type");
    }
    app.on_key(KeyCode::Enter).expect("save");
    let comments = workspace.store().comments().expect("read the comments");
    assert_eq!(comments.len(), 1, "a comment no longer saves: {comments:?}");

    assert_eq!(
        app.on_key(KeyCode::Char('q')).expect("q"),
        Action::Quit,
        "the reviewer can no longer be quit"
    );
}

/// With `$EDITOR` unset the key refuses in the status line and the terminal is
/// never given up — [`Action::Continue`] is what says so, since only
/// [`Action::Edit`] makes the loop leave the screen.
#[test]
fn v_with_no_editor_set_says_so_and_keeps_the_reviewer_running() {
    if !is_child() {
        in_child(
            "v_with_no_editor_set_says_so_and_keeps_the_reviewer_running",
            None,
        );
        return;
    }

    let workspace = Fixture::new();
    let mut app = workspace.app();
    assert_eq!(
        app.on_key(KeyCode::Char('E')).expect("v"),
        Action::Continue,
        "v gave the terminal up with no editor to give it to"
    );
    assert_eq!(app.status(), "$EDITOR is not set");
    // ...and nothing is left waiting to run, so a later pass cannot open an
    // editor the reviewer was told they did not have.
    app.run_pending_edit();
    assert_eq!(app.status(), "$EDITOR is not set");

    still_usable(&workspace, &mut app);
}

/// With one set, `v` asks the loop for the terminal, the editor runs, and the
/// reviewer carries on — including after an editor that exited non-zero, which
/// is the case a half-restored terminal would show up in.
#[test]
fn v_runs_the_editor_and_the_reviewer_survives_it() {
    if !is_child() {
        in_child(
            "v_runs_the_editor_and_the_reviewer_survives_it",
            Some(HARMLESS),
        );
        return;
    }

    let workspace = Fixture::new();
    let mut app = workspace.app();
    assert_eq!(
        app.on_key(KeyCode::Char('E')).expect("v"),
        Action::Edit,
        "v did not ask the loop for the terminal"
    );

    app.run_pending_edit();
    let status = app.status().to_owned();
    assert!(
        status.starts_with("edited ") && status.contains("a.rs:"),
        "the bar does not name what was edited: {status:?}"
    );
    assert!(
        status.ends_with(HARMLESS),
        "the bar does not name the editor: {status:?}"
    );
    assert!(
        app.alerts().is_empty(),
        "a clean edit raised an alert: {:?}",
        app.alerts()
    );

    still_usable(&workspace, &mut app);
}

/// An editor that exits non-zero is reported rather than swallowed, and leaves
/// the reviewer exactly as usable — the terminal handover is unconditional, so
/// this is the failure path that must not be a special case.
#[test]
fn an_editor_that_fails_is_reported_and_the_reviewer_survives_it() {
    if !is_child() {
        in_child(
            "an_editor_that_fails_is_reported_and_the_reviewer_survives_it",
            Some("false"),
        );
        return;
    }

    let workspace = Fixture::new();
    let mut app = workspace.app();
    assert_eq!(app.on_key(KeyCode::Char('E')).expect("v"), Action::Edit);
    app.run_pending_edit();

    let alerts: Vec<&str> = app
        .alerts()
        .iter()
        .map(|alert| alert.message.as_str())
        .collect();
    assert_eq!(alerts.len(), 1, "{alerts:?}");
    assert!(
        alerts[0].contains("exited with"),
        "a failed editor was not reported: {alerts:?}"
    );

    still_usable(&workspace, &mut app);
}

/// An `$EDITOR` that names nothing runnable cannot take the terminal down with
/// it: the spawn fails, the failure is an alert, and the review goes on.
#[test]
fn an_editor_that_cannot_be_run_is_an_alert_rather_than_an_error() {
    if !is_child() {
        in_child(
            "an_editor_that_cannot_be_run_is_an_alert_rather_than_an_error",
            Some("rv-no-such-editor-anywhere"),
        );
        return;
    }

    let workspace = Fixture::new();
    let mut app = workspace.app();
    assert_eq!(app.on_key(KeyCode::Char('E')).expect("v"), Action::Edit);
    app.run_pending_edit();

    let alerts: Vec<&str> = app
        .alerts()
        .iter()
        .map(|alert| alert.message.as_str())
        .collect();
    assert_eq!(alerts.len(), 1, "{alerts:?}");
    assert!(
        alerts[0].starts_with("could not run $EDITOR"),
        "an unrunnable editor was not reported: {alerts:?}"
    );

    still_usable(&workspace, &mut app);
}

/// The variable is the one every other tool reads. No in-process test can see
/// the *name*: a lookup of `$EDITORR` would agree with itself perfectly and
/// leave every reviewer's `$EDITOR` doing nothing, so the child above is given
/// the name as a literal and this pins that the same literal is what rv reads.
#[test]
fn the_editor_variable_is_the_one_every_other_tool_reads() {
    if !is_child() {
        in_child(
            "the_editor_variable_is_the_one_every_other_tool_reads",
            Some(HARMLESS),
        );
        return;
    }

    assert_eq!(
        std::env::var_os("EDITOR").as_deref(),
        Some(OsStr::new(HARMLESS)),
        "the child was not given the variable rv is meant to read"
    );
    let workspace = Fixture::new();
    let mut app = workspace.app();
    assert_eq!(
        app.on_key(KeyCode::Char('E')).expect("v"),
        Action::Edit,
        "rv reads some other variable than $EDITOR"
    );
}
