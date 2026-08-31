//! The mouse.

use crossterm::event::KeyCode;
use rv::app::Focus;
use rv::layout::Split;

use crate::support::*;

/// Clicking a diff line selects it and hands the keys to the diff.
#[test]
fn clicking_a_diff_line_selects_it_and_focuses_the_diff() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    app.on_key(KeyCode::Left).expect("focus the sidebar");
    assert_eq!(app.focus(), Focus::Sidebar);

    let row = diff_pane_row(&app, 100, 24, 2);
    app.on_mouse(click(60, row)).expect("click in the diff");

    assert_eq!(app.focus(), Focus::Diff, "the click moved the focus");
    assert_eq!(
        app.line_index(),
        2,
        "and selected the line under the pointer"
    );
}

/// A click below the last row of the plan selects nothing at all.
///
/// Slop that points at a row nothing was drawn on is not slop: the reviewer
/// clicked empty space, and a clamp onto the last line would be the tool
/// choosing a line they did not.
#[test]
fn clicking_below_the_last_row_of_the_diff_selects_nothing() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    app.on_key(KeyCode::Left).expect("focus the sidebar");
    let before = (app.focus(), app.line_index());

    let row = diff_pane_row(&app, 100, 24, 12);
    app.on_mouse(click(60, row)).expect("click on empty space");

    assert_eq!(
        (app.focus(), app.line_index()),
        before,
        "a click on a row nothing was painted on moved something"
    );
}

/// Clicking a comment box steps into that line's stack, on that comment.
#[test]
fn clicking_a_comment_box_focuses_the_stack_and_selects_that_comment() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, "a finding");

    let frame = frame_at(&app, 100, 24);
    let (_, row) = find_char_in(&frame, box_area(), '╭').expect("a comment box is drawn");
    app.on_mouse(click(60, row)).expect("click the box");

    assert_eq!(app.focus(), Focus::Stack);
    assert_eq!(
        app.selected_comment().expect("a selected comment").body,
        "a finding"
    );
}

/// Clicking a file row selects that file and hands the keys to the file list.
#[test]
fn clicking_a_file_row_selects_that_file_and_focuses_the_sidebar() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    assert_eq!(app.focus(), Focus::Diff);

    let row = sidebar_pane_row(&app, 100, 24, 1);
    app.on_mouse(click(3, row)).expect("click the second file");

    assert_eq!(app.focus(), Focus::Sidebar);
    assert_eq!(app.selected_file().expect("a file").path, "b.rs");
    assert_eq!(app.sidebar_row(), 1);
}

/// Clicking a directory row folds it, which is what `s` does to the row under
/// the cursor — one verb, reached two ways.
#[test]
fn clicking_a_directory_row_folds_it() {
    let workspace = Fixture::nested();
    let mut app = workspace.app();
    app.on_key(KeyCode::Char('v')).expect("view leader");
    app.on_key(KeyCode::Char('t')).expect("the tree");
    let folded = app
        .sidebar_nodes()
        .iter()
        .position(|node| node.label == "docs/specs")
        .expect("a directory row");

    let row = sidebar_pane_row(&app, 100, 24, u16::try_from(folded).expect("a small row"));
    app.on_mouse(click(3, row)).expect("click the directory");

    let labels: Vec<String> = app
        .sidebar_nodes()
        .iter()
        .map(|node| node.label.clone())
        .collect();
    assert!(
        labels.iter().any(|label| label == "docs/specs"),
        "the directory row itself is gone: {labels:?}"
    );
    assert!(
        !labels.iter().any(|label| label.ends_with("a.md")),
        "its children are still listed: {labels:?}"
    );
}

/// Dragging the divider resizes the panes and moves nothing else.
#[test]
fn dragging_the_divider_resizes_and_changes_nothing_else() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    let before = (app.file_index(), app.line_index(), app.focus());

    let divider = divider_column(&app, 100, 24);
    app.on_mouse(press(divider, 6)).expect("press the divider");
    app.on_mouse(drag(divider + 10, 6)).expect("drag");
    app.on_mouse(release(divider + 10, 6)).expect("release");

    assert!(
        app.split().ratio() > Split::DEFAULT,
        "the split did not follow the pointer: {}",
        app.split().ratio()
    );
    assert_eq!(
        (app.file_index(), app.line_index(), app.focus()),
        before,
        "the resize moved something other than the divider"
    );
}

/// The pointer stops dragging the divider when the button comes up.
#[test]
fn the_divider_stops_following_the_pointer_at_the_release() {
    let workspace = Fixture::new();
    let mut app = workspace.app();

    let divider = divider_column(&app, 100, 24);
    app.on_mouse(press(divider, 6)).expect("press");
    app.on_mouse(drag(divider + 10, 6)).expect("drag");
    app.on_mouse(release(divider + 10, 6)).expect("release");
    let settled = app.split().ratio();

    app.on_mouse(drag(divider + 25, 6)).expect("move on");
    assert_eq!(
        app.split().ratio(),
        settled,
        "the divider kept following a pointer that had let go of it"
    );
}

/// The wheel moves the view and leaves the selection where it was.
///
/// Scrolling is looking; clicking is choosing. A wheel nudge that moved the
/// selection would silently re-aim the next `c` or `d` at another line.
#[test]
fn scrolling_moves_the_view_without_moving_the_selection() {
    let workspace = Fixture::mixed();
    let mut app = workspace.app_from("@--");
    let selected = app.line_index();

    let row = diff_pane_row(&app, 100, 24, 3);
    let before = visible_row_indices(&app, 100, 23);
    app.on_mouse(scroll_down(60, row)).expect("scroll");
    let after = visible_row_indices(&app, 100, 23);

    assert_eq!(
        app.line_index(),
        selected,
        "scrolling is looking, not choosing — cursor row {}, file {:?}, \
         plan {} rows, window {before:?} then {after:?}",
        app.cursor_row(),
        app.selected_file().map(|file| file.path.clone()),
        row_count(&app, 100, 23),
    );
    assert!(
        after.start > before.start,
        "the view did not move: {before:?} then {after:?}"
    );

    app.on_mouse(scroll_up(60, row)).expect("scroll back");
    assert_eq!(
        visible_row_indices(&app, 100, 23),
        before,
        "the wheel does not come back"
    );
}

/// The same for the file list: the wheel looks ahead down a list too long for
/// the pane without moving which file is selected.
#[test]
fn scrolling_the_sidebar_looks_ahead_without_moving_the_selection() {
    let workspace = Fixture::nested();
    let mut app = workspace.app();
    app.on_key(KeyCode::Char('v')).expect("view leader");
    app.on_key(KeyCode::Char('t'))
        .expect("the tree, which is taller");

    let before = sidebar_rows(&frame_at(&app, 60, 8), 60, 8, Split::default());
    let selected = (app.sidebar_row(), app.file_index());

    let row = sidebar_pane_row(&app, 60, 8, 1);
    app.on_mouse(scroll_down(3, row))
        .expect("scroll the file list");

    let after = sidebar_rows(&frame_at(&app, 60, 8), 60, 8, Split::default());
    assert_ne!(before, after, "the file list did not scroll:\n{before:?}");
    assert_eq!(
        (app.sidebar_row(), app.file_index()),
        selected,
        "scrolling the list moved the selection"
    );
}

/// No gesture destroys review state. There is no click target for `d`, and
/// dragging a comment does nothing: the confirmation exists because deletion is
/// unrecoverable, and a mis-click is exactly the accident it guards against.
#[test]
fn no_gesture_deletes_anything() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, "a finding");
    assert_eq!(app.comments().len(), 1);

    let diff = diff_pane_row(&app, 100, 24, 1);
    let sidebar = sidebar_pane_row(&app, 100, 24, 0);
    let divider = divider_column(&app, 100, 24);
    let before = workspace_tree(workspace.root());

    for event in [
        click(60, diff),
        click(3, sidebar),
        scroll_up(60, diff),
        scroll_down(60, diff),
        press(divider, 6),
        drag(divider + 6, 6),
        release(divider + 6, 6),
        press(60, diff),
        drag(60, diff + 2),
        release(60, diff + 2),
    ] {
        app.on_mouse(event).expect("gesture");
    }

    assert_eq!(app.comments().len(), 1, "a gesture removed a comment");
    assert_eq!(
        workspace_tree(workspace.root()),
        before,
        "the mouse reached disk"
    );
}

/// A click lands on the line the frame actually painted, scrolled or not.
///
/// The scroll is the point: with the view moved off the top of the plan, a
/// hit test that forgot the window's offset still resolves to *a* line, and the
/// only way to tell is to read what was drawn on the row that was clicked.
#[test]
fn a_click_lands_on_the_line_the_frame_actually_painted() {
    let workspace = Fixture::mixed();
    let mut app = workspace.app_from("@--");

    let row = diff_pane_row(&app, 100, 24, 4);
    for _ in 0..3 {
        app.on_mouse(scroll_down(60, row)).expect("scroll");
    }

    let frame = frame_at(&app, 100, 24);
    let painted = row_in(&frame, inner(areas(100, 24, app.split()).diff), row);
    app.on_mouse(click(60, row)).expect("click");

    let selected = app.selected_diff().expect("a diff").lines[app.line_index()]
        .text
        .clone();
    assert!(
        painted.trim_end().ends_with(&selected),
        "clicked a row painted {painted:?} and selected {selected:?}"
    );
}

/// The bar is not a click target.
#[test]
fn clicking_the_bar_does_nothing() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    let before = (app.focus(), app.line_index(), app.file_index());
    let _ = frame_at(&app, 100, 24);

    app.on_mouse(click(40, 23)).expect("click the bar");

    assert_eq!(
        (app.focus(), app.line_index(), app.file_index()),
        before,
        "the status bar answered a click"
    );
}
