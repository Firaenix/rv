//! A large file draws in time that does not grow with the pane.
//!
//! Issue #27: opening a `Cargo.lock` of a few thousand lines froze the
//! reviewer. The cost was not the file but the frame — a per-row overlay
//! rebuilt the whole plan (one row per line) for every row on screen, so a
//! frame cost rows × lines. These tests pin the shape of the cost rather
//! than a stopwatch: how many plans a frame builds, and that the number does
//! not move with the pane's height.

use crossterm::event::KeyCode;
use rv::rows::plans_built;

use crate::support::*;

/// A lockfile-shaped file: many short stanzas, a version bumped in one of
/// every three, a stanza dropped from one in every seven.
fn lock(stanzas: usize, bumped: bool) -> String {
    let mut out = String::new();
    for i in 0..stanzas {
        if bumped && i % 7 == 0 {
            continue;
        }
        let minor = if bumped && i % 3 == 0 { 2 } else { 1 };
        out.push_str(&format!(
            "[[package]]\nname = \"crate{i}\"\nversion = \"0.{minor}.0\"\n\
             source = \"registry+https://github.com/rust-lang/crates.io-index\"\n\
             checksum = \"{i:064x}\"\ndependencies = [\n \"dep{i}\",\n]\n\n"
        ));
    }
    out
}

fn large_lockfile() -> Fixture {
    let workspace = Fixture::new();
    workspace.write("Cargo.lock", &lock(1500, false));
    workspace.jj(&["describe", "-m", "lock"]);
    workspace.jj(&["new"]);
    workspace.write("Cargo.lock", &lock(1500, true));
    workspace.jj(&["describe", "-m", "bump"]);
    workspace.jj(&["new"]);
    workspace
}

/// How many plans one frame at `height` rows builds.
fn plans_per_frame(app: &rv::app::App, height: u16) -> usize {
    let before = plans_built();
    let _ = frame_at(app, 120, height);
    plans_built() - before
}

#[test]
fn a_frame_builds_a_fixed_few_plans_however_tall_the_pane() {
    let workspace = large_lockfile();
    let mut app = workspace.app_from("@--");
    assert!(
        app.displayed_lines().len() > 10_000,
        "the fixture is meant to be large: {} lines",
        app.displayed_lines().len()
    );
    // With the column cursor on a word and a query lit, so both overlays are
    // live — they are what used to ask for a plan per row.
    app.on_key(KeyCode::Char('/')).expect("search");
    for character in "version".chars() {
        app.on_key(KeyCode::Char(character)).expect("type");
    }
    app.on_key(KeyCode::Enter).expect("find");

    let short = plans_per_frame(&app, 12);
    let tall = plans_per_frame(&app, 200);
    assert!(
        short <= 4,
        "a frame built {short} plans of the file; the plan is the one per-line cost and \
         a frame should need a fixed few of them"
    );
    assert_eq!(
        tall, short,
        "a 200-row pane built {tall} plans where a 12-row pane built {short}: something \
         on the per-row path is rebuilding the plan (issue #27)"
    );
}

/// The same shape for a keystroke: moving the cursor is a handful of plans,
/// not one per line or per row.
#[test]
fn a_keystroke_builds_a_fixed_few_plans() {
    let workspace = large_lockfile();
    let mut app = workspace.app_from("@--");
    let before = plans_built();
    app.on_key(KeyCode::Down).expect("j");
    app.on_key(KeyCode::End).expect("End");
    app.on_key(KeyCode::Char(']')).expect("next file");
    app.on_key(KeyCode::Char('[')).expect("back");
    let built = plans_built() - before;
    assert!(
        built <= 40,
        "four keystrokes built {built} plans of the file"
    );
}
