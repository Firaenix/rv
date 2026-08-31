//! Strategies shared by the case modules.

use crossterm::event::KeyCode;
use proptest::prelude::*;

/// Every key the reviewer might see, weighted so a random walk actually
/// navigates instead of drowning in inert keys.
pub fn any_key() -> impl Strategy<Value = KeyCode> {
    prop_oneof![
        18 => prop_oneof![
            Just(KeyCode::Char(']')),
            Just(KeyCode::Char('[')),
            // The leaders, and the keys their submenus answer: a random walk
            // that could not press them would leave commenting, deleting and
            // the view toggles out of every fuzzed invariant below.
            Just(KeyCode::Char('c')),
            Just(KeyCode::Char('g')),
            Just(KeyCode::Char('v')),
            Just(KeyCode::Char('d')),
            Just(KeyCode::Char('r')),
            Just(KeyCode::Char('q')),
            Just(KeyCode::Char('y')),
            Just(KeyCode::Char('s')),
            Just(KeyCode::Down),
            Just(KeyCode::Up),
            Just(KeyCode::Enter),
            Just(KeyCode::Esc),
            Just(KeyCode::Backspace),
        ],
        6 => any::<char>().prop_map(KeyCode::Char),
        2 => (1u8..=20).prop_map(KeyCode::F),
        4 => prop_oneof![
            Just(KeyCode::Left),
            Just(KeyCode::Right),
            Just(KeyCode::Home),
            Just(KeyCode::End),
            Just(KeyCode::PageUp),
            Just(KeyCode::PageDown),
            Just(KeyCode::Tab),
            Just(KeyCode::BackTab),
            Just(KeyCode::Delete),
            Just(KeyCode::Insert),
            Just(KeyCode::Null),
            Just(KeyCode::CapsLock),
            Just(KeyCode::NumLock),
            Just(KeyCode::PrintScreen),
            Just(KeyCode::Pause),
            Just(KeyCode::Menu),
            Just(KeyCode::KeypadBegin),
            Just(KeyCode::Modifier(
                crossterm::event::ModifierKeyCode::LeftControl
            )),
            Just(KeyCode::Media(crossterm::event::MediaKeyCode::Play)),
        ],
    ]
}

/// Comment bodies a reviewer might plausibly type, plus every markdown and
/// `rv`-protocol marker that the export has to survive verbatim.
///
/// No `'\n'`: `Enter` commits, so a newline is not typeable, and generating
/// one would test a state the keyboard cannot reach.
pub fn any_body() -> impl Strategy<Value = String> {
    let piece = prop_oneof![
        5 => "[ -~]{0,8}",
        2 => prop_oneof![
            Just("**bold**".to_owned()),
            Just("`code`".to_owned()),
            Just("```".to_owned()),
            Just("~~~".to_owned()),
            Just("### 1. heading".to_owned()),
            Just("## Open (1)".to_owned()),
            Just("**Reply:** not a real reply".to_owned()),
            Just("**Comment:** not a real comment".to_owned()),
            Just("<!-- rv:anchor id=deadbeef -->".to_owned()),
            Just("<details><summary>x</summary>".to_owned()),
        ],
        2 => prop_oneof![
            Just("héllo wörld".to_owned()),
            Just("日本語のテキスト".to_owned()),
            Just("🎉🙈 emoji".to_owned()),
            Just("عربى".to_owned()),
            Just("a\u{0301}combining".to_owned()),
        ],
        1 => prop_oneof![
            Just(" ".to_owned()),
            Just("\t".to_owned()),
            Just("\r".to_owned()),
            Just("\u{a0}".to_owned()),
            Just("\u{2028}".to_owned()),
        ],
    ];
    prop::collection::vec(piece, 0..4).prop_map(|parts| parts.concat())
}

// ---------------------------------------------------------------------------
