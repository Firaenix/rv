//! A throwaway jj workspace built by shelling out to the `jj` CLI.
//!
//! Only tests may drive the `jj` binary — production code speaks to jj-lib in
//! process. Every invocation is made hermetic with `JJ_CONFIG=/dev/null` plus a
//! fixed author, so the developer's own jj config cannot change what the tests
//! see.
//!
//! `cargo` compiles this file both as a module of the test binaries that declare
//! `mod fixture;` and as a (test-less) integration target of its own, which is why
//! the unused-code lint is silenced here.
#![allow(dead_code)]

use std::fs;
use std::path::Path;
use std::process::Command;

use tempfile::TempDir;

pub struct Fixture {
    tempdir: TempDir,
}

// A `Default` impl would hide the fact that constructing this runs `jj git init`.
#[allow(clippy::new_without_default)]
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
}
