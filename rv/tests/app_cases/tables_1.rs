//! The documented keybinding tables.

use crossterm::event::KeyCode;
use rstest::rstest;
use rv::app::Action;
use rv::app::App;
use rv::app::Focus;
use rv::app::Mode;
use rv::app::SidebarTab;
use rv::layout::Split;
use rv::tree::Sort;
use std::fs;
use std::path::Path;

use crate::support::*;

/// Every row of README's **Browsing** table, plus the keys it deliberately
/// does not bind.
///
/// Cross-checked against `app.rs::BINDINGS`, which is now the *only* thing
/// `on_key_browse` dispatches from: `Down`/`j`, `Up`/`k`, `Left`/`h`,
/// `Right`/`l`, `]`, `[`, `c`, `d`, `s`, `Tab`, `Enter`, `Esc`, `<`, `>`, `?`,
/// `q` — every one of which has a row below, and the rows after them are keys
/// the table deliberately leaves inert. The arrow is the binding and the vim
/// key its alias, so each of the four movement rows appears twice: a pair that
/// stopped agreeing would be two keys doing different things under one heading. A key that reached the handler without a row in
/// `BINDINGS` would fail one of the `unbound_*` rows here; a row in `BINDINGS`
/// that reached nothing would fail its own row.
///
/// README's table carries two rows more than this one. `Ctrl+C` is answered by
/// `on_key_event` before the mode is dispatched at all, and the page lists it
/// beside the rest because a reviewer looking for the way out does not care
/// which function answers them; it is pinned in `rv/tests/app.rs`
/// (`ctrl_c_quits_instead_of_opening_a_comment`), where a `KeyEvent` with
/// modifiers can be built, and every row here is a bare `KeyCode`. `v` spawns a
/// process and reads `$EDITOR`, which is a fact about this binary's
/// environment rather than about a key: it is pinned in
/// `rv/tests/app/editor.rs`, which re-execs itself to set one safely.
///
/// That README table is itself held to this key set by
/// `rv/tests/app.rs::the_readme_documents_every_browse_binding`, in both
/// directions, so a binding cannot ship undocumented and a row cannot outlive
/// its key.
///
/// The start state is a fresh reviewer on `alpha.rs` (five-plus diff lines,
/// first of five files) with the diff focused and **no comments anywhere in the
/// review**, so every direction has somewhere to go except `k`/`Up` and `Left`,
/// which are checked at their clamp — and the four comment keys take their
/// empty-line branch. What each of them does on a line that *has* comments is
/// pinned end-to-end in `rv/tests/app.rs`, which is where a stack can be built
/// by typing one; that is also where `d`'s confirmation lives, so no row here
/// can leave the reviewer in `Mode::ConfirmDelete`.
///
/// The focus column is what makes the movement rows mean anything now that
/// there is more than one pane: `j` moves the line *because the diff has
/// focus*, and every row here says which pane the key left the cursor in. The
/// status column is the other half of that: a key that refuses has to say so,
/// and a key that navigates has to leave the help text alone.
#[rstest]
#[case::next_line_arrow(KeyCode::Down, Action::Continue, Mode::Browse, Focus::Diff, (0, 1), None)]
#[case::previous_line_arrow(KeyCode::Up, Action::Continue, Mode::Browse, Focus::Diff, (0, 0), None)]
#[case::focus_sidebar_arrow(KeyCode::Left, Action::Continue, Mode::Browse, Focus::Sidebar, (0, 0), None)]
#[case::focus_diff_arrow(KeyCode::Right, Action::Continue, Mode::Browse, Focus::Diff, (0, 0), None)]
#[case::next_file(KeyCode::Char(']'), Action::Continue, Mode::Browse, Focus::Diff, (1, 0), None)]
#[case::previous_file(KeyCode::Char('['), Action::Continue, Mode::Browse, Focus::Diff, (0, 0), None)]
#[case::quit(KeyCode::Char('q'), Action::Quit, Mode::Browse, Focus::Diff, (0, 0), None)]
// `Enter` and `Esc` are direct.
#[case::enter_an_empty_stack(KeyCode::Enter, Action::Continue, Mode::Browse, Focus::Diff, (0, 0), Some(NO_COMMENTS))]
#[case::escape_outside_a_stack(KeyCode::Esc, Action::Continue, Mode::Browse, Focus::Diff, (0, 0), None)]
// `s` folds a comment box; on a line with none, it reports there is nothing to.
#[case::collapse_nothing(KeyCode::Char('s'), Action::Continue, Mode::Browse, Focus::Diff, (0, 0), Some(NO_COMMENTS))]
// `Tab` swaps the focus between the diff and the sidebar; from the diff it
// lands on the sidebar. It does not change which list the sidebar shows.
#[case::tab_toggles_the_panel(KeyCode::Tab, Action::Continue, Mode::Browse, Focus::Sidebar, (0, 0), None)]
#[case::open_the_help(KeyCode::Char('?'), Action::Continue, Mode::Browse, Focus::Diff, (0, 0), None)]
// `c` on a commentable line is the one leader that smart-collapses: the write
// is its only live child there, so the menu is skipped and the box opens, the
// bar naming the choice. `g` and `v` have several children or none live under
// the cursor, so they open their submenu and stay in Browse.
#[case::comment_collapses_to_write(KeyCode::Char('c'), Action::Continue, Mode::Comment, Focus::Diff, (0, 0), Some("c → write a comment"))]
#[case::goto_leader(KeyCode::Char('g'), Action::Continue, Mode::Browse, Focus::Diff, (0, 0), None)]
#[case::view_leader(KeyCode::Char('v'), Action::Continue, Mode::Browse, Focus::Diff, (0, 0), None)]
// The dropped vim letters are inert now.
#[case::vim_j_is_inert(KeyCode::Char('j'), Action::Continue, Mode::Browse, Focus::Diff, (0, 0), None)]
#[case::vim_k_is_inert(KeyCode::Char('k'), Action::Continue, Mode::Browse, Focus::Diff, (0, 0), None)]
#[case::vim_h_is_inert(KeyCode::Char('h'), Action::Continue, Mode::Browse, Focus::Diff, (0, 0), None)]
#[case::vim_l_is_inert(KeyCode::Char('l'), Action::Continue, Mode::Browse, Focus::Diff, (0, 0), None)]
// Not in the table, and therefore inert.
#[case::unbound_letter(KeyCode::Char('x'), Action::Continue, Mode::Browse, Focus::Diff, (0, 0), None)]
#[case::unbound_backspace(KeyCode::Backspace, Action::Continue, Mode::Browse, Focus::Diff, (0, 0), None)]
#[case::unbound_backtab(KeyCode::BackTab, Action::Continue, Mode::Browse, Focus::Diff, (0, 0), None)]
#[case::unbound_function(KeyCode::F(1), Action::Continue, Mode::Browse, Focus::Diff, (0, 0), None)]
// `Home`/`PgUp` at the top of the diff have nowhere to go, so they stay put.
#[case::home_at_the_top(KeyCode::Home, Action::Continue, Mode::Browse, Focus::Diff, (0, 0), None)]
#[case::page_up_at_the_top(KeyCode::PageUp, Action::Continue, Mode::Browse, Focus::Diff, (0, 0), None)]
#[case::unbound_delete(KeyCode::Delete, Action::Continue, Mode::Browse, Focus::Diff, (0, 0), None)]
fn browse_keybindings(
    mut multi_app: App,
    #[case] key: KeyCode,
    #[case] action: Action,
    #[case] mode: Mode,
    #[case] focus: Focus,
    // The selection the key should leave behind, as `(file, line)`: one
    // column rather than two, so a row still reads as a row.
    #[case] selection: (usize, usize),
    #[case] status: Option<&str>,
) {
    let app = &mut multi_app;
    assert!(
        app.files().len() >= 3,
        "the fixture lost files: {:?}",
        app.files()
    );
    assert!(
        lines(app).len() > 2,
        "alpha.rs has too few diff lines to navigate"
    );
    assert_eq!(app.focus(), Focus::Diff, "a fresh reviewer reads the diff");
    assert!(
        shared_tables().comments().is_empty(),
        "the tables' fixture has comments in it, so the three comment keys \
         would take a branch these rows do not describe"
    );

    assert_eq!(app.on_key(key).expect("handle the key"), action);
    assert_eq!(app.mode(), mode);
    assert_eq!(app.focus(), focus);
    assert_eq!((app.file_index(), app.line_index()), selection);
    // Navigating never writes the status line: the help text is what a reviewer
    // reads while they move around, and `c` on a commentable line has nothing
    // to report. Only a refusal speaks.
    assert_eq!(app.status(), status.unwrap_or(HELP));
    // Whatever the key did, it did not start a comment body behind the
    // reviewer's back.
    if mode == Mode::Browse {
        assert_eq!(app.buffer(), "");
    }
    assert_eq!(
        app.comment_index(),
        0,
        "no browsing key moves the stack cursor off the top"
    );
    // No single key in this table changes which list the sidebar shows — the
    // mode leader does that now, and `Tab` only swaps the focus.
    assert_eq!(
        app.sidebar_tab(),
        SidebarTab::Files,
        "{key:?} left the sidebar listing the wrong thing"
    );
    // ...and exactly one raises the keymap.
    assert_eq!(
        app.help_open(),
        key == KeyCode::Char('?'),
        "{key:?} left the help in the wrong state"
    );
    // The divider, the file-list shape and its order are moved only by the view
    // leader's chords now, none of which this single-key table presses.
    assert_eq!(
        app.split().ratio(),
        Split::DEFAULT,
        "{key:?} moved the divider"
    );
    assert!(!app.tree_view(), "{key:?} reshaped the file list");
    assert_eq!(app.sort(), Sort::Natural, "{key:?} reordered the file list");
    // `g` and `v` open a submenu and wait; `c` smart-collapses straight to the
    // write here, so it leaves no menu pending. Every other key clears back to
    // browsing.
    let opens_menu = matches!(key, KeyCode::Char('g' | 'v'));
    assert_eq!(
        app.pending_leader().is_some(),
        opens_menu,
        "{key:?} left the wrong leader pending"
    );
}

/// README draws the reviewer as an ASCII mock-up, status bar and all, and that
/// bar is the keymap a reader meets *first* — before either table, and in the
/// one place on the page that claims to be a picture of the running program.
///
/// So it is held to the real one rather than to a list of keys: [`HELP`] is
/// asserted equal to `App::status()` on a fresh reviewer by every row of
/// [`browse_keybindings`] above, and asserted to appear in the page here, which
/// chains the drawing to the program through the constant. The previous wave
/// changed `HELP` and left the mock-up showing the old bar, noted that nothing
/// tested it, and left it to this task; a mock-up that has drifted teaches a
/// keymap the binary does not have, which is worse than no picture at all.
///
/// Substring rather than a whole line, because the mock-up wraps the bar in the
/// box-drawing characters that make it a picture. What is pinned is the bar's
/// text, not the frame drawn around it.
#[test]
fn the_readme_mockup_draws_the_status_bar_the_reviewer_starts_on() {
    let readme = fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../README.md"))
        .expect("read README.md");
    assert!(
        readme.contains(HELP),
        "README's mock-up of the reviewer shows a status bar that is not the \
         one `App::new` starts on ({HELP:?}), so the first keymap a reader sees \
         is not this binary's"
    );
}
