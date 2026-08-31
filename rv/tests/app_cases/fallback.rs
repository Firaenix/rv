//! The `similar` fallback: what a reviewer without difftastic sees.

use crossterm::event::KeyCode;
use proptest::prelude::*;
use rv::app::Mode;
use rv::app::anchored_side;
use rv_core::anchor;
use rv_core::diff::DiffSource;
use rv_core::diff::FallbackReason;
use rv_core::diff::LineKind;
use rv_core::model::Side;
use std::cell::RefCell;

use crate::support::*;

/// The fallback diff is a different shape from difftastic's, and the pane says
/// which one it is showing.
///
/// Everything else in this file reviews through difftastic, which emits only
/// *changed* lines — so no diff anywhere else here contains a
/// [`LineKind::Context`] line or a [`DiffSource::Similar`] label, and the arms
/// of `ui::body`, `ui::title` and `app::anchored_side` that handle them were
/// never rendered or taken. This is the path every user with no `difft` on
/// `PATH` is on, and the one `RV_NO_DIFFT=1` forces.
#[test]
fn the_fallback_diff_is_labelled_and_carries_context_lines() {
    let fixture = Fixture::fallback();
    let app = fixture.fallback_app();
    let diff = app.selected_diff().expect("a loaded diff");
    assert_eq!(diff.path, "ctx.rs");
    assert_eq!(
        diff.source,
        DiffSource::Similar {
            reason: FallbackReason::NotAttempted
        }
    );
    assert!(!diff.suppressed);

    let kinds = |kind: LineKind| diff.lines.iter().filter(|line| line.kind == kind).count();
    assert!(
        kinds(LineKind::Context) >= 2,
        "the fallback diff has no context lines: {:?}",
        diff.lines
    );
    assert!(kinds(LineKind::Removed) >= 1, "{:?}", diff.lines);
    assert!(kinds(LineKind::Added) >= 2, "{:?}", diff.lines);

    // A context line belongs to the head side, and carries both numbers.
    let context = diff
        .lines
        .iter()
        .find(|line| line.kind == LineKind::Context)
        .expect("a context line");
    assert_eq!(anchored_side(context.kind), Side::Right);
    assert!(
        context.left.is_some() && context.right.is_some(),
        "{context:?}"
    );

    // The pane labels the diff by its source and prints each kind's sigil:
    // ' ' for context, '-' for removed, '+' for added, after a five-wide
    // number column.
    let frame = render(&app, 120, 20).backend().to_string();
    assert!(
        frame.contains("ctx.rs — fallback"),
        "the pane does not say the diff is a fallback:\n{frame}"
    );
    for line in &diff.lines {
        let sigil = match line.kind {
            LineKind::Context => ' ',
            LineKind::Added => '+',
            LineKind::Removed => '-',
        };
        let number = anchored_number(line).expect("every fallback line is numbered");
        let row = format!("{number:>5} {sigil}{}", line.text.trim_end());
        assert!(
            frame.contains(row.trim_end()),
            "the pane does not render {line:?} as {row:?}:\n{frame}"
        );
    }

    // The contrast that makes the paragraph above true: difftastic over the
    // very same files produces no context line at all.
    let difftastic = fixture.app();
    let structural = difftastic.selected_diff().expect("a loaded diff");
    assert!(
        matches!(structural.source, DiffSource::Difftastic { .. }),
        "{structural:?}"
    );
    assert!(
        !structural
            .lines
            .iter()
            .any(|line| line.kind == LineKind::Context),
        "difftastic produced a context line, so the fallback is not the only \
         source of one any more: {:?}",
        structural.lines
    );
}

/// Navigating, commenting, anchoring and rendering all behave the same on the
/// fallback path as on difftastic's — including on a context line, which only
/// this path produces.
///
/// Checked over every line of the fallback diff at every pane height with room
/// for a row, with the coverage receipt naming the three line kinds: a case that
/// never selected a context line would leave `anchored_side`'s `Context` arm and
/// `ui::body`'s `Context` arm unexercised, which is exactly the hole this test
/// exists to close.
#[test]
fn the_fallback_path_navigates_comments_and_anchors() {
    let fixture = Fixture::fallback();
    let app = RefCell::new(fixture.fallback_app());
    let total = {
        let app = app.borrow();
        app.displayed_lines().len()
    };
    assert!(total >= 6, "the fallback diff has only {total} lines");

    let seen = Coverage::new(&["a context line", "an added line", "a removed line"]);
    run_cases(48, (0usize..total, 4u16..24), |(index, height)| {
        fixture.clear_comments();
        let app = &mut *app.borrow_mut();
        rewind(app);
        walk_to_line(app, index);
        prop_assert_eq!(app.line_index(), index);

        let line = lines(app)[index].clone();
        seen.hit(match line.kind {
            LineKind::Context => 0,
            LineKind::Added => 1,
            LineKind::Removed => 2,
        });
        // Spelled out rather than taken from `anchored_side`: an oracle that
        // calls the function under test agrees with it by construction, and
        // "everything but a removed line is commented against the head" is a
        // claim about `rv`, not about this file's convenience helpers.
        let side = match line.kind {
            LineKind::Removed => Side::Left,
            LineKind::Added | LineKind::Context => Side::Right,
        };
        let (number, source) = match side {
            Side::Left => (line.left, CTX_BASE),
            Side::Right => (line.right, CTX_HEAD),
        };
        let number = number.expect("every fallback line is numbered on its own side");

        // The pane shows the selected line, highlighted, at this height.
        let frame = render(app, 100, height).backend().to_string();
        prop_assert!(
            frame.contains(line.text.trim_end()),
            "line {} ({:?}) is not on screen at height {}:\n{}",
            index,
            line.text,
            height,
            frame
        );
        prop_assert!(
            frame.contains("ctx.rs — fallback"),
            "the pane stopped calling this a fallback:\n{}",
            frame
        );
        prop_assert_eq!(
            printed_number(app, 100, height),
            Some(number),
            "at height {} the pane labels line {} ({:?}) with another number",
            height,
            index,
            line
        );

        // ...and a comment on it anchors where the pane said it would.
        comment(app);
        prop_assert_eq!(app.mode(), Mode::Comment);
        type_text(app, "what about this line");
        press(app, KeyCode::Enter);
        let saved = format!("comment saved at ctx.rs:{number}");
        prop_assert_eq!(app.status(), saved.as_str());

        let comments = fixture.comments();
        prop_assert_eq!(comments.len(), 1, "{:?}", comments);
        let comment = &comments[0];
        prop_assert_eq!(comment.body.as_str(), "what about this line");
        prop_assert_eq!(comment.anchor.file.as_str(), "ctx.rs");
        prop_assert_eq!(comment.anchor.side, side);
        prop_assert_eq!(comment.anchor.line, number);
        let recomputed = anchor::create("ctx.rs", side, number, source);
        prop_assert_eq!(
            comment.anchor.content_hash.as_str(),
            recomputed.content_hash.as_str(),
            "the anchor hashed the wrong side or the wrong line for {:?}",
            line
        );
        prop_assert_eq!(
            &comment.anchor.context,
            &anchor::snapshot_of(source, number)
        );
        Ok(())
    });
    seen.assert_all();
}
