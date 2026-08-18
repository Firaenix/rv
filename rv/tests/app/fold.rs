//! Collapsing a box.

use crossterm::event::KeyCode;
use rv::app::SidebarTab;

use crate::support::*;

/// `s` is a toggle: the boxes on the selected line fold away and come back.
#[test]
fn s_collapses_and_expands_the_boxes_on_the_selected_line() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, "needs a doc");
    let id = app.comments()[0].id.clone();

    assert!(
        app.collapsed().is_empty(),
        "a comment is drawn open until the reviewer folds it"
    );
    app.on_key(KeyCode::Char('s')).expect("collapse");
    assert!(app.collapsed().contains(&id), "{:?}", app.collapsed());

    app.on_key(KeyCode::Char('s')).expect("expand");
    assert!(!app.collapsed().contains(&id), "{:?}", app.collapsed());
}

/// From the diff, `s` acts on the whole line: the reviewer is folding a *line*
/// away, and leaving half of its stack open would not do that. Mixed states
/// collapse first, so one press always gets a line out of the way.
#[test]
fn from_the_diff_s_folds_the_whole_line_together() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, "first finding");
    write_comment(&mut app, "second finding");
    let line = app.line_index();
    let ids: Vec<String> = app
        .comments_for_line(line)
        .iter()
        .map(|comment| comment.id.clone())
        .collect();
    assert_eq!(ids.len(), 2);

    // Fold just one of them, from inside the stack, so the line is mixed.
    app.on_key(KeyCode::Enter).expect("enter the stack");
    app.on_key(KeyCode::Char('s')).expect("collapse the first");
    app.on_key(KeyCode::Esc).expect("back to the diff");
    assert_eq!(app.collapsed().len(), 1);

    app.on_key(KeyCode::Char('s')).expect("fold the line");
    assert!(
        ids.iter().all(|id| app.collapsed().contains(id)),
        "a mixed line collapses the rest rather than expanding the one: {:?}",
        app.collapsed()
    );

    app.on_key(KeyCode::Char('s')).expect("unfold the line");
    assert!(
        ids.iter().all(|id| !app.collapsed().contains(id)),
        "an all-collapsed line expands together: {:?}",
        app.collapsed()
    );
}

/// From inside the stack, `s` folds the one box the cursor is on — the reason
/// there is a cursor in there at all.
#[test]
fn from_the_stack_s_collapses_only_the_selected_box() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, "first finding");
    write_comment(&mut app, "second finding");
    let first = app.comments_for_line(app.line_index())[0].id.clone();

    app.on_key(KeyCode::Enter).expect("enter the stack");
    app.on_key(KeyCode::Char('s')).expect("collapse the first");

    assert!(app.collapsed().contains(&first), "{:?}", app.collapsed());
    assert_eq!(app.collapsed().len(), 1, "the other box is untouched");

    app.on_key(KeyCode::Char('j')).expect("select the second");
    app.on_key(KeyCode::Char('s')).expect("collapse the second");
    assert_eq!(app.collapsed().len(), 2, "and now both are folded");
}

/// From the sidebar's **Comments** tab, `s` folds the comment the browser's
/// cursor is on — the same rule `d` follows there, and for the same reason: a
/// key pressed in the browser acts on what the browser is showing.
///
/// The browsed comment is deliberately anchored in the *other* file from the
/// one the diff cursor is in, so the rule this replaces — fold the selected
/// line's boxes — folds nothing at all here and cannot pass by coincidence.
#[test]
fn from_the_comments_tab_s_folds_the_browsed_comment() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    app.on_key(KeyCode::Char(']')).expect("next file");
    write_comment(&mut app, "on the second file");
    let id = app.comments()[0].id.clone();
    app.on_key(KeyCode::Char('['))
        .expect("back to the first file");
    assert!(
        app.comments_for_line(app.line_index()).is_empty(),
        "the cursor is on a line that has comments, so the line rule would pass by luck"
    );

    app.on_key(KeyCode::Tab).expect("comments tab");
    app.on_key(KeyCode::Left).expect("focus the sidebar");
    app.on_key(KeyCode::Char('s'))
        .expect("fold the browsed comment");

    assert!(
        app.collapsed().contains(&id),
        "`s` in the browser folded something other than the comment it is showing: {:?}",
        app.collapsed()
    );
    assert!(
        !app.status().contains("no comments"),
        "it refused and folded anyway: {:?}",
        app.status()
    );

    app.on_key(KeyCode::Char('s')).expect("unfold it again");
    assert!(
        app.collapsed().is_empty(),
        "it is a toggle in the browser too: {:?}",
        app.collapsed()
    );
}

/// ...and it is the *browsed* comment rather than the line's, with both on
/// screen: the cursor sits on the first comment's line while the browser is on
/// the second.
#[test]
fn from_the_comments_tab_s_folds_the_browsed_comment_not_the_selected_lines() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, "first finding");
    let first = app.comments()[0].id.clone();
    app.on_key(KeyCode::Char('j')).expect("next line");
    write_comment(&mut app, "second finding");
    let second = app.comments()[1].id.clone();
    assert_ne!(first, second);
    app.on_key(KeyCode::Char('k'))
        .expect("back onto the first comment's line");

    app.on_key(KeyCode::Tab).expect("comments tab");
    app.on_key(KeyCode::Left).expect("focus the sidebar");
    app.on_key(KeyCode::Down).expect("browse to the second");
    app.on_key(KeyCode::Char('s')).expect("fold it");

    assert!(
        app.collapsed().contains(&second),
        "the browsed comment is still open: {:?}",
        app.collapsed()
    );
    assert!(
        !app.collapsed().contains(&first),
        "the comment on the selected diff line was folded instead: {:?}",
        app.collapsed()
    );
}

/// The **Files** tab keeps the older rule, because a file row selects no
/// comment: `s` there folds the boxes on the diff line the reviewer left the
/// cursor on, which is the only comment the screen is showing them.
#[test]
fn from_the_files_tab_s_still_folds_the_selected_lines_boxes() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, "needs a doc");
    let id = app.comments()[0].id.clone();

    app.on_key(KeyCode::Left).expect("focus the file list");
    assert_eq!(app.sidebar_tab(), SidebarTab::Files);
    app.on_key(KeyCode::Char('s')).expect("fold");

    assert!(
        app.collapsed().contains(&id),
        "`s` from the file list stopped folding the selected line: {:?}",
        app.collapsed()
    );
}

/// `s` with an empty browser folds nothing and says why — and says it about the
/// review rather than about a line, because a line is not what the reviewer was
/// looking at.
#[test]
fn from_an_empty_comment_browser_s_says_so() {
    let workspace = Fixture::new();
    let mut app = workspace.app();

    app.on_key(KeyCode::Tab).expect("comments tab");
    app.on_key(KeyCode::Left).expect("focus the sidebar");
    app.on_key(KeyCode::Char('s')).expect("s");

    assert!(app.collapsed().is_empty(), "{:?}", app.collapsed());
    assert!(app.status().contains("no comments"), "{:?}", app.status());
    assert!(
        !app.status().contains("this line"),
        "the browser refused with a sentence about a line it is not showing: {:?}",
        app.status()
    );
}

/// Collapse is a *view* preference, held for this session only. It is not
/// review state: another reviewer opening the same `.review/` has their own
/// idea of which boxes are in their way, and an export written from a folded
/// screen must not be a folded document.
#[test]
fn collapse_state_never_reaches_disk() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, "needs a doc");

    let before = workspace_tree(workspace.root());
    assert!(!before.is_empty(), "the review wrote nothing to compare");

    app.on_key(KeyCode::Char('s')).expect("collapse");
    assert_eq!(app.collapsed().len(), 1, "nothing was collapsed");

    let after = workspace_tree(workspace.root());
    assert_eq!(
        after, before,
        "collapsing wrote to the workspace; it is a view preference, not review state"
    );
    for (path, _, bytes) in after
        .iter()
        .filter(|(path, ..)| path.starts_with(".review"))
    {
        assert!(
            !String::from_utf8_lossy(bytes).contains("collaps"),
            "{path} mentions collapsing"
        );
    }

    // ...and a reviewer who reopens the review finds every box open again.
    let reopened = workspace.app();
    assert!(
        reopened.collapsed().is_empty(),
        "collapse survived the process it was a preference of: {:?}",
        reopened.collapsed()
    );
}

/// A comment that is deleted is not a folded comment, so its id does not stay
/// in the fold set — where it would fold whatever later comment hashed to the
/// same id (the same body, on the same line) under a preference about a
/// comment the reviewer threw away.
#[test]
fn deleting_a_folded_comment_forgets_that_it_was_folded() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, "needs a doc");
    let id = app.comments()[0].id.clone();

    app.on_key(KeyCode::Char('s')).expect("fold it away");
    assert!(app.collapsed().contains(&id));

    app.on_key(KeyCode::Char('d')).expect("ask");
    app.on_key(KeyCode::Char('y')).expect("confirm");

    assert!(
        app.collapsed().is_empty(),
        "the deleted comment is still folded: {:?}",
        app.collapsed()
    );

    // Retyped, the same comment comes back open.
    write_comment(&mut app, "needs a doc");
    assert_eq!(
        app.comments()[0].id,
        id,
        "the id is derived, so it is the same"
    );
    assert!(
        app.collapsed().is_empty(),
        "a fresh comment inherited a fold: {:?}",
        app.collapsed()
    );
}
