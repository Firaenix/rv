//! The agent's focus lines as flags in the reviewer: filed by `agent`, walked
//! with `g a` / `g A`, never blocking `--check`, and never filed twice.
#![cfg(unix)]

use std::process::Command;

use crossterm::event::KeyCode;
use rv::app::App;

use crate::agent_stub::*;
use crate::support::*;

fn ask(app: &mut App) {
    app.on_key(KeyCode::Char('K')).expect("K");
    assert!(app.agent_busy(), "K started an ask: {}", app.status());
    settle_agent(app);
}

fn goto(app: &mut App, key: char) -> (String, Option<u32>) {
    app.on_key(KeyCode::Char('g')).expect("goto leader");
    app.on_key(KeyCode::Char(key)).expect("goto child");
    (
        app.selected_file().expect("a file").path.clone(),
        app.cursor_line_number(),
    )
}

#[test]
fn focus_lines_become_agent_flags_that_g_a_walks() {
    let workspace = Fixture::new();
    let stub = Stub::new();
    stub.reply(
        "Two things.",
        "Two things.",
        &[("a.rs", 2, "x is unused"), ("b.rs", 3, "z shadows")],
    );
    let mut app = agent_app(&workspace, &stub, "on-demand", "");
    assert_eq!(stub.calls(), 0, "on-demand asks nothing by itself");

    select_line(&mut app, |line| line.text.contains("let x"));
    app.on_key(KeyCode::Char('F')).expect("flag");
    type_text(&mut app, "mine");
    app.on_key(KeyCode::Enter).expect("save the flag");
    ask(&mut app);

    let mut stored = stored_agent_flags(&workspace);
    stored.sort();
    assert_eq!(
        stored,
        [
            ("a.rs".to_owned(), 2, "x is unused".to_owned()),
            ("b.rs".to_owned(), 3, "z shadows".to_owned()),
        ]
    );
    assert_eq!(app.flags().len(), 3, "the app re-read its flags");
    assert!(
        stub.prompt().contains("line 2 of `a.rs`"),
        "{}",
        stub.prompt()
    );

    to_top(&mut app);
    assert_eq!(goto(&mut app, 'a'), ("a.rs".to_owned(), Some(2)));
    assert_eq!(goto(&mut app, 'a'), ("b.rs".to_owned(), Some(3)));
    assert_eq!(
        goto(&mut app, 'a'),
        ("a.rs".to_owned(), Some(2)),
        "wraps round"
    );
    assert_eq!(goto(&mut app, 'A'), ("b.rs".to_owned(), Some(3)));
    let text = buffer_text(&frame_at(&app, 120, 30));
    assert!(
        text.contains("✦ z shadows"),
        "agent flags read as the agent's:\n{text}"
    );
}

#[test]
fn g_a_says_so_when_the_agent_flagged_nothing() {
    let workspace = Fixture::new();
    let stub = Stub::new();
    let mut app = agent_app(&workspace, &stub, "on-demand", "");
    goto(&mut app, 'a');
    assert_eq!(app.status(), "no agent flags in this review");
}

#[test]
fn open_agent_flags_do_not_fail_the_check() {
    let workspace = Fixture::new();
    let stub = Stub::new();
    stub.reply("Start here.", "Start here.", &[("a.rs", 1, "start here")]);
    let mut app = agent_app(&workspace, &stub, "on-demand", "");
    ask(&mut app);
    assert_eq!(stored_agent_flags(&workspace).len(), 1);

    let config_home = tempfile::tempdir().expect("config home");
    let check = Command::new(env!("CARGO_BIN_EXE_rv"))
        .args(["status", "--check"])
        .current_dir(workspace.root())
        .env("XDG_CONFIG_HOME", config_home.path())
        .output()
        .expect("run rv");
    assert!(
        check.status.success(),
        "{}",
        String::from_utf8_lossy(&check.stderr)
    );
}

#[test]
fn a_re_ask_after_an_edit_files_no_duplicate() {
    let workspace = Fixture::new();
    let stub = Stub::new();
    stub.reply("Unused.", "Unused.", &[("a.rs", 2, "x is unused")]);
    let mut app = agent_app(&workspace, &stub, "on-demand", "");
    select_line(&mut app, |line| line.text.contains("let x"));
    ask(&mut app);
    assert_eq!(stored_agent_flags(&workspace).len(), 1);

    workspace.write("a.rs", "// one\n// two\nfn a() {\n    let x = 1;\n}\n");
    workspace.jj(&["describe", "-m", "two lines above"]);
    workspace.jj(&["new"]);
    app.on_key(KeyCode::Char('v')).expect("view leader");
    app.on_key(KeyCode::Char('r')).expect("refresh");
    app.finish_loading();
    app.finish_merging();
    stub.reply("Unused.", "Unused.", &[("a.rs", 4, "x is unused")]);
    select_line(&mut app, |line| line.text.contains("let x"));
    ask(&mut app);

    assert_eq!(
        stored_agent_flags(&workspace),
        [("a.rs".to_owned(), 2, "x is unused".to_owned())],
        "the moved flag is the same flag"
    );
    assert!(
        app.status().contains("1 already flagged"),
        "{}",
        app.status()
    );
}
