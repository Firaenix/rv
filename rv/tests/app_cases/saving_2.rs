//! Saving a comment.

use crossterm::event::KeyCode;
use proptest::prelude::*;
use rv::app::Mode;
use rv::app::anchored_side;
use rv_core::anchor;
use rv_core::diff::LineKind;
use rv_core::model::Side;
use std::cell::RefCell;

use crate::support::*;

/// Nothing a reviewer saves is ever lost or duplicated: after any run of
/// comments, `comments.json` holds exactly one entry per distinct
/// **(file, side, line, trimmed body)** the reviewer committed — the same body
/// re-typed at the same place upserts, and two comments that differ anywhere in
/// that tuple never collapse into one.
///
/// The key is the whole location, side included, because the *side* is where the
/// collision actually lives: `same.rs` rewrites a line without moving it, so its
/// removed and added halves are `same.rs:2` on opposite sides, and a side-blind
/// id makes one body typed on each half overwrite itself (see
/// [`both_halves_of_a_same_position_rewrite_keep_their_own_comment`], and the
/// `comment_id` doc comment for why the path cannot stand in for the side).
/// `alpha.rs` is the contrast: its rewrite sits one line lower on the head side,
/// so its two halves carry different numbers.
///
/// This is also the property `ID_CHARS` exists for: an id short enough to
/// collide makes `Store::append_comment`'s upsert overwrite an unrelated
/// comment, under a "comment saved" status line. Shrinking the id width is
/// exactly what this fails on.
///
/// Bodies are drawn from four short strings rather than `[a-z]{1,4}`: the
/// interesting cases are the ones where two writes *share* a body, and a
/// half-million-value alphabet makes them vanishingly rare. The coverage
/// receipt below is what proves the same-position pair is reached.
#[test]
fn distinct_comments_are_never_lost_to_each_other() {
    let fixture = Fixture::collisions();
    let app = RefCell::new(fixture.app());
    let count = app.borrow().files().len();
    assert_eq!(
        count,
        2,
        "the fixture lost files: {:?}",
        app.borrow().files()
    );

    // The pair whose two halves share a number is what makes this property a
    // test of the id's side-awareness rather than of its path-awareness.
    {
        let app = &mut *app.borrow_mut();
        select_path(app, "same.rs");
        assert_difftastic(app);
        let same = lines(app);
        assert!(
            same.iter().any(|line| line.kind == LineKind::Removed
                && line.left.is_some()
                && line.left == line.right)
                && same.iter().any(|line| line.kind == LineKind::Added
                    && line.right.is_some()
                    && line.left == line.right),
            "same.rs is not a same-position rewrite any more, so the collision this \
             property is about is unreachable: {same:?}"
        );
        select_path(app, "alpha.rs");
        assert_difftastic(app);
        let alpha = lines(app);
        assert!(
            alpha.iter().any(|line| match (line.left, line.right) {
                (Some(left), Some(right)) => left != right,
                _ => false,
            }),
            "alpha.rs no longer carries a pair with two different numbers: {alpha:?}"
        );
    }

    let body = prop_oneof![
        Just("a".to_owned()),
        Just("b".to_owned()),
        Just("ab".to_owned()),
        Just("ba".to_owned()),
    ];
    let write = (0usize..count, 0usize..4, body);
    let seen = Coverage::new(&["two comments distinguished only by their side"]);
    run_cases(32, prop::collection::vec(write, 1..9), |writes| {
        fixture.clear_comments();
        let app = &mut *app.borrow_mut();

        let mut expected: Vec<(String, &'static str, u32, String)> = Vec::new();
        for (file, downs, body) in &writes {
            rewind(app);
            press_n(app, KeyCode::Char(']'), *file);
            press_n(app, KeyCode::Char('j'), *downs);
            let selected = app.selected_file().cloned().expect("a file");
            let line = lines(app)
                .get(app.line_index())
                .cloned()
                .expect("both files have lines here");
            let side = anchored_side(line.kind);
            let number =
                anchored_number(&line).expect("an anchored side always carries its number");
            // The anchored path follows the side, exactly as the id's seed does.
            let path = match side {
                Side::Left => selected
                    .source_path
                    .clone()
                    .unwrap_or_else(|| selected.path.clone()),
                Side::Right => selected.path.clone(),
            };

            press(app, KeyCode::Char('c'));
            prop_assert_eq!(app.mode(), Mode::Comment);
            type_text(app, body);
            press(app, KeyCode::Enter);
            let saved = format!("comment saved at {path}:{number}");
            prop_assert_eq!(app.status(), saved.as_str());

            let entry = (path, side_tag(side), number, body.clone());
            if !expected.contains(&entry) {
                expected.push(entry);
            }
        }

        // Did this case reach the shape the id used to lose: one body on both
        // halves of one rewrite?
        if expected.iter().any(|(file, side, line, body)| {
            expected
                .iter()
                .any(|(other_file, other_side, other_line, other_body)| {
                    other_file == file
                        && other_line == line
                        && other_body == body
                        && other_side != side
                })
        }) {
            seen.hit(0);
        }

        let mut stored: Vec<(String, &'static str, u32, String)> = fixture
            .comments()
            .into_iter()
            .map(|comment| {
                (
                    comment.anchor.file,
                    side_tag(comment.anchor.side),
                    comment.anchor.line,
                    comment.body,
                )
            })
            .collect();
        let mut ids: Vec<String> = fixture
            .comments()
            .into_iter()
            .map(|comment| comment.id)
            .collect();
        stored.sort();
        expected.sort();
        prop_assert_eq!(
            &stored,
            &expected,
            "{} writes produced {} stored comments",
            writes.len(),
            stored.len()
        );
        // Two entries sharing an id would already have collapsed above, so this
        // is a receipt rather than a second chance: the store holds one id per
        // distinct location and body.
        ids.sort();
        let total = ids.len();
        ids.dedup();
        prop_assert_eq!(ids.len(), total, "two stored comments share an id");
        Ok(())
    });
    seen.assert_all();
}

/// A comment is refused *before* it is typed, never after.
///
/// The promise in `begin_comment`'s doc comment is that a reviewer is told
/// there is nothing to anchor to at the moment they press `c` — so the
/// contract is a disjunction, and both halves are checked at every reachable
/// (file, line): either `c` is refused outright and the store is untouched, or
/// the mode opens and a non-empty body *is* saved. There is no third case
/// where a typed comment is accepted and then dropped.
///
/// The fixture is built so both halves fire: `bin.dat` (binary) and
/// `blank.txt` (empty) have no diff lines at all, `alpha.rs` and `long.rs`
/// have plenty.
#[test]
fn commenting_is_refused_before_typing_or_not_at_all() {
    let fixture = Fixture::multi();
    let app = RefCell::new(fixture.app());
    let count = app.borrow().files().len();

    // Both halves of the disjunction have to be reachable, or this property
    // proves nothing.
    let (mut empty, mut nonempty) = (0, 0);
    {
        let app = &mut *app.borrow_mut();
        for index in 0..count {
            rewind(app);
            press_n(app, KeyCode::Char(']'), index);
            if lines(app).is_empty() {
                empty += 1;
            } else {
                nonempty += 1;
            }
        }
    }
    assert!(
        empty >= 2,
        "no file has an uncommentable diff; the fixture is wrong"
    );
    assert!(
        nonempty >= 2,
        "no file has a commentable diff; the fixture is wrong"
    );

    let seen = Coverage::new(&["a refused `c`", "an accepted `c`"]);
    run_cases(48, (0usize..count, 0usize..48), |(file, downs)| {
        fixture.clear_comments();
        let app = &mut *app.borrow_mut();
        rewind(app);
        press_n(app, KeyCode::Char(']'), file);
        press_n(app, KeyCode::Char('j'), downs);

        let selected = lines(app).get(app.line_index()).cloned();
        seen.hit(usize::from(selected.is_some()));
        press(app, KeyCode::Char('c'));

        if selected.is_none() {
            prop_assert_eq!(
                app.mode(),
                Mode::Browse,
                "comment mode opened on a diff with no lines"
            );
            prop_assert_eq!(app.status(), "no diff line selected, nothing to comment on");
            // Everything the reviewer types next is browsing, not a body.
            type_text(app, "wasted");
            press(app, KeyCode::Enter);
            prop_assert!(fixture.comments().is_empty(), "{:?}", fixture.comments());
            prop_assert_eq!(app.buffer(), "");
            return Ok(());
        }

        prop_assert_eq!(app.mode(), Mode::Comment);
        type_text(app, "kept");
        press(app, KeyCode::Enter);
        let comments = fixture.comments();
        prop_assert_eq!(
            comments.len(),
            1,
            "an accepted comment was dropped: {:?}",
            comments
        );
        prop_assert_eq!(comments[0].body.as_str(), "kept");
        prop_assert!(
            app.status().starts_with("comment saved at "),
            "{:?}",
            app.status()
        );
        Ok(())
    });
    seen.assert_all();
}

/// The number the diff pane prints beside a line, the number the status line
/// reports after saving, and the number `comments.json` stores are the same
/// number — on the same file.
///
/// The fixture renames `a.rs` to `b.rs` and rewrites two lines, so
/// difftastic pairs them and every paired line carries *both* a left and a
/// right number, and the base-side path differs from the head-side one. A pane
/// that labelled a removed line by its head number, or a status line that
/// named the head path, would disagree with the anchor here.
#[test]
fn the_pane_the_status_and_the_anchor_agree_on_the_line() {
    let fixture = Fixture::renamed();
    let app = RefCell::new(fixture.app());
    let lines = {
        let app = app.borrow();
        assert_difftastic(&app);
        let file = app.selected_file().expect("a file");
        assert_eq!(
            file.path,
            "b.rs",
            "jj did not record the rename; the base side has nothing to anchor to: {:?}",
            app.files()
        );
        assert_eq!(file.source_path.as_deref(), Some("a.rs"));
        app.selected_diff().expect("a diff").lines.clone()
    };
    let total = lines.len();
    // The property only bites where a line's two numbers disagree: that is the
    // case a pane labelling by the wrong side would get away with.
    let disagreeing = lines
        .iter()
        .filter(|line| match (line.left, line.right) {
            (Some(left), Some(right)) => left != right,
            _ => false,
        })
        .count();
    assert!(
        disagreeing >= 1,
        "no diff line carries two different numbers, so this proves nothing: {lines:?}"
    );

    let seen = Coverage::new(&["a base-side anchor", "a head-side anchor"]);
    run_cases((total * 8) as u32, 0usize..total, |index| {
        fixture.clear_comments();
        let app = &mut *app.borrow_mut();
        rewind(app);
        walk_to_line(app, index);
        prop_assert_eq!(app.line_index(), index);

        let printed = printed_number(app, 120, 44).ok_or_else(|| {
            TestCaseError::fail(format!(
                "no highlighted row in the diff pane at line {index}"
            ))
        })?;

        press(app, KeyCode::Char('c'));
        prop_assert_eq!(app.mode(), Mode::Comment);
        type_text(app, "why");
        press(app, KeyCode::Enter);

        let comments = fixture.comments();
        prop_assert_eq!(comments.len(), 1, "{:?}", comments);
        let anchor = &comments[0].anchor;
        let saved = format!("comment saved at {}:{}", anchor.file, anchor.line);
        prop_assert_eq!(app.status(), saved.as_str());
        prop_assert_eq!(
            printed,
            anchor.line,
            "the pane printed {} for line {} but the anchor stored {}",
            printed,
            index,
            anchor.line
        );
        // ...and the path follows the side, so the pane's file and the
        // anchor's file are the same file.
        let (expected_file, source) = match anchor.side {
            Side::Left => ("a.rs", RENAME_BASE),
            Side::Right => ("b.rs", RENAME_HEAD),
        };
        seen.hit(usize::from(anchor.side == Side::Right));
        prop_assert_eq!(anchor.file.as_str(), expected_file);
        let recomputed = anchor::create(expected_file, anchor.side, anchor.line, source);
        prop_assert_eq!(
            anchor.content_hash.as_str(),
            recomputed.content_hash.as_str()
        );
        Ok(())
    });
    seen.assert_all();
}
