//! Deleting a comment from the sidebar's comment browser.
//!
//! Split from `browser_1`, which holds how the browser is navigated; `d` is
//! what a reviewer does *to* a comment once they have found it.

use crossterm::event::KeyCode;
use rv::app::Mode;
use rv::app::SidebarTab;

use crate::support::*;

/// `d` from the browser deletes the comment the browser has selected, behind
/// the same confirmation as everywhere else.
#[test]
fn d_from_the_comment_browser_deletes_behind_the_same_confirmation() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, "first finding");
    app.on_key(KeyCode::Down).expect("next line");
    write_comment(&mut app, "second finding");

    to_comments(&mut app);
    app.on_key(KeyCode::Down).expect("select the second");
    app.on_key(KeyCode::Char('c')).expect("comment leader");
    app.on_key(KeyCode::Char('d')).expect("ask");
    assert!(
        matches!(app.mode(), Mode::ConfirmDelete { .. }),
        "it deleted without asking: {:?}",
        app.mode()
    );
    app.on_key(KeyCode::Char('y')).expect("confirm");

    let left = workspace.store().comments().expect("read");
    assert_eq!(left.len(), 1, "{left:?}");
    assert_eq!(left[0].body, "first finding", "the browsed comment went");
    assert_eq!(
        app.browsed_comment().expect("a row").body,
        "first finding",
        "the browser's cursor was left past the end of the list"
    );
}

/// ...and `n` still cancels, from here as from anywhere.
#[test]
fn d_from_the_comment_browser_can_be_declined() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, "needs a doc");

    to_comments(&mut app);
    app.on_key(KeyCode::Char('c')).expect("comment leader");
    app.on_key(KeyCode::Char('d')).expect("ask");
    app.on_key(KeyCode::Char('n')).expect("decline");

    assert_eq!(app.mode(), Mode::Browse);
    assert_eq!(workspace.store().comments().expect("read").len(), 1);
}

/// The Files tab stays inert: it selects files, and a destructive key aimed
/// into it would be aimed at a comment the reviewer cannot see.
#[test]
fn d_from_the_files_tab_still_deletes_nothing() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, "needs a doc");

    app.on_key(KeyCode::Left).expect("focus the file list");
    assert_eq!(app.sidebar_tab(), SidebarTab::Files);
    app.on_key(KeyCode::Char('c')).expect("comment leader");
    app.on_key(KeyCode::Char('d')).expect("d");

    assert_eq!(app.mode(), Mode::Browse);
    assert!(app.status().contains("not comments"), "{:?}", app.status());
    assert_eq!(workspace.store().comments().expect("read").len(), 1);
}

/// `d` with an empty browser asks nothing and says why.
#[test]
fn d_from_an_empty_comment_browser_says_so() {
    let workspace = Fixture::new();
    let mut app = workspace.app();

    to_comments(&mut app);
    app.on_key(KeyCode::Char('c')).expect("comment leader");
    app.on_key(KeyCode::Char('d')).expect("d");

    assert_eq!(app.mode(), Mode::Browse);
    assert!(app.status().contains("no comments"), "{:?}", app.status());
}
