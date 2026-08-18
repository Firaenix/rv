//! Syntax colours inside the green and the red.

use rv::app::BINDINGS;
use rv::layout::Split;

use crate::support::*;

/// A file rv has no grammar for renders plain, and the pane says why rather
/// than leaving the reviewer to guess whether the colour is broken.
#[test]
fn a_file_with_no_grammar_renders_plain_and_says_so() {
    let workspace = Fixture::plain();
    let app = workspace.app();
    let frame = frame_at(&app, 100, 24);
    let area = areas(100, 24, Split::default()).diff;
    let text = buffer_text(&frame);

    assert!(
        text.contains("no highlighting"),
        "the title does not say why the code is plain:\n{text}"
    );
    let added = row_of_sigil(&frame, area, '+');
    assert_eq!(
        distinct_foregrounds(&frame, area, added).len(),
        2,
        "a file with no grammar was coloured anyway: {:?}\n{text}",
        distinct_foregrounds(&frame, area, added)
    );
}

/// ...and a file rv *does* have a grammar for is not labelled as plain.
#[test]
fn a_highlighted_file_is_not_labelled_as_plain() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    app.finish_painting();
    let text = buffer_text(&frame_at(&app, 100, 24));
    assert!(
        !text.contains("no highlighting"),
        "a Rust file is labelled as having no grammar:\n{text}"
    );
}

/// The wash is a band across the whole pane rather than a tint that stops
/// wherever the line's text happens to end: a ragged right edge reads as a
/// rendering fault rather than as a marked line.
#[test]
fn the_wash_reaches_the_edge_of_the_pane_and_no_further() {
    let workspace = Fixture::new();
    let app = workspace.app();
    let frame = frame_at(&app, 100, 24);
    let area = areas(100, 24, Split::default()).diff;
    let added = row_of_sigil(&frame, area, '+');
    let wash = diff_bg(&frame, area, added).expect("the line is tinted");

    assert_eq!(
        frame[(area.right() - 2, added)].style().bg,
        Some(wash),
        "the tint stops short of the pane's last column:\n{}",
        buffer_text(&frame)
    );
    assert_ne!(
        frame[(area.right() - 1, added)].style().bg,
        Some(wash),
        "the tint spilled onto the pane's own border:\n{}",
        buffer_text(&frame)
    );
}

/// Every key the README's **Browsing** table documents is a row of
/// [`BINDINGS`], which is what chains the page to the code rather than to
/// [`BROWSE_KEYS`]'s hand-kept list of spellings.
///
/// `Ctrl+C` is the exception, and the only one: [`rv::app::App::on_key_event`]
/// answers it before the mode is dispatched at all — it is the universal abort,
/// and it works from behind the `?` popup too, because an abort that first asks
/// you to press `Esc` is not an abort.
#[test]
fn every_documented_browse_key_is_a_row_of_the_binding_table() {
    for key in BROWSE_KEYS {
        if *key == "`Ctrl+C`" {
            continue;
        }
        let spelled = key.replace('`', "");
        assert!(
            BINDINGS.iter().any(|binding| binding.keys == spelled),
            "the README documents {key}, which is not a row of BINDINGS: {:?}",
            BINDINGS.iter().map(|b| b.keys).collect::<Vec<_>>()
        );
    }
}

/// `?` has to be findable from inside the binary, not only from the README.
///
/// As shipped, the popup was reachable only by guessing the key: the status bar
/// is the one surface every reviewer sees and it said nothing about `?`, and
/// the README's table said nothing about it either. This pins the pointer to
/// the manual in the bar itself, and `the_readme_mockup_draws_the_status_bar…`
/// in `app_cases.rs` chains the page's picture to the same constant, so the two
/// cannot drift apart again.
#[test]
fn the_status_bar_says_where_the_keymap_is() {
    let workspace = Fixture::new();
    let app = workspace.app();
    assert!(
        app.status().contains("? help"),
        "the bar a reviewer opens on does not mention the keymap: {:?}",
        app.status()
    );
    assert!(
        last_row(&frame_at(&app, 100, 24)).contains("? help"),
        "...and it is not on screen either"
    );
}
