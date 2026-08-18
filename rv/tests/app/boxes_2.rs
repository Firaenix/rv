//! Drawing a comment box, and which pane has the focus.

use crossterm::event::KeyCode;
use ratatui::style::Color;
use ratatui::style::Modifier;
use rstest::rstest;
use rv_core::store::CommentState;

use crate::support::*;

/// A comment that is no longer open is drawn grey and dim rather than blue, and
/// opens folded: it is still exactly where the reviewer left it, without
/// competing for attention with the comments that still need answering.
///
/// Driven through the store because nothing in the reviewer can produce a
/// non-`Open` comment yet — state transitions are milestone 2's work — and a
/// `.review/` written by that milestone, or by an agent, must render sensibly
/// today rather than whenever the keyboard catches up.
#[test]
fn an_outdated_comment_is_grey_and_folded() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, "this line moved");
    let id = app.comments()[0].id.clone();

    let mut stored = workspace.store().comments().expect("read comments");
    stored[0].state = CommentState::Outdated;
    workspace
        .store()
        .append_comment(&stored[0])
        .expect("store the outdated comment");

    let reopened = workspace.app();
    assert!(
        reopened.collapsed().contains(&id),
        "an outdated comment opens expanded: {:?}",
        reopened.collapsed()
    );
    let buffer = frame_at(&reopened, 100, 24);
    let text = buffer_text(&buffer);
    let row = u16::try_from(row_holding(&buffer, "this line moved")).expect("a small row");
    let style = style_of_text(&buffer, row, &id);
    assert_eq!(
        style.fg,
        Some(Color::Gray),
        "an outdated comment is drawn as loud as an open one:\n{text}"
    );
    assert!(
        style.add_modifier.contains(Modifier::DIM),
        "an outdated comment is not dimmed:\n{text}"
    );
    assert!(
        text.contains("outdated"),
        "the row does not say why it is grey:\n{text}"
    );

    drop(app);
}

/// The selected box is kept on screen in its own right, so stepping through a
/// stack in a short pane does not leave the cursor on a box below the fold.
#[test]
fn the_selected_box_stays_on_screen_in_a_short_pane() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    for body in ["first finding", "second finding", "third finding"] {
        write_comment(&mut app, body);
    }

    app.on_key(KeyCode::Enter).expect("enter the stack");
    app.on_key(KeyCode::Char('j')).expect("second");
    app.on_key(KeyCode::Char('j')).expect("third");

    // Eight rows: a status bar, two borders, and five rows of pane — far less
    // than the three boxes need.
    let text = buffer_text(&frame_at(&app, 100, 8));
    assert!(
        text.contains("third finding"),
        "the selected box is below the fold:\n{text}"
    );
}

/// Drawing must be total. A one-column pane is where ratatui layout code
/// classically panics, and a comment box subtracts a gutter, two borders and a
/// pad from whatever width it is given.
#[rstest]
#[case(1, 1)]
#[case(2, 5)]
#[case(20, 3)]
#[case(1, 40)]
#[case(5, 2)]
#[case(3, 3)]
#[case(9, 6)]
#[case(12, 24)]
fn drawing_never_panics_at_awkward_sizes(#[case] width: u16, #[case] height: u16) {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(
        &mut app,
        "needs a doc, and a body long enough to have to wrap somewhere",
    );
    write_comment(&mut app, "second finding");

    let _ = frame_at(&app, width, height);

    app.on_key(KeyCode::Enter).expect("enter the stack");
    let _ = frame_at(&app, width, height);

    app.on_key(KeyCode::Char('s'))
        .expect("fold the selected box");
    let _ = frame_at(&app, width, height);

    app.on_key(KeyCode::Left).expect("back to the diff");
    app.on_key(KeyCode::Left).expect("onto the sidebar");
    let _ = frame_at(&app, width, height);
}
