//! Driving the reviewer the way a user does — one [`KeyCode`] at a time.

use crossterm::event::KeyCode;
use rv::app::App;
use rv::app::SidebarTab;
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
    let lines = app.displayed_lines();
    let index = lines
        .iter()
        .position(&wanted)
        .unwrap_or_else(|| panic!("no diff line matched: {lines:?}"));
    for _ in 0..index {
        app.on_key(KeyCode::Down).expect("move down a line");
    }
    assert_eq!(app.line_index(), index);
    app.displayed_lines()[index].clone()
}

/// Presses `c`, types `body`, and presses Enter — one whole comment.
pub fn write_comment(app: &mut App, body: &str) {
    app.on_key(KeyCode::Char('c')).expect("comment leader");
    // The first `c` smart-collapses to the write when it is the only live
    // comment verb; press again only if it merely opened the menu.
    if app.pending_leader().is_some() {
        app.on_key(KeyCode::Char('c')).expect("enter comment mode");
    }
    type_text(app, body);
    app.on_key(KeyCode::Enter).expect("save the comment");
}

/// Selects the review's comments tab with its direct `3` key.
pub fn to_comments(app: &mut App) {
    app.on_key(KeyCode::Char('m')).expect("mode leader");
    app.on_key(KeyCode::Char('o')).expect("the comments mode");
    assert_eq!(app.sidebar_tab(), SidebarTab::Comments);
}

/// The same, for the tab that lists the stack's changes: `2`.
pub fn to_commits(app: &mut App) {
    app.on_key(KeyCode::Char('m')).expect("mode leader");
    app.on_key(KeyCode::Char('c')).expect("the commits mode");
    assert_eq!(app.sidebar_tab(), SidebarTab::Commits);
}

/// Moves the sidebar cursor to the top of the list it is showing.
///
/// The commits tab parks the cursor on the selected file's row — position
/// preservation, navigation spec §3 — which is *below* most rows a walk is
/// looking for. A walk that means "the whole list" goes up first.
pub fn to_top(app: &mut App) {
    for _ in 0..40 {
        app.on_key(KeyCode::Up).expect("previous row");
    }
}

/// The same, for the file list: `1`.
pub fn to_files(app: &mut App) {
    app.on_key(KeyCode::Char('m')).expect("mode leader");
    app.on_key(KeyCode::Char('f')).expect("the files mode");
    assert_eq!(app.sidebar_tab(), SidebarTab::Files);
}
