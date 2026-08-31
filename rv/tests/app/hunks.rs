//! Walking hunks with `J` and `K`.

use crossterm::event::KeyCode;
use rv::app::App;
use rv::app::Focus;

use crate::support::*;

/// How many lines `wide.rs` is.
const WIDE_LINES: u32 = 30;

/// Which lines of it [`three_hunks`] rewrites, 1-based — and therefore the
/// line each of `J`'s three stops should report.
const EDITED: [u32; 3] = [3, 15, 27];

/// One file edited in three places, a dozen unchanged lines apart — the shape
/// `J` exists for.
///
/// Deliberately far apart. Difftastic emits *only* the lines that changed, so
/// three edits one line apart and three edits a dozen lines apart arrive as the
/// same run of consecutive `DiffLine`s; the distance between them is the only
/// thing that makes this three hunks rather than one, and it is exactly what a
/// boundary rule reading line kinds alone would miss.
///
/// Reviewed from `@--`, so the range holds the third change only and `wide.rs`
/// is the one file in it.
fn three_hunks() -> Fixture {
    let fixture = Fixture::new();
    fixture.write("wide.rs", &numbered("keep", WIDE_LINES));
    fixture.jj(&["describe", "-m", "a file to edit in three places"]);
    fixture.jj(&["new"]);

    let mut lines: Vec<String> = numbered("keep", WIDE_LINES)
        .lines()
        .map(str::to_owned)
        .collect();
    for line in EDITED {
        let index = usize::try_from(line).expect("a line number") - 1;
        lines[index] = format!("let edited{line} = {line};");
    }
    fixture.write("wide.rs", &format!("{}\n", lines.join("\n")));
    fixture.jj(&["describe", "-m", "three separated edits"]);
    fixture.jj(&["new"]);
    fixture
}

/// A reviewer on `wide.rs`, at the top of it.
fn on_wide() -> (Fixture, App) {
    let workspace = three_hunks();
    let app = workspace.app_from("@--");
    assert_eq!(
        app.files().len(),
        1,
        "the range should hold wide.rs alone: {:?}",
        app.files()
    );
    (workspace, app)
}

/// Which line of the file the cursor is on, on whichever side names it.
///
/// The *number*, not the row: a rewritten line is a removal and an addition
/// under one number, and which of the two a jump lands on is the row model's
/// business rather than this test's.
fn cursor_line(app: &App) -> u32 {
    app.displayed_lines()
        .get(app.line_index())
        .and_then(|line| line.right.or(line.left))
        .expect("the cursor is on a numbered line")
}

/// Presses `key` until the cursor stops moving, collecting every line it
/// rested on — the one it started on included, since with full-file context
/// the reviewer opens on the file's first line, not on a hunk at all.
fn walk(app: &mut App, direction: KeyCode) -> Vec<u32> {
    let mut seen = vec![cursor_line(app)];
    let mut previous = app.line_index();
    for _ in 0..EDITED.len() + 2 {
        app.on_key(KeyCode::Char('g')).expect("goto leader");
        app.on_key(direction).expect("a hunk key");
        if app.line_index() == previous {
            break;
        }
        previous = app.line_index();
        seen.push(cursor_line(app));
    }
    seen
}

/// Each edit is its own hunk, visited in reading order — which is the whole
/// claim, since difftastic hands rv the changed lines with no context between
/// them and nothing but their numbers says there are three edits rather than
/// one.
///
/// With full-file context shown, a reviewer opens on the file's first line
/// (line 1), not on a changed one — so `J` is asked for all three hunks,
/// the first included, unlike before this feature when opening the diff
/// alone put the cursor on the first changed line already.
#[test]
fn j_steps_forward_through_every_hunk_and_stops_at_the_last() {
    let (_workspace, mut app) = on_wide();
    assert_eq!(
        cursor_line(&app),
        1,
        "a reviewer opens on the file's first line"
    );
    assert_eq!(
        walk(&mut app, KeyCode::Down),
        [1, EDITED[0], EDITED[1], EDITED[2]]
    );
}

/// No wrap at the far end: a jump from the last hunk to the first would look
/// exactly like a jump that did nothing, so it says so instead and stays put.
#[test]
fn j_at_the_last_hunk_says_so_rather_than_wrapping() {
    let (_workspace, mut app) = on_wide();
    walk(&mut app, KeyCode::Down);
    let last = app.line_index();

    app.on_key(KeyCode::Char('g')).expect("goto leader");
    app.on_key(KeyCode::Down).expect("J at the last hunk");
    assert_eq!(app.line_index(), last, "J wrapped off the last hunk");
    assert_eq!(app.status(), "the last hunk in this file");
    assert_eq!(
        cursor_line(&app),
        *EDITED.last().expect("a last edit"),
        "J left the last hunk"
    );
}

#[test]
fn k_steps_back_through_every_hunk_and_stops_at_the_first() {
    let (_workspace, mut app) = on_wide();
    walk(&mut app, KeyCode::Down);

    let mut back = walk(&mut app, KeyCode::Up);
    back.reverse();
    assert_eq!(back, EDITED);
}

#[test]
fn k_at_the_first_hunk_says_so_rather_than_wrapping() {
    let (_workspace, mut app) = on_wide();
    let first = app.line_index();

    app.on_key(KeyCode::Char('g')).expect("goto leader");
    app.on_key(KeyCode::Up).expect("K at the first hunk");
    assert_eq!(app.line_index(), first, "K wrapped off the first hunk");
    assert_eq!(app.status(), "the first hunk in this file");
}

/// A hunk key is a *cursor* key, so it writes through `set_cursor_row` like
/// every other one — which is what keeps the row a comment box hangs from, the
/// scroll and the bar in step. Pinned by the observable half of that: `c` after
/// `J` comments on the line `J` landed on.
#[test]
fn a_hunk_jump_moves_the_row_cursor_a_comment_is_anchored_to() {
    let (workspace, mut app) = on_wide();
    app.on_key(KeyCode::Char('g')).expect("goto leader");
    app.on_key(KeyCode::Down).expect("next hunk");
    let landed = cursor_line(&app);

    app.on_key(KeyCode::Char('c')).expect("comment leader");
    if app.pending_leader().is_some() {
        app.on_key(KeyCode::Char('c')).expect("write");
    }
    for character in "on the hunk".chars() {
        app.on_key(KeyCode::Char(character)).expect("type");
    }
    app.on_key(KeyCode::Enter).expect("save");

    let comments = workspace.store().comments().expect("read the comments");
    assert_eq!(comments.len(), 1, "{comments:?}");
    assert_eq!(
        comments[0].anchor.line, landed,
        "the comment was anchored to a line J did not land on"
    );
}

/// `J` is about the diff wherever it is pressed, so it brings the focus with
/// it: the cursor it just moved is the one the next `j` should move too.
#[test]
fn a_hunk_jump_from_the_sidebar_lands_in_the_diff() {
    let (_workspace, mut app) = on_wide();
    app.on_key(KeyCode::Left).expect("focus the sidebar");
    assert_eq!(app.focus(), Focus::Sidebar);

    app.on_key(KeyCode::Char('g')).expect("goto leader");
    app.on_key(KeyCode::Down).expect("J");
    assert_eq!(app.focus(), Focus::Diff);
    assert_eq!(cursor_line(&app), EDITED[0]);
}

/// A file the change only renamed has no hunk at all. That is a different fact
/// from "you have reached the last one", and reporting the wrong one would tell
/// a reviewer they are at the end of a list that does not exist.
#[test]
fn a_file_with_no_changed_lines_says_it_has_no_hunks() {
    let workspace = Fixture::pure_rename();
    let mut app = workspace.app_from("@--");

    app.on_key(KeyCode::Char('g')).expect("goto leader");
    app.on_key(KeyCode::Down).expect("J");
    assert_eq!(app.status(), "no hunks in this file");
    app.on_key(KeyCode::Char('g')).expect("goto leader");
    app.on_key(KeyCode::Up).expect("K");
    assert_eq!(app.status(), "no hunks in this file");
}
