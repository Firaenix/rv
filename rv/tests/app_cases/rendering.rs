//! Rendering.

use crossterm::event::KeyCode;
use proptest::prelude::*;
use ratatui::layout::Rect;
use rv::app::Mode;
use std::cell::RefCell;

use crate::support::*;

/// The highlighted line is always on screen, and it is the *selected* line that
/// is highlighted.
///
/// `ui::window` centers the visible slice on the selection wherever the file
/// is long enough to center anything, which is what keeps `j` from walking the
/// highlight off the bottom of a pane. Checked over every line of a
/// forty-line diff at every pane height that has room for a row at all —
/// the failure a fixed `0..height` window would cause is invisible until the
/// selection passes the fold.
///
/// Both halves are checked *at the swept height*, which is the whole point of
/// sweeping it: `long.rs` has forty lines, so every height below ~42 makes the
/// window scroll, and the highlight then sits at a row whose position in the
/// pane is not its index in the diff. A number read off a fixed, generous
/// geometry would agree with the app on every one of those heights without ever
/// exercising the scrolled case.
#[test]
fn the_highlighted_line_is_always_rendered() {
    let fixture = shared_multi();
    let app = RefCell::new(fixture.app());
    let long = {
        let app = app.borrow();
        app.files()
            .iter()
            .position(|file| file.path == "long.rs")
            .expect("long.rs is in the review")
    };
    let total = {
        let app = &mut *app.borrow_mut();
        rewind(app);
        press_n(app, KeyCode::Char(']'), long);
        lines(app).len()
    };
    assert!(total >= 20, "long.rs produced only {total} diff lines");

    // A `Browse` bar takes one row and the pane's borders two, so a height of
    // four is the smallest that can show a line at all.
    run_cases(64, (0usize..total, 4u16..48), |(index, height)| {
        let app = &mut *app.borrow_mut();
        rewind(app);
        press_n(app, KeyCode::Char(']'), long);
        press_n(app, KeyCode::Down, index);
        prop_assert_eq!(app.line_index(), index);

        let line = lines(app)[index].clone();
        let frame = render(app, 120, height).backend().to_string();
        prop_assert!(
            frame.contains(line.text.trim_end()),
            "line {} ({:?}) is not on screen at height {}:\n{}",
            index,
            line.text,
            height,
            frame
        );

        // The number beside it is the highlighted one, at this height: "on
        // screen" means highlighted rather than merely present, and the row
        // wearing the highlight is the selected line rather than whatever
        // happens to sit at the same offset inside a scrolled window.
        prop_assert_eq!(
            printed_number(app, 120, height),
            anchored_number(&line),
            "at height {} the highlight is not on line {} ({:?}):\n{}",
            height,
            index,
            line,
            render(app, 120, height).backend().to_string()
        );
        Ok(())
    });
}

/// `ui::draw` is total and deterministic: no terminal size, no mode, no
/// selection and no comment body makes it panic, and painting the same app
/// twice paints the same cells.
///
/// Totality is the load-bearing half — a one-row or one-column terminal is
/// where ratatui layout code classically panics, a `Comment` bar asks for three
/// rows out of a frame that may have one, and the diff pane subtracts two rows
/// of border from whatever is left. Degenerate sizes are therefore *weighted
/// in* rather than left to a uniform draw over `1..40`, and the coverage receipt
/// records that they were reached: a sweep that never went below four rows
/// would report green while leaving the arithmetic this exists to check
/// untried.
///
/// What this deliberately does **not** claim any more is that "drawing is a
/// pure projection". `ui::draw(frame, app)` takes `&App`: that it moves neither
/// the selection nor the mode nor the buffer is enforced by the borrow checker
/// before a single case runs, so asserting it here proved nothing (and the old
/// `file_index == file.min(count - 1)` assertion was literally `file == file`,
/// since `file` was drawn from `0..count`). Determinism is the part the types
/// do not give for free: it is what fails if a frame ever comes to depend on a
/// clock, a counter or any other state the app does not own.
#[test]
fn drawing_never_panics_at_any_size() {
    let fixture = shared_multi();
    let app = RefCell::new(fixture.app());
    let count = app.borrow().files().len();

    let inputs = (
        prop_oneof![3 => 1u16..60, 1 => 1u16..4],
        prop_oneof![3 => 1u16..40, 1 => 1u16..4],
        0usize..count,
        0usize..48,
        prop_oneof![Just(None), any_body().prop_map(Some)],
    );
    let seen = Coverage::new(&[
        "a status bar",
        "a comment box",
        "a terminal with no room for a diff row",
    ]);
    run_cases(64, inputs, |(width, height, file, downs, body)| {
        let app = &mut *app.borrow_mut();
        rewind(app);
        press_n(app, KeyCode::Char(']'), file);
        press_n(app, KeyCode::Down, downs);
        // Only type once the box is actually open. `c` on a binary or empty
        // diff is refused, and the body would then be pressed *as browse keys*
        // — `[` and `j` and all — which moves the selection out from under the
        // case rather than filling a comment box.
        let typing = match &body {
            Some(body) => {
                comment(app);
                let opened = app.mode() == Mode::Comment;
                if opened {
                    type_text(app, body);
                }
                opened
            }
            None => false,
        };
        seen.hit(usize::from(typing));
        let bar = if typing { 3 } else { 1 };
        if height.saturating_sub(bar) <= 2 {
            seen.hit(2);
        }

        let frame = render(app, width, height);
        let buffer = frame.backend().buffer().clone();
        prop_assert_eq!(buffer.area, Rect::new(0, 0, width, height));

        // Same app, same frame: nothing outside `App` decides what is painted.
        let again = render(app, width, height);
        prop_assert_eq!(
            again.backend().buffer(),
            &buffer,
            "drawing {}x{} twice painted two different frames",
            width,
            height
        );

        press(app, KeyCode::Esc);
        Ok(())
    });
    seen.assert_all();
}

/// The same totality claim, with **comment boxes on screen** — which is where
/// the arithmetic actually is.
///
/// A box subtracts a seven-column gutter, two borders and their padding from
/// whatever width it is given, and its body is wrapped to what is left; the
/// row model then windows over rows rather than lines. Every one of those is a
/// subtraction that panics if it is not saturating, and none of them runs at
/// all in `drawing_never_panics_at_any_size`, whose fixture has no comments in
/// it by construction.
///
/// The walk is navigation only. `d` and `y` would empty the fixture under the
/// sweep — and what a delete does is pinned elsewhere — while every key here
/// changes what is *drawn*: the focus, the tab, the browser's row, which boxes
/// are folded, and which line the window is centred on.
#[test]
fn drawing_never_panics_with_comment_boxes_on_screen() {
    let fixture = Fixture::multi();
    let app = RefCell::new(fixture.app());
    {
        let app = &mut *app.borrow_mut();
        rewind(app);
        for body in [
            "first finding",
            "a second finding, long enough that it has to wrap several times over in any pane \
             narrow enough to be worth sweeping",
        ] {
            comment(app);
            type_text(app, body);
            press(app, KeyCode::Enter);
        }
        press(app, KeyCode::Down);
        comment(app);
        type_text(app, "third finding");
        press(app, KeyCode::Enter);
    }
    assert_eq!(fixture.comments().len(), 3, "{:?}", fixture.comments());

    let key = prop_oneof![
        Just(KeyCode::Down),
        Just(KeyCode::Up),
        Just(KeyCode::Enter),
        Just(KeyCode::Esc),
        Just(KeyCode::Left),
        Just(KeyCode::Right),
        Just(KeyCode::Tab),
        Just(KeyCode::Char('s')),
        Just(KeyCode::Char(']')),
        Just(KeyCode::Char('[')),
    ];
    let inputs = (
        prop_oneof![3 => 1u16..60, 1 => 1u16..5],
        prop_oneof![3 => 1u16..40, 1 => 1u16..5],
        prop::collection::vec(key, 0..12),
    );
    let seen = Coverage::new(&[
        "a terminal with no room for a diff row",
        "a box actually drawn",
    ]);
    run_cases(48, inputs, |(width, height, keys)| {
        let app = &mut *app.borrow_mut();
        rewind(app);
        for key in &keys {
            app.on_key(*key)
                .map_err(|error| TestCaseError::fail(format!("{key:?}: {error}")))?;
            let frame = render(app, width, height);
            prop_assert_eq!(
                frame.backend().buffer().area,
                Rect::new(0, 0, width, height)
            );
        }
        if height <= 3 {
            seen.hit(0);
        }
        // Whatever the walk left behind, drawn at the sizes that have
        // historically broken ratatui layout arithmetic — spelled out rather
        // than sampled, because 1x1 is the case and a uniform draw would visit
        // it once in a hundred runs.
        for (width, height) in PATHOLOGICAL {
            let frame = render(app, width, height);
            prop_assert_eq!(
                frame.backend().buffer().area,
                Rect::new(0, 0, width, height)
            );
        }
        // Asked *inside* the diff pane. The panes' own corners are rounded
        // now, so a `╭` anywhere in the frame is a pane frame and this
        // coverage probe would report every case as having drawn a box.
        if box_drawn(app, 120, 44) {
            seen.hit(1);
        }
        Ok(())
    });
    seen.assert_all();
    fixture.clear_comments();
}
