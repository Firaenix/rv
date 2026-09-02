//! The documented keybinding tables.

use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyModifiers;
use rstest::rstest;
use rv::app::Action;
use rv::app::App;
use rv::app::BrowserRow;
use rv::app::Focus;
use rv::app::Mode;
use rv::app::SidebarTab;

use crate::support::*;

/// Every key, from inside the sidebar's **Comments** tab — the one focus whose
/// keys mean something different from everywhere else, since `j`/`k` walk the
/// review's comments there rather than its files or its lines.
///
/// The start state is the browser, focused, on the first of two comments, both
/// of them on `alpha.rs` (the first file). Nothing here saves or deletes: `d`
/// is checked as far as its question, because what answering it does is pinned
/// end-to-end in `rv/tests/app.rs` and doing it here would empty the shared
/// fixture under the other cases.
#[rstest]
#[case::next_comment_arrow(
    KeyCode::Down,
    Action::Continue,
    Mode::Browse,
    Focus::Sidebar,
    SidebarTab::Comments,
    ("second finding", 0)
)]
#[case::previous_comment_arrow(
    KeyCode::Up,
    Action::Continue,
    Mode::Browse,
    Focus::Sidebar,
    SidebarTab::Comments,
    ("first finding", 0)
)]
// `Enter` jumps to the code, which hands the focus to the diff.
#[case::jump(
    KeyCode::Enter,
    Action::Continue,
    Mode::Browse,
    Focus::Diff,
    SidebarTab::Comments,
    ("first finding", 0)
)]
// The comment leader opens its submenu rather than acting; deleting is `c d`,
// pinned end-to-end in `rv/tests/app.rs`. A bare `d` is inert now.
#[case::comment_leader(KeyCode::Char('c'), Action::Continue, Mode::Browse, Focus::Sidebar, SidebarTab::Comments, ("first finding", 0))]
#[case::bare_d_is_inert(KeyCode::Char('d'), Action::Continue, Mode::Browse, Focus::Sidebar, SidebarTab::Comments, ("first finding", 0))]
// `Tab` swaps the focus to the diff; the comments list stays selected.
#[case::tab_to_the_diff(
    KeyCode::Tab,
    Action::Continue,
    Mode::Browse,
    Focus::Diff,
    SidebarTab::Comments,
    ("first finding", 0)
)]
// The comment browser has no tree to climb, so `←` leads out to the diff.
#[case::left_leads_to_the_diff(
    KeyCode::Left,
    Action::Continue,
    Mode::Browse,
    Focus::Diff,
    SidebarTab::Comments,
    ("first finding", 0)
)]
#[case::out_to_the_diff(
    KeyCode::Right,
    Action::Continue,
    Mode::Browse,
    Focus::Diff,
    SidebarTab::Comments,
    ("first finding", 0)
)]
// File navigation still means files, from here as from everywhere.
#[case::next_file(
    KeyCode::Char(']'),
    Action::Continue,
    Mode::Browse,
    Focus::Sidebar,
    SidebarTab::Comments,
    ("first finding", 1)
)]
#[case::previous_file(
    KeyCode::Char('['),
    Action::Continue,
    Mode::Browse,
    Focus::Sidebar,
    SidebarTab::Comments,
    ("first finding", 0)
)]
#[case::view_leader(
    KeyCode::Char('v'),
    Action::Continue,
    Mode::Browse,
    Focus::Sidebar,
    SidebarTab::Comments,
    ("first finding", 0)
)]
#[case::quit(
    KeyCode::Char('q'),
    Action::Quit,
    Mode::Browse,
    Focus::Sidebar,
    SidebarTab::Comments,
    ("first finding", 0)
)]
#[case::escape_is_inert(
    KeyCode::Esc,
    Action::Continue,
    Mode::Browse,
    Focus::Sidebar,
    SidebarTab::Comments,
    ("first finding", 0)
)]
#[case::unbound_function(
    KeyCode::F(1),
    Action::Continue,
    Mode::Browse,
    Focus::Sidebar,
    SidebarTab::Comments,
    ("first finding", 0)
)]
// `Home` jumps to the first comment; the browser already starts there.
#[case::home_first_comment(
    KeyCode::Home,
    Action::Continue,
    Mode::Browse,
    Focus::Sidebar,
    SidebarTab::Comments,
    ("first finding", 0)
)]
// `End` and a page down both reach the last comment; only two here.
#[case::end_last_comment(
    KeyCode::End,
    Action::Continue,
    Mode::Browse,
    Focus::Sidebar,
    SidebarTab::Comments,
    ("second finding", 0)
)]
#[case::page_down_last_comment(
    KeyCode::PageDown,
    Action::Continue,
    Mode::Browse,
    Focus::Sidebar,
    SidebarTab::Comments,
    ("second finding", 0)
)]
#[case::page_up_stays_first(
    KeyCode::PageUp,
    Action::Continue,
    Mode::Browse,
    Focus::Sidebar,
    SidebarTab::Comments,
    ("first finding", 0)
)]
#[case::unbound_backspace(
    KeyCode::Backspace,
    Action::Continue,
    Mode::Browse,
    Focus::Sidebar,
    SidebarTab::Comments,
    ("first finding", 0)
)]
fn comment_browser_keybindings(
    mut browser_app: App,
    #[case] key: KeyCode,
    #[case] action: Action,
    #[case] mode: Mode,
    #[case] focus: Focus,
    #[case] tab: SidebarTab,
    // The selection the key should leave behind, as `(browsed comment, file)`.
    //
    // The comment is named by its body rather than by a row number: the browser
    // groups its comments under file headings, so a row index is an address in
    // a list whose shape is not what this table is about — and a literal index
    // that happens to be right is the weaker assertion either way.
    #[case] selection: (&str, usize),
) {
    let app = &mut browser_app;
    assert_eq!(app.comments().len(), 2, "the browser has nothing to walk");

    assert_eq!(app.on_key(key).expect("handle the key"), action);
    // `ConfirmDelete` carries the id it is about, which no row can spell out;
    // the rows say *which* mode, and `rv/tests/app.rs` says which comment.
    assert_eq!(
        std::mem::discriminant(&app.mode()),
        std::mem::discriminant(&mode),
        "{key:?} left the reviewer in {:?}",
        app.mode()
    );
    assert_eq!(
        app.focus(),
        focus,
        "{key:?} left the cursor in the wrong pane"
    );
    assert_eq!(app.sidebar_tab(), tab);
    // Read off the browser's own row rather than through `browsed_comment`,
    // which is deliberately `None` off the Comments tab: `Tab` is one of the
    // keys under test, and its case still has a browser cursor to assert about.
    let browsed = match &app.browser_rows()[app.browser_index()] {
        BrowserRow::Comment { index, .. } => app.comments()[*index].body.clone(),
        BrowserRow::File { path, .. } => {
            panic!("{key:?} left the cursor on the {path} heading")
        }
        BrowserRow::Dir { label, .. } => {
            panic!("{key:?} left the cursor on the {label} directory row")
        }
    };
    assert_eq!(
        (browsed.as_str(), app.file_index()),
        selection,
        "{key:?} left the browser or the file list somewhere else"
    );
    assert_eq!(
        shared_browser().comments().len(),
        2,
        "{key:?} wrote to the browser's shared fixture"
    );
}

/// Every row of README's **Typing a comment** table, plus the keys it does not
/// bind. Start state: `c` pressed on `alpha.rs`'s first diff line, empty
/// buffer.
///
/// Cross-checked against `app.rs::on_key_comment`: `Esc`, `Backspace`,
/// `Enter`, `Char(_)`, and nothing else — which matches README's four rows
/// ("any character" being `Char`). No case here saves anything, so they may
/// share the read-only fixture: `Enter` on an empty buffer is a refusal.
#[rstest]
#[case::append_letter(KeyCode::Char('a'), Mode::Comment, "a", None)]
#[case::append_space(KeyCode::Char(' '), Mode::Comment, " ", None)]
#[case::append_bracket(KeyCode::Char(']'), Mode::Comment, "]", None)]
#[case::append_q_does_not_quit(KeyCode::Char('q'), Mode::Comment, "q", None)]
#[case::append_unicode(KeyCode::Char('日'), Mode::Comment, "日", None)]
#[case::backspace_on_empty(KeyCode::Backspace, Mode::Comment, "", None)]
#[case::escape_discards(KeyCode::Esc, Mode::Browse, "", Some("comment discarded"))]
#[case::enter_on_empty(KeyCode::Enter, Mode::Browse, "", Some("empty comment, nothing saved"))]
// Not in the table: a comment is a single line of text, so nothing else moves.
#[case::unbound_tab(KeyCode::Tab, Mode::Comment, "", None)]
#[case::unbound_down(KeyCode::Down, Mode::Comment, "", None)]
#[case::unbound_left(KeyCode::Left, Mode::Comment, "", None)]
#[case::unbound_delete(KeyCode::Delete, Mode::Comment, "", None)]
#[case::unbound_function(KeyCode::F(4), Mode::Comment, "", None)]
#[case::unbound_home(KeyCode::Home, Mode::Comment, "", None)]
fn comment_keybindings(
    mut multi_app: App,
    #[case] key: KeyCode,
    #[case] mode: Mode,
    #[case] buffer: &str,
    #[case] status: Option<&str>,
) {
    let app = &mut multi_app;
    assert_eq!(comment(app), Action::Continue);
    assert_eq!(
        app.mode(),
        Mode::Comment,
        "the fixture's first line is not commentable"
    );

    // Nothing typed while a comment is open ever ends the program: the whole
    // point of the mode is that `q` is a letter here.
    assert_eq!(app.on_key(key).expect("handle the key"), Action::Continue);
    assert_eq!(app.mode(), mode);
    assert_eq!(app.buffer(), buffer);
    assert_eq!(app.status(), status.unwrap_or(HELP));
    assert_eq!(
        app.focus(),
        Focus::Diff,
        "typing moved the cursor to another pane"
    );
    assert!(
        shared_tables().comments().is_empty(),
        "a keybinding case saved a comment into the tables' fixture"
    );
}

/// `on_key_event` is a thin gate in front of `on_key`: it answers Ctrl+C
/// itself and hands every other key on by its code alone.
///
/// The gate exists because the terminal is in raw mode, where no SIGINT is
/// raised on the reviewer's behalf — and where `Char('c')` with CONTROL held is
/// indistinguishable from a typed `c` once the modifiers are dropped, which is
/// what used to make the universal abort open the comment box.
#[rstest]
#[case::ctrl_c_quits(KeyCode::Char('c'), KeyModifiers::CONTROL, Action::Quit, Mode::Browse)]
#[case::ctrl_shift_c_quits(
    KeyCode::Char('c'),
    KeyModifiers::CONTROL.union(KeyModifiers::SHIFT),
    Action::Quit,
    Mode::Browse
)]
#[case::plain_c_comments(
    KeyCode::Char('c'),
    KeyModifiers::NONE,
    Action::Continue,
    Mode::Comment
)]
#[case::alt_c_comments(KeyCode::Char('c'), KeyModifiers::ALT, Action::Continue, Mode::Comment)]
#[case::ctrl_q_is_still_q(KeyCode::Char('q'), KeyModifiers::CONTROL, Action::Quit, Mode::Browse)]
#[case::ctrl_x_is_still_inert(
    KeyCode::Char('x'),
    KeyModifiers::CONTROL,
    Action::Continue,
    Mode::Browse
)]
fn modified_keys_reach_the_state_machine_by_their_code(
    mut multi_app: App,
    #[case] code: KeyCode,
    #[case] modifiers: KeyModifiers,
    #[case] action: Action,
    #[case] mode: Mode,
) {
    let app = &mut multi_app;
    assert_eq!(
        app.on_key_event(KeyEvent::new(code, modifiers))
            .expect("handle the key"),
        action
    );
    assert_eq!(app.mode(), mode);
    assert!(
        shared_tables().comments().is_empty(),
        "a modifier case saved a comment into the tables' fixture"
    );
}
