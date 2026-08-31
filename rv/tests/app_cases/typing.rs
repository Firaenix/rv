//! Typing.

use crossterm::event::KeyCode;
use proptest::prelude::*;
use rv::app::Mode;
use std::cell::RefCell;

use crate::support::*;

/// The comment buffer is exactly the characters typed into it, minus the ones
/// backspaced off — nothing lost, nothing invented, and nothing contributed by
/// a key that is not a character.
///
/// The oracle is a `Vec<char>` built alongside, which is what catches a buffer
/// edited by bytes rather than by characters: a `truncate` in place of `pop`
/// splits a multi-byte character and panics.
#[test]
fn the_buffer_is_exactly_what_was_typed() {
    let fixture = shared_multi();
    let app = RefCell::new(fixture.app());

    let key = prop_oneof![
        6 => any::<char>().prop_filter("newlines commit", |c| *c != '\n').prop_map(KeyCode::Char),
        3 => Just(KeyCode::Backspace),
        3 => prop_oneof![
            Just(KeyCode::Tab),
            Just(KeyCode::Left),
            Just(KeyCode::Delete),
            Just(KeyCode::Home),
            Just(KeyCode::F(9)),
            Just(KeyCode::Null),
            Just(KeyCode::Up),
            Just(KeyCode::PageDown),
        ],
    ];

    run_cases(48, prop::collection::vec(key, 0..40), |keys| {
        let app = &mut *app.borrow_mut();
        rewind(app);
        comment(app);
        prop_assert_eq!(app.mode(), Mode::Comment);

        let mut expected: Vec<char> = Vec::new();
        for key in &keys {
            app.on_key(*key).expect("handle a key");
            match key {
                KeyCode::Char(character) => expected.push(*character),
                KeyCode::Backspace => {
                    expected.pop();
                }
                _ => {}
            }
            let oracle: String = expected.iter().collect();
            prop_assert_eq!(app.buffer(), oracle.as_str(), "after {:?}", key);
            prop_assert_eq!(app.mode(), Mode::Comment);
        }
        // Leave the shared fixture as it was found.
        press(app, KeyCode::Esc);
        prop_assert!(fixture.comments().is_empty());
        Ok(())
    });
}

/// `Esc` discards, whatever was typed: the buffer is gone, the mode is back to
/// browsing, the store never heard of it, and the status line says so.
#[test]
fn escape_never_saves_anything() {
    let fixture = Fixture::multi();
    let app = RefCell::new(fixture.app());

    let seen = Coverage::new(&["an all-whitespace body", "a body worth losing"]);
    run_cases(48, (any_body(), 0usize..8), |(body, downs)| {
        fixture.clear_comments();
        let app = &mut *app.borrow_mut();
        rewind(app);
        press_n(app, KeyCode::Down, downs);

        comment(app);
        prop_assert_eq!(app.mode(), Mode::Comment);
        type_text(app, &body);
        prop_assert_eq!(app.buffer().chars().count(), body.chars().count());

        press(app, KeyCode::Esc);
        prop_assert_eq!(app.mode(), Mode::Browse);
        prop_assert_eq!(app.buffer(), "");
        prop_assert_eq!(app.status(), "comment discarded");
        prop_assert!(
            fixture.comments().is_empty(),
            "escaping saved {:?}",
            fixture.comments()
        );
        seen.hit(usize::from(!body.trim().is_empty()));
        Ok(())
    });
    seen.assert_all();
}
