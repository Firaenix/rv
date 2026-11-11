//! `g i`: the hover card — what it says, where its keys come from, and what
//! closes it.

use crossterm::event::KeyCode;
use rv::app::App;
use rv::app::JumpSource;
use rv::rows::plans_built;
use rv_core::symbols::SymbolKind;

use crate::support::*;

/// The reviewer on `model.rs:5`, column cursor on `Gamma`.
fn on_gamma(app: &mut App) {
    show_file(app, "model.rs");
    to_line(app, 5);
    to_word(app, "Gamma");
}

#[test]
fn g_i_describes_the_symbol_under_the_cursor() {
    let workspace = goto_fixture();
    let mut app = workspace.app();
    on_gamma(&mut app);

    chord(&mut app, 'g', 'i');

    let card = app.hover().expect("g i opened a card").clone();
    assert_eq!(card.name, "Gamma");
    assert_eq!(card.kind, Some(SymbolKind::Struct));
    assert_eq!(card.source, JumpSource::Grammar);
    let declaration = card.declaration.expect("Gamma is declared in the review");
    assert_eq!(
        (declaration.path.as_str(), declaration.line),
        ("model.rs", 1)
    );
    assert_eq!(declaration.text, ["struct Gamma {", "    value: u32,"]);
    assert!(card.references >= 1, "the use on line 5 was not counted");

    let text = buffer_text(&frame_at(&app, 120, 30));
    for wanted in ["struct Gamma", "model.rs:1", "value: u32", "tree-sitter"] {
        assert!(
            text.contains(wanted),
            "the card does not show {wanted:?}:\n{text}"
        );
    }
}

#[test]
fn the_hints_are_spelled_from_the_default_keymap() {
    let workspace = goto_fixture();
    let mut app = workspace.app();
    on_gamma(&mut app);
    chord(&mut app, 'g', 'i');
    assert_eq!(
        app.hover().expect("a card").hints,
        "g d definition · g r references · Esc close"
    );
}

/// A reviewer who moved `g d` and renamed the goto leader sees their own keys.
#[test]
fn the_hints_follow_a_remapped_keymap() {
    let workspace = goto_fixture();
    let mut app = app_configured(
        &workspace,
        "[leaders]\ngoto = \"G\"\n[goto]\ndiff_definition = \"e\"\ndiff_references = \"\"\n",
        &rv::config::Settings::default(),
    );
    on_gamma(&mut app);
    chord(&mut app, 'G', 'i');

    let hints = &app.hover().expect("a card").hints;
    assert_eq!(hints, "G e definition · Esc close");
    let text = buffer_text(&frame_at(&app, 120, 30));
    assert!(text.contains("G e definition"), "{text}");
}

#[test]
fn escape_closes_the_card_and_leaves_the_cursor_where_it_was() {
    let workspace = goto_fixture();
    let mut app = workspace.app();
    on_gamma(&mut app);
    chord(&mut app, 'g', 'i');
    let row = app.cursor_row();

    app.on_key(KeyCode::Esc).expect("Esc");

    assert!(app.hover().is_none(), "Esc left the card open");
    assert_eq!(app.cursor_row(), row);
}

/// Any command is a new question, so it closes the card about the old one —
/// and still runs.
#[test]
fn a_key_that_runs_a_command_closes_the_card_and_runs() {
    let workspace = goto_fixture();
    let mut app = workspace.app();
    on_gamma(&mut app);
    chord(&mut app, 'g', 'i');

    app.on_key(KeyCode::Down).expect("down");

    assert!(app.hover().is_none());
    assert_eq!(app.cursor_line_number(), Some(6));
}

/// `g d` on a name the review never defines has nowhere to go; the card says
/// what the grammar does know rather than leaving only a status line.
#[test]
fn g_d_that_finds_nothing_opens_the_card() {
    let workspace = goto_fixture();
    let mut app = workspace.app();
    show_file(&mut app, "model.rs");
    to_line(&mut app, 2);
    to_word(&mut app, "u32");

    chord(&mut app, 'g', 'd');

    let card = app.hover().expect("the miss raised a card");
    assert_eq!(card.name, "u32");
    assert!(card.declaration.is_none());
    let text = buffer_text(&frame_at(&app, 120, 30));
    assert!(text.contains("no definition of u32"), "{text}");
}

#[test]
fn the_card_sits_over_the_diff_pane_and_takes_its_clicks() {
    let workspace = goto_fixture();
    let mut app = workspace.app();
    on_gamma(&mut app);
    chord(&mut app, 'g', 'i');
    let _ = frame_at(&app, 120, 30);
    let layout = app.painted_layout();
    let card = layout.hover.expect("the card was laid out");
    assert!(card.height <= 12, "the card is {} rows", card.height);
    assert!(card.x >= layout.diff.x && card.right() <= layout.diff.right());
    assert!(card.y > layout.diff.y && card.bottom() < layout.diff.bottom());
    assert_eq!(
        rv::layout::hit(&layout, card.x + 1, card.y + 1),
        Some(rv::layout::Target::Hover)
    );

    let row = app.cursor_row();
    app.on_mouse(click(card.x + 1, card.y + 1))
        .expect("click the card");
    assert!(app.hover().is_none(), "a click on the card left it open");
    assert_eq!(app.cursor_row(), row, "the click reached the row beneath");
}

/// The card is computed once, when asked for: drawing it costs no plan.
#[test]
fn an_open_card_costs_a_frame_no_extra_plans() {
    let workspace = goto_fixture();
    let mut app = workspace.app();
    on_gamma(&mut app);
    // Warm, as `g i` leaves it: the bar names the enclosing symbol once it is.
    let _ = app.index();
    let _ = frame_at(&app, 120, 30);
    let before = plans_built();
    let _ = frame_at(&app, 120, 30);
    let without = plans_built() - before;

    chord(&mut app, 'g', 'i');
    let before = plans_built();
    let _ = frame_at(&app, 120, 30);
    let with = plans_built() - before;

    assert_eq!(with, without, "the card rebuilt the plan to draw");
}
