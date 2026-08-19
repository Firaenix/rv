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

/// Presses `Tab` until the sidebar is showing the review's comments.
///
/// The cycle is Files → Commits → Comments; a test that wants the browser wants
/// it whatever the cycle's length is this week.
pub fn to_comments(app: &mut App) {
    for _ in 0..8 {
        if app.sidebar_tab() == SidebarTab::Comments {
            return;
        }
        app.on_key(KeyCode::Tab).expect("switch the sidebar tab");
    }
    panic!("the comments tab is not in the Tab cycle");
}

/// The same, for the tab that lists the stack's changes.
pub fn to_commits(app: &mut App) {
    for _ in 0..8 {
        if app.sidebar_tab() == SidebarTab::Commits {
            return;
        }
        app.on_key(KeyCode::Tab).expect("switch the sidebar tab");
    }
    panic!("the commits tab is not in the Tab cycle");
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

/// The same, for the file list — which is also the tab `t` and `o` mean something
/// in.
pub fn to_files(app: &mut App) {
    for _ in 0..8 {
        if app.sidebar_tab() == SidebarTab::Files {
            return;
        }
        app.on_key(KeyCode::Tab).expect("switch the sidebar tab");
    }
    panic!("the files tab is not in the Tab cycle");
}
