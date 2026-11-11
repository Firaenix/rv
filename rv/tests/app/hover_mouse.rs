//! `hover = true`: the pointer resting on a name opens its card.

use std::time::Duration;

use crossterm::event::KeyCode;
use crossterm::event::MouseEventKind;
use rv::app::App;

use crate::support::*;

const WIDTH: u16 = 120;
const HEIGHT: u16 = 30;

fn hovering() -> rv::config::Settings {
    rv::config::Settings {
        hover: Some(true),
        ..Default::default()
    }
}

/// The screen cell over the first character of `word` on source line `number`
/// of `model.rs`, with the cursor left on line 1 so the card cannot be about
/// the cursor's word.
fn cell_over(app: &mut App, number: u32, word: &str) -> (u16, u16) {
    show_file(app, "model.rs");
    to_line(app, number);
    let row = u16::try_from(app.cursor_row()).expect("a small row");
    let at = app
        .selected_line()
        .expect("a line")
        .text
        .find(word)
        .expect("the word is on the line");
    to_line(app, 1);
    let y = diff_pane_row(app, WIDTH, HEIGHT, row);
    let x = areas(WIDTH, HEIGHT, app.split()).diff.x + 8 + u16::try_from(at).expect("narrow");
    (x, y)
}

fn rest_on(app: &mut App, (x, y): (u16, u16)) {
    app.on_mouse(mouse(MouseEventKind::Moved, x, y))
        .expect("move");
    let t0 = epoch();
    app.dwell(t0);
    app.dwell(t0 + Duration::from_millis(100));
    assert!(app.hover().is_none(), "the card opened before the dwell");
    app.dwell(t0 + Duration::from_secs(1));
}

#[test]
fn resting_on_a_name_opens_its_card() {
    let workspace = goto_fixture();
    let mut app = app_configured(&workspace, "", &hovering());
    let cell = cell_over(&mut app, 5, "Gamma");

    rest_on(&mut app, cell);

    let card = app.hover().expect("the dwell opened a card");
    assert_eq!(card.name, "Gamma");
    assert_eq!(
        app.cursor_line_number(),
        Some(1),
        "hovering moved the cursor"
    );
    let _ = frame_at(&app, WIDTH, HEIGHT);
    let placed = app.painted_layout().hover.expect("laid out");
    assert!(placed.y > cell.1, "the card covers the row it describes");
}

#[test]
fn moving_off_closes_a_pointer_card() {
    let workspace = goto_fixture();
    let mut app = app_configured(&workspace, "", &hovering());
    let cell = cell_over(&mut app, 5, "Gamma");
    rest_on(&mut app, cell);
    assert!(app.hover().is_some());

    app.on_mouse(mouse(MouseEventKind::Moved, cell.0 + 30, cell.1))
        .expect("move away");

    assert!(app.hover().is_none());
}

#[test]
fn the_pointer_opens_nothing_unless_config_asks() {
    let workspace = goto_fixture();
    let mut app = workspace.app();
    let cell = cell_over(&mut app, 5, "Gamma");
    app.on_mouse(mouse(MouseEventKind::Moved, cell.0, cell.1))
        .expect("move");
    app.dwell(epoch());
    app.dwell(epoch() + Duration::from_secs(2));
    assert!(app.hover().is_none());
    assert_eq!(app.dwell_deadline(epoch()), None, "an idle loop was woken");
}

/// A comment being typed is never interrupted by a card.
#[test]
fn the_pointer_opens_nothing_while_a_comment_is_typed() {
    let workspace = goto_fixture();
    let mut app = app_configured(&workspace, "", &hovering());
    let cell = cell_over(&mut app, 5, "Gamma");
    app.on_key(KeyCode::Char('C')).expect("start a comment");

    app.on_mouse(mouse(MouseEventKind::Moved, cell.0, cell.1))
        .expect("move");
    app.dwell(epoch());
    app.dwell(epoch() + Duration::from_secs(2));

    assert!(app.hover().is_none());
}

#[test]
fn resting_on_whitespace_opens_nothing() {
    let workspace = goto_fixture();
    let mut app = app_configured(&workspace, "", &hovering());
    let (x, y) = cell_over(&mut app, 5, "Gamma");
    rest_on(&mut app, (x + 40, y));
    assert!(app.hover().is_none());
}
