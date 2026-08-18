//! Collapsing a box.

use crossterm::event::KeyCode;

use crate::support::*;

/// Folding boxes away writes nothing. Collapse is a view preference of *this*
/// session: the next reviewer to open this `.review/`, and every LLM reading
/// the export, must see the review as it is rather than as this reviewer
/// arranged their screen.
///
/// Asserted as byte-identity of the whole **workspace** across a run of `s`
/// from both focuses, on a folded line and an unfolded one, rather than by
/// grepping one file for one word: a preference that leaked into `session.toml`
/// under some other name would pass the grep and fail this.
///
/// The workspace, not `.review/`, and that is the difference between this guard
/// and the one it replaces. Scoped to `.review/`, it only forbade folding from
/// writing *there*: a mutant that dropped the fold set into `rv-folds.txt` in
/// the workspace root — one level up, in the tree the reviewer is reading —
/// passed this test and its sibling in `--test app` both.
#[test]
fn collapsing_never_writes_to_the_workspace() {
    let fixture = Fixture::multi();
    let mut app = fixture.app();

    // Two comments on one line, and a third on the next: enough for `s` to have
    // something to do from the diff, from inside a stack, and on a line whose
    // boxes are in mixed states.
    for body in ["first finding", "second finding"] {
        press(&mut app, KeyCode::Char('c'));
        type_text(&mut app, body);
        press(&mut app, KeyCode::Enter);
    }
    press(&mut app, KeyCode::Char('j'));
    press(&mut app, KeyCode::Char('c'));
    type_text(&mut app, "third finding");
    press(&mut app, KeyCode::Enter);
    press(&mut app, KeyCode::Char('k'));
    assert_eq!(fixture.comments().len(), 3, "{:?}", fixture.comments());

    let before = workspace_tree(fixture.root());
    assert!(!before.is_empty(), "the review wrote nothing to compare");

    for key in [
        KeyCode::Char('s'), // fold the line, from the diff
        KeyCode::Enter,     // into the stack
        KeyCode::Char('s'), // unfold the selected box, leaving the line mixed
        KeyCode::Char('j'), // onto the other box
        KeyCode::Char('s'), // and fold that one
        KeyCode::Esc,       // back to the diff
        KeyCode::Char('s'), // fold the mixed line together
        KeyCode::Char('j'), // onto the next line
        KeyCode::Char('s'), // fold it too
        KeyCode::Char('s'), // and unfold it
    ] {
        press(&mut app, key);
        assert_eq!(
            workspace_tree(fixture.root()),
            before,
            "{key:?} wrote to the workspace while arranging the screen"
        );
    }
    assert!(
        !app.collapsed().is_empty(),
        "nothing ended up folded, so this proves nothing"
    );

    fixture.clear_comments();
}
