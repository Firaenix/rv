//! When the language server cannot be trusted or is not there, navigation is
//! the grammar's, unchanged, and says so.
#![cfg(unix)]

use rv::app::App;
use rv::app::DiffEngine;
use rv::session;

use crate::lsp_fake::*;
use crate::support::*;

fn on_alpha(app: &mut App) {
    show_file(app, "user.rs");
    to_line(app, 2);
    to_word(app, "alpha");
}

fn grammar_jump_works(app: &mut App) {
    on_alpha(app);
    chord(app, 'g', 'd');
    settle_lsp(app);
    assert_eq!(
        (
            app.selected_file().map(|file| file.path.as_str()),
            app.cursor_line_number()
        ),
        (Some("model.rs"), Some(5)),
        "the grammar's answer stands"
    );
}

#[test]
fn a_revision_that_is_not_the_working_copy_is_grammar_only() {
    let fixture = goto_fixture();
    let (_server, script) = fake_server("answer");
    let review = session::build(fixture.root(), None, Some("@-")).expect("build the review");
    let settings = lsp_settings(&script.display().to_string());
    let config = rv::config::parse("").expect("no keybindings");
    let mut app = App::open_with_config(review, DiffEngine::Structural, &config, &settings)
        .expect("open the reviewer");
    app.finish_loading();
    app.finish_merging();

    assert_eq!(app.navigation_label().as_deref(), Some("nav: grammar"));
    on_alpha(&mut app);
    chord(&mut app, 'g', 'd');
    assert!(
        !app.lsp_pending(),
        "nothing is asked about a revision not on disk"
    );
    assert_eq!(app.cursor_line_number(), Some(5));

    to_word(&mut app, "Gamma");
    chord(&mut app, 'g', 'i');
    let body = app.hover().expect("a card").body();
    assert!(
        body.iter()
            .any(|line| line == "grammar only — this revision is not on disk"),
        "{body:?}"
    );
    app.on_key(crossterm::event::KeyCode::Esc).expect("close");
    chord(&mut app, 'g', 'r');
    assert_eq!(
        app.jump_list_note().as_deref(),
        Some("grammar only — this revision is not on disk")
    );
}

#[test]
fn an_absent_server_is_unavailable_and_the_grammar_still_answers() {
    let fixture = goto_fixture();
    let mut app = app_configured(
        &fixture,
        "",
        &lsp_settings("/nonexistent/bin/rust-analyzer"),
    );
    show_file(&mut app, "user.rs");
    wait_for_label(&mut app, "nav: unavailable");
    grammar_jump_works(&mut app);
}

#[test]
fn a_killed_server_leaves_the_grammar_path_intact() {
    let fixture = goto_fixture();
    let (mut app, _server) = lsp_app(&fixture, "die_on_definition");
    grammar_jump_works(&mut app);
    wait_for_label(&mut app, "nav: unavailable");
    grammar_jump_works(&mut app);
}

#[test]
fn lsp_off_shows_no_nav_segment() {
    let fixture = goto_fixture();
    let mut app = fixture.app();
    assert_eq!(app.navigation_label(), None);
    on_alpha(&mut app);
    chord(&mut app, 'g', 'd');
    assert!(!app.lsp_pending());
    assert!(!buffer_text(&frame_at(&app, 200, 30)).contains("nav:"));
}
