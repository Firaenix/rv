//! The review follows the working copy: an edit saved on disk — by an agent,
//! by an editor, with no jj command run — shows up on the watch's next tick,
//! and the moment the terminal regains focus.

use std::time::Duration;
use std::time::Instant;

use crossterm::event::KeyCode;

use crate::support::*;

fn diff_text(app: &rv::app::App) -> String {
    app.displayed_lines()
        .iter()
        .map(|line| line.text.clone())
        .collect::<Vec<_>>()
        .join("")
}

/// A file edited on disk, no `jj` run: the next tick past the interval
/// refreshes and the diff shows the new text.
#[test]
fn an_edit_on_disk_is_picked_up_by_the_watch() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    assert!(diff_text(&app).contains("let x = 1;"));
    let t0 = Instant::now();
    app.auto_refresh(t0);

    workspace.write("a.rs", "fn a() {\n    let x = 42;\n}\n");
    app.auto_refresh(t0 + Duration::from_millis(100));
    assert!(
        diff_text(&app).contains("let x = 1;"),
        "a look inside the interval refreshed anyway"
    );

    app.auto_refresh(t0 + Duration::from_secs(3));
    app.finish_loading();
    app.finish_merging();
    assert!(
        diff_text(&app).contains("let x = 42;"),
        "the edit on disk did not reach the diff:\n{}",
        diff_text(&app)
    );
    assert!(app.status().starts_with("refreshed"), "{}", app.status());
}

/// Regaining focus looks at once, interval or not.
#[test]
fn regaining_focus_refreshes_at_once_when_something_moved() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    app.auto_refresh(Instant::now());

    app.on_focus_gained();
    assert!(
        !app.status().starts_with("refreshed"),
        "focus with nothing changed refreshed anyway: {}",
        app.status()
    );

    workspace.write("a.rs", "fn a() {\n    let x = 7;\n}\n");
    app.on_focus_gained();
    app.finish_loading();
    app.finish_merging();
    assert!(
        diff_text(&app).contains("let x = 7;"),
        "focus did not pick up the edit:\n{}",
        diff_text(&app)
    );
}

/// A file added beside a reviewed one is movement too: the refresh lists it.
#[test]
fn a_file_added_beside_a_reviewed_one_appears_after_the_watch_ticks() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    assert_eq!(app.files().len(), 2);
    app.auto_refresh(Instant::now());

    workspace.write("c.rs", "fn c() {}\n");
    app.on_focus_gained();
    assert_eq!(
        app.files().len(),
        3,
        "the new file is not in the review after a focus look"
    );
}

/// Nothing modal is yanked away: a half-typed comment stays put even when
/// the working copy moved under it.
#[test]
fn a_half_typed_comment_is_never_refreshed_out_from_under() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    app.auto_refresh(Instant::now());
    app.on_key(KeyCode::Char('C')).expect("start a comment");
    app.on_key(KeyCode::Char('x')).expect("type");

    workspace.write("a.rs", "fn a() {\n    let x = 9;\n}\n");
    app.on_focus_gained();
    assert_eq!(app.mode(), rv::app::Mode::Comment);
    assert_eq!(app.buffer(), "x");
    assert!(
        diff_text(&app).contains("let x = 1;"),
        "the diff moved under a comment being typed"
    );
}
