//! The cursor walks rows, so a tall comment can be read.

use std::collections::HashSet;

use crossterm::event::KeyCode;

use crate::support::*;

/// The defect, stated as a test: a comment taller than the diff pane must not
/// have rows that no cursor position can bring on screen.
///
/// It used to. The pane anchored its window on the row of the selected *diff
/// line* and `j` moved that selection to the next diff line, so a box between
/// two diff rows was stepped over rather than scrolled through: from the line
/// above you saw the box's top, from the line below its bottom, and the middle
/// was reachable from nowhere at all. What looked like scrolling jumping
/// through a comment was the pane never scrolling it.
#[test]
fn every_row_of_a_tall_comment_can_be_brought_on_screen() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, &"a very long finding. ".repeat(40));
    let height = 10;

    let mut seen: HashSet<usize> = HashSet::new();
    for _ in 0..80 {
        seen.extend(visible_row_indices(&app, 100, height));
        app.on_key(KeyCode::Down).expect("j");
    }

    let total = row_count(&app, 100, height);
    assert!(
        total > usize::from(height),
        "the comment is not taller than the pane, so this proves nothing: \
         {total} rows in {height}"
    );
    let missed: Vec<usize> = (0..total).filter(|row| !seen.contains(row)).collect();
    assert!(
        missed.is_empty(),
        "rows unreachable at any cursor position: {missed:?}"
    );
}

/// `j` steps *into* the box rather than over it, and the selection everything
/// else depends on stays the line the box hangs from.
#[test]
fn j_walks_into_a_comment_box_rather_than_over_it() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, &"a long finding. ".repeat(20));
    let line = app.line_index();
    let row = app.cursor_row();

    app.on_key(KeyCode::Down).expect("j");
    assert_eq!(
        app.line_index(),
        line,
        "the cursor left the line instead of walking into its comment"
    );
    assert!(
        app.cursor_row() > row,
        "the cursor did not move down a row: {} then {}",
        row,
        app.cursor_row()
    );
}

/// ...so `c` from inside a box comments on the line that box is about, which
/// is the only thing it could sensibly mean.
#[test]
fn commenting_from_inside_a_box_targets_the_line_the_box_belongs_to() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, &"a long finding. ".repeat(20));
    let line = app.line_index();

    app.on_key(KeyCode::Down).expect("step into the box");
    write_comment(&mut app, "a second finding");
    assert_eq!(
        app.comments_for_line(line).len(),
        2,
        "the second comment did not land on the line the box belongs to"
    );
}

/// ...and the box is somewhere the cursor walks *through*, not into: past its
/// last row is the next diff line.
#[test]
fn stepping_past_the_last_row_of_a_box_lands_on_the_next_diff_line() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, "short");
    let line = app.line_index();

    for _ in 0..8 {
        app.on_key(KeyCode::Down).expect("j");
    }
    assert!(
        app.line_index() > line,
        "the cursor never left the box: still on line {}",
        app.line_index()
    );
}

/// Folding the box the cursor is inside leaves the cursor on the line that box
/// belongs to, rather than on a row index that now means something else.
///
/// A fold is one of the three things that rebuild the plan under the cursor —
/// with a save and a delete — and a tall box collapsing to one row takes every
/// row after it with it. Left alone, the cursor would keep pointing at a row
/// past the end of the shortened plan: `line_index` would answer 0 while the
/// pane, which clamps its own anchor, scrolled to the bottom. The two would be
/// describing different places, which is the shape of the defect this whole
/// section exists to have fixed.
#[test]
fn folding_a_box_the_cursor_is_inside_keeps_the_cursor_on_its_line() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, &"a long finding. ".repeat(20));
    let line = app.line_index();
    for _ in 0..6 {
        app.on_key(KeyCode::Down).expect("step into the box");
    }
    assert_eq!(
        app.line_index(),
        line,
        "the fixture's box is not tall enough for the cursor to be inside it"
    );

    app.on_key(KeyCode::Char('s')).expect("fold it");
    assert_eq!(
        app.line_index(),
        line,
        "the fold left the cursor on another line"
    );
    let plan = app.plan();
    assert_eq!(
        plan.line_of_row(app.cursor_row()),
        Some(line),
        "the cursor is on row {} of a plan that has {} rows",
        app.cursor_row(),
        plan.rows.len()
    );
    assert_eq!(
        plan.row_of_line(line),
        Some(app.cursor_row()),
        "the cursor did not land on the folded line's own row"
    );
}
