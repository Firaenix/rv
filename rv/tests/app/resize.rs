//! Resizing the panes.

use crossterm::event::KeyCode;
use rv::layout::Split;

use crate::support::*;

#[test]
fn angle_brackets_resize_the_panes() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    let start = app.split().ratio();
    assert_eq!(
        start,
        Split::DEFAULT,
        "a reviewer opens on the default split"
    );

    app.on_key(KeyCode::Char('v')).expect("view leader");
    app.on_key(KeyCode::Char('>')).expect(">");
    assert!(
        app.split().ratio() > start,
        "the sidebar grew: {}",
        app.split().ratio()
    );
    app.on_key(KeyCode::Char('v')).expect("view leader");
    app.on_key(KeyCode::Char('<')).expect("<");
    assert_eq!(app.split().ratio(), start, "and shrank back");
}

#[test]
fn resizing_never_leaves_the_bounds_however_long_you_hold_it() {
    let workspace = Fixture::new();
    let mut app = workspace.app();

    for _ in 0..200 {
        app.on_key(KeyCode::Char('v')).expect("view leader");
        app.on_key(KeyCode::Char('>')).expect(">");
    }
    assert!(
        app.split().ratio() <= Split::MAX_RATIO,
        "held past the right bound: {}",
        app.split().ratio()
    );
    for _ in 0..400 {
        app.on_key(KeyCode::Char('v')).expect("view leader");
        app.on_key(KeyCode::Char('<')).expect("<");
    }
    assert!(
        app.split().ratio() >= Split::MIN_RATIO,
        "held past the left bound: {}",
        app.split().ratio()
    );
}

/// The accessor is not the point — the pane on screen is. A resize that moved
/// `App::split` without moving the divider would pass every test above.
#[test]
fn a_resized_pane_actually_renders_at_its_new_width() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    let before = frame_at(&app, 100, 24);

    for _ in 0..5 {
        app.on_key(KeyCode::Char('v')).expect("view leader");
        app.on_key(KeyCode::Char('>')).expect(">");
    }
    let after = frame_at(&app, 100, 24);
    assert_ne!(before, after, "the frame does not reflect the resize");

    // ...and the divider is where `layout` says it is for the *new* split, not
    // the old one: the two panes' borders moved with it.
    let divider = areas(100, 24, app.split()).divider.x;
    assert!(
        divider > areas(100, 24, Split::default()).divider.x,
        "the divider did not move: {divider}"
    );
    assert_eq!(
        after[(divider - 1, 1)].symbol(),
        "│",
        "the sidebar's right border is not against the divider:\n{}",
        buffer_text(&after)
    );
    assert_eq!(
        after[(divider + 1, 1)].symbol(),
        "│",
        "the diff's left border is not against the divider:\n{}",
        buffer_text(&after)
    );
}

/// A split is a view preference, like folding: it never reaches `.review/`, and
/// a reviewer who reopens the review gets the default back.
#[test]
fn the_split_is_not_written_anywhere() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    let before = workspace_tree(workspace.root());
    assert!(!before.is_empty(), "the review wrote nothing to compare");

    for _ in 0..3 {
        app.on_key(KeyCode::Char('v')).expect("view leader");
        app.on_key(KeyCode::Char('>')).expect(">");
    }
    assert_ne!(app.split().ratio(), Split::DEFAULT, "nothing was resized");
    assert_eq!(
        workspace_tree(workspace.root()),
        before,
        "resizing wrote to the workspace; it is a view preference, not review state"
    );

    let reopened = workspace.app();
    assert_eq!(
        reopened.split().ratio(),
        Split::DEFAULT,
        "the split survived the session it was a preference of"
    );
}
