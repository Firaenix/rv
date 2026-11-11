//! `rv browse`'s file list: sizes for counts, path and size for orders.

use crossterm::event::KeyCode;
use rv::app::App;
use rv::app::DiffEngine;
use rv::layout::Split;
use rv::session;

use super::browse_1::browse;
use crate::support::*;

/// `numbered("top", 50)`: ten 11-byte lines and forty 12-byte ones.
const TOP_BYTES: &str = "590";

#[test]
fn a_tree_rows_marker_is_a_dot_and_its_count_is_its_size() {
    let workspace = Fixture::nested();
    let app = browse(&workspace, None);

    let text = sidebar_text(&frame_at(&app, 100, 24), 100, 24, Split::default());
    let top = text
        .lines()
        .find(|row| row.contains("top.rs"))
        .unwrap_or_else(|| panic!("no top.rs row:\n{text}"));
    assert!(top.contains('·'), "{top:?}");
    assert!(top.contains(TOP_BYTES), "{top:?}");
    assert!(!text.contains('+'), "nothing was added in a tree:\n{text}");
}

#[test]
fn o_cycles_a_tree_between_path_and_size_order() {
    let workspace = Fixture::nested();
    let mut app = browse(&workspace, None);
    let shape = |app: &App| sidebar_shape(&frame_at(app, 100, 24));

    assert!(shape(&app).contains("path"), "{:?}", shape(&app));
    app.on_key(KeyCode::Char('v')).expect("view leader");
    app.on_key(KeyCode::Char('o')).expect("o");
    assert!(shape(&app).contains("size"), "{:?}", shape(&app));
    app.on_key(KeyCode::Char('v')).expect("view leader");
    app.on_key(KeyCode::Char('o')).expect("o");
    assert!(shape(&app).contains("path"), "{:?}", shape(&app));
}

#[test]
fn sorting_a_tree_by_size_puts_the_biggest_file_first() {
    let workspace = Fixture::nested();
    let mut app = browse(&workspace, None);
    app.on_key(KeyCode::Char('v')).expect("view leader");
    app.on_key(KeyCode::Char('o')).expect("order by size");

    let rows = sidebar_filled(&frame_at(&app, 100, 24), 100, 24, Split::default());
    assert!(rows[0].contains("top.rs"), "{rows:?}");
}

#[test]
fn a_tree_file_never_asks_difftastic() {
    let workspace = Fixture::new();
    let review = session::browse(workspace.root(), None, None).expect("browse the tree");
    let mut app = App::open(review, DiffEngine::Auto).expect("open the reviewer");
    assert!(!app.refining(), "a whole file was sent for refinement");
    app.on_key(KeyCode::Char(']')).expect("next file");
    assert_eq!(
        app.selected_file().map(|file| file.path.as_str()),
        Some("b.rs")
    );
    assert!(!app.refining(), "a whole file was sent for refinement");
}
