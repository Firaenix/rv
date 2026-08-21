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
#[case::next_line_letter(KeyCode::Char('j'), Action::Continue, Mode::Browse, Focus::Diff, (0, 1), None)]
#[case::next_line_arrow(KeyCode::Down, Action::Continue, Mode::Browse, Focus::Diff, (0, 1), None)]
#[case::previous_line_letter(KeyCode::Char('k'), Action::Continue, Mode::Browse, Focus::Diff, (0, 0), None)]
#[case::previous_line_arrow(KeyCode::Up, Action::Continue, Mode::Browse, Focus::Diff, (0, 0), None)]
#[case::focus_sidebar_arrow(KeyCode::Left, Action::Continue, Mode::Browse, Focus::Sidebar, (0, 0), None)]
#[case::focus_sidebar_letter(KeyCode::Char('h'), Action::Continue, Mode::Browse, Focus::Sidebar, (0, 0), None)]
#[case::focus_diff_arrow(KeyCode::Right, Action::Continue, Mode::Browse, Focus::Diff, (0, 0), None)]
#[case::focus_diff_letter(KeyCode::Char('l'), Action::Continue, Mode::Browse, Focus::Diff, (0, 0), None)]
#[case::next_file(KeyCode::Char(']'), Action::Continue, Mode::Browse, Focus::Diff, (1, 0), None)]
#[case::previous_file(KeyCode::Char('['), Action::Continue, Mode::Browse, Focus::Diff, (0, 0), None)]
#[case::comment(KeyCode::Char('c'), Action::Continue, Mode::Comment, Focus::Diff, (0, 0), None)]
#[case::quit(KeyCode::Char('q'), Action::Quit, Mode::Browse, Focus::Diff, (0, 0), None)]
// The three comment keys, on a line with no comments on it.
#[case::enter_an_empty_stack(KeyCode::Enter, Action::Continue, Mode::Browse, Focus::Diff, (0, 0), Some(NO_COMMENTS))]
#[case::escape_outside_a_stack(KeyCode::Esc, Action::Continue, Mode::Browse, Focus::Diff, (0, 0), None)]
#[case::delete_nothing(KeyCode::Char('d'), Action::Continue, Mode::Browse, Focus::Diff, (0, 0), Some(NO_COMMENTS))]
#[case::collapse_nothing(KeyCode::Char('s'), Action::Continue, Mode::Browse, Focus::Diff, (0, 0), Some(NO_COMMENTS))]
// `Tab` changes what the left column *lists* and nothing else: not the focus,
// not the selection, and not the status line — which is where the reviewer
// reads the rest of the keymap, and is not a place for a navigation key to
// announce itself. Which tab it left behind is asserted in the body.
#[case::switch_sidebar_tab(KeyCode::Tab, Action::Continue, Mode::Browse, Focus::Diff, (0, 0), None)]
// The three view keys. None of them says anything in the status line: they are
// about how the screen is arranged, and the bar is where the reviewer reads the
// rest of the keymap. What each of them actually moved is asserted in the body.
#[case::narrower_sidebar(KeyCode::Char('<'), Action::Continue, Mode::Browse, Focus::Diff, (0, 0), None)]
#[case::wider_sidebar(KeyCode::Char('>'), Action::Continue, Mode::Browse, Focus::Diff, (0, 0), None)]
// `t` and `o` reshape and reorder the file list. Neither moves the selection —
// the file being read is the reviewer's choice, not the list's — and neither
// says anything in the bar. What each of them moved is asserted in the body.
#[case::list_or_tree(KeyCode::Char('t'), Action::Continue, Mode::Browse, Focus::Diff, (0, 0), None)]
#[case::order_the_files(KeyCode::Char('o'), Action::Continue, Mode::Browse, Focus::Diff, (0, 0), None)]
// `J`/`K` walk hunks. `alpha.rs` carries two — the inserted header, then the
// rewritten line below it. With full-file context shown the two are
// separated by real `Context` rows (the untouched `pub fn alpha() {` and
// `let a01 = 1;`), so the reviewer opens standing on the header (hunk one,
// line_index 0) and `J` jumps past the intervening context to line_index 3,
// the removed half of the rewritten pair — not to line_index 1, which is
// now context. `K` from the top still has nowhere to go, which is where the
// no-wrap ruling is visible: the refusal speaks rather than jumping to the
// far end.
#[case::next_hunk(KeyCode::Char('J'), Action::Continue, Mode::Browse, Focus::Diff, (0, 3), None)]
#[case::previous_hunk_at_the_first(KeyCode::Char('K'), Action::Continue, Mode::Browse, Focus::Diff, (0, 0), Some("the first hunk in this file"))]
#[case::open_the_help(KeyCode::Char('?'), Action::Continue, Mode::Browse, Focus::Diff, (0, 0), None)]
// Not in the table, and therefore inert.
#[case::unbound_letter(KeyCode::Char('x'), Action::Continue, Mode::Browse, Focus::Diff, (0, 0), None)]
// `J` and `K` used to be inert and are now the hunk keys, above.
#[case::unbound_backspace(KeyCode::Backspace, Action::Continue, Mode::Browse, Focus::Diff, (0, 0), None)]
#[case::unbound_backtab(KeyCode::BackTab, Action::Continue, Mode::Browse, Focus::Diff, (0, 0), None)]
#[case::unbound_function(KeyCode::F(1), Action::Continue, Mode::Browse, Focus::Diff, (0, 0), None)]
#[case::unbound_page_down(KeyCode::PageDown, Action::Continue, Mode::Browse, Focus::Diff, (0, 0), None)]
#[case::unbound_home(KeyCode::Home, Action::Continue, Mode::Browse, Focus::Diff, (0, 0), None)]
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
    // Exactly one key in the table changes what the left column lists.
    assert_eq!(
        app.sidebar_tab(),
        if key == KeyCode::Tab {
            SidebarTab::Commits
        } else {
            SidebarTab::Files
        },
        "{key:?} left the sidebar listing the wrong thing"
    );
    // ...and exactly one raises the keymap.
    assert_eq!(
        app.help_open(),
        key == KeyCode::Char('?'),
        "{key:?} left the help in the wrong state"
    );
    // ...and exactly two move the divider, in the two directions their glyphs
    // point. Asserted as a direction rather than a number, so the size of one
    // nudge stays `app.rs`'s business.
    let ratio = app.split().ratio();
    match key {
        KeyCode::Char('>') => assert!(ratio > Split::DEFAULT, "> did not widen the sidebar"),
        KeyCode::Char('<') => assert!(ratio < Split::DEFAULT, "< did not narrow the sidebar"),
        _ => assert_eq!(ratio, Split::DEFAULT, "{key:?} moved the divider"),
    }
    // ...and exactly one reshapes the file list, and exactly one reorders it.
    assert_eq!(
        app.tree_view(),
        key == KeyCode::Char('t'),
        "{key:?} left the file list in the wrong shape"
    );
    assert_eq!(
        app.sort() != Sort::Natural,
        key == KeyCode::Char('o'),
        "{key:?} left the file list in the wrong order"
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
