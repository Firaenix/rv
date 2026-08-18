//! Saving a comment.

use crossterm::event::KeyCode;
use proptest::prelude::*;
use rv::app::Mode;
use rv::app::anchored_side;
use rv_core::anchor;
use rv_core::diff::LineKind;
use rv_core::model::Side;
use rv_core::store::CommentState;
use std::cell::RefCell;

use crate::support::*;

/// A comment typed one keystroke at a time reaches `comments.json` — and the
/// markdown export beside it — byte for byte, modulo the one documented
/// normalization (the body is stored trimmed).
///
/// Independent oracles, none of which re-run the app's own code:
///
/// * the stored body is `typed.trim()`;
/// * the anchor's hash and snapshot are what `anchor::create` would produce
///   from the *fixture's own constant* for the anchored side, at the number the
///   anchor stores — so reading the wrong side or the wrong commit shows up;
/// * `REVIEW-FEEDBACK.md` carries the body as one whole line, whatever
///   markdown or `rv:anchor` markers it contains;
/// * a snapshot file exists under the comment's id.
#[test]
fn a_typed_comment_reaches_the_store_byte_identically() {
    let fixture = Fixture::multi();
    let app = RefCell::new(fixture.app());

    // Indices are drawn over the diff's real length, not a fixed range: with a
    // wider range every draw past the end clamps onto the last line, and the
    // base-side arm of the coverage assertion below becomes rare.
    let total = {
        let app = app.borrow();
        assert_eq!(app.selected_file().expect("a file").path, "alpha.rs");
        assert_difftastic(&app);
        app.selected_diff().expect("a diff").lines.len()
    };
    assert!(total >= 3, "alpha.rs produced only {total} diff lines");
    let seen = Coverage::new(&[
        "an all-whitespace body",
        "a body with something in it",
        "a base-side (removed line) anchor",
        "a head-side anchor",
    ]);
    run_cases(64, (any_body(), 0usize..total), |(typed, downs)| {
        fixture.clear_comments();
        let app = &mut *app.borrow_mut();
        rewind(app);
        walk_to_line(app, downs);

        let line = lines(app)
            .get(app.line_index())
            .cloned()
            .expect("alpha.rs has a line here");
        let side = anchored_side(line.kind);
        let (source, number) = match side {
            Side::Left => (ALPHA_BASE, line.left),
            Side::Right => (ALPHA_HEAD, line.right),
        };
        let number = number.expect("an anchored side always carries its number");
        seen.hit(if side == Side::Left { 2 } else { 3 });
        seen.hit(usize::from(!typed.trim().is_empty()));

        press(app, KeyCode::Char('c'));
        prop_assert_eq!(app.mode(), Mode::Comment);
        type_text(app, &typed);
        press(app, KeyCode::Enter);

        prop_assert_eq!(app.mode(), Mode::Browse);
        prop_assert_eq!(app.buffer(), "");

        let expected = typed.trim();
        let comments = fixture.comments();
        if expected.is_empty() {
            prop_assert!(comments.is_empty(), "an all-whitespace body was saved");
            prop_assert_eq!(app.status(), "empty comment, nothing saved");
            return Ok(());
        }

        prop_assert_eq!(comments.len(), 1, "{:?}", comments);
        let comment = &comments[0];
        prop_assert_eq!(comment.body.as_str(), expected);
        prop_assert_eq!(comment.state, CommentState::Open);
        prop_assert_eq!(comment.reply.as_deref(), None);
        prop_assert_eq!(comment.anchor.file.as_str(), "alpha.rs");
        prop_assert_eq!(comment.anchor.side, side);
        prop_assert_eq!(comment.anchor.line, number);
        let saved = format!("comment saved at alpha.rs:{number}");
        prop_assert_eq!(app.status(), saved.as_str());

        // The recorded commit follows the side too: it is advisory, and its one
        // job is being a revision the quoted text can still be read out of, so
        // a comment on a removed line has to name the base.
        let expected_commit = match side {
            Side::Left => &app.session().base_commit,
            Side::Right => &app.session().head_commit,
        };
        prop_assert_eq!(
            comment.commit_id.as_str(),
            expected_commit.as_str(),
            "a {:?}-side comment recorded the other side's commit",
            side
        );

        // The hash and the snapshot come from the side the anchor names, at
        // the number it stores.
        let recomputed = anchor::create("alpha.rs", side, number, source);
        prop_assert_eq!(
            comment.anchor.content_hash.as_str(),
            recomputed.content_hash.as_str()
        );
        prop_assert_eq!(
            &comment.anchor.context,
            &anchor::snapshot_of(source, number)
        );

        prop_assert_eq!(comment.id.len(), 8, "{:?}", comment.id);
        prop_assert!(
            comment
                .id
                .chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
            "{:?} is not a lowercase hex id",
            comment.id
        );
        prop_assert!(
            fixture
                .root()
                .join(".review/snapshots")
                .join(&comment.id)
                .exists(),
            "no snapshot was written for {}",
            comment.id
        );

        // The export is rewritten with the comment in it, on one line: the
        // body cannot have been split, escaped or re-indented.
        let document = fixture.markdown();
        prop_assert!(
            document
                .lines()
                .any(|line| line == format!("**Comment:** {expected}")),
            "the export does not carry {:?} verbatim:\n{}",
            expected,
            document
        );
        Ok(())
    });
    seen.assert_all();
}

/// The same body on the two halves of a same-position rewrite is two comments,
/// not one.
///
/// difftastic pairs a rewritten line with its counterpart and gives *both*
/// halves *both* numbers, so on `same.rs` the removed half anchors to base-side
/// line 2 and the added half to head-side line 2: same change, same file, same
/// number, same body — different side. A comment id that leaves the side out of
/// its seed therefore gives both halves one id, and
/// `Store::append_comment`'s upsert replaces the reviewer's first note with
/// their second while the status line reports "comment saved" for both. That is
/// the loss `ID_CHARS = 8` spends fourteen lines of doc comment arguing must
/// never happen — reachable here with probability 1 rather than by birthday
/// chance.
#[test]
fn both_halves_of_a_same_position_rewrite_keep_their_own_comment() {
    let fixture = Fixture::collisions();
    let mut app = fixture.app();
    select_path(&mut app, "same.rs");
    assert_difftastic(&app);

    let diff_lines = lines(&app);
    let (removed_index, removed) = diff_lines
        .iter()
        .enumerate()
        .find(|(_, line)| {
            line.kind == LineKind::Removed && line.left.is_some() && line.left == line.right
        })
        .unwrap_or_else(|| {
            panic!("no paired removed line whose two numbers agree: {diff_lines:?}")
        });
    let (added_index, added) = diff_lines
        .iter()
        .enumerate()
        .find(|(_, line)| {
            line.kind == LineKind::Added && line.right.is_some() && line.left == line.right
        })
        .unwrap_or_else(|| panic!("no paired added line whose two numbers agree: {diff_lines:?}"));
    let number = removed
        .left
        .expect("a paired removed line carries its left");
    assert_eq!(
        added.right,
        Some(number),
        "the fixture's rewrite is not at the same number on both sides: {diff_lines:?}"
    );
    assert_eq!(anchored_side(removed.kind), Side::Left);
    assert_eq!(anchored_side(added.kind), Side::Right);

    // The same reviewer, the same sentence, on each half of the rewrite.
    for index in [removed_index, added_index] {
        select_path(&mut app, "same.rs");
        walk_to_line(&mut app, index);
        assert_eq!(app.line_index(), index);
        press(&mut app, KeyCode::Char('c'));
        assert_eq!(app.mode(), Mode::Comment);
        type_text(&mut app, "which of these two is right?");
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.status(), format!("comment saved at same.rs:{number}"));
    }

    let comments = fixture.comments();
    assert_eq!(
        comments.len(),
        2,
        "the second comment overwrote the first: {comments:?}"
    );
    let sides: Vec<Side> = comments.iter().map(|comment| comment.anchor.side).collect();
    assert!(
        sides.contains(&Side::Left) && sides.contains(&Side::Right),
        "both comments landed on the same side: {comments:?}"
    );
    assert_ne!(
        comments[0].id, comments[1].id,
        "two comments share one id: {comments:?}"
    );
    for comment in &comments {
        assert_eq!(comment.anchor.file, "same.rs");
        assert_eq!(comment.anchor.line, number);
        assert_eq!(comment.body, "which of these two is right?");
        // Each id owns its own snapshot, so neither comment's context was
        // overwritten by the other's.
        assert!(
            fixture
                .root()
                .join(".review/snapshots")
                .join(&comment.id)
                .exists(),
            "no snapshot for {}",
            comment.id
        );
    }
    // The base-side snapshot quotes the base file and the head-side one the
    // head file: the two comments are about genuinely different text.
    let left = comments
        .iter()
        .find(|comment| comment.anchor.side == Side::Left)
        .expect("a base-side comment");
    let right = comments
        .iter()
        .find(|comment| comment.anchor.side == Side::Right)
        .expect("a head-side comment");
    assert_eq!(left.anchor.context, SAME_BASE.lines().collect::<Vec<_>>());
    assert_eq!(right.anchor.context, SAME_HEAD.lines().collect::<Vec<_>>());
    assert_ne!(left.anchor.content_hash, right.anchor.content_hash);
}

/// A jump tells the two halves of a same-position rewrite apart.
///
/// `same.rs` rewrites line 2 without moving it, so difftastic pairs the halves
/// and both come back with `left == right == 2`: same file, same *path* (there
/// is no rename here), same number, opposite sides. The side is therefore the
/// only thing that distinguishes the two comments, and a jump that dropped it
/// from its lookup would send the reviewer to whichever half the diff lists
/// first — for both of them — while the status line named the right place.
///
/// That is not a hypothetical: dropping the side from `line_of_anchor` survives
/// every rename-based test in this suite, because for a rename the *path*
/// happens to carry the same information. This is the shape where it does not.
#[test]
fn a_jump_tells_the_two_halves_of_a_rewrite_apart() {
    let fixture = Fixture::collisions();
    let mut app = fixture.app();
    select_path(&mut app, "same.rs");
    assert_difftastic(&app);

    let diff_lines = lines(&app);
    let paired = |kind: LineKind| {
        diff_lines
            .iter()
            .position(|line| line.kind == kind && line.left.is_some() && line.left == line.right)
            .unwrap_or_else(|| {
                panic!("no paired {kind:?} line whose two numbers agree: {diff_lines:?}")
            })
    };
    let removed = paired(LineKind::Removed);
    let added = paired(LineKind::Added);
    assert_ne!(removed, added, "the two halves are the same diff line");

    // One comment on each half, in diff order, so the browser lists them in
    // that order too.
    for (index, body) in [(removed, "the old one"), (added, "the new one")] {
        select_path(&mut app, "same.rs");
        walk_to_line(&mut app, index);
        press(&mut app, KeyCode::Char('c'));
        assert_eq!(app.mode(), Mode::Comment);
        type_text(&mut app, body);
        press(&mut app, KeyCode::Enter);
    }
    assert_eq!(fixture.comments().len(), 2, "{:?}", fixture.comments());

    for (row, expected, kind) in [(0, removed, LineKind::Removed), (1, added, LineKind::Added)] {
        rewind(&mut app);
        to_comments(&mut app);
        press(&mut app, KeyCode::Left);
        press_n(&mut app, KeyCode::Down, row);
        press(&mut app, KeyCode::Enter);

        assert_eq!(app.selected_file().expect("a file").path, "same.rs");
        assert_eq!(
            app.line_index(),
            expected,
            "row {row} jumped to the other half of the rewrite: {:?}",
            lines(&app)[app.line_index()]
        );
        assert_eq!(lines(&app)[app.line_index()].kind, kind);
        assert_eq!(
            app.comments_for_line(app.line_index()).len(),
            1,
            "the line jumped to shows both comments, so the two halves are not \
             being told apart"
        );
    }
    fixture.clear_comments();
}
