//! Fixtures shared by the case modules.

use crossterm::event::KeyCode;
use rstest::fixture;
use rv::app::App;
use rv::app::DiffEngine;
use rv::app::Focus;
use rv::app::SidebarTab;
use rv::session;
use rv_core::store::Comment;
use rv_core::store::Store;
use std::fs;
use std::path::Path;
use std::process::Command;
use std::sync::OnceLock;
use tempfile::TempDir;

use super::*;

pub struct Fixture {
    tempdir: TempDir,
}

impl Fixture {
    pub fn init() -> Self {
        let fixture = Self {
            tempdir: tempfile::tempdir().expect("create temp dir"),
        };
        fixture.jj(&["git", "init", "--colocate"]);
        fixture
    }

    /// Five changed files chosen to cover the branches the diff pane and the
    /// comment refusal take *on difftastic's output*: a text file changed on
    /// both sides, a binary file, an added empty file, a long added file, and a
    /// file whose only change is indentation (which difftastic reports as no
    /// semantic change).
    ///
    /// Not every branch: difftastic emits only changed lines, so no diff here
    /// carries a [`LineKind::Context`] line, and none is labelled
    /// `DiffSource::Similar`. [`Fixture::fallback`] owns those.
    pub fn multi() -> Self {
        let fixture = Self::init();
        fixture.write("alpha.rs", ALPHA_BASE);
        fixture.write_bytes("bin.dat", b"\x00\x01\x02 base binary\x00");
        fixture.write("reindent.rs", "fn reindent() {\n    let r = 1;\n}\n");
        fixture.write("stable.txt", "never changes\n");
        fixture.jj(&["describe", "-m", "base change"]);
        fixture.jj(&["new"]);

        fixture.write("alpha.rs", ALPHA_HEAD);
        fixture.write_bytes("bin.dat", b"\x00\x09\x08 head binary\x00");
        fixture.write("reindent.rs", "fn reindent() {\n\t\tlet r = 1;\n}\n");
        fixture.write("blank.txt", "");
        fixture.write("long.rs", &long_source());
        fixture.jj(&["describe", "-m", "head change"]);
        fixture.jj(&["new"]);
        fixture
    }

    /// Two text files whose diffs between them cover both ways two comments can
    /// land on the same line number: `same.rs` rewrites line 2 *in place*, so
    /// difftastic's pair carries `left == right` and the two halves differ only
    /// by side; `alpha.rs` inserts a line above the rewrite, so its two halves
    /// carry different numbers.
    ///
    /// The fixture for [`distinct_comments_are_never_lost_to_each_other`] and
    /// [`both_halves_of_a_same_position_rewrite_keep_their_own_comment`]: both
    /// write comments, so neither may share a workspace with anything else.
    pub fn collisions() -> Self {
        let fixture = Self::init();
        fixture.write("same.rs", SAME_BASE);
        fixture.write("alpha.rs", ALPHA_BASE);
        fixture.jj(&["describe", "-m", "base change"]);
        fixture.jj(&["new"]);

        fixture.write("same.rs", SAME_HEAD);
        fixture.write("alpha.rs", ALPHA_HEAD);
        fixture.jj(&["describe", "-m", "head change"]);
        fixture.jj(&["new"]);
        fixture
    }

    /// One text file with unchanged lines around a rewritten one, reviewed
    /// through the `similar` fallback rather than difftastic (see
    /// [`Fixture::fallback_app`]).
    ///
    /// This is the only fixture that produces [`LineKind::Context`] lines and a
    /// [`DiffSource::Similar`] diff — the diff every user without `difft` on
    /// `PATH` sees, and the one `RV_NO_DIFFT=1` forces.
    pub fn fallback() -> Self {
        let fixture = Self::init();
        fixture.write("ctx.rs", CTX_BASE);
        fixture.jj(&["describe", "-m", "base change"]);
        fixture.jj(&["new"]);

        fixture.write("ctx.rs", CTX_HEAD);
        fixture.jj(&["describe", "-m", "head change"]);
        fixture.jj(&["new"]);
        fixture
    }

    /// Two files whose only change is in their line *terminators*: `eol.rs`
    /// gains a final newline, `crlf.txt` goes from CRLF to LF.
    ///
    /// The one input on which the two diff engines disagree about shape while
    /// agreeing about meaning. Both call it suppressed — but difftastic says so
    /// with `status: "unchanged"` and emits no chunks, while the `similar`
    /// fallback renders both sides to the same lines and reports them as
    /// `Context` (see `rv_core::diff::similar_diff`). So the fallback's diff is
    /// suppressed *and* has lines, which is the case the pane, `j`/`k` and the
    /// comment anchor have to agree about.
    ///
    /// Reviewed through [`Fixture::fallback_app`] for the fallback half and
    /// [`Fixture::app`] for the difftastic contrast, from the very same
    /// workspace.
    pub fn terminator() -> Self {
        let fixture = Self::init();
        fixture.write("crlf.txt", CRLF_BASE);
        fixture.write("eol.rs", EOL_BASE);
        fixture.jj(&["describe", "-m", "base change"]);
        fixture.jj(&["new"]);

        fixture.write("crlf.txt", CRLF_HEAD);
        fixture.write("eol.rs", EOL_HEAD);
        fixture.jj(&["describe", "-m", "head change"]);
        fixture.jj(&["new"]);
        fixture
    }

    /// A rename plus two rewritten lines: the fixture for everything about
    /// which *side* a comment anchors to.
    pub fn renamed() -> Self {
        let fixture = Self::init();
        fixture.write("a.rs", RENAME_BASE);
        fixture.jj(&["describe", "-m", "base change"]);
        fixture.jj(&["new"]);

        fs::remove_file(fixture.root().join("a.rs")).expect("remove a.rs");
        fixture.write("b.rs", RENAME_HEAD);
        fixture.jj(&["describe", "-m", "rename and edit"]);
        fixture.jj(&["new"]);
        fixture
    }

    /// A non-empty stack that changes no files at all: `@--..@` spans two
    /// described-but-empty changes, so `session::build` succeeds (the range is
    /// not empty) while `App` opens with zero files. That is the state
    /// `ui::draw`'s "no changed files in this range" branch exists for, and
    /// the one where every accessor on `App` has to answer `None`.
    pub fn no_files() -> Self {
        let fixture = Self::init();
        fixture.write("only.rs", "fn only() {}\n");
        fixture.jj(&["describe", "-m", "base change"]);
        fixture.jj(&["new"]);
        fixture.jj(&["describe", "-m", "an empty change"]);
        fixture.jj(&["new"]);
        fixture
    }

    pub fn root(&self) -> &Path {
        self.tempdir.path()
    }

    pub fn jj(&self, args: &[&str]) -> String {
        let output = Command::new("jj")
            .args(args)
            .current_dir(self.root())
            .env("JJ_CONFIG", "/dev/null")
            .env("JJ_USER", "rv-test")
            .env("JJ_EMAIL", "rv-test@localhost")
            .output()
            .expect("run jj");
        assert!(
            output.status.success(),
            "jj {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).expect("jj stdout is utf-8")
    }

    pub fn write(&self, rel: &str, contents: &str) {
        self.write_bytes(rel, contents.as_bytes());
    }

    pub fn write_bytes(&self, rel: &str, contents: &[u8]) {
        let path = self.root().join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parent dir");
        }
        fs::write(&path, contents).expect("write file");
    }

    /// The reviewer over `base..@`, where `base` defaults to `@--` — the
    /// change below the one every fixture above puts its head state in.
    pub fn app(&self) -> App {
        let review = session::build(self.root(), Some("@--"), None).expect("build the review");
        App::open(review, DiffEngine::Auto).expect("open the reviewer")
    }

    /// The reviewer over the same range, with difftastic bypassed: every diff
    /// comes from the `similar` fallback.
    ///
    /// Per-`App` rather than `RV_NO_DIFFT`, which is process-wide: integration
    /// tests run in parallel threads, so setting that variable here would
    /// silently swap the diff engine under every other property in this binary.
    pub fn fallback_app(&self) -> App {
        let review = session::build(self.root(), Some("@--"), None).expect("build the review");
        App::open(review, DiffEngine::Fallback).expect("open the reviewer")
    }

    /// A store handle that shares nothing with the app's own, so an assertion
    /// through it is about what reached the disk.
    pub fn store(&self) -> Store {
        Store::open(self.root()).expect("open the store")
    }

    pub fn comments(&self) -> Vec<Comment> {
        self.store().comments().expect("read comments.json")
    }

    pub fn markdown(&self) -> String {
        fs::read_to_string(self.root().join(".review/REVIEW-FEEDBACK.md")).unwrap_or_default()
    }

    /// Forgets every stored comment, so the next case starts from an empty
    /// store. Snapshots are left behind: nothing reads one except by an id
    /// `comments.json` still lists.
    pub fn clear_comments(&self) {
        let _ = fs::remove_file(self.root().join(".review/comments.json"));
        let _ = fs::remove_file(self.root().join(".review/REVIEW-FEEDBACK.md"));
    }
}

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
            press_n(&mut app, KeyCode::Char('j'), downs);
            press(&mut app, KeyCode::Char('c'));
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
    to_comments(&mut app);
    press(&mut app, KeyCode::Left);
    assert_eq!(app.sidebar_tab(), SidebarTab::Comments);
    assert_eq!(app.focus(), Focus::Sidebar);
    assert_eq!(app.browser_index(), 0);
    app
}
