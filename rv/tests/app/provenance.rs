//! `v p` and `i`: the change that last touched each head-side line.

use std::time::Duration;
use std::time::Instant;

use crossterm::event::KeyCode;
use ratatui::style::Modifier;
use rv::app::App;
use rv::app::JumpSource;
use rv::rows::plans_built;
use rv_core::diff::LineKind;

use crate::browse_1::browse;
use crate::support::*;

/// `a.rs` as [`SOURCE`] wrote it, with its middle line rewritten by a second
/// change on top.
const REWRITTEN: &str = "fn a() {\n    let x = 2;\n}\n";

const WIDTH: u16 = 120;
const HEIGHT: u16 = 30;

/// The id plus its space, ahead of the five-wide number and the sigil.
const GUTTER: u16 = 9;
const LINE_GUTTER: u16 = 7;

fn two_changes() -> Fixture {
    let workspace = Fixture::new();
    workspace.write("a.rs", REWRITTEN);
    workspace.jj(&["describe", "-m", "second change"]);
    workspace.jj(&["new"]);
    workspace
}

/// The first eight characters of `revision`'s change id.
fn short_id(workspace: &Fixture, revision: &str) -> String {
    let id = workspace.jj(&[
        "log",
        "-r",
        revision,
        "--no-graph",
        "-T",
        "change_id.short(8)",
    ]);
    id.trim().to_owned()
}

/// The gutter cells of the first diff pane row holding `text`.
fn gutter_of(app: &App, text: &str) -> String {
    let buffer = frame_at(app, WIDTH, HEIGHT);
    let pane = inner(app.painted_layout().diff);
    let row = (pane.y..pane.bottom())
        .map(|y| row_in(&buffer, pane, y))
        .find(|row| row.contains(text))
        .unwrap_or_else(|| panic!("no row holds {text:?}:\n{}", buffer_text(&buffer)));
    row.chars().take(usize::from(GUTTER)).collect()
}

fn toggle(app: &mut App) {
    chord(app, 'v', 'p');
}

#[test]
fn the_gutter_is_off_until_v_p() {
    let workspace = two_changes();
    let mut app = workspace.app_from("@--");
    assert!(!app.provenance_on());
    assert!(
        gutter_of(&app, "fn a() {").starts_with("    1"),
        "the row starts with its line number"
    );

    toggle(&mut app);
    assert!(app.provenance_on());
    toggle(&mut app);
    assert!(!app.provenance_on());
}

#[test]
fn the_gutter_names_the_change_behind_each_head_line() {
    let workspace = two_changes();
    let first = short_id(&workspace, "@--");
    let second = short_id(&workspace, "@-");
    let mut app = workspace.app_from("@--");
    toggle(&mut app);
    app.finish_provenance();

    assert_eq!(gutter_of(&app, "fn a() {").trim(), first);
    assert_eq!(gutter_of(&app, "let x = 2;").trim(), second);
    assert_eq!(
        gutter_of(&app, "let x = 1;").trim(),
        "",
        "a removed line has no head-side origin"
    );
    assert_eq!(
        gutter_of(&app, "}").trim(),
        first,
        "a run broken by another change is named again"
    );
}

#[test]
fn a_run_of_one_change_is_named_once() {
    let workspace = Fixture::new();
    let first = short_id(&workspace, "@-");
    let mut app = browse(&workspace, None);
    toggle(&mut app);
    app.finish_provenance();

    assert_eq!(gutter_of(&app, "fn a() {").trim(), first);
    assert_eq!(gutter_of(&app, "let x = 1;").trim(), "");
    assert_eq!(gutter_of(&app, "}").trim(), "");
}

#[test]
fn a_tree_review_annotates_its_whole_file() {
    let workspace = two_changes();
    let first = short_id(&workspace, "@--");
    let second = short_id(&workspace, "@-");
    let mut app = browse(&workspace, None);
    toggle(&mut app);
    app.finish_provenance();

    assert_eq!(gutter_of(&app, "fn a() {").trim(), first);
    assert_eq!(gutter_of(&app, "let x = 2;").trim(), second);
    assert_eq!(gutter_of(&app, "}").trim(), first);
}

/// The event loop's job, done here: poll until the worker's answer lands,
/// with the gutter saying it is waiting in the meantime.
#[test]
fn the_annotation_lands_in_the_background() {
    let workspace = two_changes();
    let second = short_id(&workspace, "@-");
    let mut app = workspace.app_from("@--");
    toggle(&mut app);
    assert!(app.provenance_busy(), "v p asked for the shown file");
    assert_eq!(gutter_of(&app, "let x = 2;").trim(), "…");

    let deadline = Instant::now() + Duration::from_secs(60);
    while !app.collect_provenance() {
        assert!(Instant::now() < deadline, "the annotation never landed");
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(!app.provenance_busy());
    assert_eq!(gutter_of(&app, "let x = 2;").trim(), second);
}

#[test]
fn i_names_the_origin_of_the_line_under_the_cursor() {
    let workspace = two_changes();
    let second = short_id(&workspace, "@-");
    let mut app = workspace.app_from("@--");
    select_line(&mut app, |line| line.kind == LineKind::Added);

    app.on_key(KeyCode::Char('i')).expect("i");
    assert!(app.hover().is_none(), "the card waits for the annotation");
    assert!(
        app.status().starts_with("tracing who wrote a.rs"),
        "{}",
        app.status()
    );
    app.finish_provenance();

    let card = app
        .hover()
        .expect("the card opened when the annotation landed")
        .clone();
    assert_eq!(card.name, format!("change {second}"));
    assert_eq!(card.source, JumpSource::Annotation);
    assert_eq!(card.server_text[0], "second change");
    assert!(
        card.server_text[1].starts_with("rv-test · "),
        "{:?}",
        card.server_text
    );
    assert!(
        card.server_text[1].contains(" UTC ("),
        "{:?}",
        card.server_text
    );
    let text = buffer_text(&frame_at(&app, WIDTH, HEIGHT));
    assert!(text.contains("second change"), "{text}");
    assert!(!app.provenance_on(), "i does not turn the gutter on");

    app.on_key(KeyCode::Esc).expect("esc");
    assert!(app.hover().is_none());
    app.on_key(KeyCode::Char('i')).expect("i again");
    assert_eq!(
        app.hover().map(|card| card.name.clone()),
        Some(format!("change {second}")),
        "a cached annotation answers at once"
    );
}

#[test]
fn i_on_a_removed_line_says_it_has_no_origin() {
    let workspace = two_changes();
    let mut app = workspace.app_from("@--");
    select_line(&mut app, |line| line.kind == LineKind::Removed);

    app.on_key(KeyCode::Char('i')).expect("i");

    assert!(app.hover().is_none());
    assert!(app.status().contains("no origin"), "{}", app.status());
}

#[test]
fn a_click_lands_on_the_character_under_the_pointer_with_the_gutter_on() {
    let workspace = two_changes();
    let mut app = workspace.app_from("@--");
    toggle(&mut app);
    app.finish_provenance();
    let _ = frame_at(&app, WIDTH, HEIGHT);
    let pane = app.painted_layout().diff;

    // `fn a() {` is the first row: column 3 is `a`.
    let x = pane.x + 1 + GUTTER + LINE_GUTTER + 3;
    app.on_mouse(click(x, pane.y + 1)).expect("click");

    assert_eq!(app.column(), 3, "the click did not place the column");
    assert_eq!(app.word_under_cursor().as_deref(), Some("a"));
    let buffer = frame_at(&app, WIDTH, HEIGHT);
    let cell = &buffer[(x, pane.y + 1)];
    assert_eq!(cell.symbol(), "a");
    assert!(
        cell.style().add_modifier.contains(Modifier::BOLD),
        "the column cursor's mark is not on the `a`"
    );
    assert!(
        !buffer[(x - 1, pane.y + 1)]
            .style()
            .add_modifier
            .contains(Modifier::BOLD),
        "the mark spilled left of the word"
    );
}

#[test]
fn a_frame_with_the_gutter_builds_no_more_plans() {
    let workspace = two_changes();
    let mut app = workspace.app_from("@--");
    let frame_plans = |app: &App| {
        let before = plans_built();
        let _ = frame_at(app, WIDTH, HEIGHT);
        plans_built() - before
    };
    let without = frame_plans(&app);
    toggle(&mut app);
    app.finish_provenance();
    assert_eq!(frame_plans(&app), without);
}
