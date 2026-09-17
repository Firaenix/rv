//! The flag focus: a flag under its line is something the cursor steps onto,
//! the way a comment box is something it steps into — by click or `Enter` —
//! and the bar says `FLAG` while it is there.

use crossterm::event::KeyCode;
use rv::app::Context;
use rv::app::Focus;
use rv::session;
use rv_core::model::Side;

use crate::support::*;

fn two_flags_on_line_one() -> Fixture {
    let workspace = Fixture::new();
    let review = session::read(workspace.root(), None, None).expect("read the review");
    session::flags::add_flag(&review, "a.rs", Side::Right, 1, "first").expect("flag");
    session::flags::add_flag(&review, "a.rs", Side::Right, 1, "second").expect("flag");
    workspace
}

/// Clicking a flag row steps onto that flag, not into a comment stack and
/// not onto the line it hangs off.
#[test]
fn clicking_a_flag_row_focuses_that_flag() {
    let workspace = two_flags_on_line_one();
    let mut app = workspace.app();

    let frame = frame_at(&app, 100, 24);
    let (_, row) = find_char_in(&frame, box_area(), '⚑').expect("a flag is drawn");
    app.on_mouse(click(60, row + 1))
        .expect("click the second flag");

    assert_eq!(app.focus(), Focus::Flag);
    assert_eq!(app.context(), Context::Flag);
    assert_eq!(
        app.selected_flag().expect("a selected flag").reason,
        "second"
    );
    let frame = buffer_text(&frame_at(&app, 100, 24));
    let bar = frame.lines().last().unwrap_or_default();
    assert!(bar.contains(" FLAG "), "the bar does not say FLAG:\n{bar}");
}

/// `Enter` with the row cursor resting on a flag row steps onto that flag;
/// `j`/`k` walk the line's flags; `Esc` steps back to the diff.
#[test]
fn enter_on_a_flag_row_steps_onto_it_and_j_k_walk_the_flags() {
    let workspace = two_flags_on_line_one();
    let mut app = workspace.app();
    app.on_key(KeyCode::Down).expect("onto the first flag row");
    assert_eq!(app.focus(), Focus::Diff);

    app.on_key(KeyCode::Enter).expect("onto the flag");
    assert_eq!(app.focus(), Focus::Flag);
    assert_eq!(app.selected_flag().expect("a flag").reason, "first");
    app.on_key(KeyCode::Down).expect("next flag");
    assert_eq!(app.selected_flag().expect("a flag").reason, "second");
    app.on_key(KeyCode::Down).expect("the end stays");
    assert_eq!(app.selected_flag().expect("a flag").reason, "second");
    app.on_key(KeyCode::Up).expect("prev flag");
    assert_eq!(app.selected_flag().expect("a flag").reason, "first");

    app.on_key(KeyCode::Esc).expect("back out");
    assert_eq!(app.focus(), Focus::Diff);
    assert!(
        app.selected_flag().is_none(),
        "off the flag nothing is selected"
    );
}

/// `D` on a flag asks about that flag and deletes it; `A` acknowledges that
/// flag alone, leaving the line's other flag open.
#[test]
fn shift_d_and_shift_a_act_on_the_selected_flag_only() {
    let workspace = two_flags_on_line_one();
    let mut app = workspace.app();
    app.on_key(KeyCode::Down).expect("onto the first flag row");
    app.on_key(KeyCode::Enter).expect("onto the flag");
    app.on_key(KeyCode::Down).expect("the second");

    app.on_key(KeyCode::Char('A')).expect("ack");
    let flags = workspace.store().flags().expect("read");
    assert!(
        flags[1].acknowledged,
        "A did not acknowledge the selected flag"
    );
    assert!(!flags[0].acknowledged, "A acknowledged the other flag too");

    app.on_key(KeyCode::Char('D')).expect("delete");
    assert!(
        app.status().contains("delete flag at a.rs:1"),
        "{}",
        app.status()
    );
    app.on_key(KeyCode::Char('y')).expect("confirm");
    let flags = workspace.store().flags().expect("read");
    assert_eq!(flags.len(), 1);
    assert_eq!(flags[0].reason, "first");
    assert_eq!(
        app.focus(),
        Focus::Flag,
        "a delete keeps the cursor on the flags"
    );
    assert_eq!(app.selected_flag().expect("a flag").reason, "first");

    app.on_key(KeyCode::Char('D')).expect("delete the last");
    app.on_key(KeyCode::Char('y')).expect("confirm");
    assert_eq!(
        app.focus(),
        Focus::Diff,
        "an emptied line hands the focus back"
    );
}

/// The `?` tip is generated from the keymap, so a flag's `D` shows up there
/// with no hand-listing: live on the flag, dimmed out on the plain line.
#[test]
fn the_tip_lists_delete_on_a_flag_and_not_on_its_line() {
    let workspace = two_flags_on_line_one();
    let mut app = workspace.app();

    app.on_key(KeyCode::Char('?')).expect("?");
    let line = buffer_text(&frame_at(&app, 100, 30));
    assert!(line.contains("DIFF"), "{line}");
    assert!(
        !line.contains("delete"),
        "D is not live on a bare line:\n{line}"
    );
    app.on_key(KeyCode::Esc).expect("close");

    app.on_key(KeyCode::Down).expect("onto the flag row");
    app.on_key(KeyCode::Enter).expect("onto the flag");
    app.on_key(KeyCode::Char('?')).expect("?");
    let flag = buffer_text(&frame_at(&app, 100, 30));
    assert!(flag.contains("FLAG "), "{flag}");
    assert!(flag.contains("D      delete"), "{flag}");
    assert!(flag.contains("A      ack flag"), "{flag}");
}

/// Moving the row cursor leaves the flag, as it leaves a comment stack.
#[test]
fn moving_the_line_cursor_leaves_the_flag() {
    let workspace = two_flags_on_line_one();
    let mut app = workspace.app();
    app.on_key(KeyCode::Down).expect("onto the flag row");
    app.on_key(KeyCode::Enter).expect("onto the flag");
    assert_eq!(app.focus(), Focus::Flag);

    app.on_key(KeyCode::Char(']')).expect("next file");
    assert_eq!(app.focus(), Focus::Diff);
    assert_eq!(app.flag_index(), 0);
}
