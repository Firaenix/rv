//! Alerts that float and fade.

use std::time::Duration;
use std::time::Instant;

use crossterm::event::KeyCode;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;
use rv::layout::Chrome;
use rv::layout::Split;
use rv::layout::layout;

use crate::support::*;

/// Where the toast floats at `width` x `height`, asked of the same [`layout`]
/// that painted it.
fn toast_area(width: u16, height: u16) -> Rect {
    layout(
        Rect::new(0, 0, width, height),
        Split::default(),
        Chrome {
            bar_rows: 1,
            help_open: false,
            toast: true,
            sidebar_hidden: false,
        },
    )
    .toast
    .expect("a toast has a rectangle at this size")
}

/// The colour the toast's border is drawn in.
fn toast_border_colour(buffer: &Buffer) -> Color {
    let area = toast_area(buffer.area.width, buffer.area.height);
    buffer[(area.x, area.y)]
        .style()
        .fg
        .expect("the toast's border carries a colour")
}

/// How light a colour is, for comparing one step of the fade against the next.
fn luma(colour: Color) -> f32 {
    match colour {
        Color::Rgb(red, green, blue) => {
            0.2126 * f32::from(red) + 0.7152 * f32::from(green) + 0.0722 * f32::from(blue)
        }
        other => panic!("{other:?} is not a colour with a lightness"),
    }
}

/// An alert shows up, stays a few seconds, and leaves on its own.
#[test]
fn an_alert_appears_then_leaves_on_its_own() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    let t0 = Instant::now();

    app.alert("src/old.rs is no longer in this range", t0);
    assert_eq!(app.alerts().len(), 1);
    assert!(
        buffer_text(&frame_at_time(&app, 100, 24, t0)).contains("no longer in this range"),
        "the toast is not on screen:\n{}",
        buffer_text(&frame_at_time(&app, 100, 24, t0))
    );

    app.expire_alerts(t0 + Duration::from_secs(2));
    assert_eq!(app.alerts().len(), 1, "still up at two seconds");

    app.expire_alerts(t0 + Duration::from_secs(6));
    assert!(app.alerts().is_empty(), "gone by six");
    assert!(
        !buffer_text(&frame_at_time(&app, 100, 24, t0 + Duration::from_secs(6)))
            .contains("no longer in this range")
    );
}

/// The toast takes no key and steals no focus: it is a notification, not a
/// dialog.
#[test]
fn the_toast_takes_no_key_and_steals_no_focus() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    let focus = app.focus();
    let line = app.line_index();

    app.alert("something went wrong", Instant::now());
    app.on_key(KeyCode::Char('j')).expect("j");

    assert_eq!(app.focus(), focus, "the toast took the focus");
    assert_eq!(app.line_index(), line + 1, "and j still moved the line");
    assert_eq!(app.alerts().len(), 1, "the key dismissed it");
}

/// The toast fades down over its last second rather than blinking out.
#[test]
fn the_border_dims_as_the_deadline_approaches() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    let t0 = Instant::now();
    app.alert("careful", t0);

    let bright = toast_border_colour(&frame_at_time(&app, 100, 24, t0));
    let faded = toast_border_colour(&frame_at_time(
        &app,
        100,
        24,
        t0 + Duration::from_millis(4600),
    ));

    assert_ne!(bright, faded, "the toast vanishes rather than fading");
    assert!(
        luma(faded) < luma(bright),
        "it faded up, not down: {bright:?} then {faded:?}"
    );
}

/// The event loop is told how long it may block for.
#[test]
fn the_event_loop_is_told_when_to_wake_up() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    let t0 = Instant::now();

    assert_eq!(
        app.next_deadline(t0),
        None,
        "an idle rv waits for a key, forever"
    );

    app.alert("careful", t0);
    let wait = app
        .next_deadline(t0)
        .expect("a live alert gives the loop a timeout");
    assert!(
        wait <= Duration::from_secs(5),
        "the timeout outlives the alert: {wait:?}"
    );

    app.expire_alerts(t0 + Duration::from_secs(6));
    assert_eq!(
        app.next_deadline(t0 + Duration::from_secs(6)),
        None,
        "an expired alert still asks the loop to wake up"
    );
}

/// Two alerts at once are both readable.
///
/// **Not one panel each.** `rv::layout::layout` gives the toast three rows —
/// two borders and one message — and no rectangle in this reviewer is computed
/// anywhere but there, so several alerts share the panel rather than stacking
/// down the screen. The claim the plan's version of this test makes is that
/// none of them is lost, and that is what is asserted.
#[test]
fn several_alerts_share_the_panel_and_none_is_lost() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    let t0 = Instant::now();

    app.alert("first", t0);
    app.alert("second", t0);

    let text = buffer_text(&frame_at_time(&app, 100, 24, t0));
    assert!(
        text.contains("first"),
        "the first alert is not on screen:\n{text}"
    );
    assert!(
        text.contains("second"),
        "the second alert is not on screen:\n{text}"
    );
}

/// A jump to a comment whose file has left the range is an alert, not only a
/// status: nothing moved, and a line in the bar is the easiest thing on screen
/// to miss.
#[test]
fn a_jump_to_a_file_that_left_the_range_raises_an_alert() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, "a finding");

    // A range whose only change touches no file at all, so the comment above is
    // anchored outside it.
    let mut later = workspace.app_from("@-");
    assert!(
        later.files().is_empty(),
        "the later range still holds the commented file: {:?}",
        later.files()
    );
    assert_eq!(later.comments().len(), 1);
    assert!(later.alerts().is_empty(), "{:?}", later.alerts().len());

    later.on_key(KeyCode::Left).expect("the sidebar");
    to_comments(&mut later);
    later.on_key(KeyCode::Enter).expect("jump");

    assert!(
        later.status().contains("not in this review's range"),
        "the bar says nothing about it: {:?}",
        later.status()
    );
    assert_eq!(
        later.alerts().len(),
        1,
        "a stale anchor is a status and nothing else"
    );
    assert!(
        later.alerts()[0].message.contains("a.rs"),
        "the alert does not name the file: {:?}",
        later.alerts()[0].message
    );
}

/// The same failure twice is one toast.
///
/// A panel reading `x · x` says nothing the first `x` did not, and a reviewer
/// who pressed `Enter` twice on a comment that cannot be jumped to has been told
/// once already.
#[test]
fn the_same_failure_twice_is_one_toast() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, "a finding");
    let mut later = workspace.app_from("@-");

    later.on_key(KeyCode::Left).expect("the sidebar");
    to_comments(&mut later);
    for _ in 0..3 {
        later.on_key(KeyCode::Enter).expect("jump");
    }

    assert_eq!(
        later.alerts().len(),
        1,
        "one failure, three tellings: {:?}",
        later.alerts()
    );
    let t0 = Instant::now();
    later.alert("a stale finding", t0);
    later.alert("a stale finding", t0);
    assert_eq!(
        later.alerts().len(),
        2,
        "a second, different alert is its own"
    );
}

/// An alert raised where no clock is in reach is stamped by the first pass of
/// the event loop, and ages from there.
///
/// This is what keeps `App` clock-free without leaving an alert immortal: a key
/// press knows what went wrong and nothing about the time, so the time is
/// applied by whoever has it.
#[test]
fn an_alert_raised_before_the_clock_is_known_is_stamped_by_the_first_pass() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, "a finding");
    let mut later = workspace.app_from("@-");

    later.on_key(KeyCode::Left).expect("the sidebar");
    to_comments(&mut later);
    later.on_key(KeyCode::Enter).expect("jump");
    assert_eq!(later.alerts().len(), 1);
    assert_eq!(
        later.alerts()[0].raised,
        None,
        "a key press stamped an alert with a time it could not have"
    );
    assert_eq!(
        later.next_deadline(Instant::now()),
        Some(Duration::ZERO),
        "an unstamped alert does not ask the loop to come straight back"
    );

    let t0 = Instant::now();
    later.expire_alerts(t0);
    assert_eq!(later.alerts()[0].raised, Some(t0), "the pass stamped it");

    later.expire_alerts(t0 + Duration::from_secs(2));
    assert_eq!(
        later.alerts().len(),
        1,
        "it aged from the stamp, not before"
    );
    later.expire_alerts(t0 + Duration::from_secs(6));
    assert!(later.alerts().is_empty());
}

/// Alerts are session-only, like every other preference in this reviewer.
#[test]
fn alerts_are_never_written_anywhere() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    let before = workspace_tree(workspace.root());

    let t0 = Instant::now();
    app.alert("something went wrong", t0);
    let _ = frame_at_time(&app, 100, 24, t0);
    app.expire_alerts(t0 + Duration::from_secs(6));

    assert_eq!(
        workspace_tree(workspace.root()),
        before,
        "an alert reached disk"
    );
}
