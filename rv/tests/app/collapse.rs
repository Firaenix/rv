//! Putting the sidebar away, and folding a row with `Enter` or `Space`.

use crossterm::event::KeyCode;
use rv::app::Focus;
use rv::layout::Split;
use rv::tree::NodeKind;

use crate::support::*;

/// The bar's first cell, which is the pointer's way in and out of the sidebar.
fn chevron(app: &rv::app::App, width: u16, height: u16) -> String {
    let frame = frame_at(app, width, height);
    frame[(0, height - 1)].symbol().to_owned()
}

#[test]
fn z_puts_the_sidebar_away_and_brings_it_back() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    assert!(
        !sidebar_text(&frame_at(&app, 100, 24), 100, 24, Split::default())
            .trim()
            .is_empty(),
        "the sidebar is not showing to begin with"
    );

    app.on_key(KeyCode::Char('z')).expect("hide it");
    assert!(app.sidebar_hidden());

    app.on_key(KeyCode::Char('z')).expect("bring it back");
    assert!(!app.sidebar_hidden());
}

/// The diff takes the columns the sidebar gave up, rather than leaving a gap.
#[test]
fn the_diff_takes_the_whole_width_when_the_sidebar_is_away() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    let before = row_holding(&frame_at(&app, 100, 24), "fn a()");

    app.on_key(KeyCode::Char('z')).expect("hide it");

    let frame = frame_at(&app, 100, 24);
    let row = row_holding(&frame, "fn a()");
    assert!(
        rows_of(&frame)[row].starts_with('│'),
        "the diff does not start at the left edge: {:?}",
        rows_of(&frame)[row]
    );
    assert_eq!(row, before, "hiding the sidebar moved the diff's rows");
}

/// A hidden pane must not still hold the cursor: every key would then be acting
/// on a list nobody can see.
#[test]
fn hiding_the_sidebar_takes_the_focus_with_it() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    app.on_key(KeyCode::Left).expect("focus the sidebar");
    assert_eq!(app.focus(), Focus::Sidebar);

    app.on_key(KeyCode::Char('z')).expect("hide it");

    assert_eq!(app.focus(), Focus::Diff, "the focus stayed on a hidden pane");
}

/// The whole reason the fold exists: a phone over ssh has no comfortable `z`,
/// but it has a finger.
#[test]
fn the_chevron_in_the_bar_opens_and_closes_the_sidebar() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    // One frame first, so the click resolves against geometry that was painted.
    let _ = frame_at(&app, 100, 24);
    assert_eq!(chevron(&app, 100, 24), "‹", "the control is not in the bar");

    app.on_mouse(click(0, 23)).expect("click the chevron");

    assert!(app.sidebar_hidden(), "clicking the chevron hid nothing");
    assert_eq!(
        chevron(&app, 100, 24),
        "›",
        "the control does not point the other way once the sidebar is away"
    );

    app.on_mouse(click(0, 23)).expect("click it again");
    assert!(!app.sidebar_hidden(), "the chevron is a one-way door");
}

/// The bar keeps saying what it said: the control costs a column, not a
/// segment.
#[test]
fn the_chevron_does_not_evict_the_keymap_hint() {
    let workspace = Fixture::new();
    let app = workspace.app();

    for width in [16u16, 40, 80, 120] {
        let bar = last_row(&frame_at(&app, width, 24));
        assert!(
            bar.starts_with('‹'),
            "the control is missing at {width} columns: {bar:?}"
        );
        assert!(
            bar.contains("? help"),
            "the hint went to make room for it at {width} columns: {bar:?}"
        );
    }
}

/// `Enter` and `Space` are the keys every tree in every editor folds with.
#[rstest::rstest]
#[case::enter(KeyCode::Enter)]
#[case::space(KeyCode::Char(' '))]
fn a_directory_folds_with_enter_or_space(#[case] key: KeyCode) {
    let workspace = Fixture::nested();
    let mut app = workspace.app();
    app.on_key(KeyCode::Left).expect("focus the sidebar");
    app.on_key(KeyCode::Char('t')).expect("tree");

    // Onto the first row that holds anything.
    while !matches!(
        app.nodes().get(app.sidebar_row()).map(|node| &node.kind),
        Some(NodeKind::Dir { .. })
    ) {
        app.on_key(KeyCode::Down).expect("next row");
    }
    let before = app.nodes().len();

    app.on_key(key).expect("fold");
    let folded = app.nodes().len();
    assert!(
        folded < before,
        "folding hid nothing: {before} rows before, {folded} after"
    );

    app.on_key(key).expect("unfold");
    assert_eq!(app.nodes().len(), before, "unfolding lost rows");
}

/// On a file row there is nothing to open, and `Enter` says so by doing
/// nothing rather than by pretending.
#[test]
fn enter_on_a_file_row_folds_nothing() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    app.on_key(KeyCode::Left).expect("focus the sidebar");
    let before = app.nodes().len();

    app.on_key(KeyCode::Enter).expect("enter");

    assert_eq!(app.nodes().len(), before, "a file row folded something");
    assert_eq!(app.focus(), Focus::Sidebar, "the focus moved");
}

/// Highlighting is off the drawing thread: the diff is on screen before the
/// parse lands, and the colour arrives after.
///
/// Plain is the interim state and it is not a new one — a file whose language
/// ships no grammar has always drawn plain, so a blob whose parse has not landed
/// is, for one frame, a blob with no grammar. The claim here is that both frames
/// are drawable and the second is the coloured one.
#[test]
fn the_code_draws_before_it_is_coloured() {
    let workspace = Fixture::new();
    let mut app = workspace.app();

    // The lines are there immediately, whatever the parse is doing.
    let early = frame_at(&app, 100, 24);
    assert!(
        buffer_text(&early).contains("fn a()"),
        "the diff waited for the highlighter:\n{}",
        buffer_text(&early)
    );

    app.finish_painting();
    let painted = frame_at(&app, 100, 24);
    assert_eq!(
        buffer_text(&painted),
        buffer_text(&early),
        "the swap moved the text as well as the colour"
    );

    // And the `fn` keyword is now carrying a colour of its own.
    let row = u16::try_from(row_holding(&painted, "fn a()")).expect("a small row");
    let keyword = style_of_text(&painted, row, "fn");
    assert!(
        keyword.fg.is_some(),
        "the keyword is still unpainted after the parse landed: {keyword:?}"
    );
}
