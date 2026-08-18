//! Totality and state invariants under fuzz.

use crossterm::event::KeyCode;
use proptest::prelude::*;
use rv::app::Action;
use rv::app::Mode;
use std::cell::RefCell;

use crate::support::*;

/// Every state invariant `App` has, checked after every key of an arbitrary
/// sequence — unbound keys, arbitrary `char`s, function, media and modifier
/// keys included.
///
/// The invariants, none of which any keystroke may break:
///
/// 1. `on_key` never returns `Err` and never panics.
/// 2. `line_index` indexes the selected diff's lines, or is `0` when that diff
///    has none.
/// 3. The comment buffer is empty whenever the mode is `Browse`.
/// 4. A selected file always has a loaded diff (the lazy-load invariant
///    `ui::draw`'s "no diff loaded" branch documents as unreachable), that diff
///    is *that file's* — the `diffs` vector stays parallel to `files` — and the
///    sidebar selects the file at `file_index`.
/// 5. The cursor is a **row of the plan it indexes**, and `line_index` is the
///    line that owns that row. The cursor is the state and the line is derived
///    from it (see `rv/src/app.rs`'s `cursor_rows`), so a cursor that has
///    fallen off the end of a plan something shortened under it — a fold, a
///    delete — is a reviewer whose selection and whose scroll position have
///    stopped describing the same place.
///
/// Four, not the five this used to advertise. `selected_file()` is
/// `self.review.files.get(self.file_index)`, so "the sidebar selects
/// `paths[file_index]`" is true by the definition of `Vec::get` once
/// `file_index` is in range, and cannot distinguish one implementation of `App`
/// from another; it is folded into invariant 4 as a consistency check on this
/// test's own `paths` snapshot rather than billed as a property of the app. The
/// in-range check on `file_index` that guards it stays — deleting
/// `select_file`'s bound check is what it exists to catch.
#[test]
fn state_invariants_survive_any_key_sequence() {
    let fixture = Fixture::multi();
    let app = RefCell::new(fixture.app());
    let paths: Vec<String> = app
        .borrow()
        .files()
        .iter()
        .map(|file| file.path.clone())
        .collect();
    assert!(paths.len() >= 3, "the fixture lost files: {paths:?}");

    run_cases(48, prop::collection::vec(any_key(), 0..24), |keys| {
        fixture.clear_comments();
        rewind(&mut app.borrow_mut());
        for (step, key) in keys.iter().enumerate() {
            let app = &mut *app.borrow_mut();
            // Invariant 1.
            app.on_key(*key)
                .map_err(|error| TestCaseError::fail(format!("key {step} {key:?}: {error}")))?;

            // The bound check `select_file` owes the invariants below.
            prop_assert!(
                app.file_index() < paths.len(),
                "after {key:?} at step {step}: file_index {} out of range for {} files",
                app.file_index(),
                paths.len()
            );
            // Invariant 4.
            let selected = app.selected_file().map(|file| file.path.clone());
            prop_assert_eq!(
                selected.as_deref(),
                Some(paths[app.file_index()].as_str()),
                "after {:?} at step {}: the sidebar selects nothing",
                key,
                step
            );
            let diff = app.selected_diff().ok_or_else(|| {
                TestCaseError::fail(format!(
                    "after {key:?} at step {step}: {} is selected with no diff loaded",
                    paths[app.file_index()]
                ))
            })?;
            prop_assert_eq!(
                &diff.path,
                &paths[app.file_index()],
                "after {:?} at step {}: the loaded diff belongs to another file",
                key,
                step
            );
            // Invariant 2.
            let total = diff.lines.len();
            if total == 0 {
                prop_assert_eq!(
                    app.line_index(),
                    0,
                    "after {:?} at step {}: a line is highlighted in an empty diff",
                    key,
                    step
                );
            } else {
                prop_assert!(
                    app.line_index() < total,
                    "after {key:?} at step {step}: line_index {} out of range for {total} lines",
                    app.line_index()
                );
            }
            // Invariant 5.
            let plan = app.plan();
            if plan.rows.is_empty() {
                prop_assert_eq!(
                    app.cursor_row(),
                    0,
                    "after {:?} at step {}: the cursor is off an empty plan",
                    key,
                    step
                );
            } else {
                prop_assert_eq!(
                    plan.line_of_row(app.cursor_row()),
                    Some(app.line_index()),
                    "after {:?} at step {}: the cursor is on row {} of a {}-row plan",
                    key,
                    step,
                    app.cursor_row(),
                    plan.rows.len()
                );
            }

            // Invariant 3.
            if app.mode() == Mode::Browse {
                prop_assert_eq!(
                    app.buffer(),
                    "",
                    "after {:?} at step {}: a comment body outlived comment mode",
                    key,
                    step
                );
            }
        }
        Ok(())
    });
}

/// `Quit` is returned for exactly one key in exactly one mode.
///
/// The `Comment` half is the one that matters to a reviewer: `q` is a letter
/// in a sentence, and a reviewer typing "queries the cache" must not lose the
/// review to it. The `Browse` half says `q` always works, whatever else the
/// app is in the middle of.
#[test]
fn quit_is_exactly_q_in_browse_mode() {
    let fixture = Fixture::multi();
    let app = RefCell::new(fixture.app());
    // `q` is pressed unconditionally rather than sampled: leaving it to the
    // key strategy meant the `Comment` half of the claim — the half a reviewer
    // notices — was only reached in some runs. See `Coverage`.
    let seen = Coverage::new(&["q while browsing", "q while typing"]);

    // `c` is weighted up in the prefix so the `Comment` arm is reached in most
    // cases rather than a handful: `Coverage` below is a hard assertion, and a
    // rarely-sampled arm makes it a flaky one.
    let prefix_key = prop_oneof![3 => any_key(), 1 => Just(KeyCode::Char('c'))];
    run_cases(
        48,
        (prop::collection::vec(prefix_key, 0..16), any_key()),
        |(prefix, other)| {
            fixture.clear_comments();
            let app = &mut *app.borrow_mut();
            rewind(app);
            for key in &prefix {
                app.on_key(*key).expect("handle a prefix key");
            }

            let mode = app.mode();
            // `q` closes the `?` popup rather than quitting, because quitting
            // from a help screen surprises the reviewer least sure what the
            // keys do — so "browsing" is not on its own enough to expect
            // `Quit`, and the prefix can raise the popup with a generated `?`.
            let browsing = mode == Mode::Browse && !app.help_open();
            seen.hit(if mode == Mode::Browse { 0 } else { 1 });
            let action = app
                .on_key(KeyCode::Char('q'))
                .map_err(|error| TestCaseError::fail(error.to_string()))?;
            prop_assert_eq!(
                action == Action::Quit,
                browsing,
                "q in {:?} returned {:?}",
                mode,
                action
            );
            if mode == Mode::Comment {
                prop_assert!(
                    app.buffer().ends_with('q'),
                    "q did not reach the comment buffer {:?}",
                    app.buffer()
                );
            }

            // ...and no other key returns `Quit` in either mode.
            let mode = app.mode();
            let browsing = mode == Mode::Browse && !app.help_open();
            let action = app
                .on_key(other)
                .map_err(|error| TestCaseError::fail(error.to_string()))?;
            prop_assert_eq!(
                action == Action::Quit,
                browsing && other == KeyCode::Char('q'),
                "{:?} in {:?} returned {:?}",
                other,
                mode,
                action
            );
            Ok(())
        },
    );
    seen.assert_all();
}
