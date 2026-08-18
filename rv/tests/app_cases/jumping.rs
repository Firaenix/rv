//! Jumping to a comment's code.

use crossterm::event::KeyCode;
use proptest::prelude::*;
use rv::app::Focus;
use rv::app::Mode;
use std::cell::RefCell;

use crate::support::*;

/// Every comment in the browser jumps to a line that shows it.
///
/// The oracle is the app's own display index — `comments_for_line` — rather
/// than the anchor arithmetic the jump uses, which is the point: the jump and
/// the save go through one `anchor_target`, so a jump that landed anywhere else
/// would mean the reviewer's own comment was not on the line the reviewer was
/// sent to. Written through the keyboard, so every anchor under test is one the
/// save path actually made.
#[test]
fn jumping_to_any_comment_lands_on_a_line_that_shows_it() {
    let fixture = Fixture::multi();
    let app = RefCell::new(fixture.app());

    // Only the files with diff lines can carry a comment at all; drawing from
    // the others would spend most cases writing nothing.
    let commentable: Vec<usize> = {
        let app = &mut *app.borrow_mut();
        let count = app.files().len();
        (0..count)
            .filter(|index| {
                rewind(app);
                press_n(app, KeyCode::Char(']'), *index);
                !lines(app).is_empty()
            })
            .collect()
    };
    assert!(
        commentable.len() >= 2,
        "fewer than two files can hold a comment: {commentable:?}"
    );

    let write = (proptest::sample::select(commentable), 0usize..6);
    let seen = Coverage::new(&["a jump that changed file", "a jump inside the open file"]);
    run_cases(24, prop::collection::vec(write, 1..5), |writes| {
        fixture.clear_comments();
        let app = &mut *app.borrow_mut();

        for (index, (file, downs)) in writes.iter().enumerate() {
            rewind(app);
            press_n(app, KeyCode::Char(']'), *file);
            walk_to_line(app, *downs);
            press(app, KeyCode::Char('c'));
            prop_assert_eq!(app.mode(), Mode::Comment);
            // Distinct bodies, so that two writes at one location are two
            // comments rather than one upsert of the other.
            type_text(app, &format!("finding {index}"));
            press(app, KeyCode::Enter);
        }

        let ids: Vec<String> = app.comments().iter().map(|c| c.id.clone()).collect();
        prop_assert!(!ids.is_empty(), "{:?} wrote nothing", writes);

        for (row, id) in ids.iter().enumerate() {
            rewind(app);
            press(app, KeyCode::Tab);
            press(app, KeyCode::Left);
            press_n(app, KeyCode::Down, row);
            prop_assert_eq!(
                app.browsed_comment()
                    .expect("a browsed comment")
                    .id
                    .as_str(),
                id.as_str(),
                "the browser's {}th row is not the {}th comment",
                row,
                row
            );

            let before = app.file_index();
            press(app, KeyCode::Enter);
            seen.hit(usize::from(app.file_index() == before));

            prop_assert_eq!(
                app.focus(),
                Focus::Diff,
                "the jump did not hand over the diff"
            );
            let landed = app.comments_for_line(app.line_index());
            prop_assert!(
                landed.iter().any(|comment| &comment.id == id),
                "row {} jumped to {}:{} , which does not show it: {:?}",
                row,
                app.file_index(),
                app.line_index(),
                app.status()
            );
        }
        Ok(())
    });
    seen.assert_all();
    fixture.clear_comments();
}
