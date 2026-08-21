//! Which keys the `?` contextual tip advertises in each context.
//!
//! `Binding::contexts` is a display predicate on the tip, not a dispatch
//! gate — a key with an empty `contexts` list still works when pressed, it
//! just does not show up in any tip. So a fixed keymap ergonomics review can
//! stay tests-only: adding a context to a row changes nothing about
//! behaviour, only about discoverability. This test pins the
//! discoverability the review shipped, so a later reviewer trimming
//! `contexts` in the name of tidiness cannot silently un-teach a key.

use crossterm::event::KeyCode;

use crate::support::*;

/// A key that dispatches from every context should be findable in every tip
/// that names a place where the reviewer might reach for it — otherwise the
/// tip is a manual that hides its own contents. `f` toggles the diff's
/// full-file context and is answered from anywhere, so every sidebar tip
/// mentions it too; `s` folds the thing under the cursor (a directory in
/// the file lists, a comment box on a diff line, a box in the stack), and
/// every tip whose context has something to fold names it.
#[test]
fn context_tips_advertise_the_keys_that_act_from_them() {
    let workspace = Fixture::new();
    let mut app = workspace.app();

    // Diff-focused: the DIFF tip already had `f` and `s` (via Stack), now
    // confirmed alongside its other movement keys.
    app.on_key(KeyCode::Char('?')).expect("?");
    let diff_tip = buffer_text(&frame_at(&app, 100, 30));
    assert!(diff_tip.contains("DIFF"), "diff tip:\n{diff_tip}");
    assert!(
        diff_tip.contains("full context"),
        "f is not in the diff tip:\n{diff_tip}"
    );
    assert!(
        diff_tip.contains("fold a comment"),
        "s is not in the diff tip:\n{diff_tip}"
    );

    // Sidebar-focused on the Files tab: both `f` and `s` reach the tip too.
    app.on_key(KeyCode::Esc).expect("close the tip");
    app.on_key(KeyCode::Left).expect("focus the sidebar");
    app.on_key(KeyCode::Char('?')).expect("?");
    let files_tip = buffer_text(&frame_at(&app, 100, 30));
    assert!(files_tip.contains("FILES"), "files tip:\n{files_tip}");
    assert!(
        files_tip.contains("full context"),
        "f is not in the files tip:\n{files_tip}"
    );
    assert!(
        files_tip.contains("fold a comment"),
        "s is not in the files tip:\n{files_tip}"
    );
    // ...and the symbol keys, and the direct tab shortcuts, and `Tab`.
    assert!(
        files_tip.contains("next symbol"),
        "n is not in the files tip:\n{files_tip}"
    );
    assert!(
        files_tip.contains("commits tab"),
        "2 is not in the files tip:\n{files_tip}"
    );
    assert!(
        files_tip.contains("the next tab"),
        "Tab is not in the files tip:\n{files_tip}"
    );
}
