//! The bar is the status bar.

use crossterm::event::KeyCode;

use crate::support::*;

/// The bar along the bottom is drawn from `rv::statusbar`, so it says what mode
/// the keyboard is in, which file is selected and where in the list, what is
/// under review, how many comments are open, and where the keymap is.
#[test]
fn the_bar_is_drawn_from_the_status_bars_segments() {
    let workspace = Fixture::new();
    let app = workspace.app();

    let bar = last_row(&frame_at(&app, 100, 24));
    assert!(
        bar.contains("DIFF"),
        "the context is not on the bar — the diff has the focus at launch: {bar:?}"
    );
    assert!(bar.contains("a.rs"), "nor the selected file: {bar:?}");
    assert!(bar.contains("1/2"), "nor how far through the list: {bar:?}");
    assert!(bar.contains("trunk()"), "nor what is in scope: {bar:?}");
    assert!(bar.contains("0 open"), "nor the comment count: {bar:?}");
    assert!(bar.contains("? help"), "nor where the keymap is: {bar:?}");
}

/// The mode segment names the *context* the cursor is in, and follows it: the
/// pane, the sidebar tab, the stack — not merely "browsing".
#[test]
fn the_mode_segment_follows_the_context() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    assert!(last_row(&frame_at(&app, 100, 24)).contains("DIFF"));

    app.on_key(KeyCode::Left).expect("focus the sidebar");
    assert!(last_row(&frame_at(&app, 100, 24)).contains("FILES"));

    app.on_key(KeyCode::Char(' ')).expect("mode leader");
    app.on_key(KeyCode::Char('c')).expect("the commits mode");
    assert!(last_row(&frame_at(&app, 100, 24)).contains("COMMITS"));

    app.on_key(KeyCode::Char(' ')).expect("mode leader");
    app.on_key(KeyCode::Char('m')).expect("the comments mode");
    assert!(last_row(&frame_at(&app, 100, 24)).contains("COMMENTS"));
}

/// The defect the `?` popup was a workaround for, pinned in the frame: a status
/// message is one segment among six now, so it cannot take the keymap hint's
/// place.
#[test]
fn a_status_message_can_no_longer_evict_the_keymap_hint() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, "a finding");
    assert!(
        app.status().contains("comment saved"),
        "the fixture did not produce a status message: {:?}",
        app.status()
    );

    let bar = last_row(&frame_at(&app, 120, 24));
    assert!(
        bar.contains("comment saved"),
        "the message is not on the bar at all: {bar:?}"
    );
    assert!(
        bar.contains("? help"),
        "the message took the hint's place: {bar:?}"
    );
    assert!(bar.contains("DIFF"), "...and the mode's with it: {bar:?}");
}

/// A confirmation keeps the whole row. It is a modal question whose answer
/// destroys written work, and a question that could be dropped for want of room
/// is a question the reviewer answers blind.
#[test]
fn a_confirmation_is_never_dropped_for_want_of_room() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, "a finding");
    app.on_key(KeyCode::Char('c')).expect("comment leader");
    app.on_key(KeyCode::Char('d')).expect("d");

    let bar = last_row(&frame_at(&app, 24, 24));
    assert!(
        bar.contains("delete"),
        "the question was dropped like a status message: {bar:?}"
    );
    assert!(
        bar.contains('…'),
        "...and what did not fit was not marked: {bar:?}"
    );
}

/// `RV_ASCII` is read once, by the app, and the renderer draws from what the
/// app read rather than asking the environment per frame.
///
/// "Once" is a property of the code's shape rather than of a frame — there is
/// no way to observe a syscall from out here — so what is pinned is the chain:
/// the app owns the answer, and the bar on screen agrees with it.
#[test]
fn the_powerline_glyphs_are_decided_by_the_app_at_startup() {
    let workspace = Fixture::new();
    let app = workspace.app();

    assert_eq!(
        app.ascii(),
        rv::statusbar::ascii_from(std::env::var_os(rv::statusbar::RV_ASCII).as_deref()),
        "the app did not read the switch the status bar defines"
    );
    let bar = last_row(&frame_at(&app, 100, 24));
    assert_eq!(
        bar.contains('\u{e0b0}'),
        !app.ascii(),
        "the bar drew glyphs the app did not ask for: {bar:?}"
    );
}

/// A status expires: roughly eight seconds after the loop stamps it, the bar
/// drops the segment — and the hint, the mode and the rest stay.
#[test]
fn a_status_message_expires_off_the_bar() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, "a finding");
    let t0 = std::time::Instant::now();
    app.expire_status(t0);

    let fresh = last_row(&frame_at_time(&app, 130, 24, t0));
    assert!(
        fresh.contains("comment saved"),
        "the status never reached the bar: {fresh:?}"
    );

    let later = last_row(&frame_at_time(
        &app,
        130,
        24,
        t0 + std::time::Duration::from_secs(9),
    ));
    assert!(
        !later.contains("comment saved"),
        "the status outlived its eight seconds: {later:?}"
    );
    assert!(
        later.contains("? help") && later.contains("DIFF"),
        "expiry took the rest of the bar with it: {later:?}"
    );
}

/// The position segment carries the cursor's own line, `path:line`.
#[test]
fn the_bar_names_the_line_the_cursor_is_on() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    let line = select_line(&mut app, |line| line.text.contains("let x = 1;"));
    let number = line.right.expect("an added line has a head-side number");

    let bar = last_row(&frame_at(&app, 130, 24));
    assert!(
        bar.contains(&format!("a.rs:{number}")),
        "the bar does not say which line the cursor is on: {bar:?}"
    );
}
