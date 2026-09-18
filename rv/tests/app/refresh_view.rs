//! Every display toggle the reviewer can flip survives `v r`.
//!
//! Not a list of toggles: the test walks the **runtime keymap's `v` leader**,
//! so a toggle added to the table is covered the day it lands, with nothing
//! here to update. The oracle is the painted screen — what the reviewer sees
//! before and after the refresh must match, bar excepted — so the test also
//! needs no getter per preference. `v #` and `v c` were lost on refresh for
//! as long as the preferences were carried across one hand-listed field at a
//! time; this is what makes a repeat of that impossible to ship quietly.

use crossterm::event::KeyCode;
use rv::app::App;
use rv::app::Leader;

use crate::support::*;

const WIDTH: u16 = 100;
const HEIGHT: u16 = 30;

/// Two adjacent lines rewritten, so that grouping removals before additions
/// draws differently from interleaving them — an added file, or a single
/// changed line, looks the same either way.
const BASE: &str = "fn a() {\n    let x = 1;\n}\n";
const HEAD: &str = "fn b() {\n    let x = 2;\n}\n";

fn rewritten_twice() -> Fixture {
    let workspace = Fixture::new();
    workspace.write("a.rs", HEAD);
    workspace.jj(&["describe", "-m", "rewrite two lines"]);
    workspace.jj(&["new"]);
    assert_ne!(BASE, HEAD);
    workspace
}

/// The two places a toggle can show: the commits list with the sidebar
/// focused (the change tooltip `v i` hides is drawn nowhere else), and the
/// diff with the diff focused.
#[derive(Clone, Copy, Debug)]
enum Where {
    Commits,
    Diff,
}

fn place(app: &mut App, at: Where) {
    match at {
        Where::Commits => to_commits(app),
        Where::Diff => {
            app.on_key(KeyCode::Char('m')).expect("mode leader");
            app.on_key(KeyCode::Char('d')).expect("the diff");
        }
    }
}

/// The screen without its bottom row — glyphs *and* styles, since a toggle
/// like the tint changes nothing but colour. The bar carries the status,
/// which a refresh rewrites by design. Painted once every background job —
/// the diffs, the full-context merges a fresh app starts over — has
/// finished, so two frames differ only by what the reviewer chose.
fn screen(app: &mut App) -> String {
    app.finish_loading();
    app.finish_merging();
    let buffer = frame_at(app, WIDTH, HEIGHT);
    let mut out = String::new();
    for y in 0..HEIGHT - 1 {
        for x in 0..WIDTH {
            let cell = &buffer[(x, y)];
            out.push_str(cell.symbol());
            out.push_str(&format!("{:?}", cell.style()));
        }
        out.push('\n');
    }
    out
}

#[test]
fn every_v_toggle_survives_a_refresh() {
    let workspace = rewritten_twice();
    let probe = workspace.app_from("@--");
    let leader = probe.keymap().leader_key(Leader::View);
    let toggles: Vec<(String, String, KeyCode)> = probe
        .keymap()
        .bindings()
        .iter()
        .filter(|binding| binding.leader == Some(Leader::View))
        .map(|binding| {
            (
                binding.keys_label.clone(),
                binding.what.to_owned(),
                binding.codes[0],
            )
        })
        .collect();
    assert!(
        toggles.len() > 5,
        "the v leader lists {} keys",
        toggles.len()
    );

    let mut checked = 0;
    for (key, what, code) in toggles {
        let mut seen = false;
        for at in [Where::Commits, Where::Diff] {
            let mut app = workspace.app_from("@--");
            place(&mut app, at);
            let before = screen(&mut app);

            app.on_key(KeyCode::Char(leader)).expect("view leader");
            app.on_key(code).expect(&what);
            if app.status().starts_with("refreshed") {
                // The refresh key itself proves nothing about surviving one.
                seen = true;
                continue;
            }
            let toggled = screen(&mut app);
            if toggled == before {
                continue;
            }
            seen = true;

            app.on_key(KeyCode::Char(leader)).expect("view leader");
            app.on_key(KeyCode::Char('r')).expect("refresh");
            assert!(app.status().starts_with("refreshed"), "{}", app.status());
            assert_eq!(
                screen(&mut app),
                toggled,
                "`v r` undid `v {key}` ({what}) in the {at:?} — is its state in `app::view::View`?"
            );
            checked += 1;
        }
        assert!(
            seen,
            "`v {key}` ({what}) changed nothing on screen in either place, so this test \
             cannot see whether a refresh keeps it — give it a visible effect here"
        );
    }
    assert!(checked > 5, "only {checked} toggles were checked");
}
