//! Which pane the keys act on.


use crossterm::event::KeyCode;
use rstest::rstest;
use rv::app::Focus;

use crate::support::*;

#[test]
fn left_and_right_move_focus_between_the_panes() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    assert_eq!(app.focus(), Focus::Diff, "the diff has focus on launch");

    app.on_key(KeyCode::Left).expect("left");
    assert_eq!(app.focus(), Focus::Sidebar);
    app.on_key(KeyCode::Left).expect("left again");
    assert_eq!(
        app.focus(),
        Focus::Sidebar,
        "there is nothing left of the files"
    );

    app.on_key(KeyCode::Right).expect("right");
    assert_eq!(app.focus(), Focus::Diff);
    app.on_key(KeyCode::Right).expect("right again");
    assert_eq!(
        app.focus(),
        Focus::Diff,
        "there is nothing right of the diff"
    );
}

#[rstest]
#[case(KeyCode::Char('j'), KeyCode::Char('k'))]
#[case(KeyCode::Down, KeyCode::Up)]
fn with_the_files_focused_both_key_pairs_move_the_file_selection(
    #[case] forward: KeyCode,
    #[case] back: KeyCode,
) {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    app.on_key(KeyCode::Left).expect("focus files");

    app.on_key(forward).expect("forward");
    assert_eq!(app.file_index(), 1, "moved to the second file");
    app.on_key(back).expect("back");
    assert_eq!(app.file_index(), 0, "and back to the first");
    app.on_key(back).expect("back off the top");
    assert_eq!(app.file_index(), 0, "and stays there");
}

#[rstest]
#[case(KeyCode::Char('j'), KeyCode::Char('k'))]
#[case(KeyCode::Down, KeyCode::Up)]
fn with_the_diff_focused_both_key_pairs_move_the_line(
    #[case] forward: KeyCode,
    #[case] back: KeyCode,
) {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    assert_eq!(app.focus(), Focus::Diff);

    app.on_key(forward).expect("forward");
    assert_eq!(app.line_index(), 1);
    assert_eq!(app.file_index(), 0, "the file list did not move with it");
    app.on_key(back).expect("back");
    assert_eq!(app.line_index(), 0);
}

/// Stepping to the next file and back is how a reviewer compares two files, and
/// it used to cost them their place in the first one: `]` `[` dropped the
/// highlight back to line 1 every time.
#[test]
fn leaving_a_file_and_coming_back_keeps_your_place() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    app.on_key(KeyCode::Char('j')).expect("down");
    app.on_key(KeyCode::Char('j')).expect("down");
    let was = app.line_index();
    assert!(was > 0, "the fixture has enough lines to move");

    app.on_key(KeyCode::Char(']')).expect("next file");
    assert_eq!(
        app.line_index(),
        0,
        "a file being opened for the first time opens at its top"
    );
    app.on_key(KeyCode::Char('[')).expect("back");

    assert_eq!(app.line_index(), was, "the line came back with the file");
}

/// Each file remembers its own place, not one place shared between them.
#[test]
fn each_file_keeps_its_own_place() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    app.on_key(KeyCode::Char('j')).expect("down");
    app.on_key(KeyCode::Char(']')).expect("next file");
    app.on_key(KeyCode::Char('j')).expect("down");
    app.on_key(KeyCode::Char('j')).expect("down");
    assert_eq!(app.line_index(), 2, "the second file is two lines down");

    app.on_key(KeyCode::Char('[')).expect("back");
    assert_eq!(app.line_index(), 1, "the first file is one line down");
    app.on_key(KeyCode::Char(']')).expect("forward again");
    assert_eq!(app.line_index(), 2, "and the second is still two");
}

#[test]
fn file_navigation_keys_work_from_either_pane() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    app.on_key(KeyCode::Char(']')).expect("next file");
    assert_eq!(app.file_index(), 1);
    app.on_key(KeyCode::Left).expect("focus files");
    app.on_key(KeyCode::Char('[')).expect("previous file");
    assert_eq!(app.file_index(), 0);
}

#[test]
fn frame_renders_file_list_and_diff() {
    let workspace = Fixture::new();
    let app = workspace.app();

    let rendered = render(&app);

    assert!(rendered.contains("a.rs"), "{rendered}");
    assert!(rendered.contains("let x = 1;"), "{rendered}");
}
