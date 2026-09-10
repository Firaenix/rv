//! The workspaces and reviewers a whole case module shares.
//!
//! Split from [`super::fixtures`], which builds a workspace per call: these pay
//! `jj`'s cost once for the whole binary, so only a fixture no case writes into
//! belongs here.

use crossterm::event::KeyCode;
use rstest::fixture;
use rv::app::App;
use rv::app::Focus;
use rv::app::SidebarTab;
use std::sync::OnceLock;

use super::*;

pub fn long_source() -> String {
    (1..=LONG_LINES)
        .map(|index| format!("let long{index:03} = {index};\n"))
        .collect()
}

/// The fixtures no test in this file writes a comment into, shared across the
/// whole binary so their `jj` cost is paid once.
pub fn shared_multi() -> &'static Fixture {
    static MULTI: OnceLock<Fixture> = OnceLock::new();
    MULTI.get_or_init(Fixture::multi)
}

pub fn shared_no_files() -> &'static Fixture {
    static NO_FILES: OnceLock<Fixture> = OnceLock::new();
    NO_FILES.get_or_init(Fixture::no_files)
}

/// The keybinding tables' own workspace, separate from [`shared_multi`].
///
/// The tables assert that no case of theirs put a comment in the store, and
/// that assertion has to be about a store nothing else is touching:
/// `the_buffer_is_exactly_what_was_typed`, `drawing_never_panics_at_any_size`
/// and `drawing_survives_pathological_sizes` all press `c` and type into
/// [`shared_multi`] from other test threads. They never press `Enter` today, so
/// the old shared assertion held — but by luck of what those properties happen
/// not to do, and a failure would have named the wrong test.
pub fn shared_tables() -> &'static Fixture {
    static TABLES: OnceLock<Fixture> = OnceLock::new();
    TABLES.get_or_init(Fixture::multi)
}

/// A read-only reviewer over the keybinding tables' fixture. Not `#[once]`:
/// each case wants its own `App`, and building one is cheap next to building
/// the workspace.
#[fixture]
pub fn multi_app() -> App {
    shared_tables().app()
}

/// The comment browser's own workspace: [`Fixture::multi`] with two comments
/// already saved into `alpha.rs`, so that the browser has rows to move between.
///
/// Shared, and read-only from here on: no case in the browser's table saves or
/// deletes anything, which is what lets them share one `jj` workspace. The
/// table asserts that at the end of every case.
pub fn shared_browser() -> &'static Fixture {
    static BROWSER: OnceLock<Fixture> = OnceLock::new();
    BROWSER.get_or_init(|| {
        let fixture = Fixture::multi();
        let mut app = fixture.app();
        assert_eq!(app.selected_file().expect("a file").path, "alpha.rs");
        for (downs, body) in [(0, "first finding"), (1, "second finding")] {
            press_n(&mut app, KeyCode::Down, downs);
            comment(&mut app);
            type_text(&mut app, body);
            press(&mut app, KeyCode::Enter);
        }
        assert_eq!(fixture.comments().len(), 2, "{:?}", fixture.comments());
        fixture
    })
}

/// A reviewer over [`shared_browser`], already in the comment browser with the
/// sidebar focused — reached the way a reviewer reaches it.
#[fixture]
pub fn browser_app() -> App {
    let mut app = shared_browser().app();
    // `Space m` lands the focus on the sidebar in the comments mode.
    to_comments(&mut app);
    assert_eq!(app.sidebar_tab(), SidebarTab::Comments);
    assert_eq!(app.focus(), Focus::Sidebar);
    // The browser's first *row* is a file heading, so "at the top" is said as
    // the thing it means: the cursor is on the first comment.
    assert_eq!(
        app.browsed_comment().expect("a browsed comment").body,
        "first finding"
    );
    app
}
