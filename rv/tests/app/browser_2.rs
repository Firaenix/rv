//! Browsing comments in the sidebar.

use crossterm::event::KeyCode;
use ratatui::style::Modifier;
use rv::app::App;

use crate::support::*;

/// The browser's own selection is marked, and only while the sidebar has the
/// focus — the same rule the file list follows.
#[test]
fn the_browsed_row_is_highlighted_when_the_sidebar_has_focus() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, "needs a doc");
    to_comments(&mut app);

    let reversed = |app: &App| {
        let buffer = frame_at(app, 100, 24);
        (0..24).any(|y| (0..30).any(|x| buffer[(x, y)].modifier.contains(Modifier::REVERSED)))
    };

    assert!(!reversed(&app), "the unfocused browser is reversed");
    app.on_key(KeyCode::Left).expect("focus the sidebar");
    assert!(reversed(&app), "the focused browser has no selection");
}

/// The bar drops whole segments rather than cutting one in half, at every
/// width, and the pointer to the keymap is the last thing standing.
///
/// This replaces a test that asserted the opposite — that a status line too
/// long for the terminal ends in `…`. That was true when `app.status()` *was*
/// the bar, and it was the defect: half of `deleted comment at app.rs:42` is a
/// claim about a file that does not exist, and a status that owned the whole
/// row could evict the one in-app pointer to the keys. The bar is segments now
/// (see `rv::statusbar`), so a segment either fits or is dropped whole, and the
/// hint outlives every one of them.
#[test]
fn the_bar_drops_a_segment_whole_rather_than_cutting_a_word() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, "a finding");

    for width in [16u16, 24, 40, 60, 80, 100, 120] {
        let frame = frame_at(&app, width, 24);
        let bar = last_row(&frame);
        assert!(
            !bar.contains('…'),
            "the bar cut a segment in half at {width} columns: {bar:?}"
        );
        assert!(
            bar.contains("? help"),
            "the pointer to the keymap went first at {width} columns: {bar:?}"
        );
        assert!(
            (0..width).all(|x| bg_of(&frame, x, 23).is_some()),
            "the bar left part of the row bare at {width} columns: {bar:?}"
        );
    }
}
