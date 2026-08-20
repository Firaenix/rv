//! Tests for the row model: the flat list of terminal rows a diff and its
//! comments turn into.
//!
//! [`rv::rows`] is pure — no terminal, no store, no app — so everything below
//! is a plain function call on hand-built data. That is the point of the
//! module: variable-height comment boxes are what make "which diff line is on
//! screen" and "which row is the cursor on" two different questions, and they
//! are answered here where they can be tested without a pty.
//!
//! The comment stack a line owns is supplied as a closure rather than read
//! from a `Store`, so these tests pin the *shape* of the output — how many
//! rows, in what order, holding what text — independently of how the app
//! decides which comments belong to a line.

use std::collections::HashSet;

use proptest::prelude::*;
use rv::rows::BodyKind;
use rv::rows::Plan;
use rv::rows::Row;
use rv::rows::plan;
use rv::rows::window;
use rv_core::diff::DiffLine;
use rv_core::diff::DiffSource;
use rv_core::diff::FallbackReason;
use rv_core::diff::FileDiff;
use rv_core::diff::LineKind;
use rv_core::model::Anchor;
use rv_core::model::Side;
use rv_core::store::Comment;
use rv_core::store::CommentState;

/// A diff of pure context lines, one per string, numbered 1-based on both
/// sides. The row model never looks at a line's kind or numbers — it only
/// counts them — so context lines keep the fixtures about row shape.
fn diff_of(lines: &[&str]) -> FileDiff {
    FileDiff {
        path: "a.rs".to_owned(),
        lines: lines
            .iter()
            .enumerate()
            .map(|(index, text)| {
                let number = u32::try_from(index + 1).expect("a fixture is never that long");
                DiffLine {
                    kind: LineKind::Context,
                    left: Some(number),
                    right: Some(number),
                    text: (*text).to_owned(),
                }
            })
            .collect(),
        source: DiffSource::Similar {
            reason: FallbackReason::NotAttempted,
        },
        suppressed: false,
    }
}

fn comment_with_body(body: &str) -> Comment {
    comment_with_id_and_body("aaaaaaaa", body)
}

fn comment_with_id_and_body(id: &str, body: &str) -> Comment {
    Comment {
        id: id.to_owned(),
        change_id: "nowwnlnmvkwo".to_owned(),
        commit_id: "abc123def456".to_owned(),
        anchor: Anchor {
            file: "a.rs".to_owned(),
            side: Side::Right,
            line: 1,
            content_hash: "deadbeef".to_owned(),
            context: Vec::new(),
            context_start: 1,
        },
        body: body.to_owned(),
        state: CommentState::Open,
        reply: None,
        settled_by: None,
    }
}

/// The text of every body row, in order.
fn body_rows(plan: &Plan<'_>) -> Vec<String> {
    plan.rows
        .iter()
        .filter_map(|row| match row {
            Row::BoxBody { text, .. } => Some(text.clone()),
            _ => None,
        })
        .collect()
}

/// Every body row as `(what it is, what it says)`, in order.
///
/// The kind is the whole reason it is here: [`rv::ui`] draws a reply dimmed,
/// and the only thing that knows a row is reply text rather than comment text
/// is this model. A test that read the `reply:` prefix out of the text instead
/// would be asserting on a spelling the renderer must not have to know.
fn body_rows_by_kind(plan: &Plan<'_>) -> Vec<(BodyKind, String)> {
    plan.rows
        .iter()
        .filter_map(|row| match row {
            Row::BoxBody { kind, text, .. } => Some((*kind, text.clone())),
            _ => None,
        })
        .collect()
}

#[test]
fn a_line_with_no_comments_is_one_row() {
    let diff = diff_of(&["fn a() {", "    let x = 1;", "}"]);

    let plan = plan(&diff, &|_| Vec::new(), &|_| None, &HashSet::new(), 40);

    assert_eq!(plan.rows.len(), 3);
    assert!(matches!(plan.rows[0], Row::Diff { index: 0, .. }));
}

#[test]
fn an_expanded_comment_adds_a_bordered_box_under_its_line() {
    let diff = diff_of(&["fn a() {", "    let x = 1;", "}"]);
    let comment = comment_with_body("needs a doc");

    let plan = plan(
        &diff,
        &|line| {
            if line == 1 {
                vec![&comment]
            } else {
                Vec::new()
            }
        },
        &|_| None,
        &HashSet::new(),
        40,
    );

    // three diff rows, plus top border, one wrapped body row, bottom border
    assert_eq!(plan.rows.len(), 6);
    assert!(matches!(plan.rows[1], Row::Diff { index: 1, .. }));
    assert!(matches!(plan.rows[2], Row::BoxTop { line: 1, .. }));
    assert!(matches!(plan.rows[3], Row::BoxBody { line: 1, .. }));
    assert!(matches!(plan.rows[4], Row::BoxBottom { line: 1, .. }));
    assert!(matches!(plan.rows[5], Row::Diff { index: 2, .. }));
}

#[test]
fn a_collapsed_comment_is_one_row() {
    let diff = diff_of(&["fn a() {", "    let x = 1;", "}"]);
    let comment = comment_with_body("needs a doc");
    let collapsed = HashSet::from([comment.id.clone()]);

    let plan = plan(
        &diff,
        &|line| {
            if line == 1 {
                vec![&comment]
            } else {
                Vec::new()
            }
        },
        &|_| None,
        &collapsed,
        40,
    );

    assert_eq!(plan.rows.len(), 4);
    assert!(matches!(plan.rows[2], Row::BoxCollapsed { line: 1, .. }));
}

#[test]
fn a_long_body_wraps_instead_of_truncating() {
    let diff = diff_of(&["fn a() {"]);
    let comment = comment_with_body("the quick brown fox jumps over the lazy dog again");

    let plan = plan(&diff, &|_| vec![&comment], &|_| None, &HashSet::new(), 20);

    let body = body_rows(&plan);
    assert!(body.len() > 1, "wrapped across rows");
    assert!(
        body.iter().all(|row| row.chars().count() <= 20),
        "no row exceeds the width: {body:?}"
    );
    assert_eq!(
        body.join(" ").split_whitespace().collect::<Vec<_>>(),
        "the quick brown fox jumps over the lazy dog again"
            .split_whitespace()
            .collect::<Vec<_>>(),
        "every word survives"
    );
}

#[test]
fn a_word_longer_than_the_width_is_broken_rather_than_dropped() {
    let diff = diff_of(&["fn a() {"]);
    let comment = comment_with_body("supercalifragilisticexpialidocious");

    let plan = plan(&diff, &|_| vec![&comment], &|_| None, &HashSet::new(), 10);

    let body = body_rows(&plan);
    assert!(
        body.iter().all(|row| row.chars().count() <= 10),
        "no row exceeds the width: {body:?}"
    );
    assert_eq!(
        body.concat(),
        "supercalifragilisticexpialidocious",
        "a word too long for a row is split across rows, not truncated"
    );
}

#[test]
fn a_zero_width_pane_still_makes_progress_one_character_at_a_time() {
    let diff = diff_of(&["fn a() {"]);
    let comment = comment_with_body("abc def");

    let plan = plan(&diff, &|_| vec![&comment], &|_| None, &HashSet::new(), 0);

    let body = body_rows(&plan);
    assert!(
        body.iter().all(|row| row.chars().count() == 1),
        "a zero width is treated as one column: {body:?}"
    );
    assert_eq!(body.concat(), "abcdef", "and nothing is dropped on the way");
}

#[test]
fn a_reply_renders_inside_the_same_box() {
    let diff = diff_of(&["fn a() {"]);
    let mut comment = comment_with_body("needs a doc");
    comment.reply = Some("added one".to_owned());

    let plan = plan(&diff, &|_| vec![&comment], &|_| None, &HashSet::new(), 40);

    let body = body_rows(&plan);
    assert!(
        body.iter().any(|row| row.contains("reply:")),
        "the reply is in the box: {body:?}"
    );
    assert_eq!(
        plan.rows
            .iter()
            .filter(|row| matches!(row, Row::BoxTop { .. }))
            .count(),
        1,
        "and it is the same box, not a second one"
    );
}

/// The rows of a reply say that they are a reply, so the renderer can dim them
/// without parsing the text back for a prefix it wrote itself.
#[test]
fn a_reply_marks_its_rows_as_reply_text() {
    let diff = diff_of(&["fn a() {"]);
    let mut comment = comment_with_body("needs a doc");
    comment.reply = Some("added one".to_owned());

    let plan = plan(&diff, &|_| vec![&comment], &|_| None, &HashSet::new(), 40);

    assert_eq!(
        body_rows_by_kind(&plan),
        vec![
            (BodyKind::Body, "needs a doc".to_owned()),
            (BodyKind::Reply, "reply: added one".to_owned()),
        ],
        "the body and the reply are not told apart, so nothing can dim one of them"
    );
}

/// ...and *every* row of a wrapped reply, not only the one carrying the
/// `reply:` prefix: a reply that faded back to full contrast halfway down would
/// read as the reviewer's own words again.
#[test]
fn every_wrapped_row_of_a_reply_is_marked_as_reply_text() {
    let diff = diff_of(&["fn a() {"]);
    let mut comment = comment_with_body("needs a doc that explains what this function is for");
    comment.reply = Some(
        "added one, and it now says what the function is for and what it refuses to do".to_owned(),
    );

    let plan = plan(&diff, &|_| vec![&comment], &|_| None, &HashSet::new(), 20);

    let rows = body_rows_by_kind(&plan);
    let replies: Vec<String> = rows
        .iter()
        .filter(|(kind, _)| *kind == BodyKind::Reply)
        .map(|(_, text)| text.clone())
        .collect();
    let bodies: Vec<String> = rows
        .iter()
        .filter(|(kind, _)| *kind == BodyKind::Body)
        .map(|(_, text)| text.clone())
        .collect();

    assert!(
        replies.len() > 1 && bodies.len() > 1,
        "the fixture no longer wraps either half, so this proves nothing: {rows:?}"
    );
    assert!(
        replies.join(" ").contains("what it refuses to do"),
        "the tail of the reply is not marked as reply text: {rows:?}"
    );
    assert!(
        bodies.iter().all(|text| !text.contains("added one")),
        "the comment's own body was marked as a reply: {rows:?}"
    );
}

#[test]
fn several_comments_stack_in_order() {
    let diff = diff_of(&["fn a() {"]);
    let first = comment_with_id_and_body("aaaaaaaa", "first");
    let second = comment_with_id_and_body("bbbbbbbb", "second");

    let plan = plan(
        &diff,
        &|_| vec![&first, &second],
        &|_| None,
        &HashSet::new(),
        40,
    );

    let tops: Vec<&str> = plan
        .rows
        .iter()
        .filter_map(|row| match row {
            Row::BoxTop { comment, .. } => Some(comment.id.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(tops, ["aaaaaaaa", "bbbbbbbb"], "oldest first, newest last");
}

#[test]
fn the_window_keeps_the_anchor_row_visible() {
    assert_eq!(window(100, 0, 10), 0..10, "at the top");
    assert_eq!(window(100, 99, 10), 90..100, "at the bottom");
    let visible = window(100, 50, 10);
    assert!(visible.contains(&50), "the anchor row is inside the window");
    assert_eq!(visible.len(), 10);
}

#[test]
fn the_window_survives_degenerate_sizes() {
    assert_eq!(window(0, 0, 10), 0..0, "no rows");
    assert_eq!(window(5, 0, 0), 0..0, "no height");
    assert_eq!(window(3, 1, 10), 0..3, "fewer rows than height");
}

#[test]
fn row_lookup_finds_a_line_pushed_down_by_a_tall_box() {
    let diff = diff_of(&["a", "b", "c"]);
    let comment = comment_with_body("a body long enough to wrap several times over");

    let plan = plan(
        &diff,
        &|line| {
            if line == 0 {
                vec![&comment]
            } else {
                Vec::new()
            }
        },
        &|_| None,
        &HashSet::new(),
        12,
    );

    let row = plan.row_of_line(1).expect("line 1 has a row");
    assert!(row > 1, "the box pushed line 1 down the screen");
    assert!(matches!(plan.rows[row], Row::Diff { index: 1, .. }));
}

#[test]
fn row_lookup_finds_each_box_in_a_stack_whether_it_is_open_or_collapsed() {
    let diff = diff_of(&["a", "b"]);
    let first = comment_with_id_and_body("aaaaaaaa", "first");
    let second = comment_with_id_and_body("bbbbbbbb", "second");
    let collapsed = HashSet::from([first.id.clone()]);

    let plan = plan(
        &diff,
        &|line| {
            if line == 0 {
                vec![&first, &second]
            } else {
                Vec::new()
            }
        },
        &|_| None,
        &collapsed,
        40,
    );

    let one = plan.row_of_comment(0, 0).expect("the first box has a row");
    let two = plan.row_of_comment(0, 1).expect("the second box has a row");
    assert!(
        matches!(plan.rows[one], Row::BoxCollapsed { comment, .. } if comment.id == "aaaaaaaa"),
        "a collapsed box is still selectable"
    );
    assert!(
        matches!(plan.rows[two], Row::BoxTop { comment, .. } if comment.id == "bbbbbbbb"),
        "and the box below it is found at its top border"
    );
    assert!(one < two, "the stack is in order down the screen");
}

#[test]
fn row_lookup_reports_nothing_for_a_line_or_comment_that_is_not_there() {
    let diff = diff_of(&["a", "b"]);
    let comment = comment_with_body("needs a doc");

    let plan = plan(
        &diff,
        &|line| {
            if line == 0 {
                vec![&comment]
            } else {
                Vec::new()
            }
        },
        &|_| None,
        &HashSet::new(),
        40,
    );

    assert_eq!(plan.row_of_line(7), None, "there is no eighth line");
    assert_eq!(plan.row_of_comment(0, 1), None, "there is one box, not two");
    assert_eq!(plan.row_of_comment(1, 0), None, "line 1 has no boxes");
}

proptest! {
    /// Whatever the bodies are, planning a stack draws each comment's box
    /// exactly once: no comment is dropped off the screen and none is drawn
    /// twice, which is what makes `row_of_comment` an unambiguous cursor.
    #[test]
    fn every_comment_appears_exactly_once(bodies in prop::collection::vec("[ -~]{0,80}", 0..5)) {
        let diff = diff_of(&["a", "b", "c"]);
        let comments: Vec<Comment> = bodies.iter().enumerate()
            .map(|(index, body)| comment_with_id_and_body(&format!("{index:08}"), body))
            .collect();
        let refs: Vec<&Comment> = comments.iter().collect();
        let plan = plan(&diff, &|line| if line == 0 { refs.clone() } else { Vec::new() },
                        &|_| None, &HashSet::new(), 30);
        for comment in &comments {
            let tops = plan.rows.iter().filter(|row| matches!(row,
                Row::BoxTop { comment: c, .. } if c.id == comment.id)).count();
            prop_assert_eq!(tops, 1, "each comment has exactly one box");
        }
    }

    /// Any width, any body: planning terminates, never panics, and still
    /// leaves one row per diff line. Width 0 is in range on purpose — a pane
    /// squeezed to nothing must not wrap forever.
    #[test]
    fn planning_never_panics(width in 0usize..40, body in "[ -~]{0,200}") {
        let diff = diff_of(&["a", "b"]);
        let comment = comment_with_body(&body);
        let plan = plan(&diff, &|_| vec![&comment], &|_| None, &HashSet::new(), width);
        prop_assert!(plan.rows.len() >= 2);
    }

    /// The window is the contract the diff pane scrolls by: it shows the
    /// anchor, it never runs off either end of the row list, and it fills the
    /// pane whenever there are enough rows to fill it.
    #[test]
    fn the_window_always_holds_the_anchor_and_fills_the_pane(
        rows in 0usize..200,
        anchor in 0usize..200,
        height in 0usize..40,
    ) {
        let anchor = anchor.min(rows.saturating_sub(1));
        let visible = window(rows, anchor, height);

        prop_assert!(visible.end <= rows, "the window stays inside the row list");
        prop_assert_eq!(visible.len(), height.min(rows), "it fills the pane when it can");
        if rows > 0 && height > 0 {
            prop_assert!(visible.contains(&anchor), "the anchor row is on screen");
        }
    }

    /// **Every row is reachable.** For any plan and any pane height, every row
    /// is inside the window for some cursor position.
    ///
    /// This is the assertion the defect in spec §10 would have failed, and it
    /// is a property rather than an example on purpose: the defect only bites
    /// when a comment box is taller than the pane, which no fixture in this
    /// suite happened to build, so no example test would reliably have caught
    /// it. A reviewer could see the top of a long comment from the line above
    /// it and its bottom from the line below, and its middle from nowhere at
    /// all.
    ///
    /// What makes it true is that the cursor ranges over the **rows** —
    /// `0..rows` here, and `App::cursor_row` in the reviewer. It fails the
    /// moment the cursor can only rest on a subset of them, which is what
    /// anchoring the window on the selected *diff line* did: the anchors were
    /// the diff rows, a box sits between two of them, and `window` centres, so
    /// the rows more than half a pane from either neighbour were in no window
    /// at all. Proved by vendoring a copy and restoring line-anchored
    /// windowing, which leaves rows unreachable at any cursor position.
    #[test]
    fn every_row_is_reachable(rows in 1usize..60, height in 1usize..20) {
        let mut seen = HashSet::new();
        for cursor in 0..rows {
            for row in window(rows, cursor, height) {
                seen.insert(row);
            }
        }
        prop_assert_eq!(seen.len(), rows, "some row is in no window at any cursor");
    }
}
