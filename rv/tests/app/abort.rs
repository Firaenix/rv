//! Aborting.


use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyModifiers;
use rv::app::Action;
use rv::app::Mode;

use crate::support::*;

/// In raw mode the terminal raises no SIGINT, so Ctrl+C is `rv`'s to answer —
/// and answering it with the comment box (which is what dropping the modifiers
/// does) leaves a reviewer's reflexive abort typing a note.
#[test]
fn ctrl_c_quits_instead_of_opening_a_comment() {
    let workspace = Fixture::new();
    let mut app = workspace.app();

    let action = app
        .on_key_event(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL))
        .expect("ctrl-c");

    assert_eq!(action, Action::Quit, "ctrl-c aborts the review");
    assert_eq!(
        app.mode(),
        Mode::Browse,
        "and does not open the comment buffer"
    );
}

#[test]
fn a_plain_c_still_opens_a_comment() {
    let workspace = Fixture::new();
    let mut app = workspace.app();

    let action = app
        .on_key_event(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE))
        .expect("c");

    assert_eq!(action, Action::Continue);
    assert_eq!(app.mode(), Mode::Comment, "plain c is unchanged");
}

/// The abort is an abort from anywhere: a half-typed comment is not a state a
/// reviewer has to escape from before they can leave.
#[test]
fn ctrl_c_aborts_from_inside_a_half_typed_comment() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    app.on_key(KeyCode::Char('c')).expect("enter comment mode");
    type_text(&mut app, "half a thought");

    let action = app
        .on_key_event(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL))
        .expect("ctrl-c");

    assert_eq!(action, Action::Quit);
    let comments = workspace.store().comments().expect("read comments.json");
    assert!(
        comments.is_empty(),
        "aborting saved the half-typed comment: {comments:?}"
    );
}
