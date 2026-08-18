//! The mouse.

use crossterm::event::KeyCode;
use crossterm::event::KeyModifiers;
use crossterm::event::MouseButton;
use crossterm::event::MouseEvent;
use crossterm::event::MouseEventKind;
use proptest::prelude::*;
use rv::app::Action;
use rv::app::Mode;
use std::cell::RefCell;

use crate::support::*;

/// Every gesture a terminal can report, at any coordinate — including the
/// buttons and wheels `rv` binds nothing to, which is where a handler that
/// matched too broadly would show up.
fn any_mouse() -> impl Strategy<Value = MouseEvent> {
    let kind = prop_oneof![
        6 => prop_oneof![
            Just(MouseEventKind::Down(MouseButton::Left)),
            Just(MouseEventKind::Up(MouseButton::Left)),
            Just(MouseEventKind::Drag(MouseButton::Left)),
            Just(MouseEventKind::ScrollUp),
            Just(MouseEventKind::ScrollDown),
        ],
        3 => prop_oneof![
            Just(MouseEventKind::Down(MouseButton::Right)),
            Just(MouseEventKind::Down(MouseButton::Middle)),
            Just(MouseEventKind::Up(MouseButton::Right)),
            Just(MouseEventKind::Drag(MouseButton::Middle)),
            Just(MouseEventKind::Moved),
            Just(MouseEventKind::ScrollLeft),
            Just(MouseEventKind::ScrollRight),
        ],
    ];
    (kind, 0u16..120, 0u16..48).prop_map(|(kind, column, row)| MouseEvent {
        kind,
        column,
        row,
        modifiers: KeyModifiers::NONE,
    })
}

/// No gesture, anywhere, at any terminal size, panics, quits, opens a mode or
/// destroys a comment.
///
/// The size is generated with the frame: a click resolves against the layout
/// that was painted, so painting one and then clicking into it is the whole
/// shape of the thing being fuzzed. The degenerate sizes are in on purpose —
/// a pane two rows tall has no content rows at all, and a hit test that
/// subtracted its borders without saturating would be reading a row index that
/// underflowed.
#[test]
fn no_gesture_panics_quits_or_destroys_a_comment() {
    let fixture = Fixture::multi();
    let mut app = fixture.app();
    press(&mut app, KeyCode::Char('c'));
    assert_eq!(app.mode(), Mode::Comment, "the fixture has nothing to note");
    type_text(&mut app, "a finding");
    press(&mut app, KeyCode::Enter);
    assert_eq!(fixture.comments().len(), 1);

    let app = RefCell::new(app);
    let inputs = (
        any_mouse(),
        prop_oneof![3 => 4u16..80, 1 => 1u16..4],
        prop_oneof![3 => 4u16..40, 1 => 1u16..4],
    );
    run_cases(256, inputs, |(event, width, height)| {
        let app = &mut *app.borrow_mut();
        let _ = render(app, width, height);
        let action = app.on_mouse(event).expect("a gesture");

        prop_assert_eq!(action, Action::Continue, "a gesture ended the review");
        prop_assert_eq!(app.mode(), Mode::Browse, "a gesture opened a mode");
        prop_assert_eq!(app.comments().len(), 1, "a gesture destroyed a comment");
        // The frame after the gesture is where a cursor left past the end of a
        // rebuilt plan would land.
        let _ = render(app, width, height);
        Ok(())
    });

    assert_eq!(fixture.comments().len(), 1);
    assert!(
        !fixture.markdown().contains("outdated"),
        "a gesture rewrote the export:\n{}",
        fixture.markdown()
    );
}
