//! The bar along the bottom: what mode you are in, where you are, and what is
//! in scope, in powerline segments that drop by priority when the terminal is
//! too narrow to hold them all.
//!
//! Everything here is pure. [`segments`] takes a [`View`] — a mode name, a
//! file, a revset, a count, the last thing that happened — and returns a list;
//! [`render`] turns that list into a [`Line`] of exactly the width it was asked
//! for. Neither touches [`crate::app::App`] or a `Frame`, so the bar can be
//! asserted directly rather than read back out of a rendered buffer, and
//! [`crate::ui`] is left with one call to make.
//!
//! **The status is a segment, not the bar.** rv used to paint `app.status()`
//! over the whole row, so the first `d` a reviewer pressed replaced the keymap
//! hint with `deleted comment at app.rs:42` and it never came back — the one
//! in-app reference to the keys, evicted by the first thing anybody does. Here
//! it is one segment among six and can displace nothing. It is also the *first*
//! segment dropped when the bar is short, because it is the only part of the
//! bar that stops being true on its own.
//!
//! **The `?` hint is the last segment dropped**, ahead even of the mode. A
//! reviewer on an 80-column ssh session is exactly the one who cannot afford to
//! go looking for the manual, and a bar with nothing but `? help` on it still
//! answers the only question a lost reviewer has.
//!
//! **Only what a terminal can draw reaches the bar.** A path, a revset or a
//! status built from one can carry a control character, and the two halves of
//! ratatui disagree about those: `Line::width` asks `unicode-width`, which
//! gives every character below `U+00A1` one column, while the renderer walks
//! graphemes and refuses to draw any that holds a control. A bar measured by
//! the first and painted by the second is one column short per control
//! character — and a `\x1b` that *did* reach the terminal would be a control
//! sequence a file name had smuggled onto the screen. [`printable`] drops them
//! before either measuring or painting, so the two agree and the bar carries
//! only what it can show.
//!
//! **Segments drop whole.** Nothing is ever cut mid-word: half of
//! `deleted comment at app.rs:42` is a claim about a file that does not exist,
//! and half of a revset is a revset that would select something else. The bar
//! is padded to the requested width with the fill colour instead — exactly the
//! requested width, at every width, because one column too many is silently
//! dropped by ratatui and one column too few lets the row underneath show
//! through.
//!
//! **Powerline arrows by default, `RV_ASCII` to turn them off.** The `U+E0B0`
//! chevrons need a patched font and rv cannot detect one, so a reviewer without
//! it would see tofu with no way out. The switch follows the precedent
//! `RV_NO_DIFFT` set in [`rv_core::diff`]: setting the variable is the switch,
//! whatever it is set to. Read it **once**, with [`ascii_from_env`], and carry
//! the answer — a per-frame `var_os` is a syscall on every keystroke to answer
//! a question that cannot change while the process runs.

use std::cmp::Reverse;
use std::ffi::OsStr;

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use crate::gradient::{self, Rgb, Stat};

/// The environment variable that turns the powerline glyphs off.
pub const RV_ASCII: &str = "RV_ASCII";

/// What the hint segment says. `?` is the only key the bar names, because it is
/// the key that names all the others.
pub const HINT: &str = "? help";

/// The right-pointing powerline separator, `U+E0B0`.
const ARROW: &str = "\u{e0b0}";
/// The left-pointing one, `U+E0B2`, which caps the right-aligned run.
const ARROW_LEFT: &str = "\u{e0b2}";
/// What separates two segments when the glyphs are off. A vertical bar rather
/// than nothing at all, so the bar still reads as segments on a terminal that
/// is showing no colour either.
const PIPE: &str = "|";

/// What a segment is for.
///
/// The role decides three things at once — what the segment says, what colour
/// it is drawn in, and how readily it is dropped — so a new kind of segment
/// cannot be added without answering all three.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Role {
    /// What the next keystroke will do: `BROWSE`, `COMMENT`, `CONFIRM`.
    Mode,
    /// The selected file, how far through the list it is, and the shape of its
    /// change.
    Position,
    /// The revset under review.
    Scope,
    /// How many comments are open.
    Comments,
    /// The last thing that happened.
    Status,
    /// Right-aligned, and it names `?`.
    Hint,
}

impl Role {
    /// How readily the segment is given up when the bar is too narrow: the
    /// lowest rank goes first.
    ///
    /// One ranking, in one place. The status leads because it will be untrue in
    /// eight seconds anyway; the hint is last because the narrower the
    /// terminal, the more its reader needs it; and the mode is second to last
    /// because it is the segment that says what the next keystroke means.
    const fn rank(self) -> u8 {
        match self {
            Role::Status => 0,
            Role::Scope => 1,
            Role::Position => 2,
            Role::Comments => 3,
            Role::Mode => 4,
            Role::Hint => 5,
        }
    }

    /// Whether the segment is drawn at the right-hand end rather than in the
    /// run that starts at the left.
    const fn trailing(self) -> bool {
        matches!(self, Role::Hint)
    }

    /// The colour the segment is drawn on.
    ///
    /// Neutrals, in a ramp bright enough for consecutive segments to read as
    /// separate blocks, with exactly one hue: the comment count is
    /// [`gradient::COMMENT`], because blue already means a comment everywhere
    /// else in this interface. Nothing here claims green, red, orange or
    /// magenta — those mean an addition, a removal, an alert and the focused
    /// pane, and a status bar that borrowed one would be saying something it
    /// does not mean. Colouring the mode *per context* is what the spec asks
    /// for in the end; it waits for `Context` to exist, and until then the mode
    /// is the brightest block rather than an arbitrary hue.
    fn background(self) -> Rgb {
        match self {
            Role::Mode => neutral(0.22),
            Role::Position | Role::Hint => neutral(0.56),
            Role::Scope | Role::Status => neutral(0.70),
            Role::Comments => gradient::oklab_mix(gradient::COMMENT, gradient::INK_DARK, 0.30),
        }
    }

    /// How the text on the segment is weighted. The mode is bold: it is the one
    /// segment a reviewer looks at without meaning to.
    fn modifier(self) -> Modifier {
        match self {
            Role::Mode => Modifier::BOLD,
            _ => Modifier::empty(),
        }
    }
}

/// One block of the bar.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Segment {
    pub text: String,
    pub role: Role,
}

/// Everything the bar needs to know about the session, as plain data.
///
/// A view rather than an `&App` on purpose: the bar is then testable without a
/// workspace, a store or a terminal, and [`crate::ui`] does the one thing it is
/// for — reading the app and handing over what it found.
///
/// `mode` is a name rather than a [`crate::app::Mode`] because the spec has the
/// segment naming the *context* the cursor is in (`FILES`, `DIFF`, `COMMENT`,
/// `CONFIRM`, …), which is a richer thing than the typing mode and does not
/// exist yet. A `&str` is what both can produce.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct View<'a> {
    /// What the next keystroke does, already spelled the way the bar shows it.
    pub mode: &'a str,
    /// The selected file, or `None` when the review has none.
    pub file: Option<&'a str>,
    /// Its zero-based position in the file list; shown one-based.
    pub file_index: usize,
    /// How many files the review has.
    pub file_count: usize,
    /// The shape of the selected file's change, when it has one.
    pub stat: Option<Stat>,
    /// The revset under review, as the reviewer asked for it.
    pub scope: &'a str,
    /// How many comments are open.
    pub open_comments: usize,
    /// The last thing that happened, or empty once it has expired.
    pub status: &'a str,
}

/// The bar's segments, left to right, skipping the ones with nothing to say.
///
/// The comment count is *always* present, even at zero. Any segment can be
/// dropped for want of room, so a count that vanished at zero would leave a
/// reviewer unable to tell "no comments" from "this terminal is too narrow" —
/// whereas `0 open` says one thing only.
#[must_use]
pub fn segments(view: &View<'_>) -> Vec<Segment> {
    let mut bar = Vec::with_capacity(6);
    let mut push = |role: Role, text: String| {
        if !text.is_empty() {
            bar.push(Segment { text, role });
        }
    };

    push(Role::Mode, view.mode.to_owned());
    if let Some(file) = view.file {
        // "how far through the list" needs a list, and the shape of the change
        // needs a change: a review with neither says the file's name and stops
        // rather than claiming `1/0` or `+0 -0`.
        let counter = if view.file_count > 0 {
            format!(" {}/{}", view.file_index.saturating_add(1), view.file_count)
        } else {
            String::new()
        };
        let shape = view
            .stat
            .filter(|stat| stat.total() > 0)
            .map(|stat| format!(" +{} -{}", stat.added, stat.removed))
            .unwrap_or_default();
        push(Role::Position, format!("{file}{counter}{shape}"));
    }
    push(Role::Scope, view.scope.to_owned());
    push(Role::Comments, format!("{} open", view.open_comments));
    push(Role::Status, view.status.to_owned());
    push(Role::Hint, HINT.to_owned());
    bar
}

/// The bar, exactly `width` columns wide.
///
/// Segments keep their given order; [`Role::Hint`] is drawn at the right-hand
/// end and everything else in a run from the left, with the space between them
/// filled. When the segments do not fit, the least important is dropped —
/// whole, never truncated — and the question asked again, until what is left
/// fits or nothing is left. See [`Role::rank`] for the order, and the module
/// docs for why the hint outlives the mode.
///
/// `ascii` turns the powerline glyphs off; see [`ascii_from_env`].
#[must_use]
pub fn render(segments: &[Segment], width: u16, ascii: bool) -> Line<'static> {
    let width = usize::from(width);
    let mut kept: Vec<usize> = (0..segments.len())
        .filter(|index| !segments[*index].text.is_empty())
        .collect();

    // Each drop removes at least two columns, so this cannot spin: `kept`
    // shrinks every time round and an empty bar measures zero.
    while measure(segments, &kept, ascii) > width {
        let Some(position) = kept
            .iter()
            .enumerate()
            .min_by_key(|(_, index)| (segments[**index].role.rank(), Reverse(**index)))
            .map(|(position, _)| position)
        else {
            break;
        };
        kept.remove(position);
    }

    let (trailing, leading): (Vec<usize>, Vec<usize>) = kept
        .iter()
        .partition(|index| segments[**index].role.trailing());

    let mut spans = Vec::with_capacity(kept.len() * 2 + 1);
    for (position, index) in leading.iter().enumerate() {
        let segment = &segments[*index];
        spans.push(block(segment));
        let next = leading
            .get(position + 1)
            .map_or_else(fill, |index| segments[*index].role.background());
        match (ascii, position + 1 == leading.len()) {
            // The run ends: in powerline the chevron is what carries the colour
            // across into the fill, and with the glyphs off the fill's own
            // ground is the boundary.
            (false, _) => spans.push(separator(ARROW, segment.role.background(), next)),
            (true, false) => spans.push(separator(PIPE, gradient::readable_on(next), next)),
            (true, true) => {}
        }
    }

    let padding = width.saturating_sub(measure(segments, &kept, ascii));
    if padding > 0 {
        spans.push(Span::styled(
            " ".repeat(padding),
            Style::new().bg(colour(fill())),
        ));
    }

    for (position, index) in trailing.iter().enumerate() {
        let segment = &segments[*index];
        let previous = position
            .checked_sub(1)
            .map_or_else(fill, |before| segments[trailing[before]].role.background());
        let background = segment.role.background();
        if ascii {
            if position > 0 {
                spans.push(separator(
                    PIPE,
                    gradient::readable_on(background),
                    background,
                ));
            }
        } else {
            spans.push(separator(ARROW_LEFT, background, previous));
        }
        spans.push(block(segment));
    }

    Line::from(spans)
}

/// Whether the powerline glyphs are turned off, from the process environment.
///
/// Call this **once**, at startup, and carry the answer: the variable cannot
/// change while rv runs, and a lookup per frame is a syscall per keystroke.
#[must_use]
pub fn ascii_from_env() -> bool {
    ascii_from(std::env::var_os(RV_ASCII).as_deref())
}

/// Whether the value of [`RV_ASCII`] turns the glyphs off.
///
/// Presence is the switch, exactly as it is for `RV_NO_DIFFT`: `RV_ASCII=0`
/// turns the glyphs *off* like any other value, because a reviewer who has
/// learned one escape hatch in this tool has learned both, and a variable that
/// meant one thing here and another there is worse than a variable with a blunt
/// rule. Split from [`ascii_from_env`] so it can be tested without mutating the
/// environment of a threaded test binary.
#[must_use]
pub fn ascii_from(value: Option<&OsStr>) -> bool {
    value.is_some()
}

/// How many columns the kept segments need, separators and padding spaces
/// included and the fill excluded.
///
/// Measured with the same call ratatui uses when it paints, so the arithmetic
/// that decides what to drop and the arithmetic that decides where a cell goes
/// cannot disagree about a wide character.
fn measure(segments: &[Segment], kept: &[usize], ascii: bool) -> usize {
    let (trailing, leading): (Vec<usize>, Vec<usize>) = kept
        .iter()
        .partition(|index| segments[**index].role.trailing());
    [leading, trailing]
        .iter()
        .map(|run| {
            if run.is_empty() {
                return 0;
            }
            let text: usize = run
                .iter()
                .map(|index| columns(&segments[*index].text) + 2)
                .sum();
            // One separator between each pair, plus — with the glyphs on — the
            // chevron that caps the run against the fill.
            let separators = run.len() - usize::from(ascii);
            text + separators
        })
        .sum()
}

/// One segment, padded with a space on each side so the chevrons do not sit
/// against the text.
fn block(segment: &Segment) -> Span<'static> {
    let background = segment.role.background();
    Span::styled(
        format!(" {} ", printable(&segment.text)),
        Style::new()
            .fg(colour(gradient::readable_on(background)))
            .bg(colour(background))
            .add_modifier(segment.role.modifier()),
    )
}

/// A separator between two blocks: `ink` is the colour it is drawn in and
/// `ground` what it is drawn on.
///
/// A powerline chevron is the *previous* block's colour drawn on the next
/// one's, which is what makes the boundary read as one block overlapping the
/// other rather than as two blocks with a glyph between them. A pipe is
/// ordinary text and takes the ink of the ground it sits on.
fn separator(glyph: &'static str, ink: Rgb, ground: Rgb) -> Span<'static> {
    Span::styled(glyph, Style::new().fg(colour(ink)).bg(colour(ground)))
}

/// The ground the bar's empty middle is painted on: dark enough to read as the
/// bar's own background rather than as another segment, and painted rather than
/// left bare so the row is one bar instead of two blocks with the pane showing
/// through between them.
fn fill() -> Rgb {
    neutral(0.86)
}

/// A neutral `wash` of the way from white to black.
fn neutral(wash: f32) -> Rgb {
    gradient::oklab_mix(gradient::INK_LIGHT, gradient::INK_DARK, wash)
}

fn colour(Rgb(red, green, blue): Rgb) -> Color {
    Color::Rgb(red, green, blue)
}

/// The width of some text in terminal columns, asked of ratatui rather than
/// computed here — and asked of the text ratatui will actually paint, which is
/// [`printable`] of it.
fn columns(text: &str) -> usize {
    Span::raw(printable(text)).width()
}

/// `text` with every character a terminal cannot draw taken out.
///
/// ratatui's renderer drops any grapheme containing a control character, while
/// its `width` asks `unicode-width`, which charges one column for every
/// character below `U+00A1` — control or not. Left alone the two disagree and
/// the bar comes up a column short of the `Rect` it was painted into, letting
/// the pane underneath show through the gap. Dropping the controls here makes
/// the measurement and the painting the same question, and keeps an escape
/// sequence that arrived inside a file name off the terminal into the bargain.
///
/// Filtering characters rather than graphemes is exact, not an approximation:
/// a control character always begins and ends a grapheme cluster, the sole
/// exception being `\r\n`, which is a cluster of nothing but controls. So the
/// characters removed here are precisely the graphemes ratatui would refuse.
fn printable(text: &str) -> String {
    text.chars().filter(|c| !c.is_control()).collect()
}
