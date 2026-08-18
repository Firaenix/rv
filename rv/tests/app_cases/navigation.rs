//! Navigation.

use std::cell::RefCell;
use crossterm::event::KeyCode;
use proptest::prelude::*;
use rv::app::Focus;

use crate::support::*;

/// README's `j` / `↓` and `k` / `↑` are aliases, so two reviewers pressing the
/// same moves — one on letters, one on arrows — must end up looking at exactly
/// the same thing.
///
/// Differential rather than re-derived: neither app is the oracle, which is
/// what makes this fail if either binding drifts.
///
/// `Left` and `Right` are in the sequence because they are what makes the claim
/// non-trivial now: the pair moves the *file* selection while the sidebar has
/// focus and the *line* while the diff does, so an alias that was only wired up
/// in one of the two arms shows up here.
#[test]
fn arrow_keys_are_aliases_of_the_letters() {
    let fixture = shared_multi();
    let letters = RefCell::new(fixture.app());
    let arrows = RefCell::new(fixture.app());

    #[derive(Clone, Copy, Debug)]
    enum Move {
        Forward,
        Back,
        FileNext,
        FilePrevious,
        FocusLeft,
        FocusRight,
    }

    let moves = prop_oneof![
        3 => Just(Move::Forward),
        3 => Just(Move::Back),
        1 => Just(Move::FileNext),
        1 => Just(Move::FilePrevious),
        2 => Just(Move::FocusLeft),
        2 => Just(Move::FocusRight),
    ];

    let seen = Coverage::new(&[
        "moving with the sidebar focused",
        "moving with the diff focused",
    ]);
    run_cases(48, prop::collection::vec(moves, 0..24), |sequence| {
        let letters = &mut *letters.borrow_mut();
        let arrows = &mut *arrows.borrow_mut();
        rewind(letters);
        rewind(arrows);

        for step in &sequence {
            let (letter, arrow) = match step {
                Move::Forward => (KeyCode::Char('j'), KeyCode::Down),
                Move::Back => (KeyCode::Char('k'), KeyCode::Up),
                Move::FileNext => (KeyCode::Char(']'), KeyCode::Char(']')),
                Move::FilePrevious => (KeyCode::Char('['), KeyCode::Char('[')),
                Move::FocusLeft => (KeyCode::Left, KeyCode::Left),
                Move::FocusRight => (KeyCode::Right, KeyCode::Right),
            };
            if matches!(step, Move::Forward | Move::Back) {
                seen.hit(usize::from(letters.focus() == Focus::Diff));
            }
            letters.on_key(letter).expect("letter key");
            arrows.on_key(arrow).expect("arrow key");

            prop_assert_eq!(
                (letters.file_index(), letters.line_index()),
                (arrows.file_index(), arrows.line_index()),
                "after {:?}: letters and arrows disagree",
                step
            );
            prop_assert_eq!(letters.focus(), arrows.focus());
            prop_assert_eq!(letters.mode(), arrows.mode());
            prop_assert_eq!(letters.buffer(), arrows.buffer());
            prop_assert_eq!(letters.status(), arrows.status());
        }
        Ok(())
    });
    seen.assert_all();
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

        press_n(app, KeyCode::Char('j'), downs);
        prop_assert_eq!(
            app.line_index(),
            downs.min(last),
            "{} presses of j on {} lines",
            downs,
            total
        );

        press_n(app, KeyCode::Char('k'), ups);
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
            press_n(app, KeyCode::Char('j'), *downs);
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
