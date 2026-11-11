//! A review with a struct, a function returning it, and a call to that
//! function from another file — what `g i`, `g d`, `g b`, `g o` and `g s` are
//! tested over.

use crossterm::event::KeyCode;
use rv::app::App;
use rv::app::DiffEngine;
use rv::session;

use super::Fixture;

pub const MODEL: &str =
    "struct Gamma {\n    value: u32,\n}\n\nfn alpha() -> Gamma {\n    Gamma { value: 1 }\n}\n";

pub const USER: &str = "fn beta() {\n    let made = alpha();\n}\n";

/// [`Fixture::new`]'s `a.rs` and `b.rs`, plus `model.rs` and `user.rs`.
pub fn goto_fixture() -> Fixture {
    let fixture = Fixture::new();
    fixture.write("model.rs", MODEL);
    fixture.write("user.rs", USER);
    fixture.jj(&["describe", "-m", "symbols to go to"]);
    fixture.jj(&["new"]);
    fixture
}

/// The reviewer over `fixture` with a `keybindings.toml` and `Config.toml`.
pub fn app_configured(
    fixture: &Fixture,
    keybindings: &str,
    settings: &rv::config::Settings,
) -> App {
    let review = session::build(fixture.root(), None, None).expect("build the review");
    let config = rv::config::parse(keybindings).expect("parse the keybindings");
    let mut app = App::open_with_config(review, DiffEngine::Structural, &config, settings)
        .expect("open the reviewer");
    app.finish_loading();
    app.finish_merging();
    app
}

pub fn file_named(app: &App, path: &str) -> usize {
    app.files()
        .iter()
        .position(|file| file.path == path)
        .unwrap_or_else(|| panic!("{path} is not in the review"))
}

/// Steps through the files with `]`/`[` until `path` is the one shown.
pub fn show_file(app: &mut App, path: &str) {
    let wanted = file_named(app, path);
    while app.file_index() != wanted {
        let key = if app.file_index() < wanted { ']' } else { '[' };
        app.on_key(KeyCode::Char(key)).expect("step a file");
    }
    app.finish_loading();
    app.finish_merging();
}

/// Moves the cursor onto source line `number` of the shown file.
pub fn to_line(app: &mut App, number: u32) {
    app.on_key(KeyCode::Home).expect("to the top");
    for _ in 0..200 {
        if app.cursor_line_number() == Some(number) {
            return;
        }
        app.on_key(KeyCode::Down).expect("down a row");
    }
    panic!("line {number} is not in the diff");
}

/// Walks the column cursor along the line until it reads `word`.
pub fn to_word(app: &mut App, word: &str) {
    for _ in 0..40 {
        if app.word_under_cursor().as_deref() == Some(word) {
            return;
        }
        app.on_key(KeyCode::Char('l')).expect("next word");
    }
    panic!("{word:?} is not on the selected line");
}

/// A leader and its child, as two key presses.
pub fn chord(app: &mut App, leader: char, key: char) {
    app.on_key(KeyCode::Char(leader)).expect("the leader");
    app.on_key(KeyCode::Char(key)).expect("the leader's child");
}
