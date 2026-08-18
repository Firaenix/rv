//! The bar along the bottom.
//!
//! Everything here is pure. [`rv::statusbar::segments`] takes a plain
//! description of the session — a mode name, a file, a revset, a count — and
//! returns a list; [`rv::statusbar::render`] turns that list into a
//! `ratatui::text::Line` of exactly the width it was asked for. No `App`, no
//! `Frame`, no terminal, so every ruling below is asserted directly rather than
//! read back out of a rendered buffer.
//!
//! Three rulings drive the shape of these tests:
//!
//! * **The status is a segment, not the bar.** Today a status message replaces
//!   the whole bar, so `deleted comment at app.rs:42` evicts the keymap hint
//!   permanently and a reviewer loses their only in-app reference on the first
//!   thing they do. As a segment it sits between the others and can displace
//!   nothing — and it is the *first* thing dropped when the bar is narrow,
//!   because it is the one piece of the bar that will be true again in eight
//!   seconds anyway.
//! * **The `?` hint is the last segment dropped**, ahead even of the mode,
//!   because a reviewer on a cramped terminal is exactly the one who most needs
//!   telling where the keys are.
//! * **Powerline arrows by default, `RV_ASCII` to turn them off.** The glyphs
//!   need a patched font and rv cannot detect one, so the escape hatch follows
//!   the `RV_NO_DIFFT` precedent: presence of the variable is the switch.
//!
//! The property at the bottom is the one that matters. A status bar is painted
//! into a `Rect` of a fixed width: one column too many and ratatui drops the
//! overflow silently, one column too few and whatever was on that row before
//! shows through the gap. So `render` must return *exactly* the requested width
//! at every width and for every content, which is a claim about arbitrary
//! inputs and therefore a property rather than three examples.

use std::ffi::OsStr;

use proptest::prelude::*;
use ratatui::text::Line;
use rstest::rstest;
use rv::gradient::Stat;
use rv::statusbar::{HINT, Role, Segment, View, ascii_from, ascii_from_env, render, segments};

/// The right-pointing powerline separator, `U+E0B0`. In the Private Use Area,
/// so a font without the patch shows tofu — which is the whole reason
/// `RV_ASCII` exists.
const ARROW: char = '\u{e0b0}';
/// Its left-pointing twin, `U+E0B2`, which caps the right-aligned hint.
const ARROW_LEFT: char = '\u{e0b2}';
/// Every character the bar may draw between two segments, in either mode.
const SEPARATORS: &[char] = &[ARROW, ARROW_LEFT, '|'];

/// A full bar: every segment present, nothing empty.
fn sample_view() -> View<'static> {
    View {
        mode: "BROWSE",
        file: Some("src/app.rs"),
        file_index: 2,
        file_count: 29,
        stat: Some(Stat {
            added: 12,
            removed: 3,
        }),
        scope: "trunk()..@",
        open_comments: 4,
        status: "saved comment at app.rs:42",
    }
}

fn sample_segments() -> Vec<Segment> {
    segments(&sample_view())
}

fn line_text(line: &Line<'_>) -> String {
    line.spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect()
}

/// Measured the way ratatui measures it when it paints — one call, so a test
/// and the renderer cannot disagree about what a column is.
fn line_width(line: &Line<'_>) -> usize {
    line.width()
}

fn roles(segments: &[Segment]) -> Vec<Role> {
    segments.iter().map(|segment| segment.role).collect()
}

/// The first three characters of each sample segment's text, all distinct, so
/// "did any of this segment survive" can be asked without a substring of one
/// segment matching another.
const PREFIXES: &[(&str, &str)] = &[
    ("BROWSE", "BRO"),
    ("src/app.rs 3/29 +12 -3", "src"),
    ("trunk()..@", "tru"),
    ("4 open", "4 o"),
    ("saved comment at app.rs:42", "sav"),
    (HINT, "? h"),
];

// ---------------------------------------------------------------------------
// What the bar says
// ---------------------------------------------------------------------------

#[test]
fn the_bar_names_the_mode_the_file_and_the_scope() {
    let text = line_text(&render(&sample_segments(), 100, false));
    assert!(text.contains("BROWSE"), "the mode is visible: {text}");
    assert!(text.contains("src/app.rs"), "and the selected file: {text}");
    assert!(
        text.contains("3/29"),
        "and how far through the list it is: {text}"
    );
    assert!(
        text.contains("trunk()..@"),
        "and what is being reviewed: {text}"
    );
    assert!(
        text.contains("4 open"),
        "and how many comments are open: {text}"
    );
    assert!(text.contains(HINT), "and where the keys are: {text}");
}

#[test]
fn the_mode_segment_changes_with_the_mode() {
    let browse = line_text(&render(&sample_segments(), 100, false));
    let comment = line_text(&render(
        &segments(&View {
            mode: "COMMENT",
            ..sample_view()
        }),
        100,
        false,
    ));
    assert!(browse.contains("BROWSE"), "browsing says so: {browse}");
    assert!(
        comment.contains("COMMENT"),
        "typing is visibly a different mode: {comment}"
    );
    assert!(
        !comment.contains("BROWSE"),
        "and the two do not both appear: {comment}"
    );
}

#[test]
fn the_bar_carries_its_six_segments_in_reading_order() {
    assert_eq!(
        roles(&sample_segments()),
        [
            Role::Mode,
            Role::Position,
            Role::Scope,
            Role::Comments,
            Role::Status,
            Role::Hint
        ],
        "the mode leads, the hint trails"
    );
}

#[test]
fn the_selected_file_carries_the_shape_of_its_change() {
    let position = sample_segments()
        .into_iter()
        .find(|segment| segment.role == Role::Position)
        .expect("a view with a file has a position segment");
    assert!(
        position.text.contains("+12") && position.text.contains("-3"),
        "the stat rides with the file it belongs to: {}",
        position.text
    );
}

#[test]
fn a_review_with_no_file_selected_still_renders() {
    let bar = segments(&View {
        mode: "BROWSE",
        ..View::default()
    });
    assert_eq!(
        roles(&bar),
        [Role::Mode, Role::Comments, Role::Hint],
        "no file and no revset means no position and no scope, not an empty segment"
    );
    assert_eq!(
        line_width(&render(&bar, 40, false)),
        40,
        "and the bar is still a bar"
    );
}

#[test]
fn the_comment_count_is_shown_even_when_it_is_zero() {
    // Otherwise an absent count is ambiguous: a reviewer cannot tell "no
    // comments" from "this terminal is too narrow to say". The segment is
    // present whenever there is room, so its absence means one thing only.
    let bar = segments(&View {
        open_comments: 0,
        ..sample_view()
    });
    let comments = bar
        .iter()
        .find(|segment| segment.role == Role::Comments)
        .expect("the count is always a segment");
    assert_eq!(comments.text, "0 open");
}

// ---------------------------------------------------------------------------
// The status is a segment, not the bar
// ---------------------------------------------------------------------------

#[test]
fn a_status_message_displaces_nothing() {
    // The defect this module exists to fix: a status used to replace the whole
    // bar, so the first thing a reviewer did evicted the keymap hint for the
    // rest of the session.
    let text = line_text(&render(&sample_segments(), 100, false));
    assert!(
        text.contains("saved comment at app.rs:42"),
        "the status is on the bar: {text}"
    );
    assert!(
        text.contains(HINT),
        "and the hint is still beside it: {text}"
    );
    assert!(
        text.contains("BROWSE") && text.contains("trunk()..@"),
        "and so is everything else: {text}"
    );
}

#[test]
fn an_empty_status_leaves_no_segment_behind() {
    // A status expires after roughly eight seconds. What is left must be
    // nothing at all, not a two-column coloured blob where a sentence was.
    let bar = segments(&View {
        status: "",
        ..sample_view()
    });
    assert!(
        !bar.iter().any(|segment| segment.role == Role::Status),
        "an expired status is absent, not empty: {bar:?}"
    );
}

#[test]
fn the_status_is_the_first_thing_dropped_when_the_bar_is_short() {
    let text = line_text(&render(&sample_segments(), 80, false));
    assert!(
        !text.contains("saved comment"),
        "the status goes first — it will be untrue in eight seconds anyway: {text}"
    );
    assert!(
        text.contains("trunk()..@"),
        "the scope is still there: {text}"
    );
}

// ---------------------------------------------------------------------------
// Dropping, in priority order, whole segments at a time
// ---------------------------------------------------------------------------

#[test]
fn a_bar_too_narrow_for_everything_drops_segments_rather_than_truncating_mid_word() {
    let text = line_text(&render(&sample_segments(), 24, false));
    assert!(text.contains("BROWSE"), "the mode survives: {text}");
    assert!(
        !text.contains("trunk"),
        "the scope is dropped whole, not cut in half: {text}"
    );
}

#[test]
fn the_hint_is_the_last_segment_standing() {
    // Ahead even of the mode: the reviewer on a 12-column bar is exactly the
    // one who most needs telling that `?` exists.
    let text = line_text(&render(&sample_segments(), 12, false));
    assert!(text.contains(HINT), "the hint outlives the mode: {text}");
    assert!(!text.contains("BROWSE"), "which is gone by here: {text}");
}

#[rstest]
#[case(0)]
#[case(1)]
#[case(7)]
fn a_bar_with_room_for_nothing_is_blank_rather_than_broken(#[case] width: u16) {
    let rendered = render(&sample_segments(), width, false);
    assert_eq!(line_width(&rendered), usize::from(width));
    assert!(
        line_text(&rendered).trim().is_empty(),
        "nothing fits, so nothing is shown — and nothing is half-shown"
    );
}

#[test]
fn segments_survive_in_priority_order_at_every_width() {
    // Which segments are on the bar is decided by one ranking — status, then
    // scope, then position, then comments, then the mode, and the hint last —
    // so the set of survivors is always a prefix of that ranking read
    // backwards. A bar that ever showed the scope but not the mode would mean
    // two rankings had drifted apart.
    let keep_order = [HINT, "BROWSE", "4 open", "src", "trunk", "saved"];
    for width in 0u16..=120 {
        let text = line_text(&render(&sample_segments(), width, false));
        let survivors: Vec<bool> = keep_order
            .iter()
            .map(|needle| text.contains(needle))
            .collect();
        let kept = survivors.iter().filter(|present| **present).count();
        assert_eq!(
            survivors,
            (0..keep_order.len()).map(|i| i < kept).collect::<Vec<_>>(),
            "at width {width} the survivors are not a prefix of the ranking: {text}"
        );
    }
}

#[test]
fn a_segment_is_never_half_printed() {
    for width in 0u16..=120 {
        for ascii in [false, true] {
            let text = line_text(&render(&sample_segments(), width, ascii));
            for (whole, prefix) in PREFIXES {
                assert!(
                    !text.contains(prefix) || text.contains(whole),
                    "at width {width} (ascii {ascii}) `{prefix}` appears without `{whole}`: {text}"
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Glyphs, and the escape hatch
// ---------------------------------------------------------------------------

#[test]
fn rv_ascii_replaces_the_powerline_glyphs() {
    let bar = line_text(&render(&sample_segments(), 80, false));
    let plain = line_text(&render(&sample_segments(), 80, true));
    assert!(bar.contains(ARROW), "arrows by default: {bar}");
    assert!(
        bar.contains(ARROW_LEFT),
        "including the one that caps the hint: {bar}"
    );
    assert!(
        !plain.contains(ARROW) && !plain.contains(ARROW_LEFT),
        "RV_ASCII uses no patched glyphs: {plain}"
    );
    assert!(
        plain.contains("BROWSE") && plain.contains(HINT),
        "and loses no information: {plain}"
    );
}

#[rstest]
#[case::unset(None, false)]
#[case::one(Some("1"), true)]
// Presence is the switch, exactly as `RV_NO_DIFFT` is presence: a reviewer who
// has learned one escape hatch in this tool has learned both.
#[case::zero(Some("0"), true)]
#[case::empty(Some(""), true)]
fn rv_ascii_is_a_switch_you_set_rather_than_a_value_you_parse(
    #[case] value: Option<&str>,
    #[case] expected: bool,
) {
    assert_eq!(ascii_from(value.map(OsStr::new)), expected);
}

#[test]
fn the_environment_is_read_through_the_same_function() {
    // `ascii_from_env` is meant to be called once at startup and the answer
    // carried in the app; the test only pins that it agrees with the pure form
    // for whatever this process happens to have, since mutating the
    // environment under a threaded test harness is not something to do for a
    // one-line lookup.
    assert_eq!(
        ascii_from_env(),
        ascii_from(std::env::var_os("RV_ASCII").as_deref())
    );
}

// ---------------------------------------------------------------------------
// The bar is a bar
// ---------------------------------------------------------------------------

#[test]
fn every_column_of_the_bar_is_painted() {
    // Including the empty middle. A powerline bar with a transparent stretch in
    // it does not read as one bar; it reads as two, with whatever was on that
    // row before showing through between them.
    let rendered = render(&sample_segments(), 100, false);
    for span in &rendered.spans {
        assert!(
            span.style.bg.is_some(),
            "a span with no background: {span:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// Exactly the width, always
// ---------------------------------------------------------------------------

#[rstest]
#[case(20)]
#[case(40)]
#[case(200)]
fn the_bar_fills_its_width_exactly_and_never_overflows(#[case] width: u16) {
    let rendered = render(&sample_segments(), width, false);
    assert_eq!(
        line_width(&rendered),
        usize::from(width),
        "exactly {width} columns"
    );
}

fn any_role() -> impl Strategy<Value = Role> {
    prop_oneof![
        Just(Role::Mode),
        Just(Role::Position),
        Just(Role::Scope),
        Just(Role::Comments),
        Just(Role::Status),
        Just(Role::Hint),
    ]
}

/// Printable ASCII, a few CJK ideographs and a few emoji — one column, two
/// columns, and a surrogate pair's worth of bytes — so the width arithmetic is
/// exercised over text that is not one column per `char`. The Private Use Area
/// is deliberately excluded: the powerline glyphs live there and a segment that
/// contained one would make the `RV_ASCII` property meaningless.
fn any_segment() -> impl Strategy<Value = Segment> {
    (
        any_role(),
        "[ -~\\x{4e00}-\\x{4e05}\\x{1f600}-\\x{1f602}]{0,24}",
    )
        .prop_map(|(role, text)| Segment { text, role })
}

proptest! {
    /// The property. One column too many and ratatui drops the overflow
    /// silently; one column too few and the row beneath shows through the gap.
    #[test]
    fn the_bar_is_exactly_the_width_it_was_asked_for(
        segments in prop::collection::vec(any_segment(), 0..8),
        width in 0u16..240,
        ascii in any::<bool>(),
    ) {
        let rendered = render(&segments, width, ascii);
        prop_assert_eq!(line_width(&rendered), usize::from(width));
    }

    #[test]
    fn ascii_never_emits_a_glyph_from_the_private_use_area(
        segments in prop::collection::vec(any_segment(), 0..8),
        width in 0u16..240,
    ) {
        let text = line_text(&render(&segments, width, true));
        prop_assert!(
            !text.chars().any(|c| ('\u{e000}'..='\u{f8ff}').contains(&c)),
            "a patched glyph survived RV_ASCII: {}", text,
        );
    }

    /// Whatever survives, survives whole. A segment is either printed with
    /// every one of its characters or not printed at all — the alternative is a
    /// bar that says `deleted comment at ap`, which is a claim about a file
    /// that does not exist.
    #[test]
    fn a_surviving_segment_keeps_all_of_its_characters(
        segments in prop::collection::vec(any_segment(), 0..8),
        width in 0u16..240,
        ascii in any::<bool>(),
    ) {
        let rendered = render(&segments, width, ascii);
        for span in &rendered.spans {
            let content = span.content.as_ref();
            let padding = content.chars().all(|c| c == ' ' || SEPARATORS.contains(&c));
            prop_assert!(
                padding || segments.iter().any(|segment| content == format!(" {} ", segment.text)),
                "`{}` is not a whole segment: {:?}", content, rendered,
            );
        }
    }
}
