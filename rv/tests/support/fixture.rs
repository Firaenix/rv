//! Colocated jj workspaces the tests review.
//!
//! Every `jj` invocation is made hermetic with `JJ_CONFIG=/dev/null` plus a
//! fixed author, so the developer's own jj config cannot change what the tests
//! see.

use std::fs;
use std::path::Path;
use std::process::Command;

use rv::app::App;
use rv::app::DiffEngine;
use rv::session;
use rv_core::store::Store;
use tempfile::TempDir;

/// The first file every fixture reviews. Two indented lines so that `j` has
/// somewhere to go.
pub const SOURCE: &str = "fn a() {\n    let x = 1;\n}\n";

/// The second file [`Fixture::new`] reviews. Sorts after [`SOURCE`]'s `a.rs`,
/// which keeps `a.rs` the file the reviewer opens on.
pub const SECOND: &str = "fn b() {\n    let y = 2;\n    let z = 3;\n}\n";

/// The base side of [`Fixture::renamed`]: `a.rs`, before the rename.
pub const BASE_SIDE: &str = "fn a() {\n    let x = 1;\n    let y = 2;\n    let z = 3;\n}\n";

/// The head side of [`Fixture::renamed`]: one line rewritten and one added
/// *above* it, so the changed line sits at a different number on each side —
/// 2 on the left, 3 on the right.
pub const HEAD_SIDE: &str =
    "// header\nfn a() {\n    let x = 42;\n    let y = 2;\n    let z = 3;\n}\n";

/// The base side of [`Fixture::rewritten`]: a rewrite that does *not* move, so
/// only the side can tell the two halves of the line apart. The token at
/// [`REWRITE_LITERAL_COLUMN`] is a string here and a number in
/// [`REWRITE_HEAD`], which is what catches a side-blind highlight lookup.
pub const REWRITE_BASE: &str = "fn rewrite() {\n    let value = \"aaa\";\n}\n";

/// See [`REWRITE_BASE`].
pub const REWRITE_HEAD: &str = "fn rewrite() {\n    let value = 12345;\n}\n";

/// The one changed line of [`Fixture::rewritten`], as it stands on each side.
pub const REWRITE_BASE_LINE: &str = "    let value = \"aaa\";";

/// See [`REWRITE_BASE_LINE`].
pub const REWRITE_HEAD_LINE: &str = "    let value = 12345;";

/// Where the literal starts on both sides, counted in characters.
pub const REWRITE_LITERAL_COLUMN: usize = 16;

/// [`Fixture::plain`]'s one file: a `.txt`, for which rv ships no grammar.
pub const PLAIN_TEXT: &str = "just some prose\nand a second line\n";

/// [`Fixture::commented`]'s one file: a `//` comment over a function, with a
/// keyword, a binding and a number beside it so the captures around the comment
/// are in the same frame.
pub const COMMENTED: &str = "// a note about a\nfn a() -> u32 {\n    let x = 1;\n    x\n}\n";

/// How many lines [`Fixture::mixed`]'s `added.rs` is.
pub const ADDED_LINES: u32 = 40;

/// The same for `removed.rs`, which exists at the base and is gone by the head.
pub const REMOVED_LINES: u32 = 25;

/// `count` distinct lines, each naming `prefix`.
pub fn numbered(prefix: &str, count: u32) -> String {
    (0..count)
        .map(|line| format!("{prefix} line {line}\n"))
        .collect()
}

pub struct Fixture {
    tempdir: TempDir,
}

impl Fixture {
    fn empty() -> Self {
        let fixture = Self {
            tempdir: tempfile::tempdir().expect("create temp dir"),
        };
        fixture.jj(&["git", "init", "--colocate"]);
        fixture
    }

    /// One described change adding [`SOURCE`] as `a.rs` and [`SECOND`] as
    /// `b.rs`.
    pub fn new() -> Self {
        let fixture = Self::empty();
        fixture.write("a.rs", SOURCE);
        fixture.write("b.rs", SECOND);
        fixture.jj(&["describe", "-m", "first change"]);
        fixture.jj(&["new"]);
        fixture
    }

    /// A second change that renames `a.rs` to `b.rs` and rewrites a line.
    ///
    /// Reviewed from `@--` (see [`Fixture::app_from`]) rather than the default
    /// `trunk()`, which degrades to the root commit here and would make every
    /// file a plain addition with no base side at all.
    pub fn renamed() -> Self {
        let fixture = Self::empty();
        fixture.write("a.rs", BASE_SIDE);
        fixture.jj(&["describe", "-m", "first change"]);
        fixture.jj(&["new"]);

        fs::remove_file(fixture.root().join("a.rs")).expect("remove a.rs");
        fixture.write("b.rs", HEAD_SIDE);
        fixture.jj(&["describe", "-m", "rename and edit"]);
        fixture.jj(&["new"]);
        fixture
    }

    /// A second change that rewrites one line of `rewrite.rs` **in place**.
    /// Reviewed from `@--`, like [`Fixture::renamed`].
    pub fn rewritten() -> Self {
        let fixture = Self::empty();
        fixture.write("rewrite.rs", REWRITE_BASE);
        fixture.jj(&["describe", "-m", "first change"]);
        fixture.jj(&["new"]);

        fixture.write("rewrite.rs", REWRITE_HEAD);
        fixture.jj(&["describe", "-m", "rewrite a line in place"]);
        fixture.jj(&["new"]);
        fixture
    }

    /// Nothing but additions in one file and nothing but removals in another —
    /// the two ends of the change gradient. Reviewed from `@--`.
    pub fn mixed() -> Self {
        let fixture = Self::empty();
        fixture.write("removed.rs", &numbered("gone", REMOVED_LINES));
        fixture.jj(&["describe", "-m", "first change"]);
        fixture.jj(&["new"]);

        fs::remove_file(fixture.root().join("removed.rs")).expect("remove removed.rs");
        fixture.write("added.rs", &numbered("new", ADDED_LINES));
        fixture.jj(&["describe", "-m", "one file each way"]);
        fixture.jj(&["new"]);
        fixture
    }

    /// A rename that changes **nothing** inside the file, so the review of it
    /// deliberately has no shape at all.
    pub fn pure_rename() -> Self {
        let fixture = Self::empty();
        fixture.write("a.rs", SOURCE);
        fixture.jj(&["describe", "-m", "first change"]);
        fixture.jj(&["new"]);

        fs::remove_file(fixture.root().join("a.rs")).expect("remove a.rs");
        fixture.write("b.rs", SOURCE);
        fixture.jj(&["describe", "-m", "rename and nothing else"]);
        fixture.jj(&["new"]);
        fixture
    }

    /// Four files at three depths, with a chain of single-child directories over
    /// two of them and a different size on every one.
    pub fn nested() -> Self {
        let fixture = Self::empty();
        fixture.write("docs/specs/a.md", &numbered("a", 10));
        fixture.write("docs/specs/b.md", &numbered("b", 5));
        fixture.write("src/lib.rs", &numbered("lib", 30));
        fixture.write("top.rs", &numbered("top", 50));
        fixture.jj(&["describe", "-m", "a change with directories in it"]);
        fixture.jj(&["new"]);
        fixture
    }

    /// One change adding a file whose first line is far wider than any pane.
    pub fn wide() -> Self {
        let fixture = Self::empty();
        fixture.write(
            "wide.rs",
            &format!("// {}\nfn wide() {{}}\n", "abcdefghij".repeat(20)),
        );
        fixture.jj(&["describe", "-m", "one very wide line"]);
        fixture.jj(&["new"]);
        fixture
    }

    /// One file with no grammar rv ships.
    pub fn plain() -> Self {
        let fixture = Self::empty();
        fixture.write("notes.txt", PLAIN_TEXT);
        fixture.jj(&["describe", "-m", "some prose"]);
        fixture.jj(&["new"]);
        fixture
    }

    /// One file opening with a `//` comment — see [`COMMENTED`].
    pub fn commented() -> Self {
        let fixture = Self::empty();
        fixture.write("noted.rs", COMMENTED);
        fixture.jj(&["describe", "-m", "a commented function"]);
        fixture.jj(&["new"]);
        fixture
    }

    /// Five lines of `a.rs`, then a second change that rewrites only the first
    /// of them.
    ///
    /// Reviewed from `@-`, the range holds `a.rs` — so a comment on it is in
    /// range — but its diff carries only line 1, so a comment anchored to line 4
    /// cannot be jumped to. That is the stale-anchor alert with the file-left-the-
    /// range case filtered out from under it.
    pub fn stale_line() -> Self {
        let fixture = Self::empty();
        fixture.write(
            "a.rs",
            "fn a() {\n    let w = 1;\n    let x = 2;\n    let y = 3;\n}\n",
        );
        fixture.jj(&["describe", "-m", "first change"]);
        fixture.jj(&["new"]);
        fixture
    }

    /// Rewrites `a.rs`'s first line and closes the change, so the range from
    /// `@-` holds the file with a diff that does not carry line 4.
    pub fn rewrite_first_line(&self) {
        self.write(
            "a.rs",
            "fn renamed_a() {\n    let w = 1;\n    let x = 2;\n    let y = 3;\n}\n",
        );
        self.jj(&["describe", "-m", "second change"]);
        self.jj(&["new"]);
    }

    /// One file that is a single line of `length` characters.
    pub fn with_long_line(length: usize) -> Self {
        let fixture = Self::empty();
        let line: String = std::iter::repeat_n('x', length).collect();
        fixture.write("long.rs", &format!("{line}\n"));
        fixture.jj(&["describe", "-m", "one very long line"]);
        fixture.jj(&["new"]);
        fixture
    }

    /// The workspace root.
    pub fn root(&self) -> &Path {
        self.tempdir.path()
    }

    /// Runs `jj` in the workspace and returns its stdout, panicking on failure.
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

    /// Writes a file in the working copy, creating parent directories.
    pub fn write(&self, rel: &str, contents: &str) {
        let path = self.root().join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parent dir");
        }
        fs::write(&path, contents).expect("write file");
    }

    /// The reviewer, opened over `trunk()..@` of this workspace.
    pub fn app(&self) -> App {
        let review = session::build(self.root(), None, None).expect("build the review");
        App::open(review, DiffEngine::Structural).expect("open the reviewer")
    }

    /// The reviewer, opened over `base..@` of this workspace.
    pub fn app_from(&self, base: &str) -> App {
        let review = session::build(self.root(), Some(base), None).expect("build the review");
        App::open(review, DiffEngine::Structural).expect("open the reviewer")
    }

    /// A handle on `.review/` that shares nothing with the app's own.
    pub fn store(&self) -> Store {
        Store::open(self.root()).expect("open the store")
    }

    pub fn markdown(&self) -> String {
        fs::read_to_string(self.root().join(".review/REVIEW-FEEDBACK.md"))
            .expect("read REVIEW-FEEDBACK.md")
    }
}
