//! The binding table and the `?` popup.

use crossterm::event::KeyCode;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use rstest::rstest;
use rv::app::Action;
use rv::app::BINDINGS;
use rv::app::SidebarTab;
use rv::layout::Chrome;
use rv::layout::Split;
use rv::layout::layout;

use crate::support::*;

/// The cell holding the key of the popup row that describes `what`.
///
/// Found by the description rather than by the key, because a single-character
/// key is a substring of half the screen: the row is located by the sentence
/// only it carries, and the key is then the last occurrence of `keys` in the
/// columns to its left.
fn cell_of_binding(buffer: &Buffer, keys: &str, what: &str) -> (u16, u16) {
    let rows = rows_of(buffer);
    let (y, row) = rows
        .iter()
        .enumerate()
        .find(|(_, row)| row.contains(what))
        .unwrap_or_else(|| panic!("{what:?} is not on screen:\n{}", buffer_text(buffer)));
    let at = row.find(what).expect("the row holds it");
    let before = &row[..at];
    let start = before
        .rfind(keys)
        .unwrap_or_else(|| panic!("{keys:?} is not left of {what:?} on row {y}: {row:?}"));
    let column = before[..start].chars().count();
    (
        u16::try_from(column).expect("a small column"),
        u16::try_from(y).expect("a small row"),
    )
}

/// Whether the cell at `at` is drawn dim — how the popup says a key does
/// nothing from where the cursor is.
fn is_dim(buffer: &Buffer, at: (u16, u16)) -> bool {
    buffer[at].modifier.contains(Modifier::DIM)
}

#[test]
fn question_mark_opens_the_help_and_esc_closes_it() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    assert!(
        !app.help_open(),
        "a reviewer opens on the review, not the manual"
    );

    app.on_key(KeyCode::Char('?')).expect("?");
    assert!(app.help_open());
    let frame = buffer_text(&frame_at(&app, 100, 24));
    assert!(
        frame.contains("comment"),
        "the popup lists what the keys do:\n{frame}"
    );

    app.on_key(KeyCode::Esc).expect("esc");
    assert!(!app.help_open());
    assert!(
        !buffer_text(&frame_at(&app, 100, 24)).contains("narrower sidebar"),
        "the popup is still on screen once it is closed"
    );
}

/// `?` is a toggle as well as an opener: the key that raised the manual is the
/// first one a reviewer presses again to get rid of it.
#[test]
fn question_mark_closes_the_help_it_opened() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    app.on_key(KeyCode::Char('?')).expect("?");
    app.on_key(KeyCode::Char('?')).expect("? again");
    assert!(!app.help_open());
}

#[test]
fn q_closes_the_help_rather_than_quitting() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    app.on_key(KeyCode::Char('?')).expect("?");

    let action = app.on_key(KeyCode::Char('q')).expect("q");
    assert_eq!(action, Action::Continue, "q in help closes the help");
    assert!(!app.help_open());
    assert_eq!(
        app.on_key(KeyCode::Char('q')).expect("q"),
        Action::Quit,
        "and quits once it is closed"
    );
}

/// While the manual is up every other key is inert — including the one that
/// destroys written work.
#[rstest]
#[case(KeyCode::Char('c'))]
#[case(KeyCode::Char('d'))]
#[case(KeyCode::Char('j'))]
#[case(KeyCode::Enter)]
#[case(KeyCode::Tab)]
#[case(KeyCode::Left)]
#[case(KeyCode::Char(']'))]
#[case(KeyCode::Char('s'))]
#[case(KeyCode::Char('>'))]
#[case(KeyCode::Char('t'))]
#[case(KeyCode::Char('o'))]
fn keys_are_inert_while_the_help_is_open(#[case] key: KeyCode) {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, "a finding");
    let before = workspace_tree(workspace.root());

    app.on_key(KeyCode::Char('?')).expect("?");
    let state = (
        app.mode(),
        app.focus(),
        app.file_index(),
        app.line_index(),
        app.sidebar_tab(),
        app.split().ratio(),
        app.collapsed().len(),
        app.tree_view(),
        app.sort(),
    );

    app.on_key(key).expect("key");

    assert_eq!(
        (
            app.mode(),
            app.focus(),
            app.file_index(),
            app.line_index(),
            app.sidebar_tab(),
            app.split().ratio(),
            app.collapsed().len(),
            app.tree_view(),
            app.sort(),
        ),
        state,
        "{key:?} did something while the help was open"
    );
    assert!(app.help_open(), "{key:?} closed the help");
    assert_eq!(
        workspace_tree(workspace.root()),
        before,
        "{key:?} wrote to the workspace from behind the help"
    );
}

#[test]
fn every_binding_the_handler_dispatches_appears_in_the_popup() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    app.on_key(KeyCode::Char('?')).expect("?");
    let frame = buffer_text(&frame_at(&app, 120, 40));

    assert!(!BINDINGS.is_empty(), "the binding table is empty");
    for binding in BINDINGS {
        assert!(
            frame.contains(binding.keys),
            "the popup does not list {}:\n{frame}",
            binding.keys
        );
        assert!(
            frame.contains(binding.what),
            "the popup lists {} without saying what it does:\n{frame}",
            binding.keys
        );
    }
}

/// 80x24 is what a reviewer over ssh actually has, and a keymap you must scroll
/// to read is a keymap you will not read. This is what forces the column
/// layout: sixteen bindings and their headings need twenty-one rows in one
/// list, and the popup has fourteen.
#[test]
fn the_whole_keymap_fits_at_80x24_without_scrolling() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    app.on_key(KeyCode::Char('?')).expect("?");
    let frame = buffer_text(&frame_at(&app, 80, 24));

    for binding in BINDINGS {
        assert!(
            frame.contains(binding.keys),
            "{} is off screen at 80x24:\n{frame}",
            binding.keys
        );
        assert!(
            frame.contains(binding.what),
            "{}'s description is off screen at 80x24:\n{frame}",
            binding.keys
        );
    }
    assert!(
        !frame.contains("more"),
        "something is hidden behind a scroll indicator:\n{frame}"
    );
}

/// `d` means nothing in the Files tab. A reviewer learning the tool should see
/// that the key exists and why it is inert here, not wonder whether they
/// misread the manual.
///
/// The control is in the same frame on purpose: if every row were dimmed,
/// nothing would be.
#[test]
fn a_binding_that_does_nothing_here_is_dimmed_rather_than_hidden() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    app.on_key(KeyCode::Left).expect("focus the sidebar");
    assert_eq!(app.sidebar_tab(), SidebarTab::Files);
    app.on_key(KeyCode::Char('?')).expect("?");

    let frame = frame_at(&app, 100, 30);
    assert!(
        buffer_text(&frame).contains("delete a comment"),
        "the binding was hidden rather than dimmed:\n{}",
        buffer_text(&frame)
    );
    assert!(
        is_dim(&frame, cell_of_binding(&frame, "d", "delete a comment")),
        "`d` is not shown as inactive in the file list:\n{}",
        buffer_text(&frame)
    );
    assert!(
        !is_dim(&frame, cell_of_binding(&frame, "q", "quit the review")),
        "every row is dimmed, so dimming says nothing:\n{}",
        buffer_text(&frame)
    );
}

/// ...and the same key is *not* dimmed where it does something, so the dimming
/// follows the cursor rather than being a property of the key.
#[test]
fn the_same_binding_is_live_where_it_acts_on_something() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, "a finding");
    app.on_key(KeyCode::Char('?')).expect("?");

    let frame = frame_at(&app, 100, 30);
    assert!(
        !is_dim(&frame, cell_of_binding(&frame, "d", "delete a comment")),
        "`d` is dimmed on a line that has a comment to delete:\n{}",
        buffer_text(&frame)
    );
}

#[rstest]
#[case(20, 6)]
#[case(1, 1)]
#[case(80, 1)]
#[case(2, 40)]
#[case(40, 3)]
#[case(12, 12)]
fn the_help_renders_in_a_pane_too_small_for_it(#[case] width: u16, #[case] height: u16) {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    write_comment(&mut app, "a finding");
    app.on_key(KeyCode::Char('?')).expect("?");

    let _ = frame_at(&app, width, height);
    // ...and scrolling a popup that cannot show its whole keymap is still just
    // drawing.
    for _ in 0..40 {
        app.on_key(KeyCode::Char('j')).expect("scroll");
    }
    let _ = frame_at(&app, width, height);
    for _ in 0..80 {
        app.on_key(KeyCode::Char('k')).expect("scroll back");
    }
    let _ = frame_at(&app, width, height);
}

/// The popup is drawn *over* the panes rather than beside them: what was
/// underneath is covered, which is what makes it readable.
#[test]
fn the_popup_covers_what_is_beneath_it() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    let beneath = frame_at(&app, 100, 24);
    let popup = layout(
        Rect::new(0, 0, 100, 24),
        Split::default(),
        Chrome {
            bar_rows: 1,
            help_open: true,
            info_open: false,
            toast: false,
            sidebar_hidden: false,
        },
    )
    .popup
    .expect("the popup has a rect at 100x24");

    app.on_key(KeyCode::Char('?')).expect("?");
    let over = frame_at(&app, 100, 24);

    let changed = (popup.y..popup.bottom())
        .flat_map(|y| (popup.x..popup.right()).map(move |x| (x, y)))
        .filter(|at| beneath[*at].symbol() != over[*at].symbol())
        .count();
    assert!(
        changed > 0,
        "the popup left the panes beneath it showing through:\n{}",
        buffer_text(&over)
    );
    // The bar is outside the popup and keeps its own row.
    assert_eq!(last_row(&beneath), last_row(&over), "the popup ate the bar");
}
