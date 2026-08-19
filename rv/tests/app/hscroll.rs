//! Scrolling the panes sideways: `H`, `L`, and the wheel's other axis.

use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyModifiers;
use rv::layout::Split;

use crate::support::*;

#[test]
fn l_scrolls_the_diff_sideways_and_h_scrolls_back() {
    let workspace = Fixture::wide();
    let app = {
        let mut app = workspace.app();
        app.on_key(KeyCode::Char('L')).expect("L");
        app
    };
    assert!(app.diff_hscroll() > 0, "L did not move the view");

    let frame = buffer_text(&frame_at(&app, 80, 24));
    // Eight columns scroll off `// abcde…`, and the marker leads what is left.
    assert!(
        frame.contains("…fghij"),
        "the scrolled line does not lead with the marker:\n{frame}"
    );
    assert!(
        !frame.contains("// abcdefghij"),
        "the head of the line is still on screen after scrolling:\n{frame}"
    );

    let mut app = app;
    app.on_key(KeyCode::Char('H')).expect("H");
    assert_eq!(app.diff_hscroll(), 0, "H did not scroll back");
    let frame = buffer_text(&frame_at(&app, 80, 24));
    assert!(
        frame.contains("// abcdefghij"),
        "the head of the line did not come back:\n{frame}"
    );
}

#[test]
fn a_line_the_scroll_has_passed_entirely_is_left_blank() {
    let workspace = Fixture::wide();
    let mut app = workspace.app();
    // `fn wide() {}` is 13 columns; two presses put the view past its end.
    app.on_key(KeyCode::Char('L')).expect("L");
    app.on_key(KeyCode::Char('L')).expect("L");
    let frame = buffer_text(&frame_at(&app, 80, 24));
    assert!(
        !frame.contains("wide()"),
        "the short line's text survived a scroll past its end:\n{frame}"
    );
}

#[test]
fn the_sideways_scroll_resets_when_another_file_is_selected() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    app.on_key(KeyCode::Char('L')).expect("L");
    assert!(app.diff_hscroll() > 0);
    app.on_key(KeyCode::Char(']')).expect("next file");
    assert_eq!(
        app.diff_hscroll(),
        0,
        "a scroll chosen for one file's lines followed the reviewer to the next"
    );
}

#[test]
fn l_in_the_sidebar_shifts_the_names_and_h_shifts_back() {
    let workspace = Fixture::nested();
    let mut app = workspace.app();
    app.on_key(KeyCode::Left).expect("focus the sidebar");
    app.on_key(KeyCode::Char('L')).expect("L");
    assert!(app.sidebar_hscroll() > 0, "L did not move the sidebar");

    let text = sidebar_text(&frame_at(&app, 100, 24), 100, 24, Split::default());
    assert!(
        text.contains('…'),
        "a shifted name does not say it is shifted:\n{text}"
    );
    assert!(
        !text.contains("top.rs"),
        "the head of the name is still on screen after scrolling:\n{text}"
    );

    app.on_key(KeyCode::Char('H')).expect("H");
    assert_eq!(app.sidebar_hscroll(), 0);
    let text = sidebar_text(&frame_at(&app, 100, 24), 100, 24, Split::default());
    assert!(
        text.contains("top.rs"),
        "the names did not come back:\n{text}"
    );
}

#[test]
fn the_diff_and_the_sidebar_scroll_independently() {
    let workspace = Fixture::nested();
    let mut app = workspace.app();
    app.on_key(KeyCode::Char('L')).expect("L in the diff");
    app.on_key(KeyCode::Left).expect("focus the sidebar");
    app.on_key(KeyCode::Char('L')).expect("L in the sidebar");
    app.on_key(KeyCode::Char('H')).expect("H in the sidebar");
    assert!(
        app.diff_hscroll() > 0,
        "the sidebar's H took the diff's scroll with it"
    );
    assert_eq!(app.sidebar_hscroll(), 0);
}

#[test]
fn shift_arrows_scroll_sideways_instead_of_moving_the_focus() {
    let workspace = Fixture::wide();
    let mut app = workspace.app();
    let focus = app.focus();

    app.on_key_event(KeyEvent::new(KeyCode::Right, KeyModifiers::SHIFT))
        .expect("shift+right");
    assert!(app.diff_hscroll() > 0, "shift+right did not scroll");
    assert_eq!(app.focus(), focus, "shift+right moved the focus instead");

    app.on_key_event(KeyEvent::new(KeyCode::Left, KeyModifiers::SHIFT))
        .expect("shift+left");
    assert_eq!(app.diff_hscroll(), 0, "shift+left did not scroll back");
    assert_eq!(app.focus(), focus, "shift+left moved the focus instead");
}
