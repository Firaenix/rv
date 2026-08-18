//! Driving the reviewer the way a user does — one [`KeyCode`] at a time.

use crossterm::event::KeyCode;
use rv::app::App;
use rv_core::diff::DiffLine;

/// Presses every character of `text` in order.
pub fn type_text(app: &mut App, text: &str) {
    for character in text.chars() {
        app.on_key(KeyCode::Char(character)).expect("type");
    }
}

/// Moves the highlight down to the first diff line `wanted` accepts, the way a
/// reviewer would, and returns it.
pub fn select_line(app: &mut App, wanted: impl Fn(&DiffLine) -> bool) -> DiffLine {
    let diff = app.selected_diff().expect("the selected file has a diff");
    let index = diff
        .lines
        .iter()
        .position(&wanted)
        .unwrap_or_else(|| panic!("no diff line matched: {:?}", diff.lines));
    for _ in 0..index {
        app.on_key(KeyCode::Char('j')).expect("move down a line");
    }
    assert_eq!(app.line_index(), index);
    app.selected_diff().expect("a diff").lines[index].clone()
}

/// Presses `c`, types `body`, and presses Enter — one whole comment.
pub fn write_comment(app: &mut App, body: &str) {
    app.on_key(KeyCode::Char('c')).expect("enter comment mode");
    type_text(app, body);
    app.on_key(KeyCode::Enter).expect("save the comment");
}
