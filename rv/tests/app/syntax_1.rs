//! Syntax colours inside the green and the red.

use crossterm::event::KeyCode;
use ratatui::style::Color;
use ratatui::style::Modifier;
use rstest::rstest;
use rv::gradient;
use rv::layout::Split;
use rv::ui;
use rv_core::diff::LineKind;
use rv_core::highlight::Capture;

use crate::support::*;

/// The defect a user reported: comments render too white.
///
/// `Color::Gray` is ANSI index 7 — the terminal's *white* — which is what this
/// used to send, and index 8 (bright black) is the tone every scheme defines
/// for exactly this against its own background. The distinction is not
/// cosmetic: index 7 on a light scheme is near-invisible and on a dark one is
/// as loud as the code it annotates.
#[test]
fn a_comment_uses_the_terminals_muted_tone() {
    let workspace = Fixture::commented();
    let app = workspace.app();
    let frame = frame_at(&app, 100, 24);
    let area = areas(100, 24, Split::default()).diff;
    assert_eq!(
        colour_of_first_comment(&frame, area),
        Some(Color::DarkGray),
        "comments are index 8, the tone every scheme defines for exactly this:\n{}",
        buffer_text(&frame)
    );
}

/// Every capture maps to one of the 16 indexed ANSI colours, or to nothing at
/// all.
///
/// The indexed colours are a pass-through to the reviewer's own scheme: emit
/// index 4 and the terminal substitutes whatever *its* theme calls blue. That
/// is the whole of rv's theming design, which is why there is no theme option
/// — see `ui`'s module docs for which layer owns which colour.
///
/// Punctuation, variables and anything unrecognised are deliberately
/// **unstyled**: most of a line is one or the other, and a highlighter that
/// colours the majority of the text has stopped highlighting anything.
#[rstest]
#[case::keyword(Capture::Keyword, Color::Magenta)]
#[case::function(Capture::Function, Color::Blue)]
#[case::a_type(Capture::Type, Color::Cyan)]
#[case::string(Capture::String, Color::Green)]
#[case::number(Capture::Number, Color::Yellow)]
#[case::constant(Capture::Constant, Color::Yellow)]
#[case::comment(Capture::Comment, Color::DarkGray)]
#[case::punctuation(Capture::Punctuation, Color::Reset)]
#[case::variable(Capture::Variable, Color::Reset)]
#[case::other(Capture::Other, Color::Reset)]
fn every_capture_maps_to_an_indexed_colour(#[case] capture: Capture, #[case] expected: Color) {
    assert_eq!(ui::capture_colour(capture), expected);
}

/// ...and nothing the diff pane writes a glyph in dictates an exact colour.
///
/// An `Rgb` foreground overrides the reviewer's scheme instead of deferring to
/// it, which is how a tool ends up needing a theme option it should never have
/// needed. The boundary is asserted rather than remembered: the sweep covers
/// the code, the gutter sigils and a comment box's borders — everything with a
/// glyph on it — and the frame deliberately has a comment box in it so the
/// chrome is swept too.
///
/// The **background** is the bounded exception, and it is asserted here as one
/// so that this test cannot be read as forbidding it: the wash that marks a
/// line added or removed is a truecolour mix (see
/// `the_wash_is_the_palettes_own_green_and_red`) and cannot exist in 16
/// colours. Foreground and background never contend for the same channel, so a
/// syntax colour and a wash cannot collide.
#[test]
fn code_is_painted_only_in_indexed_colours() {
    let workspace = Fixture::commented();
    let mut app = workspace.app();
    write_comment(&mut app, "a finding, so the box is swept too");
    let frame = frame_at(&app, 100, 24);
    let area = areas(100, 24, Split::default()).diff;

    let cells = diff_pane_cells(&frame, area);
    assert!(cells.len() > 20, "the pane drew almost nothing to judge");
    for (column, row) in cells {
        let fg = frame[(column, row)].style().fg;
        assert!(
            !matches!(fg, Some(Color::Rgb(..))),
            "cell ({column},{row}) dictates an exact colour instead of using \
             the terminal's: {fg:?}\n{}",
            buffer_text(&frame)
        );
    }

    assert!(
        matches!(
            ui::line_background(LineKind::Added, false),
            Some(Color::Rgb(..))
        ),
        "the wash is no longer truecolour, so this test's exception has gone \
         stale and the rule above is broader than it says"
    );
}

#[test]
fn an_added_line_has_a_green_wash_and_coloured_code() {
    let workspace = Fixture::new();
    let app = workspace.app();
    let frame = frame_at(&app, 100, 24);
    let area = areas(100, 24, Split::default()).diff;
    let added = row_of_sigil(&frame, area, '+');

    let background = diff_bg(&frame, area, added);
    assert!(
        background.is_some(),
        "an added line carries no background tint:\n{}",
        buffer_text(&frame)
    );
    let foregrounds = distinct_foregrounds(&frame, area, added);
    assert!(
        foregrounds.len() > 1,
        "the code is one flat colour rather than syntax coloured: {foregrounds:?}\n{}",
        buffer_text(&frame)
    );
}

/// The wash is drawn from `gradient::ADDED` and `gradient::REMOVED` rather than
/// from a second green and a second red beside them — so the diff and the
/// sidebar's change bar cannot drift into two palettes.
#[test]
fn the_wash_is_the_palettes_own_green_and_red() {
    for (kind, hue) in [
        (LineKind::Added, gradient::ADDED),
        (LineKind::Removed, gradient::REMOVED),
    ] {
        for selected in [false, true] {
            let colour = ui::line_background(kind, selected)
                .unwrap_or_else(|| panic!("{kind:?} selected={selected} has no tint"));
            assert!(
                on_the_ramp(colour, hue),
                "{kind:?} selected={selected} is tinted {colour:?}, which is not \
                 {hue:?} taken toward the ink"
            );
        }
    }
    assert_eq!(
        ui::line_background(LineKind::Context, false),
        None,
        "a context line is tinted, so the tint no longer means added or removed"
    );
}

/// Reversing swaps the foreground and the background, which on a tinted line
/// turns the syntax colours into the wash and the wash into the text —
/// legible in neither direction. The selection is a *brighter* tint instead.
#[test]
fn the_selected_line_is_brighter_rather_than_reversed() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    let area = areas(100, 24, Split::default()).diff;

    let frame = frame_at(&app, 100, 24);
    let selected = area.y + 1;
    assert_eq!(app.line_index(), 0, "the reviewer opens on the first line");
    for x in (area.x + 1)..area.right() - 1 {
        assert!(
            !frame[(x, selected)].modifier.contains(Modifier::REVERSED),
            "the selected line is drawn with reversed video:\n{}",
            buffer_text(&frame)
        );
    }

    let bright = diff_bg(&frame, area, selected).expect("the selection is tinted");
    let neighbour = diff_bg(&frame, area, selected + 1).expect("its neighbour is tinted");
    assert_ne!(
        bright,
        neighbour,
        "the selected line is drawn exactly like the line under it:\n{}",
        buffer_text(&frame)
    );

    // ...and the brightness moves with the cursor rather than being a property
    // of the first row.
    app.on_key(KeyCode::Char('j')).expect("j");
    let moved = frame_at(&app, 100, 24);
    assert_eq!(
        diff_bg(&moved, area, selected + 1),
        Some(bright),
        "the highlight did not move onto the next line:\n{}",
        buffer_text(&moved)
    );
    assert_eq!(
        diff_bg(&moved, area, selected),
        Some(neighbour),
        "the highlight did not leave the line it was on:\n{}",
        buffer_text(&moved)
    );
}

/// A removed line takes its colours from the **base** blob.
///
/// The fixture is a rewrite that does not move: `rewrite.rs` line 2 on both
/// sides, a string literal on the base and a number on the head, at the same
/// columns. A lookup that ignored the side would paint the removed line's
/// string with the number's colour — and a renamed file could not catch that,
/// because a rename already encodes the side in the path.
#[test]
fn a_removed_line_takes_its_colours_from_the_base_blob() {
    let workspace = Fixture::rewritten();
    let app = workspace.app_from("@--");
    let area = areas(100, 24, Split::default()).diff;
    let frame = frame_at(&app, 100, 24);

    let removed = row_of_sigil(&frame, area, '-');
    let added = row_of_sigil(&frame, area, '+');
    let text_of = |y: u16| {
        diff_rows(&frame, area)
            .into_iter()
            .find(|(row, _)| *row == y)
            .map(|(_, text)| text)
            .expect("the row is in the pane")
    };
    assert!(
        text_of(removed).contains(REWRITE_BASE_LINE),
        "the removed half does not show the base blob's text:\n{}",
        buffer_text(&frame)
    );
    assert!(
        text_of(added).contains(REWRITE_HEAD_LINE),
        "the added half does not show the head blob's text:\n{}",
        buffer_text(&frame)
    );

    // Column 7 of the pane's inner area is where a line's own text starts: a
    // five-wide number, a space and the sigil.
    let literal = area.x + 1 + 7 + u16::try_from(REWRITE_LITERAL_COLUMN).expect("a small column");
    assert_eq!(
        frame[(literal, removed)].symbol(),
        "\"",
        "the base side's literal is not where this test looks for it:\n{}",
        buffer_text(&frame)
    );
    assert_eq!(
        frame[(literal, added)].symbol(),
        "1",
        "the head side's literal is not where this test looks for it:\n{}",
        buffer_text(&frame)
    );

    let base_colour = frame[(literal, removed)].style().fg;
    let head_colour = frame[(literal, added)].style().fg;
    assert_ne!(
        base_colour,
        head_colour,
        "the two sides colour that column the same way, so this proves nothing:\n{}",
        buffer_text(&frame)
    );
    assert_eq!(
        base_colour,
        Some(ui::capture_colour(rv_core::highlight::Capture::String)),
        "the removed line's literal is not coloured as the string the base blob \
         has there — its spans came from the head side:\n{}",
        buffer_text(&frame)
    );
    assert_eq!(
        head_colour,
        Some(ui::capture_colour(rv_core::highlight::Capture::Constant)),
        "the added line's literal is not coloured as the number the head blob \
         has there:\n{}",
        buffer_text(&frame)
    );
}
