//! The column cursor: the arrows that walk it, the click that places it, and
//! the search that takes the word it is on.

use crossterm::event::KeyCode;
use rv::app::Focus;
use rv::app::Mode;
use rv::app::SidebarTab;

use crate::support::*;

/// The number field and its space plus the sigil: where a diff line's text
/// starts inside the pane.
const GUTTER: u16 = 7;

/// `←`/`→` in the diff walk the line a character at a time, and at the start
/// of the line `←` stays put: leaving the pane is `Tab`'s job.
#[test]
fn the_arrows_walk_the_column_and_never_leave_the_diff() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    assert_eq!(app.focus(), Focus::Diff, "the diff has focus on launch");
    assert_eq!(app.column(), 0);

    app.on_key(KeyCode::Right).expect("right");
    app.on_key(KeyCode::Right).expect("right");
    assert_eq!(app.column(), 2, "→ did not walk the column");
    assert_eq!(app.focus(), Focus::Diff, "→ left the pane");

    app.on_key(KeyCode::Left).expect("left");
    assert_eq!(app.column(), 1, "← did not walk the column back");
    app.on_key(KeyCode::Left).expect("left");
    assert_eq!((app.column(), app.focus()), (0, Focus::Diff));

    app.on_key(KeyCode::Left).expect("left");
    assert_eq!(
        (app.column(), app.focus()),
        (0, Focus::Diff),
        "← at the line start left the pane"
    );
}

/// Walking the cursor off the right edge of the pane scrolls the line to keep
/// it on screen, and walking back to the start scrolls it home again: the
/// cursor is how the view moves sideways.
#[test]
fn the_view_follows_the_column_cursor_sideways() {
    let workspace = Fixture::wide();
    let mut app = workspace.app();
    let _ = frame_at(&app, 80, 24);
    let diff = app.painted_layout().diff;
    let text_columns = usize::from(diff.width) - 2 - usize::from(GUTTER);

    for _ in 0..text_columns - 1 {
        app.on_key(KeyCode::Right).expect("right");
    }
    assert_eq!(
        app.diff_hscroll(),
        0,
        "the view moved before the cursor left it"
    );

    app.on_key(KeyCode::Right).expect("right off the edge");
    let column = app.column();
    let scrolled = app.diff_hscroll();
    assert!(scrolled > 0, "the view did not follow the cursor");
    // The marker takes one column once scrolled, so the cursor sits last.
    assert_eq!(column, scrolled + text_columns - 2);
    let frame = buffer_text(&frame_at(&app, 80, 24));
    assert!(
        frame.contains("…"),
        "no marker for the clipped head:\n{frame}"
    );

    for _ in 0..column {
        app.on_key(KeyCode::Left).expect("left");
    }
    assert_eq!((app.column(), app.diff_hscroll()), (0, 0));
    assert_eq!(app.focus(), Focus::Diff);
}

/// `Tab` out of the diff is a focus toggle: it must not also reset the sidebar
/// to the files list, which discarded whichever list the reviewer had chosen.
#[test]
fn tab_from_the_diff_keeps_the_tab_the_sidebar_was_left_on() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    app.on_key(KeyCode::Char('m')).expect("mode leader");
    app.on_key(KeyCode::Char('c')).expect("commits");
    assert_eq!(
        (app.focus(), app.sidebar_tab()),
        (Focus::Sidebar, SidebarTab::Commits)
    );

    app.on_key(KeyCode::Char('m')).expect("mode leader");
    app.on_key(KeyCode::Char('d')).expect("diff");
    assert_eq!(app.focus(), Focus::Diff);

    app.on_key(KeyCode::Tab).expect("tab");
    assert_eq!(
        (app.focus(), app.sidebar_tab()),
        (Focus::Sidebar, SidebarTab::Commits),
        "Tab out of the diff changed which list the sidebar shows"
    );

    // From the sidebar it still rotates: that is where mode cycling belongs.
    app.on_key(KeyCode::Tab).expect("tab");
    assert_eq!(app.sidebar_tab(), SidebarTab::Comments);
}

/// A click lands on a character, not just a row: the column it resolves is the
/// one under the pointer, which is what makes `g d` and `*` pointable.
#[test]
fn a_click_places_the_column_on_the_character_under_the_pointer() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    let _ = frame_at(&app, 100, 24);
    let pane = app.painted_layout().diff;

    // `fn a() {` is the first line of the fixture: column 3 is `a`.
    app.on_mouse(click(pane.x + 1 + GUTTER + 3, pane.y + 1))
        .expect("click");

    assert_eq!(app.column(), 3, "the click did not place the column");
    assert_eq!(app.word_under_cursor().as_deref(), Some("a"));
}

/// A mode that writes nothing answers a click like browsing does. A search is
/// typed *about* the diff, so clicking into the diff under it is not a gesture
/// anything can lose.
#[test]
fn a_click_still_moves_the_cursor_while_a_search_is_being_typed() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    app.on_key(KeyCode::Char('/')).expect("search");
    assert_eq!(app.mode(), Mode::Search);

    let _ = frame_at(&app, 100, 24);
    let pane = app.painted_layout().diff;
    app.on_mouse(click(pane.x + 1 + GUTTER + 2, pane.y + 1 + 1))
        .expect("click");

    assert_eq!(app.mode(), Mode::Search, "the click cancelled the search");
    assert_eq!(app.line_index(), 1, "the click did not move the cursor");
    assert_eq!(app.column(), 2, "the click did not place the column");
}

/// `*` takes the word the column cursor is on as the query — the search a
/// reviewer would otherwise retype off the screen.
#[test]
fn star_searches_for_the_word_under_the_cursor() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    let _ = frame_at(&app, 100, 24);
    let pane = app.painted_layout().diff;
    // `    let x = 1;` — the second line, on the `let`.
    app.on_mouse(click(pane.x + 1 + GUTTER + 4, pane.y + 1 + 1))
        .expect("click");
    assert_eq!(app.word_under_cursor().as_deref(), Some("let"));

    app.on_key(KeyCode::Char('*')).expect("star");
    assert_eq!(app.query(), "let", "`*` did not search for the word");
}
