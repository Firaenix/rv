//! Drawing a comment box, and which pane has the focus.

use crossterm::event::KeyCode;
use ratatui::style::Color;
use ratatui::style::Modifier;
use rstest::rstest;
use rv_core::model::Confidence;
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

/// A weak anchor is visible on the box itself.
///
/// The `Weak` tier means the commented *content* is gone and only its line
/// number survived — the box points at line 2 with nothing guaranteeing line 2
/// is what the remark was about. The comment still reads `open`, so nothing else
/// on the box distinguishes it from one whose code never moved, and a reviewer
/// acting on it as though it were exact is the failure the tag exists to
/// prevent. Asserted against an `Exact` box from the same fixture in the same
/// assertion, because "it says something" is only a claim if the control says
/// nothing.
#[test]
fn a_weak_anchor_is_marked_on_the_box_and_an_exact_one_is_not() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, "about this exact line");

    let exact = buffer_text(&frame_at(&workspace.app(), 100, 24));
    assert!(
        !exact.contains("weak anchor"),
        "a comment on live code is marked as drifted:\n{exact}"
    );

    // Rewritten in place: no content hash matches anywhere, but the file still
    // has a line at that number, so the cascade lands on its third tier.
    workspace.write("a.rs", "fn completely_different() {\n    let y = 9;\n}\n");
    workspace.jj(&["describe", "-m", "rewrite the file"]);
    workspace.jj(&["new"]);

    let reopened = workspace.app();
    let comment = &reopened.comments()[0];
    assert_eq!(
        reopened.confidence(comment),
        Confidence::Weak,
        "the fixture no longer produces a weak anchor: {comment:?}"
    );
    assert_eq!(
        comment.state,
        CommentState::Open,
        "a weak anchor is a placed anchor, so the state cannot be what tells them apart"
    );

    let buffer = frame_at(&reopened, 100, 24);
    let text = buffer_text(&buffer);
    assert!(
        text.contains("weak anchor"),
        "a weak anchor renders exactly like an exact one:\n{text}"
    );

    // In the alert colour and bold, not merely present: this is the one tier a
    // reviewer must not scan past.
    let row = u16::try_from(row_holding(&buffer, "weak anchor")).expect("a small row");
    let style = style_of_text(&buffer, row, "weak anchor");
    assert_eq!(
        style.fg,
        Some(Color::Yellow),
        "the weak-anchor mark is not drawn as an alert:\n{text}"
    );
    assert!(
        style.add_modifier.contains(Modifier::BOLD),
        "the weak-anchor mark is not emphasised:\n{text}"
    );

    drop(app);
}

/// A moved anchor says so too, and quietly: the content was found, just
/// somewhere else, which is worth reporting and not worth an alert.
#[test]
fn a_moved_anchor_is_marked_without_the_alert_colour() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, "about this exact line");

    // The commented line survives verbatim, pushed down by lines above it.
    workspace.write("a.rs", "// one\n// two\nfn a() {\n    let x = 1;\n}\n");
    workspace.jj(&["describe", "-m", "push the line down"]);
    workspace.jj(&["new"]);

    let reopened = workspace.app();
    assert_eq!(
        reopened.confidence(&reopened.comments()[0]),
        Confidence::Moved,
        "the fixture no longer produces a moved anchor"
    );

    let buffer = frame_at(&reopened, 100, 24);
    let text = buffer_text(&buffer);
    assert!(
        text.contains("moved"),
        "a moved anchor is silent about having moved:\n{text}"
    );
    let row = u16::try_from(row_holding(&buffer, "· moved")).expect("a small row");
    assert_ne!(
        style_of_text(&buffer, row, "· moved").fg,
        Some(Color::Yellow),
        "a moved anchor shouts as loudly as a weak one:\n{text}"
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
    app.on_key(KeyCode::Down).expect("second");
    app.on_key(KeyCode::Down).expect("third");

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
