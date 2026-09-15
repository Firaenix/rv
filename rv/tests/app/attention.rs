//! Ticking files off, flags, text search, and the column cursor's jumps.

use crossterm::event::KeyCode;
use rv::app::Focus;
use rv::app::Mode;
use rv::session;
use rv_core::model::Side;

use crate::support::*;

#[test]
fn x_ticks_the_file_folds_its_comments_and_shows_in_the_title_and_the_list() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, "look at this");
    assert!(app.collapsed().is_empty(), "a fresh comment starts open");

    app.on_key(KeyCode::Char('x')).expect("tick");

    let ticks = workspace.store().reviewed().expect("read the ticks");
    assert_eq!(ticks.len(), 1);
    assert_eq!(ticks[0].file, "a.rs");
    assert_eq!(
        ticks[0].change_id, None,
        "the diff pane shows the range's diff"
    );
    assert_eq!(app.collapsed().len(), 1, "ticking did not fold the comment");

    let text = buffer_text(&frame_at(&app, 100, 24));
    assert!(
        text.contains("✓ reviewed, 1 open"),
        "no tick in the title:\n{text}"
    );
    let sidebar = sidebar_text(
        &frame_at(&app, 100, 24),
        100,
        24,
        rv::layout::Split::default(),
    );
    assert!(
        sidebar.contains('✓'),
        "no tick on the file's row:\n{sidebar}"
    );

    app.on_key(KeyCode::Char('x')).expect("untick");
    assert!(
        workspace.store().reviewed().expect("read").is_empty(),
        "a second x did not clear the tick"
    );
}

/// A file row under a change ticks *that change's* diff of the file, and the
/// change row rolls its files' ticks up without a tick of its own.
#[test]
fn x_under_a_change_ticks_the_changes_own_diff_and_the_change_row_rolls_up() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    app.on_key(KeyCode::Char('m')).expect("mode leader");
    app.on_key(KeyCode::Char('c')).expect("commits");
    // The commits tab opens with its cursor on the selected file's row, under
    // the change that touched it.
    // The change that owns the files: "first change", below the empty one.
    let change = app
        .changes()
        .iter()
        .position(|change| change.description.starts_with("first change"))
        .expect("the change with files");
    let change_id = app.changes()[change].change_id.clone();

    app.on_key(KeyCode::Char('x')).expect("tick");
    let ticks = workspace.store().reviewed().expect("read the ticks");
    assert_eq!(ticks.len(), 1);
    assert_eq!(
        ticks[0].change_id.as_deref(),
        Some(change_id.as_str()),
        "the tick is not scoped to the change"
    );
    assert!(
        app.reviewed_mark(&ticks[0].file, None).is_none(),
        "a per-change tick leaked into the range's"
    );
    assert_eq!(
        app.change_reviewed(change),
        None,
        "one of two files ticked is not a reviewed change"
    );

    app.on_key(KeyCode::Down).expect("onto the second file");
    app.on_key(KeyCode::Char('x')).expect("tick");
    assert_eq!(workspace.store().reviewed().expect("read").len(), 2);
    assert_eq!(
        app.change_reviewed(change),
        Some(rv::app::reviewed::Freshness::Current),
        "every file ticked did not roll up"
    );
    let sidebar = sidebar_text(
        &frame_at(&app, 100, 24),
        100,
        24,
        rv::layout::Split::default(),
    );
    assert_eq!(sidebar.matches('✓').count(), 3, "{sidebar}");
}

#[test]
fn a_tick_survives_reopening_and_seeds_the_fold() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, "folded on return");
    app.on_key(KeyCode::Char('x')).expect("tick");

    let reopened = workspace.app();
    assert_eq!(
        reopened.collapsed().len(),
        1,
        "the fold was not seeded from the tick"
    );
    assert!(reopened.shown_reviewed().is_some());
}

#[test]
fn a_tick_goes_stale_when_the_file_changes_under_it() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    app.on_key(KeyCode::Char('x')).expect("tick");

    workspace.write("a.rs", "fn a() {\n    let x = 2;\n}\n");
    workspace.jj(&["status"]);
    let reopened = workspace.app();
    assert_eq!(
        reopened.shown_reviewed(),
        Some(rv::app::reviewed::Freshness::Changed),
        "a changed file still claims a current review"
    );
    let text = buffer_text(&frame_at(&reopened, 100, 24));
    assert!(
        text.contains("≈ reviewed"),
        "no stale mark in the title:\n{text}"
    );
}

#[test]
fn a_flag_from_the_cli_renders_under_its_line_and_g_f_jumps_to_it() {
    let workspace = Fixture::new();
    let review = session::read(workspace.root(), None, None).expect("read the review");
    session::flags::add_flag(&review, "a.rs", Side::Right, 2, "the initialiser changed")
        .expect("flag");

    let mut app = workspace.app();
    let text = buffer_text(&frame_at(&app, 100, 24));
    assert!(
        text.contains("⚑ the initialiser changed"),
        "the flag is not drawn under its line:\n{text}"
    );

    app.on_key(KeyCode::Char('g')).expect("goto leader");
    app.on_key(KeyCode::Char('f')).expect("next flag");
    assert_eq!(app.focus(), Focus::Diff);
    let line = app.selected_line().expect("a line under the cursor");
    assert_eq!(line.right, Some(2), "g f did not land on the flagged line");
}

/// The Flags tab lists every flag under its file; `Enter` jumps to it, `A`
/// acknowledges it, and the file's row in the Files tab carries `⚑` until
/// it has been looked at.
#[test]
fn the_flags_tab_lists_flags_and_the_file_list_marks_their_files() {
    let workspace = Fixture::new();
    let review = session::read(workspace.root(), None, None).expect("read the review");
    session::flags::add_flag(&review, "b.rs", Side::Right, 3, "check the sum").expect("flag");
    let mut app = workspace.app();

    let sidebar = sidebar_text(
        &frame_at(&app, 100, 24),
        100,
        24,
        rv::layout::Split::default(),
    );
    let flagged: Vec<&str> = sidebar.lines().filter(|row| row.contains('⚑')).collect();
    assert_eq!(flagged.len(), 1, "one file carries a flag:\n{sidebar}");
    assert!(flagged[0].contains("b.rs"), "{sidebar}");

    app.on_key(KeyCode::Char('m')).expect("mode leader");
    app.on_key(KeyCode::Char('F')).expect("flags");
    assert_eq!(app.sidebar_tab(), rv::app::SidebarTab::Flags);
    let frame = buffer_text(&frame_at(&app, 100, 24));
    assert!(frame.contains("Flags (1 open of 1)"), "{frame}");
    assert!(frame.contains(":3 ⚑ check the sum"), "{frame}");

    app.on_key(KeyCode::Enter).expect("jump");
    assert_eq!(app.focus(), Focus::Diff);
    assert_eq!(app.file_index(), 1);
    assert_eq!(app.selected_line().expect("a line").right, Some(3));

    app.on_key(KeyCode::Char('m')).expect("mode leader");
    app.on_key(KeyCode::Char('F')).expect("back to the flags");
    app.on_key(KeyCode::Char('A'))
        .expect("acknowledge from the browser");
    assert!(workspace.store().flags().expect("read")[0].acknowledged);
    let frame = buffer_text(&frame_at(&app, 100, 24));
    assert!(frame.contains("Flags (0 open of 1)"), "{frame}");

    app.on_key(KeyCode::Char('m')).expect("mode leader");
    app.on_key(KeyCode::Char('f')).expect("files");
    let sidebar = sidebar_text(
        &frame_at(&app, 100, 24),
        100,
        24,
        rv::layout::Split::default(),
    );
    assert!(
        !sidebar.contains('⚑'),
        "an acknowledged flag still marks its file:\n{sidebar}"
    );
}

/// `d` in the Flags tab deletes the browsed flag, after the same question.
#[test]
fn d_in_the_flags_tab_deletes_the_browsed_flag_after_confirming() {
    let workspace = Fixture::new();
    let review = session::read(workspace.root(), None, None).expect("read the review");
    session::flags::add_flag(&review, "a.rs", Side::Right, 1, "gone soon").expect("flag");
    let mut app = workspace.app();
    app.on_key(KeyCode::Char('m')).expect("mode leader");
    app.on_key(KeyCode::Char('F')).expect("flags");
    // `c` collapses straight onto `d`: delete is the one comment verb live
    // on a flag, so the leader does not wait for a second key.
    app.on_key(KeyCode::Char('c')).expect("comment leader");
    assert!(
        app.status().contains("delete flag at a.rs:1"),
        "{}",
        app.status()
    );
    app.on_key(KeyCode::Char('y')).expect("confirm");
    assert!(workspace.store().flags().expect("read").is_empty());
}

#[test]
fn shift_f_writes_a_flag_and_shift_a_acknowledges_it() {
    let workspace = Fixture::new();
    let mut app = workspace.app();

    app.on_key(KeyCode::Char('F')).expect("flag");
    assert_eq!(app.mode(), Mode::Flag);
    for character in "why here".chars() {
        app.on_key(KeyCode::Char(character)).expect("type");
    }
    app.on_key(KeyCode::Enter).expect("save");

    let flags = workspace.store().flags().expect("read the flags");
    assert_eq!(flags.len(), 1);
    assert_eq!(flags[0].reason, "why here");
    assert!(!flags[0].acknowledged);

    app.on_key(KeyCode::Char('A')).expect("ack");
    let flags = workspace.store().flags().expect("read the flags");
    assert!(flags[0].acknowledged, "A did not acknowledge the flag");
    let text = buffer_text(&frame_at(&app, 100, 24));
    assert!(
        text.contains("⚐ why here"),
        "an acknowledged flag is not hollow:\n{text}"
    );
}

#[test]
fn slash_finds_text_and_n_walks_the_matches() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    // Onto b.rs, whose `let y` and `let z` are two matches for "let".
    app.on_key(KeyCode::Char(']')).expect("next file");

    app.on_key(KeyCode::Char('/')).expect("search");
    assert_eq!(app.mode(), Mode::Search);
    for character in "let".chars() {
        app.on_key(KeyCode::Char(character)).expect("type");
    }
    app.on_key(KeyCode::Enter).expect("find");
    assert_eq!(app.mode(), Mode::Browse);
    let first = app.selected_line().expect("a line").right;
    assert_eq!(first, Some(2), "Enter did not land on the first match");

    app.on_key(KeyCode::Char('n')).expect("next");
    assert_eq!(app.selected_line().expect("a line").right, Some(3));
    app.on_key(KeyCode::Char('n')).expect("wrap");
    assert_eq!(
        app.selected_line().expect("a line").right,
        Some(2),
        "n did not wrap"
    );
    app.on_key(KeyCode::Char('N')).expect("back, wrapping");
    assert_eq!(app.selected_line().expect("a line").right, Some(3));
    assert!(app.status().contains("match 2 of 2"), "{}", app.status());
}

#[test]
fn l_walks_the_column_cursor_by_word_and_g_r_visits_each_reference() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    // Line 2 of a.rs is `let x = 1;`: words let, x, 1.
    app.on_key(KeyCode::Down).expect("onto line 2");
    assert_eq!(app.word_under_cursor().as_deref(), Some("let"));
    app.on_key(KeyCode::Char('l')).expect("next word");
    assert_eq!(app.word_under_cursor().as_deref(), Some("x"));
    app.on_key(KeyCode::Char('h')).expect("prev word");
    assert_eq!(app.word_under_cursor().as_deref(), Some("let"));

    // `let` appears on a.rs:2, b.rs:2 and b.rs:3; g r walks them in order.
    app.on_key(KeyCode::Char('g')).expect("goto");
    app.on_key(KeyCode::Char('r')).expect("reference");
    assert_eq!(app.file_index(), 1);
    assert_eq!(app.selected_line().expect("a line").right, Some(2));
    assert_eq!(app.word_under_cursor().as_deref(), Some("let"));
    app.on_key(KeyCode::Char('g')).expect("goto");
    app.on_key(KeyCode::Char('r')).expect("reference");
    assert_eq!(app.selected_line().expect("a line").right, Some(3));
    app.on_key(KeyCode::Char('g')).expect("goto");
    app.on_key(KeyCode::Char('r')).expect("wraps");
    assert_eq!(app.file_index(), 0);
}

#[test]
fn g_d_jumps_to_the_definition_of_the_word_under_the_cursor() {
    let workspace = Fixture::new();
    workspace.write("c.rs", "fn c() {\n    a();\n}\n");
    workspace.jj(&["status"]);
    let mut app = workspace.app();
    // Open c.rs (sorted last) and put the cursor on `a();`.
    app.on_key(KeyCode::Char(']')).expect("b.rs");
    app.on_key(KeyCode::Char(']')).expect("c.rs");
    app.on_key(KeyCode::Down).expect("onto line 2");
    assert_eq!(app.word_under_cursor().as_deref(), Some("a"));

    app.on_key(KeyCode::Char('g')).expect("goto");
    app.on_key(KeyCode::Char('d')).expect("definition");
    assert_eq!(app.file_index(), 0, "g d did not go to a.rs");
    assert_eq!(app.selected_line().expect("a line").right, Some(1));
}
