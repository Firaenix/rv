//! Deleting a comment.

use std::fs;

use crossterm::event::KeyCode;
use rstest::rstest;
use rv::app::Action;
use rv::app::Focus;
use rv::app::Mode;

use crate::support::*;

/// `d` asks before it deletes, and `y` answers. Deletion is unrecoverable, so
/// the one thing that must never happen is a mistyped key costing written work.
#[test]
fn d_then_y_deletes_the_comment_from_the_store() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, "needs a doc");
    let line = app.line_index();

    app.on_key(KeyCode::Char('c')).expect("comment leader");
    app.on_key(KeyCode::Char('d')).expect("ask");
    assert!(
        matches!(app.mode(), Mode::ConfirmDelete { .. }),
        "it asked first, rather than deleting: {:?}",
        app.mode()
    );
    assert!(
        app.status().contains("delete") && app.status().contains("a.rs:1"),
        "and said what it would delete: {:?}",
        app.status()
    );
    assert_eq!(
        workspace.store().comments().expect("read").len(),
        1,
        "asking the question did not delete anything on its own"
    );

    app.on_key(KeyCode::Char('y')).expect("confirm");

    assert_eq!(app.mode(), Mode::Browse);
    assert!(
        app.comments_for_line(line).is_empty(),
        "gone from the view: {:?}",
        app.comments_for_line(line)
    );
    assert!(
        workspace.store().comments().expect("read").is_empty(),
        "gone from a freshly opened store, which is the authority"
    );
}

/// Neither answer to `d` rewrites `REVIEW-FEEDBACK.md`. The markdown is an
/// *export* (see the storage-model spec) produced by `rv render`, and the store
/// is what a review is kept in; a delete that rewrote the export would be
/// reaching past the store to edit a document somebody else may be reading.
///
/// Both answers, in *this* file, because they fail differently. A confirmed
/// delete that rewrote the export drops whatever reply an LLM appended; a
/// **cancelled** one does that while the reviewer is being told nothing
/// happened, which is the worse of the two and the more likely keystroke — `d`
/// is next to `s` and `f`, and the answer to a mistyped one is `n`. The cancel
/// path had one guard, inside `--test app_cases`'s fuzz walk, and the two
/// targets are run separately: a wave that broke it while working in this file
/// would have seen this file stay green.
#[rstest]
#[case::confirmed(KeyCode::Char('y'), 0)]
#[case::cancelled(KeyCode::Char('n'), 1)]
fn deleting_a_comment_does_not_rewrite_the_export(#[case] answer: KeyCode, #[case] left: usize) {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, "needs a doc");

    // Seed an export by hand, so that what is on disk cannot have come from
    // this delete: any rewrite would drop the comment and this sentence both.
    const SEEDED: &str = "<!-- rv:v1 -->\nstale on purpose\n";
    let export = workspace.store().markdown_path();
    fs::write(&export, SEEDED).expect("seed an export");
    let before = fs::metadata(&export)
        .expect("stat")
        .modified()
        .expect("mtime");

    app.on_key(KeyCode::Char('c')).expect("comment leader");
    app.on_key(KeyCode::Char('d')).expect("ask");
    app.on_key(answer).expect("answer");

    assert_eq!(
        workspace.store().comments().expect("read").len(),
        left,
        "{answer:?} did not do what it says on the tin, so this proves nothing"
    );
    assert_eq!(
        fs::read_to_string(&export).expect("read the export"),
        SEEDED,
        "{answer:?} rewrote the export"
    );
    assert_eq!(
        fs::metadata(&export)
            .expect("stat")
            .modified()
            .expect("mtime"),
        before,
        "{answer:?} rewrote the export, even if with the same bytes"
    );
}

/// The other answer. `n` — or anything that is not `y` — leaves the comment
/// exactly where it was, in the view and on disk.
#[test]
fn d_then_anything_else_cancels_and_keeps_the_comment() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, "needs a doc");
    let line = app.line_index();

    app.on_key(KeyCode::Char('c')).expect("comment leader");
    app.on_key(KeyCode::Char('d')).expect("ask");
    app.on_key(KeyCode::Char('n')).expect("decline");

    assert_eq!(app.mode(), Mode::Browse);
    assert!(
        app.status().contains("cancelled"),
        "the reviewer is told nothing happened: {:?}",
        app.status()
    );
    assert_eq!(app.comments_for_line(line).len(), 1, "still there");
    let stored = workspace.store().comments().expect("read");
    assert_eq!(stored.len(), 1, "and still on disk: {stored:?}");
    assert_eq!(stored[0].body, "needs a doc");
}

/// No keystroke leaves the reviewer stuck at the question. Whatever is pressed,
/// the confirmation is answered and the app is back in `Browse` — deleting on
/// `y` and on nothing else.
#[rstest]
#[case::confirm(KeyCode::Char('y'), true)]
#[case::decline(KeyCode::Char('n'), false)]
#[case::uppercase_is_not_a_confirmation(KeyCode::Char('Y'), false)]
#[case::quit_does_not_leak_out_of_the_question(KeyCode::Char('q'), false)]
#[case::another_d(KeyCode::Char('d'), false)]
#[case::comment_key(KeyCode::Char('c'), false)]
#[case::escape(KeyCode::Esc, false)]
#[case::enter(KeyCode::Enter, false)]
#[case::space(KeyCode::Char(' '), false)]
#[case::backspace(KeyCode::Backspace, false)]
#[case::arrow(KeyCode::Left, false)]
#[case::movement(KeyCode::Down, false)]
#[case::tab(KeyCode::Tab, false)]
#[case::function(KeyCode::F(1), false)]
fn every_key_answers_the_confirmation(#[case] key: KeyCode, #[case] deletes: bool) {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, "needs a doc");

    app.on_key(KeyCode::Char('c')).expect("comment leader");
    app.on_key(KeyCode::Char('d')).expect("ask");
    let action = app.on_key(key).expect("answer");

    assert_eq!(
        action,
        Action::Continue,
        "{key:?} ended the review from inside a confirmation"
    );
    assert_eq!(
        app.mode(),
        Mode::Browse,
        "{key:?} left the reviewer waiting on a question it will never be asked again"
    );
    assert_eq!(
        workspace.store().comments().expect("read").len(),
        usize::from(!deletes),
        "{key:?} deleted the wrong number of comments"
    );
    // ...and the keystroke was consumed by the answer rather than also doing
    // whatever it means while browsing.
    assert_eq!(app.buffer(), "", "{key:?} opened a comment buffer");
    assert_eq!(app.focus(), Focus::Diff);
}

/// From the diff, `d` targets the newest comment on the line — the one a
/// reviewer has just written and is most likely to want back — and says which
/// of how many went.
#[test]
fn from_the_diff_d_targets_the_newest_and_reports_how_many_there_were() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, "first finding");
    write_comment(&mut app, "second finding");
    let line = app.line_index();

    app.on_key(KeyCode::Char('c')).expect("comment leader");
    app.on_key(KeyCode::Char('d')).expect("ask");
    app.on_key(KeyCode::Char('y')).expect("confirm");

    let left = app.comments_for_line(line);
    assert_eq!(left.len(), 1, "{left:?}");
    assert_eq!(left[0].body, "first finding", "the newest went");
    assert!(
        app.status().contains("1 of 2"),
        "and it said so: {:?}",
        app.status()
    );
    let stored = workspace.store().comments().expect("read");
    assert_eq!(stored.len(), 1, "{stored:?}");
    assert_eq!(stored[0].body, "first finding");
}

/// From inside the stack, `d` targets what the cursor is on. The two rules have
/// to differ: on the diff there is no cursor in the stack to mean anything.
#[test]
fn from_the_stack_d_targets_the_selected_comment() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, "first finding");
    write_comment(&mut app, "second finding");
    let line = app.line_index();

    app.on_key(KeyCode::Enter).expect("enter the stack");
    assert_eq!(
        app.selected_comment().expect("a selection").body,
        "first finding",
        "the cursor is on the oldest, which is not the one `d` would take from the diff"
    );
    app.on_key(KeyCode::Char('c')).expect("comment leader");
    app.on_key(KeyCode::Char('d')).expect("ask");
    app.on_key(KeyCode::Char('y')).expect("confirm");

    let left = app.comments_for_line(line);
    assert_eq!(left.len(), 1, "{left:?}");
    assert_eq!(left[0].body, "second finding", "the selected one went");
    assert_eq!(
        app.focus(),
        Focus::Stack,
        "a stack with a comment left in it keeps the cursor"
    );
    assert_eq!(
        app.selected_comment().expect("a selection").body,
        "second finding",
        "and the cursor is clamped onto what is left"
    );
}

/// Deleting the last comment on a line empties the stack, so the cursor comes
/// back to the diff rather than sitting in a pane with nothing in it.
#[test]
fn deleting_the_last_comment_on_a_line_returns_focus_to_the_diff() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, "needs a doc");

    app.on_key(KeyCode::Enter).expect("enter the stack");
    app.on_key(KeyCode::Char('c')).expect("comment leader");
    app.on_key(KeyCode::Char('d')).expect("ask");
    app.on_key(KeyCode::Char('y')).expect("confirm");

    assert_eq!(app.focus(), Focus::Diff, "no cursor left in an empty stack");
    assert_eq!(app.comment_index(), 0);
    assert!(app.selected_comment().is_none());
}

/// From the file list, `d` deletes nothing and says what it would need.
///
/// `c` does write against the selected diff line from the sidebar, and the
/// symmetry argues for `d` doing the same — but the two keys are not
/// symmetrical. `c` creates, and a comment made by mistake is undone by `d`;
/// `d` destroys, and nothing undoes it. The file list shows files, so the
/// comment `d` would take from there is one the reviewer cannot see, on a diff
/// line they may never have opened. The sidebar's *other* tab does have a
/// comment of its own selected, and `d` deletes it — see
/// `d_from_the_comment_browser_deletes_behind_the_same_confirmation`.
#[test]
fn d_from_the_file_list_deletes_nothing() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, "needs a doc");
    let line = app.line_index();

    app.on_key(KeyCode::Left).expect("focus the file list");
    assert_eq!(app.focus(), Focus::Sidebar);
    app.on_key(KeyCode::Char('c')).expect("comment leader");
    app.on_key(KeyCode::Char('d')).expect("d");

    assert_eq!(
        app.mode(),
        Mode::Browse,
        "it opened a confirmation about a comment the file list does not show"
    );
    assert!(
        app.status().contains("not comments"),
        "and it said what it would need instead: {:?}",
        app.status()
    );
    assert_eq!(app.comments_for_line(line).len(), 1, "still there");
    assert_eq!(
        workspace.store().comments().expect("read").len(),
        1,
        "and still on disk"
    );

    // ...and pressing `y` next does not delete it either: there is no question
    // outstanding for `y` to be the answer to.
    app.on_key(KeyCode::Char('y')).expect("y");
    assert_eq!(
        workspace.store().comments().expect("read").len(),
        1,
        "a `d` that refused still armed the confirmation"
    );
}

/// On a line with nothing on it, the comment leader has only one thing it can
/// do — write — so `c` skips its menu and opens the box rather than offering a
/// delete there is nothing to answer. The menu never presents `d` with no
/// comment to remove, so there is no "delete nothing" refusal to escape.
#[test]
fn c_on_an_empty_line_writes_rather_than_offering_delete() {
    let workspace = Fixture::new();
    let mut app = workspace.app();

    app.on_key(KeyCode::Char('c')).expect("comment leader");

    assert_eq!(
        app.mode(),
        Mode::Comment,
        "c on an empty line should smart-collapse to the write"
    );
    assert!(app.pending_leader().is_none(), "no menu should be waiting");
}
