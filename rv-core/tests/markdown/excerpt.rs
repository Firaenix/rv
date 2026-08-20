//! The excerpt caption: which stored line the comment is on.
//!
//! Split from [`super`] for the 400-line rule; it shares that file's fixtures.

use rv_core::markdown::render;
use rv_core::model::Side;
use rv_core::store::Comment;
use rv_core::store::CommentState;

use super::FIRST_CHANGE;
use super::comment;
use super::session;

/// The excerpt says which of its lines the comment is about.
///
/// A reviewer reading a finished export reported this as the one thing they could
/// not resolve from the file alone: the target is the sixth row in the middle of a
/// file but the third near the top, because `snapshot_of` clamps at the edges, and
/// nothing in the document said which. It matters most in the case the excerpt
/// exists for — where the file has moved on and cannot be consulted.
#[test]
fn the_excerpt_says_which_line_the_comment_is_on() {
    let source: String = (1..=20).map(|n| format!("line {n}\n")).collect();
    let built = rv_core::anchor::create("deep.rs", Side::Right, 12, &source);
    let comment = Comment {
        anchor: built,
        ..comment("aa11", FIRST_CHANGE, "deep.rs", 12, CommentState::Open)
    };
    let document = render(&session(), &[comment]);

    assert!(
        document.contains("Lines 7–17; the comment is on line 12 — row 6 of 11 below."),
        "the excerpt does not say which row is the anchored one:\n{document}"
    );
}

/// Near the top of a file the window cannot open five lines above the target, so
/// the target is *not* in the middle — which is the case the caption exists for.
#[test]
fn an_excerpt_clamped_at_the_top_says_so() {
    let source: String = (1..=20).map(|n| format!("line {n}\n")).collect();
    let built = rv_core::anchor::create("shallow.rs", Side::Right, 2, &source);
    let comment = Comment {
        anchor: built,
        ..comment("bb22", FIRST_CHANGE, "shallow.rs", 2, CommentState::Open)
    };
    let document = render(&session(), &[comment]);

    assert!(
        document.contains("Lines 1–7; the comment is on line 2 — row 2 of 7 below."),
        "a clamped excerpt is described as though it were centred:\n{document}"
    );
}

/// An anchor written before `context_start` existed gets no caption rather than a
/// guessed one: an unnumbered excerpt is honest.
#[test]
fn an_anchor_with_no_recorded_start_is_left_uncaptioned() {
    let mut entry = comment("cc33", FIRST_CHANGE, "old.rs", 4, CommentState::Open);
    entry.anchor.context_start = 0;
    let document = render(&session(), &[entry]);

    assert!(
        !document.contains("the comment is on line"),
        "a caption was invented for an anchor that records no start:\n{document}"
    );
}
