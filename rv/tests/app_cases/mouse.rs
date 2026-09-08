//! The mouse.

use crossterm::event::KeyCode;
use crossterm::event::KeyModifiers;
use crossterm::event::MouseButton;
use crossterm::event::MouseEvent;
use crossterm::event::MouseEventKind;
use proptest::prelude::*;
use rv::app::Action;
use rv::app::Mode;
use rv::app::SidebarTab;
use rv::tree::NodeKind;
use rv::ui;
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
    comment(&mut app);
    assert_eq!(app.mode(), Mode::Comment, "the fixture has nothing to note");
    type_text(&mut app, "a finding");
    press(&mut app, KeyCode::Enter);
    assert_eq!(fixture.comments().len(), 1);

    let app = RefCell::new(app);
    // The tab is generated with the gesture on purpose: every row kind — a
    // file, a directory, a change, a comment, a browser heading — lives in a
    // different one of the three lists, and a fuzz that only ever started from
    // the files list would never point the pointer at a row of another.
    let tabs = prop_oneof![
        Just(SidebarTab::Files),
        Just(SidebarTab::Commits),
        Just(SidebarTab::Comments),
    ];
    let inputs = (
        any_mouse(),
        tabs,
        prop_oneof![3 => 4u16..80, 1 => 1u16..4],
        prop_oneof![3 => 4u16..40, 1 => 1u16..4],
    );
    run_cases(256, inputs, |(event, tab, width, height)| {
        let app = &mut *app.borrow_mut();
        match tab {
            SidebarTab::Files => to_files(app),
            SidebarTab::Commits => to_commits(app),
            SidebarTab::Comments => to_comments(app),
        }
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

/// The row clicked *is* the row selected, in either node-list tab.
///
/// The safety fuzz above cannot see the Commits-tab bug it was written around:
/// a click that selected the wrong file panicked nothing and quit nothing, it
/// simply aimed the next comment at a different file's line. This is the other
/// half of the claim — a click on a file row selects **that row's** file, not
/// whatever sits at the same number in whichever index space the handler
/// happens to read. The oracle is the node under the pointer, read off the
/// same `nodes()` list the renderer painted.
///
/// On [`Fixture::multi`] the claim would pass while the bug was still in: the
/// review is a single change, so the commits tab's pair order and the
/// bookmark's file order are the same order and the two index spaces agree
/// everywhere. [`Fixture::stack`] splits the files across two changes, newest
/// first, so the spaces genuinely disagree and only one of them is right.
#[test]
fn clicking_a_file_row_selects_that_rows_file() {
    let fixture = Fixture::stack();
    let tabs = prop_oneof![Just(SidebarTab::Files), Just(SidebarTab::Commits)];
    // The click's row is generated directly rather than taken as a remainder of
    // the painted pane height: a row past the last node is its own arm — a
    // click in the pane but below the list must select nothing — and clipping it
    // into range would never sample that arm. Sizes are bounded so the sidebar
    // always has at least one content row.
    let sizes = (40u16..80, 8u16..40u16);
    let coverage = Coverage::new(&[
        "files tab",
        "commits tab",
        "click below the list",
        "a row whose two index spaces disagree",
    ]);
    let app = RefCell::new(fixture.app());
    run_cases(
        256,
        (tabs, sizes, 0usize..60),
        |(tab, (width, height), row)| {
            let app = &mut *app.borrow_mut();
            rewind(app);
            match tab {
                SidebarTab::Files => to_files(app),
                SidebarTab::Commits => to_commits(app),
                // The safety fuzz covers the comment browser; its rows are
                // comments, not files, so there is nothing here to click *at*.
                SidebarTab::Comments => return Ok(()),
            }
            let painted = {
                let _ = render(app, width, height);
                app.painted_layout()
            };
            let pane = painted.sidebar;
            // The hit test answers only absolute rows `pane.y + 1` up to the last
            // one above the bottom border. A generated row past that is outside the
            // pane — no handler runs — so it cannot test this property.
            let inner_rows = usize::from(pane.height.saturating_sub(2));
            if row >= inner_rows || pane.width < 2 {
                return Ok(());
            }
            // One column inside the left border. The chevron is the bar's first
            // cell, not a row of this pane, so no column here can fold the sidebar
            // by accident.
            let column = pane.x + 1;
            let index = ui::sidebar_index_at(app, pane, row);
            let expected =
                match index.and_then(|i| app.nodes().get(i).map(|node| node.kind.clone())) {
                    Some(NodeKind::File { index }) => match tab {
                        // The oracle is the same expression the handler must compute:
                        // a bookmark file's path is `files()[index].path`, a commit
                        // pair's is `commit_path(index)`. Anything the handler
                        // "translated" past these is the bug this catches.
                        SidebarTab::Files => Some(app.files()[index].path.clone()),
                        SidebarTab::Commits => {
                            let path = app.commit_path(index).map(str::to_string);
                            // The file a bookmark-file translation would have picked at
                            // the same number. Most rows of this fixture's commits list
                            // are the only cases that could tell the two translations
                            // apart, and a run that sampled none of them tested nothing.
                            if app.files().get(index).map(|file| file.path.clone()) != path {
                                coverage.hit(3);
                            }
                            path
                        }
                        SidebarTab::Comments => None,
                    },
                    Some(_) => {
                        // A dir, commit heading, or `..` row: the click folds it and
                        // selects nothing, so there is no selection claim to make.
                        app.on_mouse(left_click(column, pane.y + 1 + row as u16))
                            .expect("a gesture");
                        return Ok(());
                    }
                    None => {
                        // The pointer is in the pane but below the list: nothing is
                        // chosen and the selection stands. Clicking blindly would
                        // compare against a selection no case asked to change.
                        coverage.hit(2);
                        app.on_mouse(left_click(column, pane.y + 1 + row as u16))
                            .expect("a gesture");
                        return Ok(());
                    }
                };
            coverage.hit(if tab == SidebarTab::Files { 0 } else { 1 });
            app.on_mouse(left_click(column, pane.y + 1 + row as u16))
                .expect("a gesture");
            prop_assert_eq!(
                app.selected_file().map(|file| file.path.clone()),
                expected,
                "the click at row {row} of the {tab:?} list did not select the file drawn there",
                row = row,
                tab = tab
            );
            Ok(())
        },
    );
    coverage.assert_all();
}

fn left_click(column: u16, row: u16) -> MouseEvent {
    MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column,
        row,
        modifiers: KeyModifiers::NONE,
    }
}
