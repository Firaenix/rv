//! The throwaway jj workspace the `cli` cases drive the real `rv` binary against.
//!
//! A copy of `rv-core/tests/fixture.rs`, extended with the `rv` runners. It is
//! duplicated rather than shared because a `tests/` helper cannot be imported
//! across crates without publishing it from a library, and publishing a test
//! scaffold from `rv-core` would put fixture code in the shipped API.
//!
//! Only tests may drive the `jj` CLI — production code speaks to jj-lib in
//! process. Every invocation is made hermetic with `JJ_CONFIG=/dev/null` plus a
//! fixed author, so the developer's own jj config cannot change what the tests
//! see.

use std::fs;
use std::path::Path;
use std::process::Command;
use std::process::Output;

use tempfile::TempDir;

pub struct Fixture {
    tempdir: TempDir,
}

impl Fixture {
    /// Creates a colocated jj workspace in a fresh temporary directory.
    pub fn new() -> Self {
        let fixture = Self {
            tempdir: tempfile::tempdir().expect("create temp dir"),
        };
        fixture.jj(&["git", "init", "--colocate"]);
        fixture
    }

    /// The workspace root.
    pub fn root(&self) -> &Path {
        self.tempdir.path()
    }

    /// A handle on the sole stored review's directory — every fixture here
    /// opens exactly one review.
    pub fn store(&self) -> rv_core::store::Store {
        let (key, _) = rv_core::store::Store::list_reviews(self.root())
            .into_iter()
            .next()
            .expect("a stored review");
        rv_core::store::Store::open(self.root(), &key).expect("open the store")
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

    /// Describes the working-copy change and opens an empty one on top of it.
    pub fn commit(&self, message: &str) {
        self.jj(&["describe", "-m", message]);
        self.jj(&["new"]);
    }

    /// Runs the `rv` binary with the workspace as its current directory, so the
    /// default `--repo` is exercised too.
    pub fn rv(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_rv"))
            .args(args)
            .current_dir(self.root())
            .output()
            .expect("run rv")
    }

    /// The same, with `stdin` piped in — how `-m -` arrives.
    pub fn rv_with_stdin(&self, args: &[&str], stdin: &str) -> Output {
        use std::io::Write as _;
        let mut child = Command::new(env!("CARGO_BIN_EXE_rv"))
            .args(args)
            .current_dir(self.root())
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("spawn rv");
        child
            .stdin
            .take()
            .expect("a piped stdin")
            .write_all(stdin.as_bytes())
            .expect("write the body");
        child.wait_with_output().expect("run rv")
    }
}
