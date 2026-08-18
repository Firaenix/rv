//! A suppressed diff that still has lines.

use crossterm::event::KeyCode;
use rv::app::App;
use rv::app::DiffEngine;
use rv::app::Mode;
use rv::session;
use rv_core::anchor;
use rv_core::diff::DiffSource;
use rv_core::diff::LineKind;
use rv_core::model::Side;
use std::cell::RefCell;

use crate::support::*;

/// A suppressed diff with lines in it is *shown*, not replaced by a sentence:
/// the note sits above the lines, and every line `j` can reach is on screen,
/// labelled with the number a comment on it anchors to.
///
/// `suppressed` used to imply `lines.is_empty()` — it was set only from
/// difftastic's `unchanged` status, which emits no chunks — and the pane, which
/// short-circuits on the flag, was written against that. The `similar` fallback
/// now reports a terminator-only change (a final newline appearing, CRLF
/// becoming LF) as suppressed *with* all-`Context` lines, because the difference
/// is real and the fallback says so explicitly rather than going silent. That
/// left the pane showing one sentence over a diff `line_count` was still
/// counting for `j`/`k` and `prepare_comment` was still willing to anchor to:
/// the reviewer could put the highlight, and a comment, on a line the pane was
/// not drawing.
///
/// So this pins the agreement rather than the fix: whatever the pane draws,
/// `j`/`k` walks, and whatever `j`/`k` walks can be commented on and anchors
/// where the pane said it would.
#[test]
fn a_suppressed_fallback_diff_shows_the_lines_it_lets_you_navigate() {
    let fixture = Fixture::terminator();
    let app = RefCell::new(fixture.fallback_app());

    for (path, head) in [("crlf.txt", CRLF_HEAD), ("eol.rs", EOL_HEAD)] {
        // The shape the rest of this test is about: suppressed, and not empty.
        let lines = {
            let app = &mut *app.borrow_mut();
            select_path(app, path);
            let diff = app.selected_diff().expect("a loaded diff");
            assert_eq!(diff.source, DiffSource::Similar, "{diff:?}");
            assert!(
                diff.suppressed,
                "{path} is not a suppressed diff, so this proves nothing: {diff:?}"
            );
            assert!(
                !diff.lines.is_empty(),
                "{path}'s suppressed diff has no lines, so this proves nothing: {diff:?}"
            );
            for (index, line) in diff.lines.iter().enumerate() {
                let number = u32::try_from(index + 1).expect("a small line number");
                assert_eq!(line.kind, LineKind::Context, "{line:?}");
                assert_eq!(line.left, Some(number), "{line:?}");
                assert_eq!(line.right, Some(number), "{line:?}");
            }
            diff.lines.clone()
        };

        // Every line, at every pane height with room for a row: the highlight
        // is on screen wearing the right number, and the row beside it is the
        // line's own text under the `Context` sigil.
        //
        // Swept exhaustively rather than sampled because the interesting
        // heights are the two smallest ones — a pane with room for the note and
        // one line, and a pane with room for only one row at all — and a
        // uniform draw over the range would visit them rarely enough to make
        // the receipt flaky.
        for (index, line) in lines.iter().enumerate() {
            let number = anchored_number(line).expect("a numbered context line");
            for height in 4u16..24 {
                let app = &mut *app.borrow_mut();
                select_path(app, path);
                walk_to_line(app, index);
                assert_eq!(app.line_index(), index);

                let frame = render(app, 100, height).backend().to_string();
                assert_eq!(
                    printed_number(app, 100, height),
                    Some(number),
                    "at height {height} the pane does not show line {index} of {path} \
                     ({line:?}) highlighted:\n{frame}"
                );
                let row = format!("{number:>5}  {}", line.text);
                assert!(
                    frame.contains(&row),
                    "at height {height} the pane does not draw {line:?} as {row:?}:\n{frame}"
                );
                // The note is a *header*, not a replacement: it appears above
                // the lines wherever the pane has a row to spare for it, and
                // gives that row back rather than hiding the selection when it
                // does not. A `Browse` bar takes one row and the pane's borders
                // two, so five is the first height with room for both.
                assert_eq!(
                    frame.contains(SUPPRESSED),
                    height >= 5,
                    "at height {height} the suppression note is in the wrong place:\n{frame}"
                );
            }
        }

        // ...and a comment on any of those lines anchors where the pane said.
        for (index, line) in lines.iter().enumerate() {
            let number = anchored_number(line).expect("a numbered context line");
            fixture.clear_comments();
            let app = &mut *app.borrow_mut();
            select_path(app, path);
            walk_to_line(app, index);

            press(app, KeyCode::Char('c'));
            assert_eq!(
                app.mode(),
                Mode::Comment,
                "commenting was refused on line {index} of {path}, which the pane draws: \
                 {:?}",
                app.status()
            );
            type_text(app, "is this terminator deliberate?");
            press(app, KeyCode::Enter);
            assert_eq!(app.status(), format!("comment saved at {path}:{number}"));

            let comments = fixture.comments();
            assert_eq!(comments.len(), 1, "{comments:?}");
            let anchor = &comments[0].anchor;
            assert_eq!(anchor.side, Side::Right);
            assert_eq!(anchor.file, path);
            assert_eq!(anchor.line, number);
            let recomputed = anchor::create(path, Side::Right, number, head);
            assert_eq!(
                anchor.content_hash, recomputed.content_hash,
                "the anchor hashed something other than {line:?}"
            );
        }
    }
    fixture.clear_comments();
}

/// The other half of the same flag: a suppressed diff with *no* lines is the
/// sentence and nothing else, and `c` on it is refused.
///
/// difftastic reports both files of [`Fixture::terminator`] as `unchanged` and
/// emits no chunks for either, so the very same workspace that produces the
/// case above produces this one through the other engine. Without this, a pane
/// that simply deleted the suppression branch would still pass the test above.
#[test]
fn a_suppressed_diff_with_no_lines_is_the_sentence_alone() {
    let fixture = Fixture::terminator();
    let mut app = fixture.app();

    for path in ["crlf.txt", "eol.rs"] {
        select_path(&mut app, path);
        assert_difftastic(&app);
        let diff = app.selected_diff().expect("a loaded diff");
        assert!(diff.suppressed, "{diff:?}");
        assert!(
            diff.lines.is_empty(),
            "difftastic emitted chunks for a terminator-only change, so this \
             case is no longer the empty one: {diff:?}"
        );

        let frame = render(&app, 100, 24).backend().to_string();
        assert!(
            frame.contains(SUPPRESSED),
            "the pane does not say the diff is suppressed:\n{frame}"
        );

        // Nothing to put the highlight on, so nothing to comment on either.
        press(&mut app, KeyCode::Char('c'));
        assert_eq!(app.mode(), Mode::Browse);
        assert_eq!(app.status(), "no diff line selected, nothing to comment on");
    }
    assert!(fixture.comments().is_empty(), "{:?}", fixture.comments());
}

/// A [`session::Review`] whose session covers no change refuses to save a
/// comment instead of storing one attributed to nothing.
///
/// `prepare_comment` calls this refusal defence in depth, and it used to call it
/// unreachable on the grounds that `rv_core::vcs::Repository::stack` returns
/// `EmptyRange` for an empty range. That is true of `session::build`, and only
/// of `session::build`: `session::Review` is `pub` with `pub` fields, so a
/// caller can assemble one with an empty `changes` — which is exactly what this
/// does, so that the branch has a test behind it rather than a claim.
#[test]
fn a_review_with_no_changes_refuses_to_attribute_a_comment() {
    let fixture = Fixture::fallback();
    let mut review = session::build(fixture.root(), Some("@--"), None).expect("build the review");
    assert!(
        !review.session.changes.is_empty(),
        "the range was empty before this test emptied it"
    );
    review.session.changes.clear();

    let mut app = App::open(review, DiffEngine::Auto).expect("open the reviewer");
    assert!(!lines(&app).is_empty(), "the fixture has nothing to select");

    // The refusal is at Enter, not at `c`: there *is* a line to anchor to, and
    // what is missing is the change to attribute the comment to.
    press(&mut app, KeyCode::Char('c'));
    assert_eq!(app.mode(), Mode::Comment);
    type_text(&mut app, "who changed this?");
    press(&mut app, KeyCode::Enter);

    assert_eq!(app.status(), "the review covers no change to comment on");
    assert_eq!(app.mode(), Mode::Browse);
    assert_eq!(app.buffer(), "");
    assert!(fixture.comments().is_empty(), "{:?}", fixture.comments());
    assert!(
        fixture.markdown().is_empty(),
        "a refused comment rewrote the export:\n{}",
        fixture.markdown()
    );
}
