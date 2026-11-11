//! `g d`/`g r` with a language server: the grammar answers at once, the
//! server's answer replaces it only while the reviewer is still on the request.
#![cfg(unix)]

use crossterm::event::KeyCode;
use rv::app::App;
use rv::app::JumpList;
use rv::app::Mode;

use crate::lsp_fake::*;
use crate::support::*;

/// On `user.rs:2`, column cursor on the call to `alpha`.
fn on_alpha(app: &mut App) {
    show_file(app, "user.rs");
    to_line(app, 2);
    to_word(app, "alpha");
}

fn shown_path(app: &App) -> &str {
    &app.selected_file().expect("a file").path
}

#[test]
fn the_grammar_jump_is_immediate_and_the_server_answer_replaces_it() {
    let fixture = goto_fixture();
    let (mut app, _server) = lsp_app(&fixture, "answer");
    on_alpha(&mut app);

    chord(&mut app, 'g', 'd');
    assert_eq!(
        (shown_path(&app), app.cursor_line_number()),
        ("model.rs", Some(5)),
        "the grammar's `fn alpha` is where the cursor lands first"
    );
    assert!(app.lsp_pending(), "and the server was asked as well");

    settle_lsp(&mut app);
    assert_eq!(
        (shown_path(&app), app.cursor_line_number()),
        ("model.rs", Some(1)),
        "the server's answer moved the reviewer who had not moved"
    );
    assert_eq!(app.word_under_cursor().as_deref(), Some("Gamma"));
    assert!(app.status().contains("rust-analyzer"), "{}", app.status());
    assert_eq!(
        app.navigation_label().as_deref(),
        Some("nav: rust-analyzer")
    );

    chord(&mut app, 'g', 'b');
    assert_eq!(
        (shown_path(&app), app.cursor_line_number()),
        ("user.rs", Some(2)),
        "one g b undoes the whole g d, not just the server's correction"
    );
}

#[test]
fn a_reviewer_who_moved_on_is_not_moved_again() {
    let fixture = goto_fixture();
    let (mut app, _server) = lsp_app(&fixture, "answer");
    on_alpha(&mut app);

    chord(&mut app, 'g', 'd');
    app.on_key(KeyCode::Down).expect("move away");
    let moved_to = app.cursor_line_number();
    settle_lsp(&mut app);

    assert_eq!(app.cursor_line_number(), moved_to);
    assert_eq!(shown_path(&app), "model.rs");
    assert!(
        app.status().contains("model.rs:1"),
        "the answer is still reported: {}",
        app.status()
    );
}

#[test]
fn a_superseded_answer_is_discarded() {
    let fixture = goto_fixture();
    let (mut app, _server) = lsp_app(&fixture, "answer");
    on_alpha(&mut app);

    chord(&mut app, 'g', 'd');
    assert_eq!(app.cursor_line_number(), Some(5));
    // Asked from the very spot the grammar landed on, so only the newer
    // request standing in its way keeps the definition answer out.
    chord(&mut app, 'g', 'i');
    settle_lsp(&mut app);

    assert_eq!(
        app.cursor_line_number(),
        Some(5),
        "the first g d's answer never moved the cursor"
    );
    let card = app.hover().expect("the card is still open");
    assert_eq!(card.source, rv::app::JumpSource::LanguageServer);
}

#[test]
fn the_server_references_replace_the_grammar_list() {
    let fixture = goto_fixture();
    let (mut app, _server) = lsp_app(&fixture, "answer");
    on_alpha(&mut app);

    chord(&mut app, 'g', 'r');
    assert_eq!(app.mode(), Mode::References, "the grammar's list is open");
    assert_eq!(app.jump_list_note(), None);
    settle_lsp(&mut app);

    assert_eq!(app.mode(), Mode::References);
    assert_eq!(app.jump_list(), JumpList::References);
    assert_eq!(app.jump_list_note().as_deref(), Some("rust-analyzer"));
    let rows: Vec<(String, u32)> = app
        .references()
        .iter()
        .map(|reference| (app.reference_path(reference).to_owned(), reference.line))
        .collect();
    assert_eq!(
        rows,
        [
            ("model.rs".to_owned(), 1),
            ("user.rs".to_owned(), 2),
            ("/nonexistent/elsewhere.rs".to_owned(), 5),
        ],
        "the server's references replaced the grammar's"
    );
    let text = buffer_text(&frame_at(&app, 120, 30));
    assert!(
        text.contains("outside /nonexistent/elsewhere.rs:5"),
        "{text}"
    );
    assert!(text.contains("· rust-analyzer"), "{text}");
}

#[test]
fn a_location_outside_the_review_is_listed_but_not_opened() {
    let fixture = goto_fixture();
    let (mut app, _server) = lsp_app(&fixture, "answer");
    on_alpha(&mut app);
    chord(&mut app, 'g', 'r');
    settle_lsp(&mut app);
    assert_eq!(app.references().len(), 3);

    app.on_key(KeyCode::End).expect("to the outside row");
    app.on_key(KeyCode::Enter).expect("try to open it");

    assert_eq!(app.mode(), Mode::References, "the list stays open");
    assert!(
        app.status().contains("outside this review"),
        "{}",
        app.status()
    );
    assert_eq!(shown_path(&app), "user.rs");
}
