//! The before/after block an outdated comment expands into: the code it was
//! written against, diffed against the code standing there now.
//!
//! Split out of [`super::boxes_1`], which the block's own tests pushed past
//! reading size. Storage spec §4 is the ruling these assert.

use std::collections::HashSet;

use crossterm::event::KeyCode;
use rv::app::App;
use rv::rows::BodyKind;
use rv::rows::Row;
use rv_core::diff::DiffSource;
use rv_core::diff::LineKind;
use rv_core::store::CommentState;

use crate::support::*;

/// Puts `body` on `a.rs`'s second line, rewrites that line, and files the
/// comment as outdated — the state an agent's `.review/`, or a rebase, arrives
/// in.
///
/// Through the store rather than through a derivation, for the reason
/// `an_outdated_comment_is_grey_and_folded` gives: the reviewer cannot yet reach
/// the state from the keyboard. The rewrite is what gives the block something to
/// show — the stored excerpt and the code now standing in its place genuinely
/// differ.
fn outdated_over_rewritten_code(workspace: &Fixture, body: &str) -> App {
    let mut app = workspace.app();
    select_line(&mut app, |line| line.text.contains("let x = 1;"));
    write_comment(&mut app, body);

    workspace.write("a.rs", "fn a() {\n    let x = 99;\n}\n");
    workspace.jj(&["describe", "-m", "rewrite the commented line"]);
    workspace.jj(&["new"]);

    let mut stored = workspace.store().comments().expect("read comments");
    stored[0].state = CommentState::Outdated;
    workspace
        .store()
        .append_comment(&stored[0])
        .expect("store the outdated comment");

    drop(app);
    workspace.app()
}

/// Expanding an outdated comment opens the before/after block: the code the
/// comment was written against, against the code standing there now, inside the
/// same box (storage spec §4).
///
/// This is what the anchor's stored context is *for*. Without it an outdated
/// comment is a grey row asserting that something changed and declining to say
/// what, which is the one question a reviewer coming back to a review after an
/// agent has been editing needs answered.
///
/// Asserted on the rendered cells, and on the *sides*: a block that printed both
/// versions without saying which was which would pass a text-only check while
/// telling the reviewer nothing.
#[test]
fn an_expanded_outdated_comment_shows_the_stored_context_against_the_code_now() {
    let workspace = Fixture::new();
    let mut app = outdated_over_rewritten_code(&workspace, "this is about the old line");

    // It opens folded, as every settled comment does, so the block is behind the
    // same `s` that expands any other box.
    let folded = buffer_text(&frame_at(&app, 100, 30));
    assert!(
        !folded.contains("when this was written"),
        "a folded outdated comment already spends rows on its block:\n{folded}"
    );

    select_line(&mut app, |line| line.text.contains("let x = 99;"));
    app.on_key(KeyCode::Char('s')).expect("expand the box");

    let buffer = frame_at(&app, 100, 30);
    let inside = text_in(&buffer, box_area());
    assert!(
        inside.contains("when this was written") && inside.contains("now"),
        "the block does not say which half is which:\n{inside}"
    );
    assert!(
        inside.contains("- ") && inside.contains("    let x = 1;"),
        "the block does not show the code the comment was written against:\n{inside}"
    );
    assert!(
        inside.contains("+ ") && inside.contains("    let x = 99;"),
        "the block does not show the code that is there now:\n{inside}"
    );

    // Inside the box, not beside it: the rows are between the comment's own
    // borders, which is what makes it a contained block rather than a modal.
    let rows = rows_of(&buffer);
    let top = row_holding(&buffer, "this is about the old line");
    let bottom = rows
        .iter()
        .enumerate()
        .skip(top)
        .find(|(_, row)| row.contains('╰'))
        .map(|(index, _)| index)
        .expect("the box has a bottom border");
    let stored_line = row_holding(&buffer, "let x = 1;");
    assert!(
        top < stored_line && stored_line < bottom,
        "the before/after escaped the box it belongs to:\n{}",
        buffer_text(&buffer)
    );

    // Red for what was, green for what is: the two hues this interface already
    // spends on a removal and an addition.
    let row = u16::try_from(stored_line).expect("a small row");
    assert_eq!(
        style_of_text(&buffer, row, "- ").fg,
        Some(colour(rv::gradient::REMOVED)),
        "the stored line is not marked as the old one:\n{inside}"
    );
}

/// The block is drawn by the in-process engine and never by difftastic.
///
/// difftastic is a flat ~26 ms process spawn per call, and this block renders on
/// the paint path — the comment browser can hold many outdated rows, so
/// `diff::compute` would put one spawn per row into every frame. Storage spec §4
/// rules it `compute_with(.., false)` for that reason and for a second one: a
/// slice of stored context lines is not a parseable file, so the language
/// difftastic infers from the path would be right about the file and wrong about
/// the fragment.
///
/// Proved by contrast rather than by inspecting the call: the app runs
/// `DiffEngine::Structural`, so in this very process, with `difft` on the same
/// `PATH`, the pane's own diff *is* difftastic's — and the block's is not.
#[test]
fn the_before_after_block_never_spawns_difftastic() {
    let workspace = Fixture::new();
    let app = outdated_over_rewritten_code(&workspace, "about the old line");

    let pane = app.selected_diff().expect("a loaded diff");
    assert!(
        matches!(pane.source, DiffSource::Difftastic { .. }),
        "difftastic is not reachable here, so this test proves nothing: {:?}",
        pane.source
    );

    let comment = &app.comments()[0];
    let drift = app.drift(comment).expect("the comment was surveyed");
    let block = drift
        .before_after
        .as_ref()
        .expect("an outdated comment carries a before/after");
    assert!(
        !matches!(block.source, DiffSource::Difftastic { .. }),
        "the before/after block spawned difftastic on the paint path: {:?}",
        block.source
    );
    assert!(
        block
            .lines
            .iter()
            .any(|line| line.kind == LineKind::Removed),
        "the block found nothing to show, so its engine is untested: {:?}",
        block.lines
    );
}

/// A comment on the head side of a file the head no longer has cannot be placed
/// at all, so the block says so and prints the stored lines alone — still the
/// most useful thing available (storage spec §4).
///
/// [`Fixture::mixed`] deletes `removed.rs` between the two endpoints, so it is a
/// file the review lists and the head has no blob for, and no rename record
/// leads anywhere from it. That is a state an agent's `.review/` really reaches:
/// a finding filed against a head-side line of a file a later change dropped.
#[test]
fn a_comment_whose_anchor_cannot_be_placed_says_so_and_shows_what_it_was_written_against() {
    let workspace = Fixture::mixed();
    let mut app = workspace.app_from("@--");
    select_line(&mut app, |line| line.text.contains("new line 3"));
    write_comment(&mut app, "about code that has gone");
    drop(app);

    let mut displaced = workspace
        .store()
        .comments()
        .expect("read comments")
        .remove(0);
    displaced.anchor.file = "removed.rs".to_owned();
    workspace
        .store()
        .append_comment(&displaced)
        .expect("store the displaced comment");

    let reopened = workspace.app_from("@--");
    let comment = &reopened.comments()[0];
    assert_eq!(
        comment.state,
        CommentState::Outdated,
        "an anchor with no blob to resolve against was not derived outdated"
    );
    let drift = reopened.drift(comment).expect("the comment was surveyed");
    assert!(
        !drift.located,
        "an anchor with no text to resolve against was reported as placed"
    );

    let block = drift
        .before_after
        .as_ref()
        .expect("an outdated comment carries a before/after");
    assert!(
        block
            .lines
            .iter()
            .any(|line| line.text.contains("new line 3")),
        "the stored lines were dropped along with the anchor: {:?}",
        block.lines
    );

    // And the block says which of the two it is showing, rather than presenting
    // orphaned context as though it were a diff against something.
    //
    // Through `rv::rows::plan` rather than through a frame, because a box only
    // draws under a diff line its anchor matches — and an anchor with no blob
    // behind it has, by construction, no such line. `plan` is the pure
    // state-to-rows function that decides the note, so this asserts the very
    // rows the pane would draw if the comment had a line to hang from.
    let diff = reopened.selected_diff().expect("a loaded diff").clone();
    let plan = rv::rows::plan(
        &diff.lines,
        &|line| if line == 0 { vec![comment] } else { Vec::new() },
        &|_| Some(drift),
        &HashSet::new(),
        60,
    );
    let notes: Vec<&str> = plan
        .rows
        .iter()
        .filter_map(|row| match row {
            Row::BoxBody {
                text,
                kind: BodyKind::Note,
                ..
            } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert!(
        notes
            .iter()
            .any(|note| note.contains("could not be located")),
        "the block presents orphaned context as a before/after: {notes:?}"
    );
}
