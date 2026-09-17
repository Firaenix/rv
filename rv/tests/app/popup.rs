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
use rv::layout::HelpChrome;
use rv::layout::Split;
use rv::layout::layout;
use rv::session;

use crate::support::*;

/// The cell holding the key of the popup row reading `keys`, a gap, `what`.
///
/// Both halves are matched together, because a description recurs — `delete`
/// is under `D`, `c d` and `Space d` — and a single-character key is a
/// substring of half the screen: only the pair names one cell.
fn cell_of_binding(buffer: &Buffer, keys: &str, what: &str) -> (u16, u16) {
    let rows = rows_of(buffer);
    for (y, row) in rows.iter().enumerate() {
        let mut from = 0;
        while let Some(at) = row[from..].find(what).map(|at| at + from) {
            let before = row[..at].trim_end();
            if before.ends_with(keys)
                && row[..at].len() - before.len() >= 2
                && before[..before.len() - keys.len()].ends_with(' ')
                    | before[..before.len() - keys.len()].ends_with('│')
            {
                let column = before[..before.len() - keys.len()].chars().count();
                return (
                    u16::try_from(column).expect("a small column"),
                    u16::try_from(y).expect("a small row"),
                );
            }
            from = at + what.len();
        }
    }
    panic!("no row reads `{keys}  {what}`:\n{}", buffer_text(buffer));
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
    assert!(!app.help_full(), "the first press is the contextual tip");
    let frame = buffer_text(&frame_at(&app, 100, 24));
    assert!(
        frame.contains("comment"),
        "the tip lists what the keys do here:\n{frame}"
    );

    app.on_key(KeyCode::Esc).expect("esc");
    assert!(!app.help_open());
    assert!(
        !buffer_text(&frame_at(&app, 100, 24)).contains("? all keys"),
        "the tip is still on screen once it is closed"
    );
}

/// The first `?` answers "what can I do here" with a tip in the corner; the
/// second unrolls the whole keymap; the third puts it away.
#[test]
fn question_mark_walks_tip_then_keymap_then_closed() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    app.on_key(KeyCode::Char('?')).expect("?");
    assert!(app.help_open() && !app.help_full(), "first: the tip");
    app.on_key(KeyCode::Char('?')).expect("? again");
    assert!(app.help_full(), "second: the whole keymap");
    app.on_key(KeyCode::Char('?')).expect("? once more");
    assert!(!app.help_open(), "third: closed");
}

/// `?` shows the layers overview: the leaders every key hangs off, and how to
/// reach them, in the corner above the bar's own hint. It still names where the
/// reviewer is in its title, and `? ?` unrolls the whole keymap.
#[test]
fn the_layers_overview_sits_in_the_corner_and_names_the_context() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    app.on_key(KeyCode::Char('?')).expect("?");

    let frame = frame_at(&app, 100, 24);
    let text = buffer_text(&frame);
    assert!(
        text.contains("DIFF"),
        "the overview names the context the cursor is in:\n{text}"
    );
    // The four leaders are the whole point of the overview.
    for leader in ["mode", "goto", "comment", "view"] {
        assert!(
            text.contains(leader),
            "the overview does not advertise the {leader} layer:\n{text}"
        );
    }
    assert!(
        !text.contains("narrow sidebar"),
        "the overview should not unroll the whole manual:\n{text}"
    );

    // In the corner: the tip's frame touches the right-hand edge, on the row
    // above the bar.
    let rows = rows_of(&frame);
    let above_bar = &rows[rows.len() - 2];
    assert!(
        above_bar.trim_end().ends_with('╯'),
        "the overview's corner is not above the hint:\n{text}"
    );

    // The title follows the focus: the sidebar's overview names the file list.
    app.on_key(KeyCode::Esc).expect("close");
    app.on_key(KeyCode::Left).expect("focus the sidebar");
    app.on_key(KeyCode::Char('?')).expect("?");
    let text = buffer_text(&frame_at(&app, 100, 24));
    assert!(
        text.contains("FILES"),
        "the overview title did not follow the focus:\n{text}"
    );
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
#[case(KeyCode::Char('C'))]
#[case(KeyCode::Char('D'))]
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
    app.on_key(KeyCode::Char('?'))
        .expect("? again, for the whole keymap");
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

/// The keymap is dealt into columns and nothing is packed by hand, so the
/// popup is not held to any one terminal size — but it must fit a normal
/// one without scrolling, and every row must be reachable at 80x24, the size
/// a reviewer over ssh actually has.
#[test]
fn the_whole_keymap_fits_at_100x30_without_scrolling() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    app.on_key(KeyCode::Char('?')).expect("?");
    app.on_key(KeyCode::Char('?'))
        .expect("? again, for the whole keymap");
    let frame = buffer_text(&frame_at(&app, 100, 30));

    for binding in BINDINGS {
        assert!(
            frame.contains(binding.keys),
            "{} is off screen at 100x30:\n{frame}",
            binding.keys
        );
        assert!(
            frame.contains(binding.what),
            "{}'s description is off screen at 100x30:\n{frame}",
            binding.keys
        );
    }
    assert!(
        !frame.contains("more"),
        "something is hidden behind a scroll indicator:\n{frame}"
    );
}

/// At 80x24 the keymap takes the whole screen and may still need one
/// scroll; every binding is on one of the screens `j` reaches, and the hint
/// says so until the last of them.
#[test]
fn every_binding_is_reachable_by_scrolling_at_80x24() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    app.on_key(KeyCode::Char('?')).expect("?");
    app.on_key(KeyCode::Char('?'))
        .expect("? again, for the whole keymap");

    let mut seen = String::new();
    for _ in 0..20 {
        let frame = buffer_text(&frame_at(&app, 80, 24));
        seen.push_str(&frame);
        if !frame.contains("more") {
            break;
        }
        app.on_key(KeyCode::Char('j')).expect("scroll");
    }
    for binding in BINDINGS {
        assert!(
            seen.contains(binding.keys) && seen.contains(binding.what),
            "{} ({}) is on no screen at 80x24:\n{seen}",
            binding.keys,
            binding.what
        );
    }
}

/// The popup is drawn from the *runtime* keymap: a key the config rebinds,
/// adds in one pane, or moves onto another leader is listed as it now is,
/// and a leader the config moved is spelled with its new key — nothing in
/// the popup is a hand-maintained copy of the table.
#[test]
fn the_popup_lists_the_keymap_as_the_config_left_it() {
    let workspace = Fixture::new();
    let review = session::build(workspace.root(), None, None).expect("build the review");
    let config = rv::config::parse(
        "[leaders]\ngoto = \"G\"\n[keys]\ncomment_write = \"w\"\n[keys.files]\nfiles_cycle_sort = \"O\"\n",
    )
    .expect("parse the config");
    let mut app = rv::app::App::open_with_config(
        review,
        rv::app::DiffEngine::Structural,
        &config,
        &rv::config::Settings::default(),
    )
    .expect("open the reviewer");
    app.finish_loading();
    app.on_key(KeyCode::Char('?')).expect("?");
    app.on_key(KeyCode::Char('?'))
        .expect("? again, for the whole keymap");
    let frame = frame_at(&app, 120, 40);
    let text = buffer_text(&frame);

    // The rebound key replaces the shipped one.
    cell_of_binding(&frame, "w", "comment");
    assert!(
        !text.contains("C        comment"),
        "the old key survived:\n{text}"
    );
    // The moved leader is spelled with its new key on every child.
    cell_of_binding(&frame, "G n", "next sym");
    assert!(
        !text.contains("g n"),
        "the old leader key survived:\n{text}"
    );
    // A pane-scoped addition is listed under that pane.
    let rows = rows_of(&frame);
    let files_heading = rows
        .iter()
        .position(|row| row.contains("Files list"))
        .expect("the files section");
    let (_, y) = cell_of_binding(&frame, "O", "order");
    assert!(
        usize::from(y) > files_heading,
        "the scoped bind is not under the files heading:\n{text}"
    );
}

/// `D` means nothing in the Files tab. A reviewer learning the tool should see
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
    app.on_key(KeyCode::Char('?'))
        .expect("? again, for the whole keymap");

    let frame = frame_at(&app, 100, 30);
    assert!(
        buffer_text(&frame).contains("delete"),
        "the binding was hidden rather than dimmed:\n{}",
        buffer_text(&frame)
    );
    assert!(
        is_dim(&frame, cell_of_binding(&frame, "D", "delete")),
        "`D` is not shown as inactive in the file list:\n{}",
        buffer_text(&frame)
    );
    assert!(
        !is_dim(&frame, cell_of_binding(&frame, "q", "quit")),
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
    app.on_key(KeyCode::Char('?'))
        .expect("? again, for the whole keymap");

    let frame = frame_at(&app, 100, 30);
    assert!(
        !is_dim(&frame, cell_of_binding(&frame, "D", "delete")),
        "`D` is dimmed on a line that has a comment to delete:\n{}",
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
    app.on_key(KeyCode::Char('?'))
        .expect("? again, for the whole keymap");

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
            help: HelpChrome::Full,
            tooltip: None,
            toast: false,
            sidebar_hidden: false,
        },
    )
    .popup
    .expect("the popup has a rect at 100x24");

    app.on_key(KeyCode::Char('?')).expect("?");
    app.on_key(KeyCode::Char('?'))
        .expect("? again, for the whole keymap");
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

/// The keymap fills a wide popup: on a wide terminal the columns are dealt
/// shorter and more of them, spread across the width, rather than three
/// tall ones pressed into the left-hand fifth of the screen.
#[test]
fn a_wide_popup_spreads_its_columns_across_the_width() {
    let workspace = Fixture::new();
    let mut app = workspace.app();
    app.on_key(KeyCode::Char('?')).expect("?");
    app.on_key(KeyCode::Char('?'))
        .expect("? again, for the whole keymap");
    let frame = buffer_text(&frame_at(&app, 160, 45));

    let widest_row = frame
        .lines()
        .filter(|row| row.starts_with('│'))
        .map(|row| row.trim_end_matches('│').trim_end().chars().count())
        .max()
        .unwrap_or(0);
    assert!(
        widest_row > 100,
        "the keymap uses {widest_row} of 160 columns:\n{frame}"
    );
    let used_rows = frame
        .lines()
        .filter(|row| row.starts_with('│') && !row.trim_matches('│').trim().is_empty())
        .count();
    assert!(
        used_rows < 40,
        "the keymap is one tall column: {used_rows} rows\n{frame}"
    );
}
