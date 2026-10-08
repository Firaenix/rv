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

    app.on_key(KeyCode::Char('X')).expect("tick");

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

    app.on_key(KeyCode::Char('X')).expect("untick");
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

    app.on_key(KeyCode::Char('X')).expect("tick");
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
    app.on_key(KeyCode::Char('X')).expect("tick");
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
    app.on_key(KeyCode::Char('X')).expect("tick");

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
    app.on_key(KeyCode::Char('X')).expect("tick");

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

/// Arrows walk the flag rows, stepping over headings, and `Space` opens its
/// menu rather than collapsing onto the delete in it.
#[test]
fn arrows_walk_the_flags_tab_and_space_never_lands_on_delete() {
    let workspace = Fixture::new();
    let review = session::read(workspace.root(), None, None).expect("read the review");
    session::flags::add_flag(&review, "a.rs", Side::Right, 1, "one").expect("flag");
    session::flags::add_flag(&review, "a.rs", Side::Right, 2, "two").expect("flag");
    session::flags::add_flag(&review, "b.rs", Side::Right, 2, "three").expect("flag");
    let mut app = workspace.app();
    app.on_key(KeyCode::Char('m')).expect("mode leader");
    app.on_key(KeyCode::Char('F')).expect("flags");

    let reasons = |app: &rv::app::App| app.browsed_flag().map(|flag| flag.reason.clone());
    assert_eq!(reasons(&app).as_deref(), Some("one"));
    app.on_key(KeyCode::Down).expect("down");
    assert_eq!(reasons(&app).as_deref(), Some("two"));
    app.on_key(KeyCode::Down)
        .expect("down, over the b.rs heading");
    assert_eq!(reasons(&app).as_deref(), Some("three"));
    app.on_key(KeyCode::Down).expect("down at the end stays");
    assert_eq!(reasons(&app).as_deref(), Some("three"));
    app.on_key(KeyCode::Up).expect("up");
    assert_eq!(reasons(&app).as_deref(), Some("two"));
    app.on_key(KeyCode::End).expect("end");
    assert_eq!(reasons(&app).as_deref(), Some("three"));
    app.on_key(KeyCode::Home).expect("home");
    assert_eq!(reasons(&app).as_deref(), Some("one"));

    app.on_key(KeyCode::Char(' ')).expect("space");
    assert_eq!(app.mode(), Mode::Browse, "Space collapsed onto delete");
    assert_eq!(app.pending_leader(), Some(rv::app::Leader::Context));
    app.on_key(KeyCode::Esc).expect("close the menu");
    assert_eq!(workspace.store().flags().expect("read").len(), 3);
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
    // Delete is the one comment verb live on a flag, but a leader never
    // collapses onto a delete: `c` opens the menu and `d` picks it.
    app.on_key(KeyCode::Char('c')).expect("comment leader");
    assert_eq!(app.mode(), Mode::Browse, "c collapsed onto delete");
    app.on_key(KeyCode::Char('d')).expect("delete");
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

/// `g r` opens the references as a list to choose from. The cursor starts on
/// the one the old walk would have jumped to, the arrows move it, and `Enter`
/// is what lands.
#[test]
fn l_walks_the_column_cursor_by_word_and_g_r_lists_every_reference() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    // Line 2 of a.rs is `let x = 1;`: words let, x, 1.
    app.on_key(KeyCode::Down).expect("onto line 2");
    assert_eq!(app.word_under_cursor().as_deref(), Some("let"));
    app.on_key(KeyCode::Char('l')).expect("next word");
    assert_eq!(app.word_under_cursor().as_deref(), Some("x"));
    app.on_key(KeyCode::Char('h')).expect("prev word");
    assert_eq!(app.word_under_cursor().as_deref(), Some("let"));

    // `let` appears on a.rs:2, b.rs:2 and b.rs:3.
    app.on_key(KeyCode::Char('g')).expect("goto");
    app.on_key(KeyCode::Char('r')).expect("references");
    assert_eq!(app.mode(), Mode::References);
    assert_eq!(app.reference_word(), "let");
    let listed: Vec<(usize, u32)> = app
        .references()
        .iter()
        .map(|reference| (reference.file, reference.line))
        .collect();
    assert_eq!(
        listed,
        [(0, 2), (1, 2), (1, 3)],
        "every reference is listed"
    );
    assert!(
        app.references()[0].text.contains("let x = 1;"),
        "a row carries its line, so the list can be read without jumping"
    );
    assert_eq!(
        app.reference_index(),
        1,
        "the cursor opens on the next reference, where the walk used to land"
    );

    // Nothing has moved until Enter: the list is a question, not a jump.
    assert_eq!(app.file_index(), 0);

    app.on_key(KeyCode::Down).expect("down the list");
    assert_eq!(app.reference_index(), 2);
    app.on_key(KeyCode::Down).expect("wrapping round the end");
    assert_eq!(app.reference_index(), 0);
    app.on_key(KeyCode::Up).expect("back round the start");
    assert_eq!(app.reference_index(), 2);

    app.on_key(KeyCode::Enter).expect("jump");
    assert_eq!(app.mode(), Mode::Browse);
    assert_eq!(app.file_index(), 1, "Enter took the chosen reference");
    assert_eq!(app.selected_line().expect("a line").right, Some(3));
    assert_eq!(app.word_under_cursor().as_deref(), Some("let"));
}

/// Esc leaves the review where it was: a list opened by accident costs nothing.
#[test]
fn esc_closes_the_reference_list_without_moving() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    app.on_key(KeyCode::Down).expect("onto line 2");
    let before = (app.file_index(), app.line_index());

    app.on_key(KeyCode::Char('g')).expect("goto");
    app.on_key(KeyCode::Char('r')).expect("references");
    app.on_key(KeyCode::Down).expect("move the list cursor");
    app.on_key(KeyCode::Esc).expect("cancel");

    assert_eq!(app.mode(), Mode::Browse);
    assert!(
        app.references().is_empty(),
        "the list is not state to carry"
    );
    assert_eq!((app.file_index(), app.line_index()), before);
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

/// The list is what the grammar found, not what the text says. A name in a
/// comment is not a use of it, which is the difference between moving around
/// the code and jumping between lookalike words.
#[test]
fn the_reference_list_skips_a_name_a_comment_merely_mentions() {
    let workspace = Fixture::new();
    workspace.write(
        "c.rs",
        "// a() is the one to look at
fn c() {
    a();
}
",
    );
    workspace.jj(&["status"]);
    let mut app = workspace.app();
    app.on_key(KeyCode::Char(']')).expect("b.rs");
    app.on_key(KeyCode::Char(']')).expect("c.rs");
    // Line 3 is `    a();`.
    app.on_key(KeyCode::Down).expect("line 2");
    app.on_key(KeyCode::Down).expect("line 3");
    assert_eq!(app.word_under_cursor().as_deref(), Some("a"));

    app.on_key(KeyCode::Char('g')).expect("goto");
    app.on_key(KeyCode::Char('r')).expect("references");

    assert!(
        app.jump_list() == rv::app::JumpList::References,
        "a Rust file has a grammar, so the list is the grammar's"
    );
    let lines: Vec<(usize, u32)> = app
        .references()
        .iter()
        .map(|reference| (reference.file, reference.line))
        .collect();
    assert!(
        lines.contains(&(2, 3)),
        "the call site is a reference: {lines:?}"
    );
    assert!(
        !lines.contains(&(2, 1)),
        "the comment is not a use of the name: {lines:?}"
    );
}

/// A file no grammar claims still answers, and the panel says it is matching
/// text rather than pretending the name is nowhere.
#[test]
fn a_file_with_no_grammar_falls_back_to_naming_lines() {
    let workspace = Fixture::new();
    workspace.write(
        "run.sh",
        "setup
setup
",
    );
    workspace.jj(&["status"]);
    let mut app = workspace.app();
    for _ in 0..3 {
        app.on_key(KeyCode::Char(']')).expect("next file");
    }
    assert_eq!(app.word_under_cursor().as_deref(), Some("setup"));

    app.on_key(KeyCode::Char('g')).expect("goto");
    app.on_key(KeyCode::Char('r')).expect("references");

    assert!(
        app.jump_list() == rv::app::JumpList::Lines,
        "bash has no tags query in rv, so this is a text match"
    );
    assert_eq!(app.references().len(), 2, "both lines name it");
}
