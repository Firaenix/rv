//! The comment stack.


use crossterm::event::KeyCode;
use rstest::rstest;
use rv::app::Focus;

use crate::support::*;

/// `Enter` steps into the comments on the selected line, and `Esc` steps back
/// out — the round trip a reviewer makes to pick one comment out of a stack.
#[test]
fn enter_steps_into_the_comment_stack_and_esc_leaves_it() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, "needs a doc");

    app.on_key(KeyCode::Enter).expect("enter the stack");
    assert_eq!(app.focus(), Focus::Stack);
    assert_eq!(
        app.comment_index(),
        0,
        "the stack opens on its first comment"
    );
    assert_eq!(
        app.selected_comment().expect("a selected comment").body,
        "needs a doc"
    );

    app.on_key(KeyCode::Esc).expect("leave the stack");
    assert_eq!(app.focus(), Focus::Diff);
    assert!(
        app.selected_comment().is_none(),
        "nothing is selected once the cursor is back on the diff"
    );
}

/// A focus a reviewer cannot get out of is a trap, so `Left` leaves the stack
/// as surely as `Esc` does — the same key that leaves every other focus.
#[test]
fn left_also_leaves_the_stack() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, "needs a doc");

    app.on_key(KeyCode::Enter).expect("enter the stack");
    assert_eq!(app.focus(), Focus::Stack);
    app.on_key(KeyCode::Left).expect("left");
    assert_eq!(app.focus(), Focus::Diff);
}

/// `Enter` on a line with nothing on it says so rather than moving the cursor
/// into an empty stack, which would be a focus with nothing in it and no
/// obvious way back.
#[test]
fn enter_on_a_line_without_comments_says_so_and_stays_put() {
    let workspace = Fixture::new();
    let mut app = workspace.app();

    app.on_key(KeyCode::Enter).expect("enter");

    assert_eq!(app.focus(), Focus::Diff, "focus did not move");
    assert!(
        app.status().contains("no comments"),
        "and it said why: {:?}",
        app.status()
    );
}

/// Inside the stack the movement keys move between comments, and they clamp at
/// both ends the way they do everywhere else in the reviewer.
#[rstest]
#[case(KeyCode::Char('j'), KeyCode::Char('k'))]
#[case(KeyCode::Down, KeyCode::Up)]
fn both_key_pairs_move_between_the_comments_in_a_stack(
    #[case] forward: KeyCode,
    #[case] back: KeyCode,
) {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, "first finding");
    write_comment(&mut app, "second finding");

    app.on_key(KeyCode::Enter).expect("enter the stack");
    assert_eq!(
        app.selected_comment().expect("the first").body,
        "first finding",
        "the stack opens on the oldest comment"
    );

    app.on_key(forward).expect("next");
    assert_eq!(
        app.selected_comment().expect("the second").body,
        "second finding"
    );
    app.on_key(forward).expect("past the end");
    assert_eq!(
        app.selected_comment().expect("still the second").body,
        "second finding",
        "the cursor stops at the newest rather than wrapping"
    );

    app.on_key(back).expect("back");
    assert_eq!(
        app.selected_comment().expect("the first again").body,
        "first finding"
    );
    app.on_key(back).expect("past the start");
    assert_eq!(
        app.selected_comment().expect("still the first").body,
        "first finding"
    );
    assert_eq!(
        app.line_index(),
        0,
        "moving inside the stack did not move the diff underneath it"
    );
}

/// `c` means the same thing inside the stack as outside it: another comment on
/// the line the reviewer is looking at, added to the end of that line's stack.
#[test]
fn c_from_the_stack_adds_another_comment_to_the_same_line() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, "first finding");
    let line = app.line_index();

    app.on_key(KeyCode::Enter).expect("enter the stack");
    write_comment(&mut app, "second finding");

    let on_line = app.comments_for_line(line);
    assert_eq!(on_line.len(), 2, "both are on the line: {on_line:?}");
    assert_eq!(on_line[1].body, "second finding", "the new one is last");
    assert_eq!(
        app.focus(),
        Focus::Stack,
        "saving from the stack leaves the cursor where it was"
    );
    assert_eq!(
        workspace.store().comments().expect("read comments").len(),
        2,
        "and both reached the store"
    );
}

/// The stack index belongs to the line it was opened on, so moving the
/// selection puts it back at the top rather than leaving it pointing at another
/// line's comment.
#[test]
fn moving_the_selection_puts_the_stack_index_back_at_the_top() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, "first finding");
    write_comment(&mut app, "second finding");

    app.on_key(KeyCode::Enter).expect("enter the stack");
    app.on_key(KeyCode::Char('j')).expect("select the second");
    assert_eq!(app.comment_index(), 1);

    app.on_key(KeyCode::Left).expect("back to the diff");
    app.on_key(KeyCode::Char('j')).expect("next line");
    assert_eq!(app.focus(), Focus::Diff);
    assert_eq!(app.comment_index(), 0, "the stack index came back to 0");
}

/// Navigating out of a stack leaves it — **whatever is on the line navigated
/// to**.
///
/// Entering a stack is something a reviewer does on purpose, with `Enter`, on a
/// line they picked. `]` is not that, so it may not hand the focus on: landing
/// inside the next file's stack, one the reviewer never opened, points `d` and
/// `s` at a comment they have not seen and did not select — and `d` is
/// unrecoverable.
///
/// Both files carry a comment on the line `]` lands on, which is the whole
/// point of the fixture below. An earlier version of this test commented on one
/// file only, so the focus left the stack because the new line's stack was
/// *empty*; it passed against an implementation that kept the focus whenever
/// the new line had comments, which is the bug. The `stack ahead` assertion is
/// there to keep it from going quiet that way again.
#[test]
fn navigating_to_another_file_leaves_the_stack_even_when_that_line_has_comments() {
    let workspace = Fixture::new();
    let mut app = workspace.app();

    // A comment on b.rs's first line, reached and left the way a reviewer
    // would, so that `]` below lands on a line that is *not* comment-free.
    app.on_key(KeyCode::Char(']')).expect("next file");
    write_comment(&mut app, "on the second file");
    app.on_key(KeyCode::Char('[')).expect("back to the first");
    assert_eq!(app.file_index(), 0);

    write_comment(&mut app, "first finding");
    write_comment(&mut app, "second finding");
    app.on_key(KeyCode::Enter).expect("enter the stack");
    app.on_key(KeyCode::Char('j')).expect("select the second");
    assert_eq!(app.focus(), Focus::Stack);
    assert_eq!(app.comment_index(), 1);

    app.on_key(KeyCode::Char(']')).expect("next file");

    assert_eq!(
        app.comments_for_line(app.line_index()).len(),
        1,
        "the line `]` landed on has no stack, so this test proves nothing"
    );
    assert_eq!(
        app.focus(),
        Focus::Diff,
        "`]` carried the cursor into a stack the reviewer never entered"
    );
    assert_eq!(app.comment_index(), 0, "the stack index came back to 0");
    assert!(
        app.selected_comment().is_none(),
        "a comment is selected on a line the reviewer only just arrived at: {:?}",
        app.selected_comment()
    );
}

/// The same rule on the way back: `[` out of a stack lands on the diff, not
/// inside the previous file's stack.
#[test]
fn navigating_back_to_a_file_with_comments_also_leaves_the_stack() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, "on the first file");

    app.on_key(KeyCode::Char(']')).expect("next file");
    write_comment(&mut app, "on the second file");
    app.on_key(KeyCode::Enter).expect("enter the stack");
    assert_eq!(app.focus(), Focus::Stack);

    app.on_key(KeyCode::Char('[')).expect("back to the first");

    assert_eq!(
        app.comments_for_line(app.line_index()).len(),
        1,
        "the line `[` landed on has no stack, so this test proves nothing"
    );
    assert_eq!(
        app.focus(),
        Focus::Diff,
        "`[` carried the cursor into a stack the reviewer never entered"
    );
}
