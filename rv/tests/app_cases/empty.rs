//! A review that changed no files.

use crossterm::event::KeyCode;
use proptest::prelude::*;
use ratatui::layout::Rect;
use rstest::rstest;
use rv::app::Mode;
use rv::session;
use rv_core::diff;
use rv_core::diff::FileDiff;
use std::cell::RefCell;

use crate::support::*;

/// A range with changes but no changed files: every accessor answers `None`,
/// no key can panic, commenting is refused, and the pane says so at every
/// size.
///
/// `session::build` rejects an *empty range*, so this state is only reachable
/// the way the fixture builds it — two described changes that touch nothing —
/// and it is the state `ui::draw`'s "no changed files in this range" branch
/// and `ListState::with_selected(None)` exist for.
#[test]
fn a_review_with_no_files_is_inert_but_alive() {
    let fixture = shared_no_files();
    let app = RefCell::new(fixture.app());
    {
        let app = app.borrow();
        assert!(app.files().is_empty(), "{:?}", app.files());
        assert!(app.selected_file().is_none());
        assert!(app.selected_diff().is_none());
    }

    run_cases(
        32,
        (prop::collection::vec(any_key(), 0..16), 1u16..40, 1u16..24),
        |(keys, width, height)| {
            let app = &mut *app.borrow_mut();
            for key in &keys {
                app.on_key(*key)
                    .map_err(|error| TestCaseError::fail(format!("{key:?}: {error}")))?;
                prop_assert_eq!(app.file_index(), 0);
                prop_assert_eq!(app.line_index(), 0);
                prop_assert!(app.selected_file().is_none());
                prop_assert!(app.selected_diff().is_none());
                // With nothing to anchor to, comment mode can never open, so
                // no keystroke can ever become a body.
                prop_assert_eq!(app.mode(), Mode::Browse);
                prop_assert_eq!(app.buffer(), "");
            }

            // `Esc` first: a generated `?` leaves the keymap up, and every
            // other key is inert behind it — including the `c` this case is
            // about. `Esc` closes it and is a no-op otherwise.
            press(app, KeyCode::Esc);
            prop_assert!(!app.help_open());
            press(app, KeyCode::Char('c'));
            prop_assert_eq!(app.mode(), Mode::Browse);
            prop_assert_eq!(app.status(), "no diff line selected, nothing to comment on");
            prop_assert!(fixture.comments().is_empty());

            let frame = render(app, width, height);
            prop_assert_eq!(frame.backend().buffer().area.width, width);
            Ok(())
        },
    );
}

/// The pathological terminal sizes, spelled out as cases rather than sampled,
/// including the `Comment` bar asking for three rows out of one.
#[rstest]
#[case::single_cell(1, 1)]
#[case::one_row(80, 1)]
#[case::one_column(1, 40)]
#[case::two_by_five(2, 5)]
#[case::five_by_two(5, 2)]
#[case::three_by_three(3, 3)]
#[case::bar_only(40, 1)]
#[case::bar_plus_one(40, 2)]
#[case::comment_bar_exactly(40, 3)]
#[case::tall_and_thin(2, 60)]
fn drawing_survives_pathological_sizes(#[case] width: u16, #[case] height: u16) {
    let fixture = shared_multi();
    let mut app = fixture.app();

    let count = app.files().len();
    for file in 0..count {
        rewind(&mut app);
        press_n(&mut app, KeyCode::Char(']'), file);
        press_n(&mut app, KeyCode::Char('j'), 60);

        let browse = render(&app, width, height);
        assert_eq!(
            browse.backend().buffer().area,
            Rect::new(0, 0, width, height)
        );

        press(&mut app, KeyCode::Char('c'));
        if app.mode() == Mode::Comment {
            type_text(&mut app, "a comment being typed into a very small terminal");
        }
        let comment = render(&app, width, height);
        assert_eq!(
            comment.backend().buffer().area,
            Rect::new(0, 0, width, height)
        );
        press(&mut app, KeyCode::Esc);
    }
    assert!(fixture.comments().is_empty());
}

/// Every file's diff is the diff of *that file's* two blobs, on every visit:
/// the sidebar's cache can neither serve one file's lines under another's name
/// nor hand back something the repository does not say.
///
/// The oracle is an independent recomputation — the base and head blobs read
/// straight out of the repository (the base side at its own path, so a rename
/// still diffs against the file it came from) and handed to
/// [`rv_core::diff::compute`] — rather than the app's own earlier answer.
/// Comparing pass 2 with pass 1 would prove nothing: `load_selected`
/// early-returns on a cached diff, so all three passes read the same value, and
/// the comparison could only fail if `Clone` or `PartialEq` were broken.
/// "Stable" has to mean "still equal to what these blobs diff to".
#[test]
fn revisiting_a_file_returns_the_same_diff() {
    let fixture = shared_multi();
    let mut app = fixture.app();
    let count = app.files().len();
    assert!(count >= 3, "the fixture lost files");

    // Built once, from a second review of the same range: nothing here goes
    // through `App`.
    let review = session::build(fixture.root(), Some("@--"), None).expect("build the review");
    assert_eq!(review.files.len(), count);
    let expected: Vec<FileDiff> = review
        .files
        .iter()
        .map(|file| {
            let base_path = file.source_path.as_deref().unwrap_or(&file.path);
            let old = review
                .repo
                .read_blob(&review.session.base_commit, base_path)
                .expect("read the base blob");
            let new = review
                .repo
                .read_blob(&review.session.head_commit, &file.path)
                .expect("read the head blob");
            diff::compute(old.as_deref(), new.as_deref(), &file.path)
        })
        .collect();

    for pass in 0..3 {
        rewind(&mut app);
        for (index, expected) in expected.iter().enumerate() {
            press_n(&mut app, KeyCode::Char(']'), if index == 0 { 0 } else { 1 });
            assert_eq!(app.file_index(), index);
            let path = app.files()[index].path.clone();
            assert_eq!(path, review.files[index].path);
            let diff = app.selected_diff().expect("a loaded diff");
            assert_eq!(diff.path, path);
            assert_eq!(
                diff, expected,
                "pass {pass}: the diff the app serves for {path} is not the diff of its blobs"
            );
        }
    }
}
