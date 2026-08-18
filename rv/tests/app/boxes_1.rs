//! Drawing a comment box, and which pane has the focus.

use std::fs;

use crossterm::event::KeyCode;
use ratatui::style::Color;
use ratatui::style::Modifier;
use rv::app::App;
use rv::session;

use crate::support::*;

/// A saved comment is drawn as a bordered box hanging off the line it is
/// anchored to — the whole point of this milestone, and the thing a reviewer
/// could not see at all before it.
///
/// Asserted on the *cells* rather than on the text: "blue and bordered" is the
/// requirement, and a test that only greps for the body passes against an
/// unstyled box. The rounded corners are what distinguish a comment box from
/// the panes' own plain borders.
#[test]
fn a_comment_renders_as_a_blue_bordered_box_under_its_line() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    select_line(&mut app, |line| line.text.contains("let x = 1;"));
    write_comment(&mut app, "needs a doc");

    let buffer = frame_at(&app, 100, 24);
    let text = buffer_text(&buffer);
    assert!(
        text.contains("needs a doc"),
        "the body is on screen:\n{text}"
    );
    // Asked inside the diff pane: the panes' own corners are rounded too, so a
    // `╭` at the edge of the frame is a frame rather than a box.
    let inside = text_in(&buffer, box_area());
    assert!(
        inside.contains('╭') && inside.contains('╰'),
        "the box has borders:\n{text}"
    );
    assert!(
        styled_blue_in(&buffer, box_area(), '╭'),
        "the border is blue, which is the requirement:\n{text}"
    );
    assert!(
        styled_blue_in(&buffer, box_area(), '╰'),
        "and so is its other end:\n{text}"
    );

    // ...and it hangs off *its own* line, in order: top border, body, bottom.
    let rows = rows_of(&buffer);
    let anchored = row_holding(&buffer, "let x = 1;");
    assert!(
        rows[anchored + 1].contains('╭'),
        "the box does not open directly under the line it is about:\n{text}"
    );
    assert!(
        rows[anchored + 2].contains("needs a doc") && rows[anchored + 2].contains('│'),
        "the body is not inside the box:\n{text}"
    );
    assert!(
        rows[anchored + 3].contains('╰'),
        "the box does not close under its body:\n{text}"
    );
}

/// The box is indented to the diff's gutter, so it reads as hanging off the
/// line rather than as another pane.
#[test]
fn a_comment_box_is_indented_to_the_diff_gutter() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, "needs a doc");

    let buffer = frame_at(&app, 100, 24);
    let (corner_x, _) = find_char_in(&buffer, box_area(), '╭').expect("a box top is on screen");
    // Counted in characters, not bytes: the panes' own borders are multi-byte,
    // so a byte offset is not a column.
    let sigil_row = rows_of(&buffer)[row_holding(&buffer, "+fn a() {")].clone();
    let sigil = sigil_row
        .char_indices()
        .position(|(offset, _)| sigil_row[offset..].starts_with("+fn a() {"))
        .expect("the added line carries its sigil");

    assert_eq!(
        usize::from(corner_x),
        sigil + 1,
        "the box does not start one column past the sigil, where the line's own \
         text starts:\n{}",
        buffer_text(&buffer)
    );
}

/// Border and title are blue; the body keeps the terminal's own foreground, so
/// the part being *read* is at full contrast.
#[test]
fn the_box_body_keeps_the_default_foreground() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, "needs a doc");

    let buffer = frame_at(&app, 100, 24);
    let body_row = u16::try_from(row_holding(&buffer, "needs a doc")).expect("a small row");

    assert_eq!(
        style_of_text(&buffer, body_row, "needs a doc").fg,
        Some(Color::Reset),
        "the comment body is recoloured, which is what makes it hard to read:\n{}",
        buffer_text(&buffer)
    );
    // The box's own left side, not the sidebar's border, which is the first
    // `│` on the row.
    assert_eq!(
        style_of_text(&buffer, body_row, "│ needs a doc").fg,
        Some(Color::Blue),
        "the box's side is not blue:\n{}",
        buffer_text(&buffer)
    );
}

/// Writes `reply` under every entry of the export and folds it back into the
/// store, which is the two halves of the LLM loop: the agent appends to the
/// document, and the next rewrite of it moves what the agent wrote into
/// `comments.json`.
///
/// Through the document rather than by editing the store, because that is the
/// only way a reply is ever created — there is no key for one — and a fixture
/// that invented one would be testing a state the product cannot reach.
fn reply_through_the_document(workspace: &Fixture, reply: &str) {
    let replied = insert_reply(&workspace.markdown(), reply);
    fs::write(
        workspace.root().join(".review/REVIEW-FEEDBACK.md"),
        &replied,
    )
    .expect("write the replied-to markdown");
    let review = session::build(workspace.root(), None, None).expect("build the review");
    session::write_markdown(&review).expect("fold the reply back into the store");
}

/// A reply is drawn inside the comment's own box, dimmed: it is part of that
/// conversation, and a reviewer scanning a screen of boxes has to be able to
/// tell their own words from the agent's answer without reading either.
///
/// Asserted on the **style**, not only on the text — the `reply:` prefix has
/// been on screen since the row model shipped, and a test that greps for it
/// passes against a reply drawn exactly like the body above it. The body is
/// checked in the same frame as the control: if everything were dimmed, nothing
/// would be.
#[test]
fn a_reply_renders_dimmed_inside_the_same_box() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, "needs a doc");
    reply_through_the_document(&workspace, "added one");
    drop(app);

    // Reopened, because the store is where the folded reply landed.
    let app = workspace.app();
    assert_eq!(
        app.comments()[0].reply.as_deref(),
        Some("added one"),
        "the fixture never got a reply into the store, so this proves nothing"
    );

    let buffer = frame_at(&app, 100, 24);
    let text = buffer_text(&buffer);
    let reply_row = u16::try_from(row_holding(&buffer, "reply: added one")).expect("a small row");
    let body_row = u16::try_from(row_holding(&buffer, "needs a doc")).expect("a small row");

    assert!(
        rows_of(&buffer)[usize::from(reply_row)].contains('│'),
        "the reply is not inside a box:\n{text}"
    );
    assert_eq!(
        reply_row,
        body_row + 1,
        "the reply is not under the body it answers:\n{text}"
    );

    let reply_style = style_of_text(&buffer, reply_row, "reply: added one");
    assert!(
        reply_style.add_modifier.contains(Modifier::DIM),
        "the reply is drawn exactly like the comment it answers:\n{text}"
    );
    assert_eq!(
        reply_style.fg,
        Some(Color::Reset),
        "the reply was recoloured rather than dimmed, which costs it the \
         contrast the body has:\n{text}"
    );
    assert!(
        !style_of_text(&buffer, body_row, "needs a doc")
            .add_modifier
            .contains(Modifier::DIM),
        "the comment's own body is dimmed too, so nothing tells the two apart:\n{text}"
    );
}

/// Focus is shown with a `▸` on the focused pane's title and a bold border —
/// never with colour, because blue already means "comment".
#[test]
fn the_focused_pane_is_marked() {
    let workspace = Fixture::new();
    let mut app = workspace.app();

    let diff_focused = buffer_text(&frame_at(&app, 100, 24));
    app.on_key(KeyCode::Left).expect("focus files");
    let files_focused = buffer_text(&frame_at(&app, 100, 24));

    assert_ne!(
        diff_focused, files_focused,
        "focus is invisible on screen:\n{files_focused}"
    );
    assert!(
        files_focused.contains("▸ Files"),
        "the focused pane's title is not marked:\n{files_focused}"
    );
    assert!(
        !files_focused.contains("▸ a.rs"),
        "the unfocused diff is marked too:\n{files_focused}"
    );
    assert!(
        diff_focused.contains("▸ a.rs") && !diff_focused.contains("▸ Files"),
        "the mark did not move with the focus:\n{diff_focused}"
    );
}

/// The same, in the borders: exactly one pane is bold, and it is the one the
/// next keystroke lands in.
#[test]
fn only_the_focused_panes_border_is_bold() {
    let workspace = Fixture::new();
    let mut app = workspace.app();

    // The bar is along the bottom, so both panes start at row 0 and row 1 is a
    // border cell of each: the sidebar's left edge at column 0, and the diff's
    // at column 30 — 30% of 100, less the divider column the sidebar gives up.
    let bold = |app: &App| {
        let buffer = frame_at(app, 100, 24);
        (
            buffer[(0, 1)].modifier.contains(Modifier::BOLD),
            buffer[(30, 1)].modifier.contains(Modifier::BOLD),
        )
    };

    assert_eq!(bold(&app), (false, true), "the diff has focus on launch");
    app.on_key(KeyCode::Left).expect("focus files");
    assert_eq!(bold(&app), (true, false), "the mark did not move");
    app.on_key(KeyCode::Right).expect("focus the diff");
    assert_eq!(bold(&app), (false, true), "and did not come back");
}

/// The sidebar's selection is `REVERSED` only while the sidebar has the focus;
/// unfocused it drops to a dim mark, so there is exactly one place on screen
/// the next keystroke will land.
#[test]
fn the_unfocused_sidebar_dims_its_selection() {
    let workspace = Fixture::new();
    let mut app = workspace.app();

    // Scanned inside the sidebar's own columns: the diff pane's title names
    // `a.rs` too, and it is the *list row* whose highlight this is about.
    let reversed = |app: &App| {
        let buffer = frame_at(app, 100, 24);
        (0..24).any(|y| {
            let row: String = (0..30).map(|x| buffer[(x, y)].symbol()).collect();
            row.contains("a.rs")
                && (0..30).any(|x| buffer[(x, y)].modifier.contains(Modifier::REVERSED))
        })
    };

    assert!(!reversed(&app), "the unfocused file list is still reversed");
    app.on_key(KeyCode::Left).expect("focus files");
    assert!(reversed(&app), "the focused file list lost its highlight");
}

/// Inside a stack the selected box is brighter and bold, so a reviewer can see
/// which of several comments `d` and `s` are aimed at.
#[test]
fn the_selected_box_in_the_stack_is_brighter_and_bold() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, "first finding");
    write_comment(&mut app, "second finding");

    let browsing = frame_at(&app, 100, 24);
    let first_corner = find_char_in(&browsing, box_area(), '╭').expect("a box top");
    assert_eq!(
        browsing[first_corner].style().fg,
        Some(Color::Blue),
        "an unselected box is not plain blue"
    );

    app.on_key(KeyCode::Enter).expect("enter the stack");
    let selected = frame_at(&app, 100, 24);
    let corner = &selected[first_corner];
    assert_eq!(
        corner.style().fg,
        Some(Color::LightBlue),
        "the selected box is not brighter:\n{}",
        buffer_text(&selected)
    );
    assert!(
        corner.modifier.contains(Modifier::BOLD),
        "the selected box is not bold:\n{}",
        buffer_text(&selected)
    );

    // ...and the box the cursor is *not* on stays plain, so "selected" means
    // one box rather than the whole stack.
    app.on_key(KeyCode::Char('j')).expect("select the second");
    let moved = frame_at(&app, 100, 24);
    assert_eq!(
        moved[first_corner].style().fg,
        Some(Color::Blue),
        "the highlight did not move off the first box:\n{}",
        buffer_text(&moved)
    );
}

/// A folded comment is one row: no borders, and the body still readable enough
/// to find it again.
#[test]
fn a_collapsed_box_is_a_single_row() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, "needs a doc");
    let id = app.comments()[0].id.clone();

    app.on_key(KeyCode::Char('s')).expect("fold it away");

    let buffer = frame_at(&app, 100, 24);
    let text = buffer_text(&buffer);
    let inside = text_in(&buffer, box_area());
    assert!(
        !inside.contains('╭') && !inside.contains('╰'),
        "a folded comment still draws a box:\n{text}"
    );
    let rows = rows_of(&buffer);
    let anchored = row_holding(&buffer, "fn a() {");
    assert!(
        rows[anchored + 1].contains(&id) && rows[anchored + 1].contains("needs a doc"),
        "the folded row does not say what it is folding:\n{text}"
    );
}
