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
use std::process::Command;

use proptest::prelude::*;
use ratatui::buffer::{Buffer, Cell};
use ratatui::layout::Rect;
use ratatui::text::Line;
use ratatui::widgets::Widget;
use rstest::rstest;
use rv::gradient::Stat;
use rv::statusbar::{
    HINT, RV_ASCII, Role, Segment, View, ascii_from, ascii_from_env, render, segments,
};

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
        change: "ytskpxpw close the alias bypass".to_owned(),
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

/// What `Line::width()` claims. **Not** what ratatui paints — see [`painted`].
fn line_width(line: &Line<'_>) -> usize {
    line.width()
}

/// A cell the bar cannot possibly draw, so an untouched column is obvious. The
/// bar paints every cell with an explicit foreground and background, and
/// ratatui blanks the hidden half of a wide grapheme to a plain space, so
/// nothing it draws can be mistaken for this.
const UNPAINTED: Cell = Cell::new("\u{2588}");

/// How much room is left to the right of the bar when it is painted, so that a
/// bar which is too *long* is as visible as one which is too short.
const PROBE: u16 = 8;

/// The columns ratatui actually paints, which is not what [`line_width`] says.
///
/// `Line::width()` measures with `unicode-width`, which gives a C0 control one
/// column; the renderer walks graphemes and drops every one that holds a
/// control character, so it gives that same control none. A bar proved exact
/// against `Line::width()` can therefore still leave its last column unpainted
/// and let the pane underneath show through. This paints the line into a buffer
/// wider than the bar and reports `(the unbroken run of columns from the
/// left-hand end, every column painted anywhere)` — equal unless the bar left a
/// hole in itself, and both equal to the requested width unless it came up
/// short or ran over.
fn painted(line: &Line<'_>, width: u16) -> (usize, usize) {
    let area = Rect::new(0, 0, width.saturating_add(PROBE), 1);
    let mut buffer = Buffer::filled(area, UNPAINTED);
    line.render(area, &mut buffer);
    let cells = buffer.content();
    let run = cells
        .iter()
        .position(|cell| *cell == UNPAINTED)
        .unwrap_or(cells.len());
    let total = cells.iter().filter(|cell| **cell != UNPAINTED).count();
    (run, total)
}

/// The bar as the terminal receives it: the symbol in every cell of the row
/// ratatui painted, so "at the right-hand end" can be asked of columns rather
/// than of a concatenation of spans.
fn painted_text(line: &Line<'_>, width: u16) -> String {
    let area = Rect::new(0, 0, width, 1);
    let mut buffer = Buffer::filled(area, Cell::EMPTY);
    line.render(area, &mut buffer);
    buffer.content().iter().map(Cell::symbol).collect()
}

/// The characters of `text` a terminal can actually show. ratatui refuses to
/// paint a grapheme holding a control character, so this is what a segment
/// contributes to the bar however it was spelled.
fn printable(text: &str) -> String {
    text.chars().filter(|c| !c.is_control()).collect()
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
fn the_bar_carries_its_segments_in_reading_order() {
    assert_eq!(
        roles(&sample_segments()),
        [
            Role::Mode,
            Role::Position,
            // Narrower than the review and wider than one file, and read
            // together with the scope beside it.
            Role::Change,
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
    let text = line_text(&render(&sample_segments(), 130, false));
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
    // Wide enough for every segment but the status, which is what makes this
    // about the ranking rather than about arithmetic: at 130 the whole bar fits.
    let text = line_text(&render(&sample_segments(), 108, false));
    assert!(
        !text.contains("saved comment"),
        "the status goes first — it will be untrue in eight seconds anyway: {text}"
    );
    assert!(
        text.contains("trunk()..@"),
        "the scope is still there: {text}"
    );
    assert!(
        text.contains("ytskpxpw"),
        "and so is the change the cursor is in: {text}"
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
#[case::glyphs(false)]
#[case::ascii(true)]
fn the_hint_is_drawn_at_the_right_hand_end(#[case] ascii: bool) {
    // Not merely present. The hint is the one segment that is right-aligned,
    // and a reviewer looks for `? help` in the corner the way they look for a
    // clock there. A hint that had drifted into the left-hand run would satisfy
    // every "is it on the bar" assertion and still be in the wrong place, with
    // the bar's empty middle trailing off the right-hand end after it.
    let painted = painted_text(&render(&sample_segments(), 100, ascii), 100);
    assert!(
        painted.ends_with(&format!(" {HINT} ")),
        "the last columns of the row are the hint: {painted}"
    );
    assert!(
        painted.starts_with(" BROWSE "),
        "and the first are the mode, at the other end: {painted}"
    );
    let block = format!(" {HINT} ");
    let middle = painted[..painted.len() - block.len()].trim_end_matches(ARROW_LEFT);
    assert!(
        middle.ends_with(' '),
        "with the bar's empty middle between them, not another segment: {painted}"
    );
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

/// Tells a re-executed copy of this test binary which half of the environment
/// check it is running, and stops it re-executing itself again.
const CHILD: &str = "RV_STATUSBAR_ENV_CHILD";

#[test]
fn the_switch_is_the_variable_the_spec_names() {
    // The *name* is half the contract, and it is the half no in-process test
    // can see. `ascii_from_env` reading `RV_ASCI` would agree with
    // `ascii_from(var_os("RV_ASCI"))` perfectly, satisfy every test below, and
    // leave every reviewer's `RV_ASCII=1` doing nothing at all. Only a process
    // that has the variable actually set can tell the two spellings apart, and
    // setting one in a threaded test binary is undefined behaviour in Rust
    // 2024 — so the check runs in a child of this process, which is given the
    // name as a literal rather than through the constant it is meant to pin.
    assert_eq!(
        RV_ASCII, "RV_ASCII",
        "the name the spec, the README and the reviewer all use"
    );

    if let Some(marker) = std::env::var_os(CHILD) {
        assert_eq!(
            ascii_from_env(),
            marker == *OsStr::new("set"),
            "the child sees the switch its parent set"
        );
        return;
    }

    for marker in ["set", "unset"] {
        let mut child = Command::new(std::env::current_exe().expect("this test binary"));
        child
            .args([
                "--exact",
                "--test-threads=1",
                "the_switch_is_the_variable_the_spec_names",
            ])
            .env(CHILD, marker);
        if marker == "set" {
            child.env("RV_ASCII", "1");
        } else {
            child.env_remove("RV_ASCII");
        }

        let output = child.output().expect("re-run this test binary");
        let log = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success(),
            "with RV_ASCII {marker} the child failed:\n{log}{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            log.contains("1 passed"),
            "the child ran the check rather than filtering it away:\n{log}"
        );
    }
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

#[rstest]
#[case::nul("\u{0}")]
#[case::escape("\u{1b}")]
#[case::carriage_return_line_feed("\r\n")]
#[case::delete("\u{7f}")]
#[case::c1("\u{9b}")]
fn a_control_character_costs_the_bar_no_column(#[case] smuggled: &str) {
    // The case the width property could not reach until its alphabet grew, kept
    // here by name as well: a repository can hand rv a path with an escape in
    // it, `unicode-width` charges a column for it and ratatui paints none, so a
    // bar that trusted the first measurement would be short by exactly the
    // number of controls in it. Every column of the row is painted, and the
    // control never reaches the terminal.
    let bar = segments(&View {
        file: Some(&format!("src/{smuggled}app.rs")),
        status: &format!("saved{smuggled} comment"),
        ..sample_view()
    });
    for width in [24u16, 60, 100] {
        for ascii in [false, true] {
            let rendered = render(&bar, width, ascii);
            assert_eq!(
                painted(&rendered, width),
                (usize::from(width), usize::from(width)),
                "at width {width} (ascii {ascii}) the bar is not the width it was asked for"
            );
            assert!(
                !painted_text(&rendered, width).chars().any(char::is_control),
                "and nothing a file name smuggled in reached the terminal"
            );
        }
    }
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

/// Printable ASCII, a few CJK ideographs, a few emoji **and the control
/// characters** — one column, two columns, a surrogate pair's worth of bytes,
/// and the case where the two measurements disagree.
///
/// The controls are the point. A segment's text is a file path, a revset or a
/// status line built from one, and a repository can hand rv a path with a
/// `\x1b` in it. `unicode-width` gives every character below `U+00A1` one
/// column, controls included, so `Line::width()` counts them; ratatui's
/// renderer drops every grapheme holding a control and paints none. Without
/// them in the alphabet the width property cannot reach the one case where
/// measuring and painting come to different answers.
///
/// The Private Use Area is still deliberately excluded: the powerline glyphs
/// live there and a segment that contained one would make the `RV_ASCII`
/// property meaningless.
fn any_segment() -> impl Strategy<Value = Segment> {
    (
        any_role(),
        "[ -~\\x{0}-\\x{1f}\\x{7f}-\\x{9f}\\x{4e00}-\\x{4e05}\\x{1f600}-\\x{1f602}]{0,24}",
    )
        .prop_map(|(role, text)| Segment { text, role })
}

proptest! {
    /// The property. One column too many and ratatui drops the overflow
    /// silently; one column too few and the row beneath shows through the gap.
    ///
    /// Asserted against the columns ratatui *paints*, not against
    /// `Line::width()`. The two are not the same function: `Line::width()` asks
    /// `unicode-width`, which gives every character below `U+00A1` one column
    /// — a `\x1b` smuggled in by a file path included — while the renderer
    /// walks graphemes and refuses to draw any that holds a control. A bar
    /// measured by the first and drawn by the second comes up one column short
    /// per control character, and `Line::width()` reports it as perfect. Both
    /// are checked, because the claimed width is what ratatui compares against
    /// the `Rect` before deciding whether to truncate.
    #[test]
    fn the_bar_is_exactly_the_width_it_was_asked_for(
        segments in prop::collection::vec(any_segment(), 0..8),
        width in 0u16..240,
        ascii in any::<bool>(),
    ) {
        let rendered = render(&segments, width, ascii);
        let (run, total) = painted(&rendered, width);
        prop_assert_eq!(run, usize::from(width), "painted columns, from the left");
        prop_assert_eq!(total, usize::from(width), "and nothing painted beyond them");
        prop_assert_eq!(line_width(&rendered), usize::from(width), "and it says so too");
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
    ///
    /// "Every one of its characters" means every one the terminal can show:
    /// ratatui will not paint a control character whatever the bar does with
    /// it, so a segment's contribution is [`printable`] of its text. Dropping a
    /// character nothing could draw is not truncation; dropping one that could
    /// be drawn is what this forbids.
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
                padding || segments.iter().any(|segment| content == format!(" {} ", printable(&segment.text))),
                "`{}` is not a whole segment: {:?}", content, rendered,
            );
        }
    }
}
