//! `g o` opens a file by name; `g s` searches every file for text.

use crossterm::event::KeyCode;
use rv::app::Focus;
use rv::app::JumpList;
use rv::app::Mode;
use rv::app::Picking;

use crate::support::*;

#[test]
fn g_o_opens_the_file_typed() {
    let workspace = goto_fixture();
    let mut app = workspace.app();
    app.on_key(KeyCode::Tab).expect("to the sidebar");

    chord(&mut app, 'g', 'o');
    assert_eq!((app.mode(), app.picking()), (Mode::Pick, Picking::File));
    type_text(&mut app, "user");
    let text = buffer_text(&frame_at(&app, 120, 30));
    assert!(text.contains("Open a file"), "{text}");
    assert!(text.contains("▸ user.rs"), "{text}");
    app.on_key(KeyCode::Enter).expect("open it");

    assert_eq!(app.mode(), Mode::Browse);
    assert_eq!(
        app.picking(),
        Picking::Symbol,
        "the next / is a symbol pick"
    );
    assert_eq!(app.file_index(), file_named(&app, "user.rs"));
    assert_eq!(app.focus(), Focus::Diff);
    chord(&mut app, 'g', 'b');
    assert_eq!(app.file_index(), file_named(&app, "a.rs"), "g o is a jump");
}

/// A hit in the file's own name ranks above one in its directory, and letters
/// typed in order still match.
#[test]
fn the_file_picker_ranks_names_ahead_of_directories() {
    let workspace = Fixture::new();
    workspace.write("alpha/notes.rs", "fn n() {}\n");
    workspace.write("beta/alpha.rs", "fn a() {}\n");
    workspace.jj(&["describe", "-m", "nested"]);
    workspace.jj(&["new"]);
    let mut app = workspace.app();

    chord(&mut app, 'g', 'o');
    type_text(&mut app, "alpha");
    let ranked: Vec<&str> = app
        .file_matches()
        .iter()
        .map(|file| app.files()[*file].path.as_str())
        .collect();
    assert_eq!(ranked, ["beta/alpha.rs", "alpha/notes.rs"]);

    app.on_key(KeyCode::Esc).expect("cancel");
    chord(&mut app, 'g', 'o');
    type_text(&mut app, "bal");
    assert_eq!(app.file_matches(), [file_named(&app, "beta/alpha.rs")]);
}

#[test]
fn escape_leaves_the_file_picker_without_moving() {
    let workspace = goto_fixture();
    let mut app = workspace.app();
    chord(&mut app, 'g', 'o');
    type_text(&mut app, "model");
    app.on_key(KeyCode::Esc).expect("cancel");
    assert_eq!(app.mode(), Mode::Browse);
    assert_eq!(app.file_index(), file_named(&app, "a.rs"));
    assert_eq!(app.buffer(), "");
}

/// `/` searches the diff on screen; `g s` searches every file and lists the
/// lines, and choosing one lands the column cursor on the hit.
#[test]
fn g_s_lists_every_line_holding_the_text_across_files() {
    let workspace = goto_fixture();
    let mut app = workspace.app();

    chord(&mut app, 'g', 's');
    assert_eq!((app.mode(), app.picking()), (Mode::Pick, Picking::Text));
    type_text(&mut app, "alpha");
    app.on_key(KeyCode::Enter).expect("search");

    assert_eq!(app.mode(), Mode::References);
    assert_eq!(app.jump_list(), JumpList::Matches);
    let hits: Vec<(String, u32)> = app
        .references()
        .iter()
        .map(|hit| (app.files()[hit.file].path.clone(), hit.line))
        .collect();
    assert_eq!(
        hits,
        [("model.rs".to_owned(), 5), ("user.rs".to_owned(), 2)]
    );
    let text = buffer_text(&frame_at(&app, 120, 30));
    assert!(text.contains("Lines holding alpha"), "{text}");

    app.on_key(KeyCode::Down).expect("to the second hit");
    app.on_key(KeyCode::Enter).expect("jump");
    assert_eq!(app.file_index(), file_named(&app, "user.rs"));
    assert_eq!(app.cursor_line_number(), Some(2));
    assert_eq!(app.word_under_cursor().as_deref(), Some("alpha"));
    chord(&mut app, 'g', 'b');
    assert_eq!(app.file_index(), file_named(&app, "a.rs"));
}

#[test]
fn g_s_with_no_hit_says_so() {
    let workspace = goto_fixture();
    let mut app = workspace.app();
    chord(&mut app, 'g', 's');
    type_text(&mut app, "nowhere at all");
    app.on_key(KeyCode::Enter).expect("search");
    assert_eq!(app.mode(), Mode::Browse);
    assert!(app.status().contains("no match"), "{}", app.status());
}
