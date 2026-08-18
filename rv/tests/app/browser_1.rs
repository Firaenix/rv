//! Browsing comments in the sidebar.

use crossterm::event::KeyCode;
use rstest::rstest;
use rv::app::App;
use rv::app::Focus;
use rv::app::Mode;
use rv::app::SidebarTab;
use rv_core::diff::LineKind;
use rv_core::model::Side;

use crate::support::*;

/// Walks the sidebar's comment browser to row `index` and presses `Enter`,
/// exactly the way a reviewer does — no test-only entry point into the jump.
fn jump_to_row(app: &mut App, index: usize) {
    to_comments(app);
    app.on_key(KeyCode::Left).expect("focus the sidebar");
    for _ in 0..index {
        app.on_key(KeyCode::Down).expect("next row");
    }
    app.on_key(KeyCode::Enter).expect("jump");
}

#[test]
fn tab_cycles_the_sidebar_through_its_three_lists() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    assert_eq!(app.sidebar_tab(), SidebarTab::Files, "files by default");

    app.on_key(KeyCode::Tab).expect("tab");
    assert_eq!(app.sidebar_tab(), SidebarTab::Commits);
    app.on_key(KeyCode::Tab).expect("tab");
    assert_eq!(app.sidebar_tab(), SidebarTab::Comments);
    app.on_key(KeyCode::Tab).expect("tab round");
    assert_eq!(
        app.sidebar_tab(),
        SidebarTab::Files,
        "the cycle does not come back round"
    );
}

/// From any focus, and without disturbing anything else: `Tab` is about what
/// the left column *lists*, not about where the cursor is.
#[rstest]
#[case(&[])]
#[case(&[KeyCode::Left])]
#[case(&[KeyCode::Enter])]
fn tab_switches_the_tab_from_any_focus(#[case] approach: &[KeyCode]) {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, "needs a doc");
    for key in approach {
        app.on_key(*key).expect("get into position");
    }
    let (focus, file, line) = (app.focus(), app.file_index(), app.line_index());

    app.on_key(KeyCode::Tab).expect("tab");

    assert_eq!(app.sidebar_tab(), SidebarTab::Commits);
    assert_eq!(app.focus(), focus, "tab moved the cursor to another pane");
    assert_eq!((app.file_index(), app.line_index()), (file, line));
    assert_eq!(app.mode(), Mode::Browse);
}

#[test]
fn the_comment_browser_lists_every_comment_in_the_review() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, "first finding");
    app.on_key(KeyCode::Char(']')).expect("next file");
    write_comment(&mut app, "second finding");

    to_comments(&mut app);
    app.on_key(KeyCode::Left).expect("focus the sidebar");

    assert_eq!(
        app.browsed_comment().expect("a first row").body,
        "first finding",
        "the browser opens on the oldest comment"
    );
    app.on_key(KeyCode::Down).expect("next row");
    assert_eq!(
        app.browsed_comment().expect("a second row").body,
        "second finding",
        "the browser lists comments from every file, not only the open one"
    );
    app.on_key(KeyCode::Down).expect("past the end");
    assert_eq!(
        app.browsed_comment().expect("still the second").body,
        "second finding",
        "the cursor stops at the newest rather than wrapping"
    );
    app.on_key(KeyCode::Up).expect("back");
    assert_eq!(
        app.browsed_comment().expect("the first again").body,
        "first finding"
    );
    assert_eq!(
        app.file_index(),
        1,
        "walking the comment browser moved the file selection"
    );
}

/// The whole point of the browser: reading a comment and looking at the code it
/// is about are one keystroke apart.
#[test]
fn enter_on_a_comment_row_jumps_to_its_code() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    // Comment on the second file, then walk away to the first.
    app.on_key(KeyCode::Char(']')).expect("next file");
    app.on_key(KeyCode::Char('j')).expect("move down");
    let commented_file = app.file_index();
    let commented_line = app.line_index();
    write_comment(&mut app, "look at this");
    app.on_key(KeyCode::Char('['))
        .expect("back to the first file");
    assert_ne!(
        app.file_index(),
        commented_file,
        "the walk away did nothing"
    );

    jump_to_row(&mut app, 0);

    assert_eq!(app.file_index(), commented_file, "landed on the right file");
    assert_eq!(app.line_index(), commented_line, "and the right line");
    assert_eq!(
        app.focus(),
        Focus::Diff,
        "with the diff focused, ready to act"
    );
    assert_eq!(
        app.comments_for_line(app.line_index()).len(),
        1,
        "the comment is not on the line the jump landed on"
    );
    assert!(
        app.status().contains("b.rs"),
        "the jump does not say where it went: {:?}",
        app.status()
    );
}

/// The jump uses the same anchor key the save used, so it lands where the
/// comment says it is even when the two sides of the diff number that line
/// differently.
#[test]
fn a_jump_lands_on_the_line_the_comment_is_anchored_to() {
    let workspace = Fixture::renamed();
    let mut app = workspace.app_from("@--");
    let removed = select_line(&mut app, |line| {
        line.kind == LineKind::Removed && line.text.contains("let x = 1;")
    });
    let line = app.line_index();
    write_comment(&mut app, "why was this rewritten?");
    assert_eq!(
        removed.left,
        Some(2),
        "the fixture stopped pairing the rewrite"
    );
    assert_eq!(removed.right, Some(3), "{removed:?}");

    // Walk away, then come back through the browser.
    app.on_key(KeyCode::Char('k')).expect("up");
    app.on_key(KeyCode::Char('k')).expect("up");
    jump_to_row(&mut app, 0);

    assert_eq!(
        app.line_index(),
        line,
        "the jump used a different rule from the save: {:?}",
        app.selected_diff().expect("a diff").lines[app.line_index()]
    );
    assert_eq!(app.comments_for_line(app.line_index()).len(), 1);
}

/// A comment whose file is not in the review's range is **not listed**.
///
/// `.review/` outlives any one range, so a comment written last week against a
/// wider revset can be anchored to a file this range does not touch. It used to
/// appear in the browser and answer `Enter` with an alert, which is a row that
/// exists only to refuse — a reviewer counting their open comments was counting
/// jumps they could not make.
///
/// The comment is not deleted: the store keeps it, the export still carries it,
/// and a wider range shows it again. It is only absent from the list of things
/// this review can take you to.
#[test]
fn a_comment_outside_the_range_is_not_listed() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, "on a file in the range");
    store_variant(&workspace, "deadbee1", "gone.rs", 1);

    let reopened = workspace.app();

    assert_eq!(
        reopened.comments().len(),
        1,
        "the browser lists a comment the range cannot reach: {:?}",
        reopened
            .comments()
            .iter()
            .map(|comment| comment.anchor.file.as_str())
            .collect::<Vec<_>>()
    );
    assert!(
        reopened
            .comments()
            .iter()
            .all(|comment| comment.anchor.file != "gone.rs"),
        "the out-of-range comment is still listed"
    );
    // Still on disk, which is the half that must not change: hiding a row is not
    // deleting a comment.
    let stored = workspace.store().comments().expect("read the store");
    assert_eq!(stored.len(), 2, "filtering the list deleted a comment");
    assert!(
        stored
            .iter()
            .any(|comment| comment.anchor.file == "gone.rs"),
        "the out-of-range comment left the store"
    );
    drop(app);
}

/// A comment whose line has left the diff still opens its file: being in the
/// right file with a warning beats staying put.
#[test]
fn a_jump_to_a_missing_line_opens_the_file_anyway() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    app.on_key(KeyCode::Char(']')).expect("next file");
    write_comment(&mut app, "on the second file");
    store_variant(&workspace, "deadbee2", "a.rs", 99);

    let mut reopened = workspace.app();
    reopened.on_key(KeyCode::Char(']')).expect("open b.rs");
    jump_to_row(&mut reopened, 1);

    assert_eq!(
        reopened.selected_file().expect("a file").path,
        "a.rs",
        "the jump did not open the file the comment names"
    );
    assert_eq!(reopened.line_index(), 0, "and put the cursor at its top");
    assert_eq!(reopened.focus(), Focus::Diff);
    assert!(
        reopened.status().contains("99") && reopened.status().contains("not in this diff"),
        "the jump did not say what it could not find: {:?}",
        reopened.status()
    );
    drop(app);
}

/// Stores a copy of the review's first comment under a different id, file and
/// line — the two shapes a jump has to fail honestly on, neither of which the
/// keyboard can produce.
fn store_variant(workspace: &Fixture, id: &str, file: &str, line: u32) {
    let mut comment = workspace
        .store()
        .comments()
        .expect("read comments")
        .swap_remove(0);
    comment.id = id.to_owned();
    comment.anchor.file = file.to_owned();
    comment.anchor.line = line;
    comment.anchor.side = Side::Right;
    workspace
        .store()
        .append_comment(&comment)
        .expect("store the variant");
}

/// `d` from the browser deletes the comment the browser has selected, behind
/// the same confirmation as everywhere else.
#[test]
fn d_from_the_comment_browser_deletes_behind_the_same_confirmation() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, "first finding");
    app.on_key(KeyCode::Char('j')).expect("next line");
    write_comment(&mut app, "second finding");

    to_comments(&mut app);
    app.on_key(KeyCode::Left).expect("focus the sidebar");
    app.on_key(KeyCode::Down).expect("select the second");
    app.on_key(KeyCode::Char('d')).expect("ask");
    assert!(
        matches!(app.mode(), Mode::ConfirmDelete { .. }),
        "it deleted without asking: {:?}",
        app.mode()
    );
    app.on_key(KeyCode::Char('y')).expect("confirm");

    let left = workspace.store().comments().expect("read");
    assert_eq!(left.len(), 1, "{left:?}");
    assert_eq!(left[0].body, "first finding", "the browsed comment went");
    assert_eq!(
        app.browsed_comment().expect("a row").body,
        "first finding",
        "the browser's cursor was left past the end of the list"
    );
}

/// ...and `n` still cancels, from here as from anywhere.
#[test]
fn d_from_the_comment_browser_can_be_declined() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, "needs a doc");

    to_comments(&mut app);
    app.on_key(KeyCode::Left).expect("focus the sidebar");
    app.on_key(KeyCode::Char('d')).expect("ask");
    app.on_key(KeyCode::Char('n')).expect("decline");

    assert_eq!(app.mode(), Mode::Browse);
    assert_eq!(workspace.store().comments().expect("read").len(), 1);
}

/// The Files tab stays inert: it selects files, and a destructive key aimed
/// into it would be aimed at a comment the reviewer cannot see.
#[test]
fn d_from_the_files_tab_still_deletes_nothing() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, "needs a doc");

    app.on_key(KeyCode::Left).expect("focus the file list");
    assert_eq!(app.sidebar_tab(), SidebarTab::Files);
    app.on_key(KeyCode::Char('d')).expect("d");

    assert_eq!(app.mode(), Mode::Browse);
    assert!(app.status().contains("not comments"), "{:?}", app.status());
    assert_eq!(workspace.store().comments().expect("read").len(), 1);
}

/// `d` with an empty browser asks nothing and says why.
#[test]
fn d_from_an_empty_comment_browser_says_so() {
    let workspace = Fixture::new();
    let mut app = workspace.app();

    to_comments(&mut app);
    app.on_key(KeyCode::Left).expect("focus the sidebar");
    app.on_key(KeyCode::Char('d')).expect("d");

    assert_eq!(app.mode(), Mode::Browse);
    assert!(app.status().contains("no comments"), "{:?}", app.status());
}

#[test]
fn the_comment_browser_renders_path_line_and_state() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, "needs a doc");
    to_comments(&mut app);

    let buffer = frame_at(&app, 100, 24);
    let text = buffer_text(&buffer);

    assert!(
        text.contains("Comments (1)"),
        "the tab is unmistakable:\n{text}"
    );
    assert!(
        text.contains("a.rs:1"),
        "the file and line are named:\n{text}"
    );
    assert!(
        text.contains("needs a doc"),
        "the body is previewed:\n{text}"
    );
    assert!(text.contains("open"), "the state is shown:\n{text}");
    assert!(
        !text.contains("Files ("),
        "the file list is still on screen beside it:\n{text}"
    );
}

#[test]
fn an_empty_comment_browser_says_so() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    to_comments(&mut app);

    let text = buffer_text(&frame_at(&app, 100, 24));

    assert!(
        text.contains("no comments"),
        "an empty review does not explain itself:\n{text}"
    );
}
