//! Resolving and abandoning a comment.

use crossterm::event::KeyCode;
use ratatui::style::Modifier;
use rv::app::Focus;
use rv_core::store::CommentState;
use rv_core::store::SettledBy;

use crate::support::*;

/// The state the store holds for the review's one comment.
fn stored_state(workspace: &Fixture) -> (CommentState, Option<SettledBy>) {
    let comments = workspace.store().comments().expect("read the comments");
    let comment = comments.first().expect("one comment");
    (comment.state, comment.settled_by)
}

#[rstest::rstest]
#[case::resolve(KeyCode::Char('r'), CommentState::Resolved)]
#[case::abandon(KeyCode::Char('a'), CommentState::Abandoned)]
fn a_key_settles_the_comment_and_records_who_did_it(
    #[case] key: KeyCode,
    #[case] expected: CommentState,
) {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, "needs a second look");

    app.on_key(KeyCode::Char('c')).expect("comment leader");
    app.on_key(key).expect("settle the comment");

    assert_eq!(
        stored_state(&workspace),
        (expected, Some(SettledBy::User)),
        "the store did not record the state and the actor"
    );
}

/// Pressing the same key again is the undo, which is why neither asks first.
#[rstest::rstest]
#[case::resolve(KeyCode::Char('r'))]
#[case::abandon(KeyCode::Char('a'))]
fn pressing_it_twice_puts_the_comment_back_to_open(#[case] key: KeyCode) {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, "on second thoughts");

    app.on_key(KeyCode::Char('c')).expect("comment leader");
    app.on_key(key).expect("settle");
    app.on_key(KeyCode::Char('c')).expect("comment leader");
    app.on_key(key).expect("unsettle");

    assert_eq!(
        stored_state(&workspace),
        (CommentState::Open, None),
        "reopening left the comment settled, or left an actor on an open comment"
    );
}

/// The two are not interchangeable: settling one way from the other is a change
/// of mind, not a toggle back to open.
#[test]
fn abandoning_a_resolved_comment_abandons_it() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, "fixed, or maybe not");

    app.on_key(KeyCode::Char('c')).expect("comment leader");
    app.on_key(KeyCode::Char('r')).expect("resolve");
    app.on_key(KeyCode::Char('c')).expect("comment leader");
    app.on_key(KeyCode::Char('a')).expect("abandon");

    assert_eq!(stored_state(&workspace).0, CommentState::Abandoned);
}

/// Settling is not deleting: the comment stays in the review, and its words
/// stay with it.
#[test]
fn a_settled_comment_is_still_in_the_store_with_its_body() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, "the body survives");

    app.on_key(KeyCode::Char('c')).expect("comment leader");
    app.on_key(KeyCode::Char('a')).expect("abandon");

    let comments = workspace.store().comments().expect("read the comments");
    assert_eq!(comments.len(), 1, "abandoning removed the comment");
    assert_eq!(comments[0].body, "the body survives");
}

/// The screen has to say which of the two happened. Resolved earns a tick;
/// abandoned is struck through, because *fixed* and *dropped unfixed* are
/// different conclusions and a reader who cannot tell them apart is reading a
/// summary that lies.
#[test]
fn the_two_settled_states_are_told_apart_on_screen() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, "a remark");

    app.on_key(KeyCode::Char('c')).expect("comment leader");
    app.on_key(KeyCode::Char('r')).expect("resolve");
    let resolved = frame_at(&app, 100, 24);
    assert!(
        buffer_text(&resolved).contains("resolved"),
        "a resolved comment does not say so:\n{}",
        buffer_text(&resolved)
    );

    app.on_key(KeyCode::Char('c')).expect("comment leader");
    app.on_key(KeyCode::Char('a')).expect("abandon");
    let frame = frame_at(&app, 100, 24);
    let text = buffer_text(&frame);
    assert!(
        text.contains("abandoned"),
        "an abandoned comment does not say so:\n{text}"
    );

    let row = u16::try_from(row_holding(&frame, "abandoned")).expect("a small row");
    let style = style_of_text(&frame, row, "abandoned");
    assert!(
        style.add_modifier.contains(Modifier::CROSSED_OUT),
        "an abandoned comment is not struck through: {style:?}"
    );
}

/// An agent settling its own finding is allowed; hiding that it did is not.
#[test]
fn an_agent_settled_comment_says_it_was_the_agent() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, "found by a model");

    let id = workspace.store().comments().expect("read")[0].id.clone();
    workspace
        .store()
        .settle_comment(&id, CommentState::Resolved, SettledBy::Agent)
        .expect("settle as the agent");

    // Reopened rather than refreshed in place: this is the reviewer coming back
    // to a review a model has been working through, which is the whole reason
    // the actor is recorded.
    let reopened = workspace.app();
    let text = buffer_text(&frame_at(&reopened, 100, 24));
    assert!(
        text.contains("resolved by agent"),
        "the screen does not say the agent resolved it:\n{text}"
    );
}

/// The file list selects files, so there is nothing under the cursor to settle
/// — and saying so beats settling something the reviewer cannot see.
#[rstest::rstest]
#[case::resolve(KeyCode::Char('r'))]
#[case::abandon(KeyCode::Char('a'))]
fn the_file_list_settles_nothing_and_says_why(#[case] key: KeyCode) {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, "a remark");
    app.on_key(KeyCode::Left).expect("focus the file list");
    assert_eq!(app.focus(), Focus::Sidebar);

    app.on_key(KeyCode::Char('c')).expect("comment leader");
    app.on_key(key).expect("press it from the file list");

    assert_eq!(
        stored_state(&workspace).0,
        CommentState::Open,
        "a key pressed at the file list settled a comment"
    );
    assert!(
        app.status().contains("comments"),
        "the refusal does not say what the key is for: {:?}",
        app.status()
    );
}

/// `outdated` is derived on load, and un-derives itself when the code comes
/// back.
///
/// `rv_core::anchor::resolve` had been written and tested since milestone 1 and
/// was called by nothing, so a comment read as `open` however far its code had
/// moved — including a comment about a line that no longer existed. The store
/// still holds `open`, because the state is a fact about the *current* text and
/// a stored flag would need something to invalidate it.
#[test]
fn a_comment_whose_line_has_gone_reads_outdated_without_being_stored_so() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, "about this exact line");
    assert_eq!(stored_state(&workspace).0, CommentState::Open);

    // The file is cut down until the commented line's number does not exist:
    // no content match anywhere, and no line to fall back on weakly.
    workspace.write("a.rs", "");
    workspace.jj(&["describe", "-m", "empty the file"]);
    workspace.jj(&["new"]);

    let reopened = workspace.app();
    assert_eq!(
        reopened.comments().first().map(|comment| comment.state),
        Some(CommentState::Outdated),
        "the comment still claims to describe code that has gone"
    );
    assert_eq!(
        stored_state(&workspace).0,
        CommentState::Open,
        "the derived state was written to disk"
    );
}

/// A line rewritten *in place* keeps its comment, weakly: the content is gone
/// but "line n of this file" still exists, and the anchor falls back to it
/// (the branch spec §9's third tier) rather than declaring the comment
/// unplaceable while its line is visibly there.
#[test]
fn a_rewritten_line_keeps_its_comment_on_the_weak_tier() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, "about this exact line");

    workspace.write("a.rs", "fn completely_different() {\n    let y = 9;\n}\n");
    workspace.jj(&["describe", "-m", "rewrite the file"]);
    workspace.jj(&["new"]);

    let reopened = workspace.app();
    assert_eq!(
        reopened.comments().first().map(|comment| comment.state),
        Some(CommentState::Open),
        "a weak anchor is a placed anchor, not an outdated one"
    );
}

/// And a comment whose code is still there is not swept up with it.
#[test]
fn a_comment_on_live_code_stays_open() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, "about live code");

    let reopened = workspace.app();
    assert_eq!(
        reopened.comments().first().map(|comment| comment.state),
        Some(CommentState::Open)
    );
}

/// A settled comment is not re-opened as outdated by a later edit: it was
/// addressed, which is a fact about what happened rather than about the text.
#[test]
fn a_resolved_comment_does_not_become_outdated() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, "fixed, then the file moved on");
    app.on_key(KeyCode::Char('c')).expect("comment leader");
    app.on_key(KeyCode::Char('r')).expect("resolve");

    workspace.write("a.rs", "fn completely_different() {\n    let y = 9;\n}\n");
    workspace.jj(&["describe", "-m", "rewrite the file"]);
    workspace.jj(&["new"]);

    let reopened = workspace.app();
    assert_eq!(
        reopened.comments().first().map(|comment| comment.state),
        Some(CommentState::Resolved),
        "a resolved comment was reported as outdated"
    );
}
