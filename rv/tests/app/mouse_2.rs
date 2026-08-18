//! The mouse.


use crossterm::event::KeyCode;
use rv::app::Mode;

use crate::support::*;

/// The mouse is inert while a comment is being typed.
///
/// A click that moved the selection under a half-typed comment would save that
/// comment against a line the reviewer never chose — the same silent re-aiming
/// the wheel is kept away from, with a body attached.
#[test]
fn the_mouse_is_inert_while_a_comment_is_being_typed() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    let row = diff_pane_row(&app, 100, 24, 2);
    let sidebar = sidebar_pane_row(&app, 100, 24, 1);
    let divider = divider_column(&app, 100, 24);

    app.on_key(KeyCode::Char('c')).expect("open the box");
    let before = (
        app.focus(),
        app.line_index(),
        app.file_index(),
        app.split().ratio(),
    );

    for event in [
        click(60, row),
        click(3, sidebar),
        scroll_down(60, row),
        press(divider, 6),
        drag(divider + 10, 6),
        release(divider + 10, 6),
    ] {
        app.on_mouse(event).expect("gesture");
    }

    assert_eq!(app.mode(), Mode::Comment, "a gesture left the comment box");
    assert_eq!(
        (
            app.focus(),
            app.line_index(),
            app.file_index(),
            app.split().ratio()
        ),
        before,
        "a gesture moved something under a half-typed comment"
    );
}

/// While the `?` popup is up the pointer moves nothing under it, and the wheel
/// scrolls the keymap exactly as `j` and `k` do.
#[test]
fn the_mouse_is_inert_while_the_help_is_open() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    let row = diff_pane_row(&app, 100, 24, 2);
    let sidebar = sidebar_pane_row(&app, 100, 24, 1);

    app.on_key(KeyCode::Char('?')).expect("?");
    let before = (app.focus(), app.line_index(), app.file_index());

    app.on_mouse(click(50, 12)).expect("click inside the popup");
    app.on_mouse(click(60, row)).expect("click behind it");
    app.on_mouse(click(3, sidebar)).expect("click beside it");
    assert!(app.help_open(), "a click closed the keymap");
    assert_eq!(
        (app.focus(), app.line_index(), app.file_index()),
        before,
        "a click reached through the keymap"
    );

    app.on_mouse(scroll_down(50, 12))
        .expect("scroll the keymap");
    assert_eq!(app.help_scroll(), 1, "the wheel scrolls the keymap");
    app.on_mouse(scroll_up(50, 12)).expect("scroll back");
    assert_eq!(app.help_scroll(), 0);
}
