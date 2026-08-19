//! Zooming the sidebar into a directory or a change, and back out.

use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyModifiers;
use rv::layout::Split;

use crate::support::*;

/// A nested workspace with the sidebar focused and drawn as a tree, which is
/// where a zoom has something to zoom into.
fn tree_sidebar() -> (Fixture, rv::app::App) {
    let workspace = Fixture::nested();
    let mut app = workspace.app();
    app.on_key(KeyCode::Char('t')).expect("tree view");
    app.on_key(KeyCode::Left).expect("focus the sidebar");
    // The cursor opens on the selected file's row; the directory above it is
    // the thing to zoom into.
    app.on_key(KeyCode::Up).expect("onto the directory row");
    (workspace, app)
}

#[test]
fn enter_zooms_into_a_directory_and_esc_backs_out() {
    let (_workspace, mut app) = tree_sidebar();
    let before = sidebar_text(&frame_at(&app, 100, 24), 100, 24, Split::default());
    assert!(
        before.contains("docs/specs") && before.contains("top.rs"),
        "the tree shows the whole review before any zoom:\n{before}"
    );

    app.on_key(KeyCode::Enter).expect("zoom in");
    let zoomed = sidebar_text(&frame_at(&app, 100, 24), 100, 24, Split::default());
    assert!(
        zoomed.contains("▴") && zoomed.contains("docs/specs"),
        "the view leads with the Up row naming where the reviewer is:\n{zoomed}"
    );
    assert!(
        zoomed.contains("a.md") && zoomed.contains("b.md"),
        "the directory's files are the view:\n{zoomed}"
    );
    assert!(
        !zoomed.contains("top.rs"),
        "nothing outside the zoom is drawn:\n{zoomed}"
    );

    app.on_key(KeyCode::Esc).expect("back out");
    let back = sidebar_text(&frame_at(&app, 100, 24), 100, 24, Split::default());
    assert!(
        back.contains("top.rs"),
        "esc did not lead back out:\n{back}"
    );
    // The cursor is left on the directory that was zoomed into: the reviewer is
    // at that directory, looking at it from outside now.
    app.on_key(KeyCode::Enter).expect("zoom straight back in");
    let again = sidebar_text(&frame_at(&app, 100, 24), 100, 24, Split::default());
    assert!(
        again.contains("▴") && !again.contains("top.rs"),
        "the cursor was not left on the row it came from:\n{again}"
    );
}

#[test]
fn enter_on_the_up_row_also_backs_out() {
    let (_workspace, mut app) = tree_sidebar();
    app.on_key(KeyCode::Enter).expect("zoom in");
    // The Up row is the first row of a zoomed view and the zoom leaves the
    // cursor on it.
    app.on_key(KeyCode::Enter).expect("enter on the Up row");
    let text = sidebar_text(&frame_at(&app, 100, 24), 100, 24, Split::default());
    assert!(
        text.contains("top.rs"),
        "enter on ▴ did not back out:\n{text}"
    );
}

#[test]
fn space_still_folds_rather_than_zooming() {
    let (_workspace, mut app) = tree_sidebar();
    app.on_key(KeyCode::Char(' ')).expect("fold the directory");
    let text = sidebar_text(&frame_at(&app, 100, 24), 100, 24, Split::default());
    assert!(
        !text.contains("a.md") && !text.contains("▴"),
        "space zoomed or failed to fold:\n{text}"
    );
    assert!(
        text.contains("top.rs"),
        "the rest of the tree is still the view:\n{text}"
    );
}

#[test]
fn enter_zooms_into_a_change_in_the_commits_tab() {
    let workspace = Fixture::nested();
    let mut app = workspace.app();
    app.on_key(KeyCode::Left).expect("focus the sidebar");
    app.on_key(KeyCode::Tab).expect("the commits tab");
    // Tab-entry parks the cursor on the selected file's row; the walk down
    // starts at the top. The stack lists newest first and `@` is an empty,
    // undescribed change; the described one is the row beneath it.
    to_top(&mut app);
    while !matches!(
        app.commit_nodes().get(app.sidebar_row()).map(|n| &n.kind),
        Some(rv::tree::NodeKind::Commit { .. })
    ) || !matches!(
        app.commit_nodes()
            .get(app.sidebar_row() + 1)
            .map(|n| &n.kind),
        Some(rv::tree::NodeKind::File { .. })
    ) {
        app.on_key(KeyCode::Down).expect("next row");
    }

    app.on_key(KeyCode::Enter).expect("zoom into the change");
    let text = sidebar_text(&frame_at(&app, 100, 24), 100, 24, Split::default());
    assert!(
        text.contains("▴"),
        "the change did not become the view:\n{text}"
    );
    // The Up row names the change — id and as much of the subject as the pane
    // can hold.
    assert!(
        text.contains("a chang"),
        "the Up row does not name the change:\n{text}"
    );
    assert!(
        text.contains("top.rs"),
        "the change's files are the view:\n{text}"
    );
}

#[test]
fn a_zoom_whose_key_no_longer_names_a_row_is_dormant_not_wrong() {
    let (_workspace, mut app) = tree_sidebar();
    app.on_key(KeyCode::Enter).expect("zoom in");
    // `t` flattens the tree: there is no directory row to carve to any more,
    // so the whole list is the view rather than an error or an empty pane.
    app.on_key(KeyCode::Char('t')).expect("flat list");
    let text = sidebar_text(&frame_at(&app, 100, 24), 100, 24, Split::default());
    assert!(
        text.contains("top.rs"),
        "a dormant zoom hid the review:\n{text}"
    );
}

/// `Shift+→` and `Shift+←` are the arrows one layer deeper: in the sidebar
/// they walk the tree the way `Enter` and `Esc` do.
#[test]
fn shift_arrows_walk_into_folders_and_back_out() {
    let (_workspace, mut app) = tree_sidebar();
    app.on_key_event(KeyEvent::new(KeyCode::Right, KeyModifiers::SHIFT))
        .expect("shift+right");
    let zoomed = sidebar_text(&frame_at(&app, 100, 24), 100, 24, Split::default());
    assert!(
        zoomed.contains("▴") && !zoomed.contains("top.rs"),
        "shift+right did not zoom in:\n{zoomed}"
    );

    app.on_key_event(KeyEvent::new(KeyCode::Left, KeyModifiers::SHIFT))
        .expect("shift+left");
    let back = sidebar_text(&frame_at(&app, 100, 24), 100, 24, Split::default());
    assert!(
        back.contains("top.rs"),
        "shift+left did not back out:\n{back}"
    );
}
