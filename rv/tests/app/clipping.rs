//! Never clipping content silently.


use crossterm::event::KeyCode;
use rv::layout::Split;

use crate::support::*;

/// A diff line too long for the pane says so. Neither pane wraps or scrolls
/// horizontally, and this repository contains 154-character lines: a review
/// tool that silently hides the code being judged is failing at its one job.
#[test]
fn a_long_diff_line_is_marked_rather_than_silently_clipped() {
    let workspace = Fixture::with_long_line(200);
    let app = workspace.app();

    let buffer = frame_at(&app, 60, 24);
    let text = buffer_text(&buffer);

    assert!(
        text.contains('…'),
        "a clipped line says so; silent truncation hides the code under review:\n{text}"
    );
    // ...and the marker sits against the pane's own right-hand border, so what
    // it reports is the edge of the pane rather than something dropped out of
    // the middle of the line.
    //
    // Cut to the diff pane's own columns: the file list clips its rows with the
    // same marker, and at sixty columns it is clipping `long.rs` too — so the
    // first `…` on this frame row belongs to the other pane.
    let y = u16::try_from(row_holding(&buffer, "xxx")).expect("a small row");
    let row = row_in(&buffer, areas(60, 24, Split::default()).diff, y);
    let after: String = row.chars().skip_while(|c| *c != '…').skip(1).collect();
    assert_eq!(
        after, "│",
        "the marker is not against the pane's edge: {row:?}"
    );
}

/// ...and it is *clipped*, not wrapped. The row model is built on one row per
/// diff line, and a reviewer counting lines against a file needs that
/// correspondence: a wrapped line would put the highlight and the line's own
/// number on different rows from the rest of it.
#[test]
fn a_long_diff_line_is_never_wrapped_onto_a_second_row() {
    let workspace = Fixture::with_long_line(200);
    let app = workspace.app();

    let buffer = frame_at(&app, 60, 24);
    let rows = rows_of(&buffer);
    let carrying: Vec<&String> = rows.iter().filter(|row| row.contains("xxx")).collect();

    assert_eq!(
        carrying.len(),
        1,
        "one diff line was drawn on {} rows:\n{}",
        carrying.len(),
        buffer_text(&buffer)
    );
}

/// A short line is left exactly as it was: the marker is a report of clipping,
/// not decoration.
#[test]
fn a_line_that_fits_is_not_marked() {
    let workspace = Fixture::new();
    let app = workspace.app();

    let text = buffer_text(&frame_at(&app, 100, 24));

    assert!(
        !text.contains('…'),
        "a pane with room to spare still claims it clipped something:\n{text}"
    );
    assert!(text.contains("    let x = 1;"), "{text}");
}

/// The comment bar follows the end of what is being typed. Past the bar's
/// width the reviewer used to be typing blind — the bar kept showing the
/// opening words while the cursor was 80 characters further on.
#[test]
fn the_comment_buffer_shows_the_tail_while_typing_past_the_width() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    app.on_key(KeyCode::Char('c')).expect("begin a comment");
    type_text(&mut app, "HEAD");
    type_text(&mut app, &"x".repeat(200));
    type_text(&mut app, "TAIL");

    let text = buffer_text(&frame_at(&app, 40, 24));

    assert!(
        text.contains("TAIL"),
        "what is being typed is not on screen:\n{text}"
    );
    assert!(
        !text.contains("HEAD"),
        "the bar is showing the start of a buffer whose end is where the cursor is:\n{text}"
    );
    assert_eq!(
        app.buffer().chars().count(),
        208,
        "the bar's window ate the buffer itself"
    );
}

/// A comment that fits is shown whole, from its first character: the tail is
/// what a long buffer falls back to, not what every buffer gets.
#[test]
fn a_short_comment_is_shown_from_its_beginning() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    app.on_key(KeyCode::Char('c')).expect("begin a comment");
    type_text(&mut app, "needs a doc");

    let text = buffer_text(&frame_at(&app, 40, 24));

    assert!(text.contains("needs a doc"), "{text}");
}
