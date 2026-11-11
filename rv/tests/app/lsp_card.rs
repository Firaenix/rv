//! The hover card's server section, and the status bar's `nav:` segment.
#![cfg(unix)]

use rv::app::JumpSource;
use rv::statusbar::{Role, View, render, segments};

use crate::lsp_fake::*;
use crate::support::*;

#[test]
fn g_i_fills_the_card_with_the_server_hover_sanitized() {
    let fixture = goto_fixture();
    let (mut app, _server) = lsp_app(&fixture, "answer");
    show_file(&mut app, "user.rs");
    to_line(&mut app, 2);
    to_word(&mut app, "alpha");

    chord(&mut app, 'g', 'i');
    assert_eq!(app.hover().expect("a card").source, JumpSource::Grammar);
    settle_lsp(&mut app);

    let card = app.hover().expect("the card is still open").clone();
    assert_eq!(card.source, JumpSource::LanguageServer);
    assert_eq!(
        card.server_text,
        ["fn alpha() -> Gamma", "", "makes a Gamma"]
    );
    assert!(HOVER_DOC.contains('\u{1b}'));
    let text = buffer_text(&frame_at(&app, 120, 30));
    for wanted in ["fn alpha() -> Gamma", "makes a Gamma", "language server"] {
        assert!(text.contains(wanted), "the card lacks {wanted:?}:\n{text}");
    }
}

#[test]
fn indexing_progress_reaches_the_status_bar() {
    let fixture = goto_fixture();
    let (mut app, _server) = lsp_app(&fixture, "progress");
    show_file(&mut app, "user.rs");
    wait_for_label(&mut app, "nav: rust-analyzer indexing 42%");
    let text = buffer_text(&frame_at(&app, 200, 30));
    assert!(text.contains("nav: rust-analyzer indexing 42%"), "{text}");
}

fn full_view(navigation: &str) -> View<'static> {
    View {
        mode: "DIFF",
        file: Some("rv/src/app/symbols.rs"),
        line: Some(120),
        symbol: "in fn goto_definition".to_owned(),
        file_index: 3,
        file_count: 40,
        scope: "trunk()..@",
        change: "ytskpxpw add lsp navigation".to_owned(),
        open_comments: 2,
        status: "saved comment",
        navigation: navigation.to_owned(),
        ..View::default()
    }
}

#[test]
fn the_nav_segment_gives_way_before_what_matters_at_80_columns() {
    let bar = segments(&full_view("nav: rust-analyzer indexing 42%"));
    assert!(bar.iter().any(|segment| segment.role == Role::Navigation));
    let narrow = render(&bar, 80, true).to_string();
    let without = render(&segments(&full_view("")), 80, true).to_string();
    assert_eq!(
        narrow, without,
        "at 80 columns the nav segment displaces nothing"
    );
    let wide = render(&bar, 220, true).to_string();
    assert!(wide.contains("nav: rust-analyzer indexing 42%"), "{wide}");
    assert!(
        !segments(&full_view(""))
            .iter()
            .any(|segment| segment.role == Role::Navigation),
        "an empty label means no segment"
    );
}
