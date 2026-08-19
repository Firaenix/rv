//! The two RGB colours in this interface — an addition's green and a removal's
//! red — and the arithmetic between them.
//!
//! Everything else the chrome draws is an ANSI index from [`crate::theme`],
//! resolved by the terminal's own theme. These two are RGB because they carry a
//! *proportion*: the gradient across a sidebar row and the wash under a diff
//! line are blends, and an index cannot be blended. Colour maths lives here
//! rather than in [`crate::ui`] so it can be tested without a terminal.
//!
//! The gradient is *diverging*. A row that is two thirds additions is green for
//! two thirds of its width and red for the rest, and the two hands meet at a
//! tight light seam rather than blending straight into one another: green and
//! red sit at opposite ends of Oklab's `a` axis, so a direct interpolation
//! crosses a dull mid-grey exactly where the eye is trying to read the boundary.
//! Pivoting through a bright neutral means each half only ever desaturates
//! toward the seam and back, so no cell is a mixture of the two hues.
//!
//! The seam is defined *relative* to the endpoints — a step above the lighter of
//! them in Oklab `L`, capped short of white — because an absolute white flares
//! on a dark terminal and vanishes on a light one. It is also kept narrow: the
//! bar exists to show a proportion, and a wide blend destroys the thing it is
//! drawing.

use std::ops::{Add, AddAssign};

/// A 24-bit colour. Deliberately not a `ratatui::style::Color`: this module is
/// arithmetic, and the mapping onto whatever depth the terminal actually
/// supports belongs to [`crate::ui`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Rgb(pub u8, pub u8, pub u8);

/// An addition. The only thing in this interface that is green.
pub const ADDED: Rgb = Rgb(46, 160, 67);
/// A removal. The only thing in this interface that is red.
pub const REMOVED: Rgb = Rgb(218, 54, 51);

/// Ink for text over a light tint, and its opposite. Pure black and pure white
/// rather than something softer: over a background that swings from a dark
/// green through a near-white seam to a dark red, the extremes are the only
/// pair that clears WCAG AA at every point along the ramp.
pub const INK_DARK: Rgb = Rgb(0, 0, 0);
/// Ink for text over a dark tint. See [`INK_DARK`].
pub const INK_LIGHT: Rgb = Rgb(255, 255, 255);

/// How far above the lighter endpoint the seam sits, in Oklab `L`.
const PIVOT_STEP: f32 = 0.30;
/// The seam never climbs past this `L`, which is short of white (`L` = 1.0).
const PIVOT_CEILING: f32 = 0.94;
/// What share of the row the seam blends across: half, centred on the
/// proportion. Wide on purpose — the gradient runs across a row's own *text*
/// now, and a text gradient is read as a wash of colour rather than as a bar,
/// so a tight seam would just look like two inks meeting at a typo.
const SEAM_SHARE: f32 = 0.5;

/// The seam the two halves meet at: a step brighter than the lighter endpoint
/// in Oklab `L`, capped short of white, with its chroma taken away entirely.
///
/// Relative rather than absolute, so it reads as a highlight on a dark terminal
/// and does not vanish on a light one. Neutral rather than nearly neutral,
/// because that is what guarantees neither hand can carry a trace of the
/// other's hue across the boundary.
#[must_use]
pub fn pivot() -> Rgb {
    let lighter = to_oklab(ADDED).l.max(to_oklab(REMOVED).l);
    from_oklab(Oklab {
        l: (lighter + PIVOT_STEP).min(PIVOT_CEILING),
        a: 0.0,
        b: 0.0,
    })
}

/// The colour for column `column` of a `width`-wide row at the given ratio.
///
/// Green on the left, red on the right, blending through the [`pivot`] across
/// a [`SEAM_SHARE`] of the row centred on the proportion: each half only ever
/// desaturates toward the pivot, so no cell is a mixture of the two hues, and
/// outside the seam the flat end colours are returned unchanged.
///
/// A ratio of 1.0 is green edge to edge and 0.0 is red edge to edge — the seam
/// shrinks as it approaches an end rather than hanging off it. A degenerate
/// width still yields a colour rather than dividing by zero.
#[must_use]
pub fn column_colour(ratio: f32, column: u16, width: u16) -> Rgb {
    let ratio = if ratio.is_nan() {
        0.0
    } else {
        ratio.clamp(0.0, 1.0)
    };
    let span = f32::from(width);
    let boundary = ratio * span;

    // Half the seam's width, shrunk so it can never hang off either end: at a
    // ratio of 1.0 there is no seam at all, only green.
    let half = (span * SEAM_SHARE / 2.0).min(boundary).min(span - boundary);

    // Where this cell's centre falls relative to the boundary.
    let offset = f32::from(column) + 0.5 - boundary;
    if half <= 0.0 {
        return if offset < 0.0 { ADDED } else { REMOVED };
    }
    if offset <= -half {
        return ADDED;
    }
    if offset >= half {
        return REMOVED;
    }
    if offset < 0.0 {
        oklab_mix(ADDED, pivot(), (offset + half) / half)
    } else {
        oklab_mix(pivot(), REMOVED, offset / half)
    }
}

/// Interpolate between two colours through Oklab, where a straight line is a
/// straight line to the eye as well as to the arithmetic.
///
/// `t` is clamped to `0.0..=1.0`; extrapolating past an endpoint would leave
/// sRGB entirely.
#[must_use]
pub fn oklab_mix(a: Rgb, b: Rgb, t: f32) -> Rgb {
    let t = if t.is_nan() { 0.0 } else { t.clamp(0.0, 1.0) };
    let (x, y) = (to_oklab(a), to_oklab(b));
    from_oklab(Oklab {
        l: x.l + (y.l - x.l) * t,
        a: x.a + (y.a - x.a) * t,
        b: x.b + (y.b - x.b) * t,
    })
}

/// Whichever of [`INK_DARK`] and [`INK_LIGHT`] contrasts more with `background`.
///
/// A tinted row is only worth drawing if the name on it is still readable, and
/// the gradient runs from a dark green through a near-white seam to a dark red,
/// so the answer changes from cell to cell along a single row.
#[must_use]
pub fn readable_on(background: Rgb) -> Rgb {
    if contrast(INK_DARK, background) >= contrast(INK_LIGHT, background) {
        INK_DARK
    } else {
        INK_LIGHT
    }
}

/// How many lines a file (or a subtree of them) added and removed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stat {
    pub added: u32,
    pub removed: u32,
}

impl Stat {
    /// Every changed line, both hands together.
    #[must_use]
    pub const fn total(self) -> u32 {
        self.added.saturating_add(self.removed)
    }

    /// The share of the change that is additions, or `None` when nothing
    /// changed — a pure rename has no shape, and inventing a ratio for it would
    /// paint a gradient over a file that did not move a line.
    #[must_use]
    pub fn added_ratio(self) -> Option<f32> {
        match self.total() {
            0 => None,
            total => Some(self.added as f32 / total as f32),
        }
    }
}

impl Add for Stat {
    type Output = Self;

    /// Saturating, so a directory row can stand for its whole subtree without a
    /// pathological review panicking the draw.
    fn add(self, other: Self) -> Self {
        Self {
            added: self.added.saturating_add(other.added),
            removed: self.removed.saturating_add(other.removed),
        }
    }
}

impl AddAssign for Stat {
    fn add_assign(&mut self, other: Self) {
        *self = *self + other;
    }
}

// ---------------------------------------------------------------------------
// Oklab
// ---------------------------------------------------------------------------

/// Björn Ottosson's Oklab: perceptual lightness `l` with two opponent axes,
/// `a` running green-to-red and `b` blue-to-yellow.
#[derive(Clone, Copy)]
struct Oklab {
    l: f32,
    a: f32,
    b: f32,
}

fn to_oklab(colour: Rgb) -> Oklab {
    let r = to_linear(colour.0);
    let g = to_linear(colour.1);
    let b = to_linear(colour.2);

    let long = 0.412_221_46 * r + 0.536_332_55 * g + 0.051_445_995 * b;
    let medium = 0.211_903_5 * r + 0.680_699_5 * g + 0.107_396_96 * b;
    let short = 0.088_302_46 * r + 0.281_718_85 * g + 0.629_978_7 * b;

    let (long, medium, short) = (long.cbrt(), medium.cbrt(), short.cbrt());
    Oklab {
        l: 0.210_454_26 * long + 0.793_617_8 * medium - 0.004_072_047 * short,
        a: 1.977_998_5 * long - 2.428_592_2 * medium + 0.450_593_7 * short,
        b: 0.025_904_037 * long + 0.782_771_77 * medium - 0.808_675_77 * short,
    }
}

fn from_oklab(lab: Oklab) -> Rgb {
    let long = lab.l + 0.396_337_78 * lab.a + 0.215_803_76 * lab.b;
    let medium = lab.l - 0.105_561_346 * lab.a - 0.063_854_17 * lab.b;
    let short = lab.l - 0.089_484_18 * lab.a - 1.291_485_5 * lab.b;

    let (long, medium, short) = (
        long * long * long,
        medium * medium * medium,
        short * short * short,
    );
    let r = 4.076_741_7 * long - 3.307_711_6 * medium + 0.230_969_94 * short;
    let g = -1.268_438 * long + 2.609_757_4 * medium - 0.341_319_38 * short;
    let b = -0.004_196_086_3 * long - 0.703_418_6 * medium + 1.707_614_7 * short;

    Rgb(to_encoded(r), to_encoded(g), to_encoded(b))
}

fn to_linear(channel: u8) -> f32 {
    let u = f32::from(channel) / 255.0;
    if u <= 0.040_45 {
        u / 12.92
    } else {
        ((u + 0.055) / 1.055).powf(2.4)
    }
}

/// Linear light back to an sRGB byte.
///
/// The clamp is the gamut, not a rounding guard. Oklab is not a box inside
/// sRGB, so interpolating between two in-gamut colours can leave the cube:
/// compress the excursion back onto the face here, before the transfer function
/// is asked for the root of a negative number.
///
/// Once it is inside `0.0..=1.0` the encoded value cannot exceed 255, so the
/// cast has nothing left to lose — but a channel that escaped and then wrapped
/// instead of saturating would punch a black cell into the middle of the ramp,
/// which is what `mixing_out_of_gamut_clamps_instead_of_wrapping` watches for.
fn to_encoded(linear: f32) -> u8 {
    let linear = linear.clamp(0.0, 1.0);
    let encoded = if linear <= 0.003_130_8 {
        12.92 * linear
    } else {
        1.055 * linear.powf(1.0 / 2.4) - 0.055
    };
    (encoded * 255.0).round() as u8
}

/// WCAG relative luminance, the only place this file cares about brightness in
/// a way Oklab's `L` cannot answer: contrast ratios are defined against it.
fn relative_luminance(colour: Rgb) -> f32 {
    0.2126 * to_linear(colour.0) + 0.7152 * to_linear(colour.1) + 0.0722 * to_linear(colour.2)
}

fn contrast(a: Rgb, b: Rgb) -> f32 {
    let (x, y) = (relative_luminance(a), relative_luminance(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}
