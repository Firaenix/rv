//! The file list as a counted tree.

use crossterm::event::KeyCode;
use ratatui::style::Color;
use rv::app::App;
use rv::gradient;
use rv::layout::Split;

use crate::support::*;

/// `t` flips the file list between whole paths and a directory tree, and the
/// pane says which it is showing.
///
/// A chain of single-child directories is one row: `docs/specs` and not `docs`
/// over `specs`. A 29-file review has perhaps 40 rows to spend, and a tree that
/// spent half of them on punctuation would be worse than the list it replaced.
#[test]
fn t_toggles_the_sidebar_between_a_list_and_a_tree() {
    let workspace = Fixture::nested();
    let mut app = workspace.app();

    let list = sidebar_text(&frame_at(&app, 100, 24), 100, 24, Split::default());
    assert!(
        list.contains("docs/specs/a.md"),
        "the flat list names whole paths:\n{list}"
    );
    assert!(
        sidebar_shape(&frame_at(&app, 100, 24)).contains("list"),
        "the pane does not say it is a list: {:?}",
        sidebar_shape(&frame_at(&app, 100, 24))
    );

    app.on_key(KeyCode::Char('t')).expect("t");
    let tree = sidebar_text(&frame_at(&app, 100, 24), 100, 24, Split::default());
    assert_ne!(list, tree, "the sidebar did not change shape");
    assert!(
        tree.contains("docs/specs"),
        "the single-child chain is one row:\n{tree}"
    );
    assert!(
        !tree.contains("docs/specs/a.md"),
        "a file under it is still named by its whole path:\n{tree}"
    );
    assert!(
        tree.contains("a.md") && tree.contains("b.md") && tree.contains("top.rs"),
        "the tree lost a file the flat list had:\n{tree}"
    );
    assert!(
        sidebar_shape(&frame_at(&app, 100, 24)).contains("tree"),
        "the pane does not say it is a tree: {:?}",
        sidebar_shape(&frame_at(&app, 100, 24))
    );

    app.on_key(KeyCode::Char('t')).expect("t again");
    assert_eq!(
        sidebar_text(&frame_at(&app, 100, 24), 100, 24, Split::default()),
        list,
        "t is a toggle, not a one-way door"
    );
}

/// `s` on a directory row folds it away — the project's one verb for *fold the
/// thing under the cursor*, which is already what it means for a comment box
/// and for a browsed comment.
#[test]
fn s_folds_a_directory_row_and_hides_the_files_under_it() {
    let workspace = Fixture::nested();
    let mut app = workspace.app();
    app.on_key(KeyCode::Char('t')).expect("the tree");
    app.on_key(KeyCode::Left).expect("focus the file list");
    // The review opens on the first file, which is under `docs/specs`; one
    // step up is the directory row itself.
    app.on_key(KeyCode::Up).expect("onto the directory row");

    app.on_key(KeyCode::Char('s')).expect("fold it");
    let folded = sidebar_text(&frame_at(&app, 100, 24), 100, 24, Split::default());
    assert!(
        folded.contains("docs/specs"),
        "the directory row itself is gone:\n{folded}"
    );
    assert!(
        !folded.contains("a.md") && !folded.contains("b.md"),
        "its files are still on screen:\n{folded}"
    );
    assert!(
        folded.contains("top.rs") && folded.contains("lib.rs"),
        "folding one directory took its siblings with it:\n{folded}"
    );

    app.on_key(KeyCode::Char('s')).expect("unfold it");
    assert!(
        sidebar_text(&frame_at(&app, 100, 24), 100, 24, Split::default()).contains("a.md"),
        "s did not put the directory back"
    );
}

/// A row that holds others says what the whole subtree costs, folded or not.
///
/// A folded row that hid its own weight would be a row you have to expand to
/// judge, which is the work folding it was meant to save.
#[test]
fn a_directory_row_carries_its_whole_subtrees_count() {
    let workspace = Fixture::nested();
    let mut app = workspace.app();
    app.on_key(KeyCode::Char('t')).expect("the tree");

    let frame = frame_at(&app, 100, 24);
    let row = sidebar_row_for(&frame, "docs/specs");
    let text: String = (0..100).map(|x| frame[(x, row)].symbol()).collect();
    assert!(
        text.contains("+15"),
        "a 10-line file and a 5-line file under one row add up to 15: {text:?}"
    );
}

/// Every row says what it costs to review.
#[test]
fn every_row_shows_what_it_costs_to_review() {
    let workspace = Fixture::mixed();
    let app = workspace.app_from("@--");

    let text = sidebar_text(&frame_at(&app, 100, 24), 100, 24, Split::default());
    assert!(text.contains("+40"), "additions are shown:\n{text}");
    assert!(text.contains("25"), "and so are removals:\n{text}");
}

/// A row too narrow for both gives up its counts and keeps its path.
///
/// The path is the row's identity and the counts are context. The change bar
/// has already gone by this point — see
/// `the_bar_is_dropped_before_the_counts_are` — so the order in which the pane
/// gives things up is bar, counts, path, each more the row's identity than the
/// last.
#[test]
fn a_narrow_sidebar_drops_the_counts_before_the_path() {
    let workspace = Fixture::mixed();
    let mut app = workspace.app_from("@--");
    for _ in 0..30 {
        app.on_key(KeyCode::Char('<')).expect("squeeze the sidebar");
    }

    let split = app.split();
    let text = sidebar_text(&frame_at(&app, 60, 24), 60, 24, split);
    assert!(
        text.contains("added"),
        "the path went before the counts did:\n{text}"
    );
    assert!(
        !text.contains("+40"),
        "the counts survived a column that cannot hold both:\n{text}"
    );
}

/// `o` cycles the order, and the pane names it — a list whose order you cannot
/// see is a list you cannot trust.
#[test]
fn o_cycles_the_order_and_the_sidebar_says_which() {
    let workspace = Fixture::nested();
    let mut app = workspace.app();
    let shape = |app: &App| sidebar_shape(&frame_at(app, 100, 24));

    assert!(shape(&app).contains("natural"), "{:?}", shape(&app));
    app.on_key(KeyCode::Char('o')).expect("o");
    assert!(shape(&app).contains("added"), "{:?}", shape(&app));
    app.on_key(KeyCode::Char('o')).expect("o");
    assert!(shape(&app).contains("removed"), "{:?}", shape(&app));
    app.on_key(KeyCode::Char('o')).expect("o");
    assert!(
        shape(&app).contains("natural"),
        "it does not cycle: {:?}",
        shape(&app)
    );
}

/// ...and the rows actually move when it does.
#[test]
fn sorting_by_additions_puts_the_biggest_file_first() {
    let workspace = Fixture::nested();
    let mut app = workspace.app();

    let natural = sidebar_filled(&frame_at(&app, 100, 24), 100, 24, Split::default());
    assert!(
        natural[0].contains("a.md"),
        "the natural order is path order: {natural:?}"
    );

    app.on_key(KeyCode::Char('o')).expect("order by additions");
    let by_size = sidebar_filled(&frame_at(&app, 100, 24), 100, 24, Split::default());
    assert!(
        by_size[0].contains("top.rs"),
        "the 50-line file is not first: {by_size:?}"
    );
    let mut sorted = by_size.clone();
    sorted.sort();
    let mut was = natural.clone();
    was.sort();
    assert_eq!(sorted, was, "an order that loses a file is worse than none");
}

/// The counts carry the colour: the additions in the palette's green, the
/// removals in its red, as a foreground on the terminal's own ground.
#[test]
fn the_sidebar_colours_the_counts_by_the_shape_of_the_change() {
    let workspace = Fixture::mixed();
    let app = workspace.app_from("@--");
    let frame = frame_at(&app, 100, 24);

    let added = sidebar_row_for(&frame, "added.rs");
    let removed = sidebar_row_for(&frame, "removed.rs");
    assert_eq!(
        style_of_text(&frame, added, "+40").fg,
        Some(colour(gradient::ADDED)),
        "the additions are not the palette's green:\n{}",
        sidebar_text(&frame, 100, 24, Split::default())
    );
    assert_eq!(
        style_of_text(&frame, added, "-0").fg,
        Some(colour(gradient::REMOVED)),
        "the removals are not the palette's red"
    );
    assert_eq!(
        style_of_text(&frame, removed, "-25").fg,
        Some(colour(gradient::REMOVED)),
        "and the other row disagrees with the first"
    );
}

/// **No row is washed.** Spec §7 rules it out after two rounds of looking at
/// the running tool: a full-row wash reads as a selection and competes with the
/// real one, and even a text-width wash paints over the indentation and the
/// fold marks, which in tree mode *are* the structure. The only full-row
/// background in this pane is the selection.
#[test]
fn no_row_of_the_file_list_is_painted_over() {
    let workspace = Fixture::mixed();
    let mut app = workspace.app_from("@--");

    for focused in [false, true] {
        if focused {
            app.on_key(KeyCode::Left).expect("focus the file list");
        }
        let frame = frame_at(&app, 100, 24);
        let area = inner(areas(100, 24, Split::default()).sidebar);
        for y in area.y..area.bottom() {
            for x in area.x..area.right() {
                assert_eq!(
                    bg_of(&frame, x, y),
                    None,
                    "({x},{y}) is painted over with the file list {}:\n{}",
                    if focused { "focused" } else { "unfocused" },
                    sidebar_text(&frame, 100, 24, Split::default())
                );
            }
        }
    }
}

/// The proportion survives as a small bar beside the counts, on a row with the
/// columns to spare — a mark on the row rather than the row itself.
#[test]
fn a_row_with_room_to_spare_draws_its_proportion_as_a_bar() {
    let workspace = Fixture::mixed();
    let mut app = workspace.app_from("@--");
    for _ in 0..12 {
        app.on_key(KeyCode::Char('>')).expect("widen the sidebar");
    }

    let split = app.split();
    let frame = frame_at(&app, 120, 24);
    let area = inner(areas(120, 24, split).sidebar);
    let added = sidebar_row_for_in(&frame, area, "added.rs");
    let removed = sidebar_row_for_in(&frame, area, "removed.rs");

    let bar_of = |row: u16| -> Vec<Option<Color>> {
        (area.x..area.right())
            .filter(|x| frame[(*x, row)].symbol() == "\u{2588}")
            .map(|x| frame[(x, row)].style().fg)
            .collect()
    };
    let green = bar_of(added);
    let red = bar_of(removed);
    assert!(
        !green.is_empty(),
        "no bar on a row with room for one:\n{}",
        text_in(&frame, area)
    );
    assert!(
        green
            .iter()
            .all(|ink| *ink == Some(colour(gradient::ADDED))),
        "a file that is nothing but additions has a bar that is not all green: {green:?}"
    );
    assert!(
        red.iter()
            .all(|ink| *ink == Some(colour(gradient::REMOVED))),
        "a file that is nothing but removals has a bar that is not all red: {red:?}"
    );
}

/// ...and it is the first thing given up, ahead of the counts, which are given
/// up ahead of the path. Each is more the row's identity than the last.
#[test]
fn the_bar_is_dropped_before_the_counts_are() {
    let workspace = Fixture::nested();
    let mut app = workspace.app();

    // At the default split these paths leave no room for a bar beside them.
    let text = sidebar_text(&frame_at(&app, 100, 24), 100, 24, Split::default());
    assert!(
        !text.contains('\u{2588}'),
        "the bar was drawn by clipping the names:\n{text}"
    );
    assert!(
        text.contains("+10"),
        "and it took the counts with it:\n{text}"
    );

    // Squeezed further, the counts go too and the names stay.
    for _ in 0..30 {
        app.on_key(KeyCode::Char('<')).expect("squeeze the sidebar");
    }
    let split = app.split();
    let text = sidebar_text(&frame_at(&app, 60, 24), 60, 24, split);
    assert!(
        !text.contains("+10") && !text.contains('\u{2588}'),
        "the counts outlived the path:\n{text}"
    );
    assert!(text.contains("top.rs"), "the path went first:\n{text}");
}

/// A change with no shape says nothing: no counts, no bar, and none of the
/// palette's colours. A gradient over zero changed lines would be inventing a
/// ratio.
#[test]
fn a_pure_rename_is_left_neutral() {
    let workspace = Fixture::pure_rename();
    let app = workspace.app_from("@--");
    let frame = frame_at(&app, 100, 24);
    let area = inner(areas(100, 24, Split::default()).sidebar);

    let row = sidebar_row_for(&frame, "b.rs");
    let text = row_in(&frame, area, row);
    assert!(
        !text.contains('+') && !text.contains('\u{2588}'),
        "a rename that changed no line was counted anyway: {text:?}"
    );
    for x in area.x..area.right() {
        let ink = frame[(x, row)].style().fg;
        assert!(
            ink != Some(colour(gradient::ADDED)) && ink != Some(colour(gradient::REMOVED)),
            "column {x} of a rename that changed no line carries a change colour"
        );
    }
}
