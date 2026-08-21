//! Full-file context: every line of the file, changed lines still marked,
//! everything else plain context — spec §4.4/§4.5 of
//! `docs/superpowers/specs/2026-08-21-rv-full-file-context-design.md`.

use crossterm::event::KeyCode;
use rv::layout::Split;
use rv::ui;
use rv_core::diff::DiffSource;
use rv_core::diff::LineKind;

use crate::support::*;

/// A context line renders with **no** diff background — the same claim
/// `ui::line_background` already makes, checked here against a real frame —
/// and the changed line beside it still keeps its wash: full context adds
/// rows around a hunk, it does not erase the hunk's own marking.
#[test]
fn a_context_line_carries_no_diff_wash_and_the_changed_line_still_does() {
    assert_eq!(
        ui::line_background(LineKind::Context, false),
        None,
        "an unselected context line must carry no background at all"
    );
    // `Fixture::rewritten` edits one line of an *existing* file, so
    // difftastic reports a changed-only diff (a `Removed`/`Added` pair) and
    // the surrounding lines — `fn rewrite() {` and the closing `}` — are
    // genuinely full-file context this feature adds, not part of a
    // whole-file creation where every line is legitimately `Added`. The
    // closing `}` is checked rather than the opening line: the reviewer
    // opens with the cursor on the first hunk, and a *selected* context
    // line legitimately carries the theme's own dark-grey band — that is
    // the selection highlight, not a diff wash, and is a different claim
    // from the one this test is making.
    let workspace = Fixture::rewritten();
    let mut app = workspace.app_from("@--");
    app.finish_painting();

    let frame = frame_at(&app, 100, 24);
    let area = areas(100, 24, Split::default()).diff;
    let closing = row_holding(&frame, "}");

    assert_eq!(
        diff_bg(&frame, area, u16::try_from(closing).expect("a small row")),
        None,
        "the untouched closing line carries a diff background:\n{}",
        buffer_text(&frame)
    );

    let added = row_of_sigil(&frame, area, '+');
    assert!(
        diff_bg(&frame, area, added).is_some(),
        "the changed line lost its background under full context:\n{}",
        buffer_text(&frame)
    );
}

/// The full file is on screen, not only the lines difftastic reported as
/// different — the reviewer's actual complaint. `SOURCE` is three lines;
/// showing only the changed line would carry one row, not three.
#[test]
fn the_whole_file_is_on_screen_not_only_the_diff() {
    let workspace = Fixture::new();
    let app = workspace.app();
    assert_eq!(
        app.displayed_lines().len(),
        3,
        "the full three-line file is not what the pane is asked to draw: {:?}",
        app.displayed_lines()
    );
    let frame = frame_at(&app, 100, 24);
    let text = buffer_text(&frame);
    assert!(
        text.contains("fn a() {"),
        "the opening line is missing:\n{text}"
    );
    assert!(text.contains('}'), "the closing line is missing:\n{text}");
}

/// A comment box still lands on the correct row after the row stream grew
/// from full-file context: the box hangs directly under the line it is
/// anchored to, wherever that line now sits among the file's other rows.
#[test]
fn a_comment_box_lands_on_the_correct_row_once_the_stream_has_grown() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    assert!(
        app.displayed_lines().len() > 1,
        "need more than one row for this to prove anything"
    );

    select_line(&mut app, |line| line.text.contains("let x = 1;"));
    write_comment(&mut app, "a note on the changed line");

    let buffer = frame_at(&app, 100, 24);
    let rows = rows_of(&buffer);
    let anchored = row_holding(&buffer, "let x = 1;");
    assert!(
        rows[anchored + 1].contains('╭'),
        "the box does not open directly under the line it annotates, now that \
         full context sits above and below it:\n{}",
        buffer_text(&buffer)
    );
    assert!(
        rows[anchored + 2].contains("a note on the changed line"),
        "the body is not under the box's own line:\n{}",
        buffer_text(&buffer)
    );
}

/// A binary file: `merge_context` is never called (there is no text to
/// walk), the pane still says so rather than drawing rows, and
/// `App::context_bailed` reports `false` — the merge was never attempted,
/// so it cannot have been declined.
#[test]
fn a_binary_file_is_unaffected_by_full_context() {
    let workspace = Fixture::new();
    std::fs::write(workspace.root().join("logo.bin"), [0u8, 1, 2, 3]).expect("write binary");
    workspace.jj(&["describe", "-m", "add a binary file"]);
    workspace.jj(&["new"]);

    let mut app = workspace.app();
    for _ in 0..app.files().len() {
        if app
            .selected_file()
            .is_some_and(|file| file.path == "logo.bin")
        {
            break;
        }
        app.on_key(KeyCode::Char(']')).expect("next file");
    }
    assert_eq!(
        app.selected_file().map(|file| file.path.as_str()),
        Some("logo.bin")
    );

    let diff = app.selected_diff().expect("a diff").clone();
    assert_eq!(diff.source, DiffSource::Binary, "{diff:?}");
    assert!(
        app.displayed_lines().is_empty(),
        "a binary file's displayed lines are not empty: {:?}",
        app.displayed_lines()
    );
    assert!(
        !app.context_bailed(),
        "a binary file never attempts a merge"
    );

    let frame = frame_at(&app, 100, 24);
    assert!(
        buffer_text(&frame).contains("binary file, not shown by line"),
        "the pane no longer says the file is binary:\n{}",
        buffer_text(&frame)
    );
}

/// A suppressed diff with **no** lines (difftastic's chunk-less `unchanged`
/// status): `merge_context` is not called — there is no anchor to walk
/// from — so `context_bailed` is `false`, and the pane still shows the
/// "no semantic change" sentence rather than a row list.
#[test]
fn a_suppressed_empty_diff_never_attempts_a_merge() {
    let workspace = Fixture::pure_rename();
    let app = workspace.app_from("@--");
    let diff = app.selected_diff().expect("a diff").clone();
    assert!(diff.suppressed, "{diff:?}");
    assert!(diff.lines.is_empty(), "{diff:?}");
    assert!(app.displayed_lines().is_empty());
    assert!(
        !app.context_bailed(),
        "a suppressed empty diff never attempts a merge"
    );

    let frame = frame_at(&app, 100, 24);
    assert!(
        buffer_text(&frame).contains("no semantic change"),
        "the pane no longer says the diff is suppressed:\n{}",
        buffer_text(&frame)
    );
}

/// §4.6: a reformatted region that the syntax-aware merge cannot pair
/// line-for-line triggers rv's `--byte-limit 0` retry against difftastic's
/// line-oriented engine. The retry succeeds on this fixture, so the pane
/// swaps to the full-context view with a title suffix that names the
/// engine that produced it, and `context_bailed` is `false`.
#[test]
fn a_reformatted_region_recovers_through_the_line_oriented_retry() {
    let workspace = Fixture::reformatted_gap();
    let app = workspace.app_from("@--");
    let diff = app.selected_diff().expect("a diff").clone();
    assert!(
        matches!(diff.source, DiffSource::Difftastic { .. }),
        "is difft on PATH? {diff:?}"
    );
    assert!(!diff.suppressed, "{diff:?}");
    assert!(!diff.lines.is_empty(), "{:?}", diff.lines);

    // The retry ran and produced a mergeable answer, so the syntax-aware
    // decline (§3) no longer surfaces as "context unavailable" — that suffix
    // is reserved for the case where the retry *also* declined.
    assert!(
        !app.context_bailed(),
        "the §4.6 retry should have recovered on this fixture, so bailed \
         must be false: displayed_lines={:?}",
        app.displayed_lines()
    );
    // The pane's line count grew past the changed-only view: the retry
    // supplied context lines the syntax-aware answer did not.
    assert!(
        app.displayed_lines().len() > diff.lines.len(),
        "the retry did not add context lines: displayed={:?} diff={:?}",
        app.displayed_lines(),
        diff.lines,
    );
    // The DiffSource is mutated to carry `line_oriented: true` — that is
    // what the title suffix reads.
    let source_after = app.selected_diff().expect("a diff").source.clone();
    assert!(
        matches!(
            source_after,
            DiffSource::Difftastic {
                line_oriented: true,
                ..
            }
        ),
        "the file's DiffSource does not carry line_oriented=true: {source_after:?}"
    );

    let frame = frame_at(&app, 120, 24);
    assert!(
        buffer_text(&frame).contains("full context (line diff)"),
        "the pane does not name the line-oriented retry:\n{}",
        buffer_text(&frame)
    );
    assert!(
        !buffer_text(&frame).contains("full context unavailable"),
        "the pane still says context unavailable despite a successful retry:\n{}",
        buffer_text(&frame)
    );
}
