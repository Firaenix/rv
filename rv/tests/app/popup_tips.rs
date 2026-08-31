//! The `?` layers overview, in each context.
//!
//! `?` no longer lists a context's individual keys — it shows the leaders every
//! key hangs off (mode, goto, comment, view) and the moves that reach them, so a
//! reviewer learns the *shape* of the keymap rather than one screen of it. The
//! whole map is one more `?` away.

use crossterm::event::KeyCode;

use crate::support::*;

/// The overview advertises all four leaders, and follows the reviewer from one
/// context to the next while doing so.
#[test]
fn the_overview_advertises_the_leaders_from_every_context() {
    let workspace = Fixture::new();
    let mut app = workspace.app();

    // Diff-focused.
    app.on_key(KeyCode::Char('?')).expect("?");
    let diff = buffer_text(&frame_at(&app, 100, 30));
    assert!(diff.contains("DIFF"), "diff overview:\n{diff}");
    for leader in ["mode", "goto", "comment", "view"] {
        assert!(
            diff.contains(leader),
            "{leader} missing from the diff overview:\n{diff}"
        );
    }

    // Sidebar-focused on the Files list: the same leaders, a different title.
    app.on_key(KeyCode::Esc).expect("close the overview");
    app.on_key(KeyCode::Left).expect("focus the sidebar");
    app.on_key(KeyCode::Char('?')).expect("?");
    let files = buffer_text(&frame_at(&app, 100, 30));
    assert!(files.contains("FILES"), "files overview:\n{files}");
    for leader in ["mode", "goto", "comment", "view"] {
        assert!(
            files.contains(leader),
            "{leader} missing from the files overview:\n{files}"
        );
    }
}
