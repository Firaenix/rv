//! Navigation.

use crossterm::event::KeyCode;
use proptest::prelude::*;
use rv::app::Focus;
use std::cell::RefCell;

use crate::support::*;

/// Movement is the arrows alone — the vim letters `h`/`j`/`k`/`l` were dropped,
/// and pressing one must do nothing rather than quietly move the cursor. The
/// arrows still move: `↓`/`↑` walk the focused pane and `←`/`→` change which
/// pane has the focus.
#[test]
fn the_vim_letters_are_inert_and_the_arrows_move() {
    let fixture = shared_multi();
    let app = &mut fixture.app();
    rewind(app);

    // The diff is focused on a fresh reviewer, and `↓` walks its lines.
    assert_eq!(app.focus(), Focus::Diff);
    let start = app.line_index();
    press(app, KeyCode::Down);
    assert_eq!(app.line_index(), start + 1, "↓ did not move the cursor");
    press(app, KeyCode::Up);
    assert_eq!(app.line_index(), start, "↑ did not move it back");

    // None of the dropped letters move the cursor or change the focus.
    for letter in ['h', 'j', 'k', 'l'] {
        let before = (app.focus(), app.file_index(), app.line_index());
        press(app, KeyCode::Char(letter));
        assert_eq!(
            (app.focus(), app.file_index(), app.line_index()),
            before,
            "{letter} moved something — the vim aliases are supposed to be gone"
        );
        // A pressed letter that opened a leader would strand the next key; make
        // sure nothing is left pending.
        assert!(
            app.pending_leader().is_none(),
            "{letter} opened a menu it should not have"
        );
    }

    // `←` takes the focus to the sidebar; `→` brings it back to the diff.
    press(app, KeyCode::Left);
    assert_eq!(app.focus(), Focus::Sidebar, "← did not move the focus");
    press(app, KeyCode::Right);
    assert_eq!(app.focus(), Focus::Diff, "→ did not bring it back");
}

/// The highlight's closed form: `n` presses of `j` from the top land on
/// `min(n, lines - 1)`, and `m` presses of `k` from there land on
/// `saturating_sub`. Recomputed from the diff's own length rather than by
/// replaying the loop, so an off-by-one in either clamp shows up.
///
/// The round trip is the second half: `j` cannot outrun the file, so `j` then
/// `k` the same number of times is always back at the top — however far past
/// the end the walk tried to go.
#[test]
fn line_navigation_clamps_at_both_ends() {
    let fixture = shared_multi();
    let app = RefCell::new(fixture.app());
    // `long.rs` is the file with enough lines for a walk to be interesting.
    let long = {
        let app = app.borrow();
        app.files()
            .iter()
            .position(|file| file.path == "long.rs")
            .expect("long.rs is in the review")
    };

    run_cases(64, (0usize..60, 0usize..60), |(downs, ups)| {
        let app = &mut *app.borrow_mut();
        rewind(app);
        press_n(app, KeyCode::Char(']'), long);
        prop_assert_eq!(app.file_index(), long);

        let total = lines(app).len();
        prop_assert!(total >= 20, "long.rs produced only {} diff lines", total);
        let last = total - 1;

        press_n(app, KeyCode::Down, downs);
        prop_assert_eq!(
            app.line_index(),
            downs.min(last),
            "{} presses of j on {} lines",
            downs,
            total
        );

        press_n(app, KeyCode::Up, ups);
        prop_assert_eq!(
            app.line_index(),
            downs.min(last).saturating_sub(ups),
            "{} presses of j then {} of k on {} lines",
            downs,
            ups,
            total
        );

        if ups >= downs {
            prop_assert_eq!(
                app.line_index(),
                0,
                "j x{} then k x{} did not return to the top",
                downs,
                ups
            );
        }
        Ok(())
    });
}

/// `Home`/`End` jump the diff cursor to the first and last line, and `PgUp`/
/// `PgDn` move it a screenful without ever running off either end.
///
/// `End` then `Home` is the round trip: whatever a page did on the way, the
/// cursor is back at the top. The page size comes from the last painted frame,
/// and an unpainted app falls back to a fixed page — so this asserts direction
/// and the clamps, not a literal jump distance.
#[test]
fn page_and_jump_keys_reach_the_ends_and_clamp() {
    let fixture = shared_multi();
    let app = RefCell::new(fixture.app());
    let long = {
        let app = app.borrow();
        app.files()
            .iter()
            .position(|file| file.path == "long.rs")
            .expect("long.rs is in the review")
    };

    let app = &mut *app.borrow_mut();
    rewind(app);
    press_n(app, KeyCode::Char(']'), long);
    assert_eq!(app.file_index(), long);
    let last = lines(app).len() - 1;
    assert!(last >= 19, "long.rs is too short to page through");
    assert_eq!(app.focus(), Focus::Diff);

    press(app, KeyCode::End);
    assert_eq!(app.line_index(), last, "End did not reach the last line");
    press(app, KeyCode::Home);
    assert_eq!(app.line_index(), 0, "Home did not return to the first line");

    press(app, KeyCode::PageUp);
    assert_eq!(app.line_index(), 0, "PgUp ran off the top");
    press(app, KeyCode::PageDown);
    let after_page = app.line_index();
    assert!(after_page > 0, "PgDn did not move the cursor");
    assert!(after_page <= last, "PgDn ran off the end");
    press(app, KeyCode::PageUp);
    assert_eq!(
        app.line_index(),
        0,
        "PgDn then PgUp did not return to the top"
    );
}

/// The sidebar's closed form, and the invariant that replaced the line reset:
/// every file keeps its *own* place, and `[` `]` gives it back.
///
/// Walking away from a file and back used to drop the highlight to line 1, so
/// comparing two files cost the reviewer their position in both. The oracle is
/// one remembered position per file, clamped to that file's own diff — a single
/// shared position, or a reset on the way in, both fail it.
#[test]
fn file_navigation_walks_in_range_and_keeps_each_files_place() {
    let fixture = shared_multi();
    let app = RefCell::new(fixture.app());
    let count = app.borrow().files().len();
    assert!(count >= 3, "the fixture lost files");

    // How long each file's diff is, so the oracle clamps `j` where the app
    // does. Two of `multi`'s files have no lines at all, which is the clamp
    // worth having in the model.
    let totals: Vec<usize> = {
        let app = &mut *app.borrow_mut();
        rewind(app);
        (0..count)
            .map(|index| {
                press_n(app, KeyCode::Char(']'), usize::from(index > 0));
                lines(app).len()
            })
            .collect()
    };
    assert!(
        totals.iter().any(|total| *total > 2),
        "no file has room to move in: {totals:?}"
    );

    // `true` is `]`, `false` is `[`; the `j`s before each step are what gives
    // the file being left a place to be remembered at.
    let step = (any::<bool>(), 0usize..6);
    let seen = Coverage::new(&["a file returned to at a line it was left on"]);
    run_cases(48, prop::collection::vec(step, 0..20), |steps| {
        let app = &mut *app.borrow_mut();
        rewind(app);

        let mut expected = 0usize;
        let mut places = vec![0usize; count];
        for (forward, downs) in &steps {
            press_n(app, KeyCode::Down, *downs);
            places[expected] = (places[expected] + downs).min(totals[expected].saturating_sub(1));
            prop_assert_eq!(
                app.line_index(),
                places[expected],
                "{} presses of j in file {}",
                downs,
                expected
            );

            press(app, KeyCode::Char(if *forward { ']' } else { '[' }));
            expected = if *forward {
                (expected + 1).min(count - 1)
            } else {
                expected.saturating_sub(1)
            };
            prop_assert_eq!(
                app.file_index(),
                expected,
                "walking {:?} left the sidebar somewhere else",
                steps
            );
            if places[expected] > 0 {
                seen.hit(0);
            }
            prop_assert_eq!(
                app.line_index(),
                places[expected],
                "file {} did not come back to the line it was left on",
                expected
            );
        }
        Ok(())
    });
    seen.assert_all();
}
