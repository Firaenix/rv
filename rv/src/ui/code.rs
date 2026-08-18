//! One line of code: its number, its sigil, its wash, and its syntax colours.
//!
//! The wash is a **background** and a syntax colour is a **foreground**, so the
//! chrome this interface owns and the code the reviewer's own theme owns never
//! contend for the same channel. See spec §6 for the whole ruling, and
//! [`capture_colour`] for why only indexed colours reach code text.

use ratatui::style::Color;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
use rv_core::diff::DiffLine;
use rv_core::highlight::Capture;
use rv_core::highlight::Highlights;
use rv_core::model::Side;

use super::text::clip_row;
use crate::app::App;
use crate::app::anchored_side;
use crate::gradient;

/// How far an added or removed line's tint is taken toward
/// [`gradient::INK_DARK`]. Far: the wash marks a line, it does not shout at
/// one, and the syntax colours on top need the contrast more than the tint.
const WASH: f32 = 0.74;

/// The same for the selected line — the same hue a step brighter, rather than
/// `REVERSED`, which would put the syntax colours into the wash.
const WASH_SELECTED: f32 = 0.50;

/// The selected line's tint where the line is neither added nor removed. A
/// context line carries no hue of its own, so the highlight is a neutral band
/// rather than a colour that would claim one.
const WASH_SELECTED_CONTEXT: f32 = 0.78;

/// The two blobs' highlight spans for the file being drawn, one per side,
/// fetched once per frame.
///
/// Per frame rather than per row because [`App::highlights`] resolves a
/// `(commit, path)` key and a diff pane is forty rows: asking once and handing
/// the answer down the row loop is two lookups a frame rather than eighty.
#[derive(Clone, Copy)]
pub(super) struct Highlighting<'a> {
    left: Option<&'a Highlights>,
    right: Option<&'a Highlights>,
}

impl<'a> Highlighting<'a> {
    pub(super) fn of(app: &'a App) -> Self {
        Self {
            left: app.highlights(Side::Left),
            right: app.highlights(Side::Right),
        }
    }

    /// The spans for `line`, taken from the blob on **the side the line is
    /// anchored to** and looked up at that side's own number.
    ///
    /// [`anchored_side`] is asked here and nowhere else in this pane, and there
    /// is deliberately no fallback to the other side's number: a removed line
    /// looked up at its head-side number would be painted with the colours of
    /// whatever now stands there — a lie told in a colour rather than in words,
    /// and invisible to any test whose fixture renames the file.
    fn spans(&self, line: &DiffLine) -> &'a [rv_core::highlight::Span] {
        let side = anchored_side(line.kind);
        let (highlights, number) = match side {
            Side::Left => (self.left, line.left),
            Side::Right => (self.right, line.right),
        };
        match (highlights, number) {
            (Some(highlights), Some(number)) => highlights.line(number),
            _ => &[],
        }
    }

}

/// One line of the diff, washed by what kind of line it is, syntax coloured on
/// top, and clipped where there was more of it than the pane could show.
///
/// The wash goes on every cell of the row, not only the ones with text on them,
/// so an added line reads as a band rather than as a ragged edge.
pub(super) fn diff_row(
    highlighting: Highlighting<'_>,
    index: usize,
    line: &DiffLine,
    selected_line: usize,
    width: usize,
) -> Line<'static> {
    let selected = index == selected_line;
    // The gutter keeps the kind's hue and takes the bright version of it on the
    // selected row: the same green on the brighter green band is a `+` a
    // reviewer has to look for, and the sigil is the one part of the row that
    // still says *added* on a terminal that renders no background at all.
    let (sigil, colour) = match (line.kind, selected) {
        (rv_core::diff::LineKind::Added, false) => ('+', Color::Green),
        (rv_core::diff::LineKind::Added, true) => ('+', Color::LightGreen),
        (rv_core::diff::LineKind::Removed, false) => ('-', Color::Red),
        (rv_core::diff::LineKind::Removed, true) => ('-', Color::LightRed),
        (rv_core::diff::LineKind::Context, false) => (' ', Color::Gray),
        (rv_core::diff::LineKind::Context, true) => (' ', Color::White),
    };
    let number = match line_number(line) {
        Some(number) => format!("{number:>5}"),
        None => " ".repeat(5),
    };

    let ground = match line_background(line.kind, selected) {
        Some(background) => Style::default().bg(background),
        None => Style::default(),
    };
    let mut spans = vec![Span::styled(format!("{number} {sigil}"), ground.fg(colour))];
    spans.extend(highlighted(&line.text, highlighting.spans(line), ground));
    clip_row(spans, width, ground)
}

/// `text` cut into styled spans by `highlights`, with the gaps between them
/// left on the terminal's own foreground.
///
/// The spans were measured against the *blob* line and `text` is the diff's
/// rendering of it, which need not be byte-for-byte the same — difftastic does
/// its own thing with whitespace. So every offset is clamped to `text` and
/// walked back to a character boundary, a span that clamps to nothing is
/// dropped, and an offset behind where the walk has reached is skipped.
fn highlighted(
    text: &str,
    highlights: &[rv_core::highlight::Span],
    ground: Style,
) -> Vec<Span<'static>> {
    let mut spans: Vec<Span<'static>> = Vec::with_capacity(highlights.len() * 2 + 1);
    let mut at = 0usize;
    for span in highlights {
        let start = boundary(text, span.start as usize);
        let end = boundary(text, span.end as usize);
        if start < at || end <= start {
            continue;
        }
        if start > at {
            spans.push(Span::styled(text[at..start].to_owned(), ground));
        }
        spans.push(Span::styled(
            text[start..end].to_owned(),
            ground.fg(capture_colour(span.capture)),
        ));
        at = end;
    }
    if at < text.len() {
        spans.push(Span::styled(text[at..].to_owned(), ground));
    }
    spans
}

/// The largest index at or below `at` that is a character boundary of `text`.
fn boundary(text: &str, at: usize) -> usize {
    let mut at = at.min(text.len());
    while at > 0 && !text.is_char_boundary(at) {
        at -= 1;
    }
    at
}

/// The background a diff line of this kind is drawn on, or `None` where it is
/// left on the terminal's own ground.
///
/// Public because it is the one place the answer is decided: anything asking
/// "which row is the selected one" reads from it rather than keeping a second
/// copy of the palette. The hues are [`gradient::ADDED`] and
/// [`gradient::REMOVED`] themselves, so the diff pane and the sidebar's change
/// bar cannot end up with two greens and two reds that drift.
#[must_use]
pub fn line_background(kind: rv_core::diff::LineKind, selected: bool) -> Option<Color> {
    use rv_core::diff::LineKind;
    let (hue, wash) = match (kind, selected) {
        (LineKind::Added, false) => (gradient::ADDED, WASH),
        (LineKind::Added, true) => (gradient::ADDED, WASH_SELECTED),
        (LineKind::Removed, false) => (gradient::REMOVED, WASH),
        (LineKind::Removed, true) => (gradient::REMOVED, WASH_SELECTED),
        (LineKind::Context, false) => return None,
        (LineKind::Context, true) => (gradient::INK_LIGHT, WASH_SELECTED_CONTEXT),
    };
    let gradient::Rgb(red, green, blue) = gradient::oklab_mix(hue, gradient::INK_DARK, wash);
    Some(Color::Rgb(red, green, blue))
}

/// The foreground one kind of source token is painted with.
///
/// Every value is one of the **16 indexed ANSI colours**, which are a
/// pass-through to the reviewer's own scheme rather than a palette rv chose: an
/// `Rgb` value would dictate an exact colour and ignore the theme, which is
/// what makes a syntax theme something a user then has to configure. rv should
/// never need a theme option, because rv should never be the thing deciding.
/// Spec §6 holds the whole ruling; `rv/tests/app` asserts the boundary in
/// cells.
///
/// [`Capture::Punctuation`], [`Capture::Variable`] and [`Capture::Other`] keep
/// the terminal's own foreground: most of a line is one of the three, and
/// colouring the majority of the text is how a highlighter stops being one.
#[must_use]
pub fn capture_colour(capture: Capture) -> Color {
    match capture {
        Capture::Keyword => Color::Magenta,
        Capture::Function => Color::Blue,
        Capture::Type => Color::Cyan,
        Capture::String => Color::Green,
        // tree-sitter-rust reports integer and float literals as
        // `constant.builtin`, so Rust numbers arrive as `Constant`; the two
        // share a colour because they are the same thing to a reader.
        Capture::Number | Capture::Constant => Color::Yellow,
        // Index 8, the tone every scheme defines for exactly this. It was index
        // 7 — the terminal's *white* — which is as loud as the code it
        // annotates on a dark scheme and near-invisible on a light one.
        Capture::Comment => Color::DarkGray,
        Capture::Punctuation | Capture::Variable | Capture::Other => Color::Reset,
    }
}

/// The line number to label a diff line with: the one on the side a comment
/// there would anchor to.
///
/// Not `right.or(left)`: difftastic aligns a changed line with its counterpart
/// and gives the pair *both* numbers, so labelling a removed line by its
/// head-side number showed one number while the store held the other. The
/// fallback to the other side is orientation only, for a line with no number of
/// its own — such a line cannot be commented on at all.
fn line_number(line: &DiffLine) -> Option<u32> {
    match anchored_side(line.kind) {
        Side::Left => line.left.or(line.right),
        Side::Right => line.right.or(line.left),
    }
}
