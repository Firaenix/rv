//! Focus colours the border.

use crossterm::event::KeyCode;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use rv::gradient;
use rv::layout::Split;

use crate::support::*;

/// The focused pane's border is the magenta accent and the other pane's is not.
#[test]
fn the_focused_pane_border_is_the_accent_and_the_other_is_not() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    let rects = areas(100, 24, Split::default());
    let corner = |frame: &Buffer, area: Rect| frame[(area.x, area.y)].style().fg;
    let accent = Some(colour(gradient::FOCUS));

    let frame = frame_at(&app, 100, 24);
    assert_eq!(
        corner(&frame, rects.diff),
        accent,
        "the diff has focus at launch and its border is not the accent"
    );
    assert_ne!(
        corner(&frame, rects.sidebar),
        accent,
        "the sidebar does not have focus and its border is the accent anyway"
    );

    app.on_key(KeyCode::Left).expect("focus the sidebar");
    let frame = frame_at(&app, 100, 24);
    assert_eq!(
        corner(&frame, rects.sidebar),
        accent,
        "the accent did not move"
    );
    assert_ne!(corner(&frame, rects.diff), accent, "...and did not let go");
}

/// The accent is none of the colours that already mean something.
#[test]
fn the_accent_is_none_of_the_colours_that_already_mean_something() {
    for taken in [
        gradient::ADDED,
        gradient::REMOVED,
        gradient::COMMENT,
        gradient::ALERT,
    ] {
        assert_ne!(
            gradient::FOCUS,
            taken,
            "the focus accent must be unambiguous"
        );
    }
}

/// The `▸` on the focused pane's title survives the colour.
///
/// Redundant on purpose: a sixteen-colour terminal, or a reader who does not
/// separate magenta from red, still needs to know where the keys go.
#[test]
fn the_title_marker_survives_the_colour() {
    let workspace = Fixture::new();
    let app = workspace.app();
    assert!(buffer_text(&frame_at(&app, 100, 24)).contains('▸'));
}

/// The panes' borders are rounded.
#[test]
fn the_panes_borders_are_rounded() {
    let workspace = Fixture::new();
    let app = workspace.app();
    let frame = frame_at(&app, 100, 24);
    let rects = areas(100, 24, Split::default());

    for area in [rects.sidebar, rects.diff] {
        assert_eq!(
            frame[(area.x, area.y)].symbol(),
            "╭",
            "a pane's top-left corner is square"
        );
        assert_eq!(
            frame[(area.x, area.bottom() - 1)].symbol(),
            "╰",
            "a pane's bottom-left corner is square"
        );
    }
}
