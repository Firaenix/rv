//! The palette and the change gradient.
//!
//! The gradient is a *diverging* scale: green on the left, red on the right,
//! meeting at a tight light seam. Everything here is terminal-free arithmetic,
//! so it can be pinned exactly rather than eyeballed through a pty.

use std::collections::HashSet;

use proptest::prelude::*;
use rstest::rstest;
use rv::gradient::{
    ADDED, ALERT, COMMENT, FOCUS, INK_DARK, INK_LIGHT, REMOVED, Rgb, Stat, column_colour,
    oklab_mix, pivot, readable_on,
};

/// Rec. 709 luma over the encoded channels — the same rough brightness the eye
/// reads off a terminal cell.
fn luma(c: Rgb) -> f32 {
    0.2126 * f32::from(c.0) + 0.7152 * f32::from(c.1) + 0.0722 * f32::from(c.2)
}

/// WCAG relative luminance, used only to check that text stays readable.
fn relative_luminance(c: Rgb) -> f32 {
    fn channel(u: u8) -> f32 {
        let u = f32::from(u) / 255.0;
        if u <= 0.04045 {
            u / 12.92
        } else {
            ((u + 0.055) / 1.055).powf(2.4)
        }
    }
    0.2126 * channel(c.0) + 0.7152 * channel(c.1) + 0.0722 * channel(c.2)
}

fn contrast(a: Rgb, b: Rgb) -> f32 {
    let (x, y) = (relative_luminance(a), relative_luminance(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

// ---------------------------------------------------------------------------
// The two hands of the gradient
// ---------------------------------------------------------------------------

#[test]
fn a_pure_addition_is_green_all_the_way_across() {
    for column in 0..40 {
        assert_eq!(
            column_colour(1.0, column, 40),
            ADDED,
            "column {column} is green"
        );
    }
}

#[test]
fn a_pure_deletion_is_red_all_the_way_across() {
    for column in 0..40 {
        assert_eq!(column_colour(0.0, column, 40), REMOVED);
    }
}

#[test]
fn an_even_split_changes_hand_at_the_middle() {
    assert_eq!(
        column_colour(0.5, 0, 40),
        ADDED,
        "the left end is fully green"
    );
    assert_eq!(
        column_colour(0.5, 39, 40),
        REMOVED,
        "the right end is fully red"
    );
}

#[test]
fn the_boundary_is_blended_rather_than_a_hard_edge() {
    let middle: Vec<Rgb> = (17..23)
        .map(|column| column_colour(0.5, column, 40))
        .collect();
    let distinct: HashSet<_> = middle.iter().map(|c| (c.0, c.1, c.2)).collect();
    assert!(distinct.len() > 2, "the boundary interpolates: {middle:?}");
}

#[test]
fn the_seam_is_the_brightest_part_of_the_row() {
    // The whole point of pivoting through a light neutral: green and red sit at
    // opposite ends of Oklab's `a` axis, so blending them directly crosses a
    // dull mid-grey exactly where the eye is trying to read the boundary. The
    // seam must be brighter than both ends, not darker.
    let seam = luma(column_colour(0.5, 20, 40));
    assert!(seam > luma(ADDED), "the seam is lighter than the green end");
    assert!(seam > luma(REMOVED), "and lighter than the red end");
}

#[test]
fn no_cell_is_ever_a_mixture_of_the_two_hues() {
    // Each half desaturates toward the pivot and back, so a cell is green-ish or
    // red-ish or neutral — never olive, never brown.
    for column in 0..40 {
        let Rgb(r, g, _) = column_colour(0.5, column, 40);
        let muddy = r > 90 && g > 90 && r.abs_diff(g) < 25 && (u16::from(r) + u16::from(g)) < 380;
        assert!(
            !muddy,
            "column {column} is mud: {:?}",
            column_colour(0.5, column, 40)
        );
    }
}

#[test]
fn the_seam_is_tight_enough_to_still_read_as_a_proportion() {
    // A wide blend destroys the thing the bar is drawing: you can no longer see
    // where two thirds ends and one third begins.
    let flat_green = (0..40)
        .filter(|c| column_colour(0.66, *c, 40) == ADDED)
        .count();
    let flat_red = (0..40)
        .filter(|c| column_colour(0.66, *c, 40) == REMOVED)
        .count();
    assert!(
        flat_green + flat_red >= 34,
        "at most a few columns are in the seam"
    );
    assert!(
        flat_green > flat_red,
        "and two thirds still reads as two thirds"
    );
}

#[test]
fn an_even_split_really_is_even() {
    // Cells are half-open ranges, not points: the boundary at `ratio * width`
    // falls between two column centres, and forgetting the half-cell offset
    // hands one extra column to the green side without failing anything else.
    for width in [8u16, 20, 40, 41] {
        let green = (0..width)
            .filter(|c| column_colour(0.5, *c, width) == ADDED)
            .count();
        let red = (0..width)
            .filter(|c| column_colour(0.5, *c, width) == REMOVED)
            .count();
        assert_eq!(green, red, "width {width} splits {green} green / {red} red");
    }
}

#[test]
fn a_narrow_row_gets_a_narrower_seam() {
    // The seam is `min(4, width / 4)` columns: on a twelve-column sidebar a
    // four-column blend would be a third of the whole bar.
    for (width, most) in [(4u16, 1usize), (8, 2), (12, 3), (40, 4)] {
        let seam = (0..width)
            .filter(|c| {
                let x = column_colour(0.5, *c, width);
                x != ADDED && x != REMOVED
            })
            .count();
        assert!(
            seam <= most,
            "width {width} spends {seam} columns on the seam"
        );
    }
}

#[rstest]
#[case(0.0)]
#[case(0.5)]
#[case(1.0)]
fn a_one_column_row_still_produces_a_colour(#[case] ratio: f32) {
    let _ = column_colour(ratio, 0, 1);
}

#[rstest]
#[case(0.0)]
#[case(0.5)]
#[case(1.0)]
fn a_zero_width_row_still_produces_a_colour(#[case] ratio: f32) {
    // Nothing should be drawn, but a pane can be one column narrower than its
    // border and the caller must not have to special-case it.
    let _ = column_colour(ratio, 0, 0);
}

#[test]
fn the_flat_ends_are_contiguous_so_the_bar_reads_left_to_right() {
    // Green prefix, seam, red suffix — never a stripe of green inside the red.
    for step in 0..=20u16 {
        let ratio = f32::from(step) / 20.0;
        let hands: Vec<u8> = (0..40)
            .map(|c| match column_colour(ratio, c, 40) {
                x if x == ADDED => 0,
                x if x == REMOVED => 2,
                _ => 1,
            })
            .collect();
        assert!(
            hands.windows(2).all(|w| w[0] <= w[1]),
            "ratio {ratio} is not ordered green → seam → red: {hands:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// The pivot
// ---------------------------------------------------------------------------

#[test]
fn the_pivot_is_a_step_above_the_lighter_end_and_stops_short_of_white() {
    let p = pivot();
    assert!(luma(p) > luma(ADDED), "brighter than the green end");
    assert!(luma(p) > luma(REMOVED), "brighter than the red end");
    assert!(
        p.0 < 245 && p.1 < 245 && p.2 < 245,
        "not white — pure white flares on a dark terminal: {p:?}"
    );
}

#[test]
fn the_pivot_carries_no_hue_of_its_own() {
    // A neutral seam is what guarantees each half only ever desaturates toward
    // it: neither hand can pick up a trace of the other's colour on the way.
    let Rgb(r, g, b) = pivot();
    assert_eq!((r, g), (b, b), "the pivot is neutral: {:?}", pivot());
}

// ---------------------------------------------------------------------------
// Oklab mixing
// ---------------------------------------------------------------------------

#[test]
fn mixing_lands_on_its_own_endpoints() {
    for (a, b) in [(ADDED, REMOVED), (COMMENT, ALERT), (pivot(), FOCUS)] {
        let start = oklab_mix(a, b, 0.0);
        let end = oklab_mix(a, b, 1.0);
        assert_eq!(start, a, "t = 0 is the first colour");
        assert_eq!(end, b, "t = 1 is the second colour");
    }
}

#[test]
fn mixing_happens_in_oklab_not_in_the_encoded_channels() {
    // Halfway between black and white is Oklab `L` = 0.5, which encodes to
    // about 99 — not the 128 a naive channel lerp gives, nor the 188 of a
    // linear-light one. Every claim this module makes about the seam rests on
    // the space being perceptual.
    let Rgb(r, g, b) = oklab_mix(Rgb(0, 0, 0), Rgb(255, 255, 255), 0.5);
    assert_eq!((g, b), (r, r), "grey stays grey");
    assert!(
        (95..=103).contains(&r),
        "the midpoint of black and white is {r}, which is not an Oklab midpoint"
    );
}

#[test]
fn mixing_is_the_same_from_either_side() {
    for i in 0..=10u16 {
        let t = f32::from(i) / 10.0;
        let forward = oklab_mix(ADDED, REMOVED, t);
        let back = oklab_mix(REMOVED, ADDED, 1.0 - t);
        for (x, y) in [
            (forward.0, back.0),
            (forward.1, back.1),
            (forward.2, back.2),
        ] {
            assert!(x.abs_diff(y) <= 1, "t = {t}: {forward:?} vs {back:?}");
        }
    }
}

#[test]
fn mixing_out_of_gamut_clamps_instead_of_wrapping() {
    // Oklab is not a box inside sRGB: interpolating between two in-gamut
    // colours can leave the cube. Cyan and magenta both pin blue at the top and
    // every step between them stays there — a cast that wrapped past 255
    // instead of clamping would drop blue to near zero and punch a black cell
    // into the middle of the ramp.
    for i in 0..=100u16 {
        let t = f32::from(i) / 100.0;
        let Rgb(_, _, blue) = oklab_mix(Rgb(0, 255, 255), Rgb(255, 0, 255), t);
        assert!(blue > 200, "t = {t}: blue wrapped to {blue}");
    }
}

#[rstest]
#[case(-1.0, ADDED)]
#[case(2.0, REMOVED)]
fn mixing_outside_the_unit_interval_does_not_extrapolate(#[case] t: f32, #[case] expected: Rgb) {
    assert_eq!(oklab_mix(ADDED, REMOVED, t), expected);
}

// ---------------------------------------------------------------------------
// The rest of the palette
// ---------------------------------------------------------------------------

/// A named claim about one palette colour's channels — "added is green", and
/// the arithmetic that says so.
type HueCheck = (&'static str, Rgb, fn([u8; 3]) -> bool);

#[test]
fn every_colour_in_the_palette_means_exactly_one_thing() {
    let named = [
        ("added", ADDED),
        ("removed", REMOVED),
        ("comment", COMMENT),
        ("alert", ALERT),
        ("focus", FOCUS),
        ("pivot", pivot()),
    ];
    for (i, (a, ca)) in named.iter().enumerate() {
        for (b, cb) in named.iter().skip(i + 1) {
            assert_ne!(ca, cb, "{a} and {b} are the same colour");
        }
    }

    // And each one still is the hue its meaning is named after: nothing green
    // may come to mean anything but an addition.
    let channels = |c: Rgb| [c.0, c.1, c.2];
    let hues: [HueCheck; 5] = [
        ("added is green", ADDED, |c| {
            c[1] > c[0] + 60 && c[1] > c[2] + 60
        }),
        ("removed is red", REMOVED, |c| {
            c[0] > c[1] + 60 && c[0] > c[2] + 60
        }),
        ("comment is blue", COMMENT, |c| {
            c[2] > c[0] + 60 && c[2] > c[1] + 60
        }),
        ("alert is orange", ALERT, |c| {
            c[0] > c[1] && c[1] > c[2] + 60
        }),
        ("focus is magenta", FOCUS, |c| {
            c[0] > c[1] + 60 && c[2] > c[1] + 60
        }),
    ];
    for (meaning, colour, holds) in hues {
        assert!(holds(channels(colour)), "{meaning}, but it is {colour:?}");
    }

    let green_in = |c: Rgb| i16::from(c.1);
    assert!(
        green_in(ALERT) - green_in(REMOVED) > 50,
        "the alert orange must not read as a second red"
    );
}

#[rstest]
#[case(ADDED)]
#[case(REMOVED)]
#[case(COMMENT)]
#[case(ALERT)]
#[case(FOCUS)]
fn the_ink_over_a_palette_colour_clears_wcag_aa(#[case] background: Rgb) {
    let ink = readable_on(background);
    assert!(
        ink == INK_DARK || ink == INK_LIGHT,
        "the ink is one of the two: {ink:?}"
    );
    assert!(
        contrast(ink, background) >= 4.5,
        "{background:?} takes {ink:?} at {:.2}:1",
        contrast(ink, background)
    );
}

#[test]
fn the_ink_never_loses_to_the_other_choice() {
    for i in 0..=64u16 {
        for background in [
            oklab_mix(ADDED, pivot(), f32::from(i) / 64.0),
            oklab_mix(pivot(), REMOVED, f32::from(i) / 64.0),
        ] {
            let ink = readable_on(background);
            let other = if ink == INK_DARK { INK_LIGHT } else { INK_DARK };
            assert!(
                contrast(ink, background) >= contrast(other, background),
                "{background:?} took the worse ink"
            );
            assert!(
                contrast(ink, background) >= 4.5,
                "{background:?} at {:.2}:1",
                contrast(ink, background)
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Stats
// ---------------------------------------------------------------------------

#[test]
fn a_file_with_no_line_changes_has_no_ratio() {
    assert_eq!(
        Stat {
            added: 0,
            removed: 0
        }
        .added_ratio(),
        None
    );
    assert_eq!(
        Stat {
            added: 3,
            removed: 1
        }
        .added_ratio(),
        Some(0.75)
    );
}

#[test]
fn a_stat_counts_both_hands() {
    assert_eq!(
        Stat {
            added: 3,
            removed: 1
        }
        .total(),
        4
    );
    assert_eq!(
        Stat::default(),
        Stat {
            added: 0,
            removed: 0
        }
    );
}

#[test]
fn stats_add_up_so_a_directory_can_stand_for_its_subtree() {
    let subtree = [
        Stat {
            added: 3,
            removed: 1,
        },
        Stat {
            added: 0,
            removed: 5,
        },
        Stat::default(),
    ];
    let total = subtree.into_iter().fold(Stat::default(), |a, b| a + b);
    assert_eq!(
        total,
        Stat {
            added: 3,
            removed: 6
        }
    );
    assert_eq!(total.added_ratio(), Some(3.0 / 9.0));
}

#[test]
fn a_stat_that_would_overflow_saturates_rather_than_panicking() {
    let huge = Stat {
        added: u32::MAX,
        removed: u32::MAX,
    };
    assert_eq!(huge.total(), u32::MAX);
    assert_eq!((huge + huge).added, u32::MAX);
}

// ---------------------------------------------------------------------------
// Properties
// ---------------------------------------------------------------------------

proptest! {
    #[test]
    fn every_column_of_every_row_produces_a_colour(
        ratio in 0.0f32..=1.0,
        width in 0u16..200,
        offset in 0u16..200,
    ) {
        let column = if width == 0 { offset } else { offset % width };
        let _ = column_colour(ratio, column, width);
    }

    #[test]
    fn the_ends_stay_flat_wherever_there_is_room_for_them(
        ratio in 0.0f32..=1.0,
        width in 1u16..200,
    ) {
        let w = f32::from(width);
        if ratio * w >= 3.0 {
            prop_assert_eq!(column_colour(ratio, 0, width), ADDED);
        }
        if (1.0 - ratio) * w >= 3.0 {
            prop_assert_eq!(column_colour(ratio, width - 1, width), REMOVED);
        }
    }

    #[test]
    fn the_bar_never_goes_darker_than_its_darker_end(
        ratio in 0.0f32..=1.0,
        width in 1u16..200,
    ) {
        // The clamp on the way out of Oklab is what holds this up. Without it a
        // channel a hair past 255 wraps to nothing and the ramp gets a black
        // cell punched into it, which is far more visible than the rounding
        // error that caused it.
        let floor = luma(ADDED).min(luma(REMOVED)) - 1.0;
        for column in 0..width {
            let c = column_colour(ratio, column, width);
            prop_assert!(luma(c) >= floor, "column {} is {:?}", column, c);
        }
    }

    #[test]
    fn no_row_at_any_width_has_a_muddy_cell(
        ratio in 0.0f32..=1.0,
        width in 1u16..200,
    ) {
        for column in 0..width {
            let Rgb(r, g, _) = column_colour(ratio, column, width);
            let muddy = r > 90 && g > 90 && r.abs_diff(g) < 25
                && (u16::from(r) + u16::from(g)) < 380;
            prop_assert!(!muddy, "column {} is mud: {:?}", column, Rgb(r, g, 0));
        }
    }

    #[test]
    fn the_seam_never_swallows_more_than_four_columns(
        ratio in 0.0f32..=1.0,
        width in 1u16..200,
    ) {
        let seam = (0..width)
            .filter(|c| {
                let x = column_colour(ratio, *c, width);
                x != ADDED && x != REMOVED
            })
            .count();
        prop_assert!(seam <= 4, "{} columns are in the seam", seam);
    }
}
