//! Browsing comments in the sidebar: how the browser is grouped, and how its
//! cursor and the pointer agree about which comment a row holds.

use crossterm::event::KeyCode;
use ratatui::style::Modifier;
use rv::app::App;
use rv::app::BrowserRow;
use rv::app::Focus;
use rv::layout::Split;

use crate::support::*;

/// A review with a comment in each of its two files, written the way a
/// reviewer writes them — `b.rs` first, so store order and `(file, line)`
/// order disagree and a browser that kept store order cannot pass by luck.
///
/// The `b.rs` comment is on the file's *second* line and the `a.rs` one on its
/// first, so the two orderings differ in both dimensions at once.
fn two_files(workspace: &Fixture) -> App {
    let mut app = workspace.app();
    app.on_key(KeyCode::Char(']')).expect("b.rs");
    app.on_key(KeyCode::Char('j')).expect("second line");
    write_comment(&mut app, "on b");
    app.on_key(KeyCode::Char('[')).expect("a.rs");
    write_comment(&mut app, "on a");
    assert_eq!(
        app.comments()
            .iter()
            .map(|comment| comment.body.as_str())
            .collect::<Vec<&str>>(),
        vec!["on b", "on a"],
        "the store stopped listing comments oldest first, so the ordering \
         assertions below would pass without proving anything"
    );
    to_comments(&mut app);
    app
}

/// Comments are grouped under a heading naming their file, in `(file, line)`
/// order — asserted on the cells the reviewer actually sees.
///
/// Store order is `b.rs` then `a.rs`; the browser shows `a.rs` first, and each
/// comment is indented under its own file's heading. A flat list in store order
/// fails on the very first row.
#[test]
fn the_browser_groups_comments_under_file_headings_in_file_and_line_order() {
    let workspace = Fixture::new();
    let app = two_files(&workspace);

    let drawn = sidebar_filled(&frame_at(&app, 100, 24), 100, 24, Split::default());
    let rows: Vec<&str> = drawn.iter().map(|row| row.trim_end()).collect();

    assert_eq!(
        rows,
        vec!["a.rs", "  a.rs:1 open on a", "b.rs", "  b.rs:2 open on b"],
        "the browser is not grouped by file in (file, line) order"
    );
}

/// A heading is a heading and a comment row is a comment row, in the model the
/// cursor indexes — so the drawing above is not the only thing that knows.
#[test]
fn the_browsers_rows_are_headings_and_comments() {
    let workspace = Fixture::new();
    let app = two_files(&workspace);

    let kinds: Vec<BrowserRow> = app.browser_rows();
    assert!(
        matches!(
            kinds.as_slice(),
            [
                BrowserRow::File(first),
                BrowserRow::Comment(_),
                BrowserRow::File(second),
                BrowserRow::Comment(_),
            ] if first == "a.rs" && second == "b.rs"
        ),
        "{kinds:?}"
    );
}

/// The cursor is a **row**, and `j`/`k` still reach every comment: they step
/// over the headings rather than parking on them.
///
/// This is the whole of the row/index split. A cursor that indexed comments
/// would select `on a` and draw its highlight on the `b.rs` heading; one that
/// indexed rows without skipping would need an extra `j` per file and would
/// leave `d` aimed at nothing on the rows in between.
#[test]
fn the_browser_cursor_walks_comments_and_never_rests_on_a_heading() {
    let workspace = Fixture::new();
    let mut app = two_files(&workspace);
    app.on_key(KeyCode::Left).expect("focus the sidebar");

    assert_eq!(
        app.browsed_comment().expect("a first comment").body,
        "on a",
        "the browser did not open on the first comment of the first file"
    );
    assert_eq!(
        app.browser_index(),
        1,
        "the cursor is on row 0, which is a heading"
    );

    app.on_key(KeyCode::Down).expect("next comment");
    assert_eq!(
        app.browsed_comment().expect("a second comment").body,
        "on b",
        "`j` did not step over the b.rs heading onto the comment under it"
    );

    app.on_key(KeyCode::Down).expect("past the end");
    assert_eq!(
        app.browsed_comment().expect("still the last").body,
        "on b",
        "the cursor ran off the end of the list"
    );

    app.on_key(KeyCode::Up).expect("back");
    assert_eq!(app.browsed_comment().expect("the first again").body, "on a");

    app.on_key(KeyCode::Up).expect("past the top");
    assert_eq!(
        app.browsed_comment().expect("still the first").body,
        "on a",
        "`k` parked on the heading above the first comment, leaving `d` and \
         `s` with no target"
    );
}

/// A click selects the comment on the row the reviewer pointed at — including
/// a row that is below a heading, which is where a cursor that counted comments
/// instead of rows would be off by one per file.
#[test]
fn a_click_below_a_heading_selects_the_comment_that_was_drawn_there() {
    let workspace = Fixture::new();
    let mut app = two_files(&workspace);

    // The last row of the four, which sits below *both* headings — so an
    // off-by-one of either sign lands somewhere else.
    let frame = frame_at(&app, 100, 24);
    let area = inner(areas(100, 24, Split::default()).sidebar);
    let target = sidebar_row_for_in(&frame, area, "b.rs:2");
    assert_eq!(
        target - area.y,
        3,
        "the fixture stopped drawing the b.rs comment on the fourth row"
    );

    app.on_mouse(click(area.x + 1, target)).expect("click");

    assert_eq!(app.focus(), Focus::Sidebar, "the click took no focus");
    assert_eq!(
        app.browsed_comment().expect("a clicked comment").body,
        "on b",
        "the click selected a comment other than the one on the row it landed on"
    );
}

/// ...and a click on a heading lands on the heading, rather than being nudged
/// onto a comment the reviewer did not point at.
#[test]
fn a_click_on_a_heading_selects_the_heading() {
    let workspace = Fixture::new();
    let mut app = two_files(&workspace);

    let frame = frame_at(&app, 100, 24);
    let area = inner(areas(100, 24, Split::default()).sidebar);
    let heading = sidebar_row_for_in(&frame, area, "b.rs");

    app.on_mouse(click(area.x + 1, heading)).expect("click");

    assert!(
        app.browsed_comment().is_none(),
        "a heading row reported a selected comment: {:?}",
        app.browsed_comment()
    );
    assert!(
        matches!(app.browser_rows()[app.browser_index()], BrowserRow::File(_)),
        "the click was moved off the row it landed on"
    );
}

/// `Enter` on a heading opens the file it names, at its top, with the diff
/// focused.
///
/// A heading names a file and nothing else, so that is the one thing it can
/// defensibly mean: jumping to some comment under it would be choosing one the
/// reviewer did not point at.
#[test]
fn enter_on_a_heading_opens_the_file_it_names() {
    let workspace = Fixture::new();
    let mut app = two_files(&workspace);
    // Start on the *other* file, so opening `b.rs` is a move rather than a
    // no-op that would pass however `Enter` behaved.
    assert_eq!(app.selected_file().expect("a file").path, "a.rs");

    let frame = frame_at(&app, 100, 24);
    let area = inner(areas(100, 24, Split::default()).sidebar);
    let heading = sidebar_row_for_in(&frame, area, "b.rs");
    app.on_mouse(click(area.x + 1, heading)).expect("click");
    app.on_key(KeyCode::Enter).expect("enter");

    assert_eq!(
        app.selected_file().expect("a file").path,
        "b.rs",
        "`Enter` on the b.rs heading did not open b.rs"
    );
    assert_eq!(app.line_index(), 0, "and did not put the cursor at its top");
    assert_eq!(app.focus(), Focus::Diff, "and did not hand over the diff");
    assert!(
        app.status().contains("b.rs"),
        "it went somewhere without saying so: {:?}",
        app.status()
    );
}

/// No row of the comment browser is painted over — the standing sidebar
/// ruling, which the heading rows have to respect too. Selection is the only
/// full-row background in this pane.
#[test]
fn no_row_of_the_comment_browser_is_painted_over() {
    let workspace = Fixture::new();
    let app = two_files(&workspace);
    let frame = frame_at(&app, 100, 24);
    let area = inner(areas(100, 24, Split::default()).sidebar);

    for y in area.y..area.bottom() {
        for x in area.x..area.right() {
            assert_eq!(
                bg_of(&frame, x, y),
                None,
                "cell ({x}, {y}) of the unfocused comment browser carries a \
                 background:\n{}",
                text_in(&frame, area)
            );
        }
    }
}

/// The collective note (branch-reviewer spec §7.2), and the honest half of it:
/// while blobs are still unread the note says how much it has checked, and it
/// only speaks about the whole review once every file has an answer.
///
/// Blobs load lazily, per spec §7, so a bare "2 files with no semantic change"
/// would be a claim about the review made from the part of it that happens to
/// have been opened. The qualifier leads, so a border too narrow to hold the
/// note truncates a partial answer into a shorter partial one.
#[test]
fn the_note_says_how_much_of_the_review_it_has_actually_checked() {
    let workspace = Fixture::reindented();
    let mut app = workspace.app_from("@--");
    to_comments(&mut app);

    let counted = app.suppression();
    assert_eq!(counted.total, 3, "the fixture stopped having three files");
    assert!(
        counted.checked < counted.total,
        "every file was diffed before a key was pressed, so the partial state \
         this test is about never happens: {counted:?}"
    );

    let note = sidebar_shape(&frame_at(&app, 100, 24));
    assert!(
        note.contains("no semantic change"),
        "the browser says nothing about the files with no semantic change: \
         {note:?}"
    );
    assert!(
        note.contains(&format!("{}/{}", counted.suppressed, counted.checked)),
        "the partial note does not carry the denominator that says it is \
         partial: {note:?}"
    );
}

/// ...and once every file has been read, the count settles on the truth: two
/// of the three files carry no semantic change, and the note drops the
/// qualifier because there is no longer anything unknown to qualify.
#[test]
fn the_note_settles_on_the_true_count_once_every_file_is_checked() {
    let workspace = Fixture::reindented();
    let mut app = workspace.app_from("@--");
    // Walk the whole review, which is what makes every file's diff known —
    // the same way a reviewer comes to know it.
    for _ in 0..app.files().len() {
        app.on_key(KeyCode::Char(']')).expect("next file");
    }
    to_comments(&mut app);

    let counted = app.suppression();
    assert_eq!(
        (counted.checked, counted.total),
        (3, 3),
        "walking the review left a file unread: {counted:?}"
    );
    assert_eq!(
        counted.suppressed, 2,
        "the two reindented files are not the two counted: {counted:?}"
    );

    let note = sidebar_shape(&frame_at(&app, 100, 24));
    assert!(
        note.contains("2 · no semantic change"),
        "the settled note does not state the count plainly: {note:?}"
    );
    assert!(
        !note.contains("2/3"),
        "the note still hedges after every file has been checked: {note:?}"
    );
}

/// A review where nothing was reindented says nothing at all. A permanent
/// `0 of 3` would spend the border on a fact about the loader.
#[test]
fn a_review_with_no_suppressed_file_carries_no_note() {
    let workspace = Fixture::new();
    let app = two_files(&workspace);

    let note = sidebar_shape(&frame_at(&app, 100, 24));
    assert!(
        !note.contains("no semantic change"),
        "a review with nothing suppressed claimed otherwise: {note:?}"
    );
}

/// The browser's own selection is marked, and only while the sidebar has the
/// focus — the same rule the file list follows.
#[test]
fn the_browsed_row_is_highlighted_when_the_sidebar_has_focus() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, "needs a doc");
    to_comments(&mut app);

    let reversed = |app: &App| {
        let buffer = frame_at(app, 100, 24);
        (0..24).any(|y| (0..30).any(|x| buffer[(x, y)].modifier.contains(Modifier::REVERSED)))
    };

    assert!(!reversed(&app), "the unfocused browser is reversed");
    app.on_key(KeyCode::Left).expect("focus the sidebar");
    assert!(reversed(&app), "the focused browser has no selection");
}

/// The bar drops whole segments rather than cutting one in half, at every
/// width, and the pointer to the keymap is the last thing standing.
///
/// This replaces a test that asserted the opposite — that a status line too
/// long for the terminal ends in `…`. That was true when `app.status()` *was*
/// the bar, and it was the defect: half of `deleted comment at app.rs:42` is a
/// claim about a file that does not exist, and a status that owned the whole
/// row could evict the one in-app pointer to the keys. The bar is segments now
/// (see `rv::statusbar`), so a segment either fits or is dropped whole, and the
/// hint outlives every one of them.
#[test]
fn the_bar_drops_a_segment_whole_rather_than_cutting_a_word() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, "a finding");

    for width in [16u16, 24, 40, 60, 80, 100, 120] {
        let frame = frame_at(&app, width, 24);
        let bar = last_row(&frame);
        assert!(
            !bar.contains('…'),
            "the bar cut a segment in half at {width} columns: {bar:?}"
        );
        assert!(
            bar.contains("? help"),
            "the pointer to the keymap went first at {width} columns: {bar:?}"
        );
        assert!(
            (0..width).all(|x| bg_of(&frame, x, 23).is_some()),
            "the bar left part of the row bare at {width} columns: {bar:?}"
        );
    }
}
