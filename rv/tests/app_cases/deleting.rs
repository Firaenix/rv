//! Deleting a comment.

use crossterm::event::KeyCode;
use proptest::prelude::*;
use rv::app::Action;
use rv::app::Mode;
use std::cell::RefCell;

use crate::support::*;

/// `d`'s question is answered by *every* key, and the disk agrees with the
/// answer: `y` deletes, and anything else writes nothing whatsoever.
///
/// Both halves matter for different reasons. A confirmation that some key fails
/// to dismiss is a reviewer stuck in a mode with no way out but Ctrl+C, which is
/// the failure `on_key_confirm_delete` takes the mode out of the app *before*
/// branching in order to make unrepresentable. And a cancel that still touched
/// the workspace would mean "no" cost the reviewer something — checked here as
/// byte-identity of the whole tree rather than as a comment count, because
/// the export, the snapshots and the comments are all things a cancel must
/// leave alone.
#[test]
fn no_key_leaves_the_reviewer_stuck_at_a_confirmation() {
    let fixture = Fixture::multi();
    let app = RefCell::new(fixture.app());

    // `y` is weighted in rather than left to the key strategy: the confirmed
    // branch is the one that writes, and sampling it rarely would make the
    // receipt below flaky instead of informative.
    let answer = prop_oneof![3 => any_key(), 1 => Just(KeyCode::Char('y'))];
    let seen = Coverage::new(&["a cancelled deletion", "a confirmed deletion"]);
    run_cases(64, (answer, 0usize..4), |(key, downs)| {
        fixture.clear_comments();
        let app = &mut *app.borrow_mut();
        rewind(app);
        press_n(app, KeyCode::Char('j'), downs);

        press(app, KeyCode::Char('c'));
        prop_assert_eq!(app.mode(), Mode::Comment);
        type_text(app, "delete me");
        press(app, KeyCode::Enter);
        prop_assert_eq!(
            fixture.comments().len(),
            1,
            "the case has nothing to delete"
        );

        press(app, KeyCode::Char('d'));
        prop_assert!(
            matches!(app.mode(), Mode::ConfirmDelete { .. }),
            "d deleted without asking, or did not ask: {:?}",
            app.mode()
        );
        let before = workspace_tree(fixture.root());

        let action = app
            .on_key(key)
            .map_err(|error| TestCaseError::fail(error.to_string()))?;
        prop_assert_eq!(
            action,
            Action::Continue,
            "{:?} ended the review from inside a confirmation",
            key
        );
        prop_assert_eq!(
            app.mode(),
            Mode::Browse,
            "{:?} left the reviewer waiting at the question",
            key
        );

        let confirmed = key == KeyCode::Char('y');
        seen.hit(usize::from(confirmed));
        if confirmed {
            prop_assert!(
                fixture.comments().is_empty(),
                "y did not delete: {:?}",
                fixture.comments()
            );
        } else {
            prop_assert_eq!(
                fixture.comments().len(),
                1,
                "{:?} deleted a comment it was not asked to",
                key
            );
            prop_assert_eq!(
                workspace_tree(fixture.root()),
                before,
                "{:?} wrote to the workspace while cancelling",
                key
            );
        }
        Ok(())
    });
    seen.assert_all();
}
