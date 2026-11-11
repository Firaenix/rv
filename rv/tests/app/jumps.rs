//! `g b`: back along the jumps, across files.

use rv::app::Focus;

use crate::support::*;

#[test]
fn g_b_returns_from_a_definition_in_another_file() {
    let workspace = goto_fixture();
    let mut app = workspace.app();
    show_file(&mut app, "user.rs");
    to_line(&mut app, 2);
    to_word(&mut app, "alpha");
    let column = app.column();

    chord(&mut app, 'g', 'd');
    assert_eq!(app.file_index(), file_named(&app, "model.rs"));
    assert_eq!(app.cursor_line_number(), Some(5));
    assert_eq!(app.jump_depth(), 1);

    chord(&mut app, 'g', 'b');
    assert_eq!(app.file_index(), file_named(&app, "user.rs"));
    assert_eq!(app.cursor_line_number(), Some(2));
    assert_eq!(app.column(), column, "the column cursor did not come back");
    assert_eq!(app.focus(), Focus::Diff);
    assert_eq!(app.jump_depth(), 0);
}

/// The stack walks back through every hop, newest first.
#[test]
fn g_b_walks_back_through_a_chain_of_jumps() {
    let workspace = goto_fixture();
    let mut app = workspace.app();
    show_file(&mut app, "user.rs");
    to_line(&mut app, 2);
    to_word(&mut app, "alpha");
    chord(&mut app, 'g', 'd');
    to_word(&mut app, "Gamma");
    chord(&mut app, 'g', 'd');
    assert_eq!(app.cursor_line_number(), Some(1), "onto struct Gamma");
    assert_eq!(app.jump_depth(), 2);

    chord(&mut app, 'g', 'b');
    assert_eq!(
        (app.file_index(), app.cursor_line_number()),
        (file_named(&app, "model.rs"), Some(5))
    );
    chord(&mut app, 'g', 'b');
    assert_eq!(
        (app.file_index(), app.cursor_line_number()),
        (file_named(&app, "user.rs"), Some(2))
    );
}

#[test]
fn g_b_with_nowhere_to_go_says_so() {
    let workspace = goto_fixture();
    let mut app = workspace.app();
    let place = (app.file_index(), app.cursor_row());

    chord(&mut app, 'g', 'b');

    assert_eq!((app.file_index(), app.cursor_row()), place);
    assert!(app.status().contains("no jump"), "{}", app.status());
}

/// A flag jump is a jump too.
#[test]
fn a_flag_jump_can_be_walked_back() {
    let workspace = goto_fixture();
    let mut app = workspace.app();
    show_file(&mut app, "user.rs");
    to_line(&mut app, 2);
    app.on_key(crossterm::event::KeyCode::Char('F'))
        .expect("flag");
    type_text(&mut app, "look");
    app.on_key(crossterm::event::KeyCode::Enter).expect("save");
    show_file(&mut app, "a.rs");

    chord(&mut app, 'g', 'f');
    assert_eq!(app.file_index(), file_named(&app, "user.rs"));

    chord(&mut app, 'g', 'b');
    assert_eq!(app.file_index(), file_named(&app, "a.rs"));
}
