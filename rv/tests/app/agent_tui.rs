//! The agent in the reviewer: the bar's segment, the corner over the bar, the
//! answer popup, `Esc`'s place in line, orientation's once-per-blob rule, and
//! the reviewer with no agent or a missing one.
#![cfg(unix)]

use crossterm::event::KeyCode;
use rv::app::App;

use crate::agent_stub::*;
use crate::support::*;

const WIDTH: u16 = 140;
const HEIGHT: u16 = 30;

/// The row above the bar: the diff pane's bottom border, where the corner is.
fn border_row(app: &App) -> String {
    let text = buffer_text(&frame_at(app, WIDTH, HEIGHT));
    text.lines()
        .nth(usize::from(HEIGHT) - 2)
        .expect("a border row")
        .to_owned()
}

fn bar(app: &App) -> String {
    last_row(&frame_at(app, WIDTH, HEIGHT))
}

#[test]
fn without_an_agent_there_is_no_segment_no_corner_and_no_ask() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    assert_eq!(app.agent_label(), None);
    assert_eq!(app.corner_tip(), None);
    assert!(!bar(&app).contains("agent:"));
    assert!(!border_row(&app).contains("flagged"));
    app.on_key(KeyCode::Char('K')).expect("K");
    assert!(!app.agent_busy());
    assert!(
        app.status().contains("no agent is configured"),
        "{}",
        app.status()
    );
}

#[test]
fn an_absent_binary_is_unavailable_and_raises_nothing() {
    let workspace = Fixture::new();
    let settings = agent_settings(std::path::Path::new("rv-no-such-agent-binary"), "files");
    let mut app = app_configured(&workspace, "", &settings);
    settle_agent(&mut app);
    assert_eq!(app.agent_label().as_deref(), Some("agent: unavailable"));
    assert!(bar(&app).contains("agent: unavailable"), "{}", bar(&app));
    app.on_key(KeyCode::Char('K')).expect("K");
    settle_agent(&mut app);
    assert!(!app.agent_busy());
    assert!(app.alerts().is_empty(), "{:?}", app.alerts());
}

/// The diff pane's first rows, where the thinking tooltip floats.
fn top_rows(app: &App) -> String {
    let text = buffer_text(&frame_at(app, WIDTH, HEIGHT));
    text.lines().take(4).collect::<Vec<_>>().join("\n")
}

#[test]
fn a_running_ask_is_a_tooltip_top_right_and_not_a_bar_segment() {
    let workspace = Fixture::new();
    let stub = Stub::new();
    let mut app = agent_app(&workspace, &stub, "on-demand", "");
    assert!(!top_rows(&app).contains("thinking"), "{}", top_rows(&app));
    stub.hang();
    app.on_key(KeyCode::Char('K')).expect("K");

    let top = top_rows(&app);
    let tooltip = top
        .lines()
        .find(|row| row.contains("maki thinking about"))
        .unwrap_or_else(|| panic!("no thinking tooltip:\n{top}"));
    assert!(tooltip.contains("Esc stop"), "{tooltip}");
    let diff = app.painted_layout().diff;
    let ends_at = tooltip.trim_end().chars().count();
    assert!(
        ends_at > usize::from(diff.right()) - 4,
        "the tooltip is not at the right edge:\n{top}"
    );
    assert_eq!(app.agent_label().as_deref(), Some("agent: maki"));
    assert!(!bar(&app).contains("thinking"), "{}", bar(&app));

    app.on_key(KeyCode::Esc).expect("Esc");
    assert!(!top_rows(&app).contains("thinking"), "{}", top_rows(&app));
}

#[test]
fn esc_stops_an_ask_only_when_nothing_else_answers_it() {
    let workspace = Fixture::new();
    let stub = Stub::new();
    let mut app = agent_app(&workspace, &stub, "on-demand", "");
    select_line(&mut app, |line| line.text.contains("let x"));
    app.on_key(KeyCode::Char('F')).expect("flag");
    type_text(&mut app, "look");
    app.on_key(KeyCode::Enter).expect("save the flag");
    app.on_key(KeyCode::Down).expect("onto the flag row");
    app.on_key(KeyCode::Enter).expect("onto the flag");
    assert_eq!(app.context().name(), "FLAG");
    stub.hang();
    app.on_key(KeyCode::Char('K')).expect("K");
    assert!(app.agent_busy());

    app.on_key(KeyCode::Char('g')).expect("goto leader");
    app.on_key(KeyCode::Esc).expect("Esc closes the menu");
    assert!(app.agent_busy(), "the menu took that Esc");
    app.on_key(KeyCode::Esc).expect("Esc steps off the flag");
    assert_eq!(app.context().name(), "DIFF");
    assert!(app.agent_busy(), "stepping back took that Esc");
    app.on_key(KeyCode::Esc).expect("Esc stops the ask");
    assert!(!app.agent_busy());
    assert!(
        app.status().starts_with("stopped asking"),
        "{}",
        app.status()
    );
    settle_agent(&mut app);
    assert!(app.answer().is_none(), "a stopped ask has no answer");
}

#[test]
fn orientation_asks_once_per_file_and_blob() {
    let workspace = Fixture::new();
    let stub = Stub::new();
    let mut app = agent_app(&workspace, &stub, "files", "");
    assert_eq!(stub.calls(), 1, "the first file is oriented on opening");
    assert!(stub.prompt().contains("`a.rs`"), "{}", stub.prompt());

    app.on_key(KeyCode::Char(']')).expect("next file");
    settle_agent(&mut app);
    assert_eq!(stub.calls(), 2);
    app.on_key(KeyCode::Char('[')).expect("back");
    settle_agent(&mut app);
    app.on_key(KeyCode::Char(']')).expect("again");
    settle_agent(&mut app);
    assert_eq!(stub.calls(), 2, "an unchanged file is never asked twice");

    app.on_key(KeyCode::Char('[')).expect("back to a.rs");
    workspace.write("a.rs", "fn a() {\n    let x = 2;\n}\n");
    workspace.jj(&["describe", "-m", "edit a"]);
    workspace.jj(&["new"]);
    app.on_key(KeyCode::Char('v')).expect("view leader");
    app.on_key(KeyCode::Char('r')).expect("refresh");
    settle_agent(&mut app);
    assert_eq!(stub.calls(), 3, "an edited file is a new question");
}

#[test]
fn the_corner_names_the_purpose_the_count_and_the_keys_as_remapped() {
    let workspace = Fixture::new();
    let stub = Stub::new();
    stub.reply(
        "It defines a.",
        "Defines \u{1b}[31ma\u{1b}[0m.",
        &[("a.rs", 2, "x is unused")],
    );
    let keys = "[leaders]\ngoto = \"G\"\n[keys]\nagent_ask = \"Q\"\n";
    let app = agent_app(&workspace, &stub, "files", keys);
    let corner = border_row(&app);
    for wanted in [
        "Defines a.",
        "✦ 1 flagged",
        "Q ask",
        "G a next flag",
        "a answer",
    ] {
        assert!(corner.contains(wanted), "{wanted:?} missing: {corner}");
    }
    assert!(!corner.contains("K ask"), "{corner}");
    assert!(!corner.contains('\u{1b}'));
}

#[test]
fn a_opens_the_whole_answer_with_its_cost_and_esc_closes_it() {
    let workspace = Fixture::new();
    let stub = Stub::new();
    stub.reply(
        "The first line of prose.\nA second.",
        "Short.",
        &[("a.rs", 2, "x is unused")],
    );
    let mut app = agent_app(&workspace, &stub, "on-demand", "");
    select_line(&mut app, |line| line.text.contains("let x"));
    app.on_key(KeyCode::Char('K')).expect("K");
    settle_agent(&mut app);
    app.on_key(KeyCode::Char('a')).expect("a");
    assert!(app.answer_open());
    let text = buffer_text(&frame_at(&app, WIDTH, HEIGHT));
    for wanted in [
        "Agent on a.rs:2",
        "Short.",
        "The first line of prose.",
        "a.rs:2 high — x is unused",
        "maki · 2.5s · $0.0421",
    ] {
        assert!(text.contains(wanted), "{wanted:?} missing:\n{text}");
    }
    app.on_key(KeyCode::Char('D')).expect("D is inert here");
    assert_eq!(app.mode(), rv::app::Mode::Browse);
    app.on_key(KeyCode::Esc).expect("Esc");
    assert!(!app.answer_open());
}

#[test]
fn the_agent_segment_is_absent_when_empty_and_outlasts_the_status() {
    use rv::statusbar::{Role, View, render, segments};
    let base = View {
        mode: "DIFF",
        file: Some("rv/src/app/agent.rs"),
        scope: "trunk()..@",
        status: "a status long enough to be the first thing a narrow bar gives up",
        ..View::default()
    };
    assert!(
        segments(&base)
            .iter()
            .all(|segment| segment.role != Role::Agent)
    );
    let with_agent = segments(&View {
        agent: "agent: maki".to_owned(),
        ..base.clone()
    });
    let painted: String = render(&with_agent, 80, true)
        .spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect();
    assert!(painted.contains("agent: maki"), "{painted}");
    assert!(!painted.contains("narrow bar"), "{painted}");
}

#[test]
fn k_from_the_file_list_orients_the_file_and_the_corner_remembers_it() {
    let workspace = Fixture::new();
    let stub = Stub::new();
    stub.reply("It defines a.", "Defines a.", &[]);
    let mut app = agent_app(&workspace, &stub, "on-demand", "");
    to_files(&mut app);
    app.on_key(KeyCode::Char('K')).expect("K");
    settle_agent(&mut app);
    assert!(
        stub.prompt().contains("what is this file for"),
        "{}",
        stub.prompt()
    );
    assert!(
        border_row(&app).contains("Defines a."),
        "{}",
        border_row(&app)
    );

    app.on_key(KeyCode::Char(']')).expect("next file");
    settle_agent(&mut app);
    assert!(!border_row(&app).contains("Defines a."));
    app.on_key(KeyCode::Char('[')).expect("back");
    settle_agent(&mut app);
    assert!(
        border_row(&app).contains("Defines a."),
        "{}",
        border_row(&app)
    );
    app.on_key(KeyCode::Char('K')).expect("K again");
    assert!(
        !app.agent_busy(),
        "a cached orientation is shown, not re-asked"
    );
    assert_eq!(stub.calls(), 1);
}
