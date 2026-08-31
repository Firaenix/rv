//! The file list as a counted tree.

use crossterm::event::KeyCode;
use ratatui::style::Modifier;
use rv::layout::Split;

use crate::support::*;

/// The colours are computed once, when the review is opened, and never move.
#[test]
fn the_colours_do_not_move_as_you_browse() {
    let workspace = Fixture::nested();
    let mut app = workspace.app();
    let before = sidebar_inks(&frame_at(&app, 100, 24));
    assert!(
        before.iter().any(Option::is_some),
        "the file list drew no colour at all, so this proves nothing"
    );

    for _ in 0..3 {
        app.on_key(KeyCode::Char(']')).expect("next file");
    }
    assert_eq!(
        sidebar_inks(&frame_at(&app, 100, 24)),
        before,
        "the colours were recomputed as files were opened"
    );
}

/// The shape, the order and the folds are this session's, like every other view
/// preference in this reviewer.
#[test]
fn the_shape_and_the_order_never_reach_disk() {
    let workspace = Fixture::nested();
    let mut app = workspace.app();
    let before = workspace_tree(workspace.root());

    app.on_key(KeyCode::Char('v')).expect("view leader");
    app.on_key(KeyCode::Char('t')).expect("the tree");
    app.on_key(KeyCode::Char('v')).expect("view leader");
    app.on_key(KeyCode::Char('o')).expect("order by additions");
    app.on_key(KeyCode::Left).expect("focus the file list");
    app.on_key(KeyCode::Char('s')).expect("fold something");
    app.on_key(KeyCode::Char('v')).expect("view leader");
    app.on_key(KeyCode::Char('o')).expect("order by removals");

    assert_eq!(
        workspace_tree(workspace.root()),
        before,
        "how one reviewer likes their file list is not review state"
    );
}

/// Walking onto a directory row moves the cursor and leaves the diff alone: the
/// reviewer chose the file they are reading, and a folder is a thing to fold
/// rather than a file to open.
#[test]
fn the_cursor_can_rest_on_a_directory_without_changing_the_diff() {
    let workspace = Fixture::nested();
    let mut app = workspace.app();
    app.on_key(KeyCode::Char('v')).expect("view leader");
    app.on_key(KeyCode::Char('t')).expect("the tree");
    app.on_key(KeyCode::Left).expect("focus the file list");

    let file = app.file_index();
    app.on_key(KeyCode::Up).expect("onto the directory row");
    assert_eq!(app.file_index(), file, "a directory row selected a file");
    assert!(
        buffer_text(&frame_at(&app, 100, 24)).contains("a.md"),
        "the diff pane stopped showing the file that is selected"
    );

    app.on_key(KeyCode::Down).expect("back onto the file");
    assert_eq!(app.file_index(), file, "and coming back moved it");
}

/// The file list's cursor is on the row that holds the selected file, whatever
/// order the rows are in and however the file came to be selected.
///
/// The two are different numbers the moment an order moves a row: under
/// `added` the review's first file is the *third* row here, and `]` from it
/// lands on the fourth. A cursor that stayed at the row number would highlight
/// a file nobody selected — and `s`, which acts on the row, would be aimed at
/// it.
#[test]
fn the_file_lists_cursor_follows_the_file_that_is_selected() {
    let workspace = Fixture::nested();
    let mut app = workspace.app();
    // Natural order is path order: a.md (10), b.md (5), lib.rs (30), top.rs
    // (50). By additions it is top.rs, lib.rs, a.md, b.md.
    assert_eq!((app.file_index(), app.sidebar_row()), (0, 0));

    app.on_key(KeyCode::Char('v')).expect("view leader");
    app.on_key(KeyCode::Char('o')).expect("order by additions");
    assert_eq!(app.file_index(), 0, "reordering moved the selection");
    assert_eq!(
        app.sidebar_row(),
        2,
        "the cursor stayed at a row number instead of following the file"
    );

    app.on_key(KeyCode::Char(']')).expect("next file");
    assert_eq!(app.file_index(), 1, "] did not move to the next file");
    assert_eq!(
        app.sidebar_row(),
        3,
        "the 5-line file is the last row under this order"
    );

    // ...and the pane highlights that row rather than the file's own index.
    app.on_key(KeyCode::Left).expect("focus the file list");
    let frame = frame_at(&app, 100, 24);
    let area = inner(areas(100, 24, Split::default()).sidebar);
    let highlighted = (area.y..area.bottom())
        .find(|y| frame[(area.x, *y)].modifier.contains(Modifier::REVERSED))
        .expect("the focused file list highlights a row");
    assert!(
        row_in(&frame, area, highlighted).contains("b.md"),
        "the highlight is on {:?}",
        row_in(&frame, area, highlighted)
    );
}

/// `t` and `o` are preferences about the file list, so from the comment browser
/// they refuse and say where the list they are about is.
#[test]
fn the_view_keys_say_they_are_about_the_file_list() {
    let workspace = Fixture::nested();
    let mut app = workspace.app();
    to_comments(&mut app);
    let tree = app.tree_view();
    let sort = app.sort();

    app.on_key(KeyCode::Char('v')).expect("view leader");
    app.on_key(KeyCode::Char('t')).expect("t");
    assert_eq!(app.tree_view(), tree, "t reshaped a list nobody can see");
    assert!(
        app.status().contains("file list"),
        "t refused without saying why: {:?}",
        app.status()
    );

    app.on_key(KeyCode::Char('v')).expect("view leader");
    app.on_key(KeyCode::Char('o')).expect("o");
    assert_eq!(app.sort(), sort, "o reordered a list nobody can see");
    assert!(
        app.status().contains("file list"),
        "o refused without saying why: {:?}",
        app.status()
    );
}

/// Directory and file rows carry nerd-font icons beside their marks — a folder
/// for what holds things, a file for what is held. They are patched-font
/// glyphs, so the same `RV_ASCII` that turns the powerline arrows off turns
/// them off too; with the switch unset (as here) they are on.
#[test]
fn the_tree_carries_nerdfont_icons_unless_ascii_asks_otherwise() {
    let workspace = Fixture::nested();
    let mut app = workspace.app();
    app.on_key(crossterm::event::KeyCode::Char('v'))
        .expect("view leader");
    app.on_key(crossterm::event::KeyCode::Char('t'))
        .expect("tree view");

    let text = sidebar_text(
        &frame_at(&app, 100, 24),
        100,
        24,
        rv::layout::Split::default(),
    );
    assert!(
        text.contains('\u{f07c}'),
        "no open-folder icon on an open directory:\n{text}"
    );
    assert!(
        text.contains('\u{f15b}'),
        "no file icon on a file row:\n{text}"
    );

    // Folding swaps the folder icon for its closed form.
    app.on_key(crossterm::event::KeyCode::Left)
        .expect("focus the sidebar");
    app.on_key(crossterm::event::KeyCode::Up)
        .expect("onto the directory row");
    app.on_key(crossterm::event::KeyCode::Char('s'))
        .expect("fold");
    let text = sidebar_text(
        &frame_at(&app, 100, 24),
        100,
        24,
        rv::layout::Split::default(),
    );
    assert!(
        text.contains('\u{f07b}'),
        "no closed-folder icon on a folded directory:\n{text}"
    );
}
