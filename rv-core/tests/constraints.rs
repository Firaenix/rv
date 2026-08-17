//! Mechanical enforcement of the three architectural constraints that keep
//! `rv-core` terminal-free, jj-lib's blast radius confined to one file, and
//! rv's behavior independent of the user's jj config. These are read from
//! disk (the manifest and every `.rs` file under `src/`) rather than
//! asserted against in-memory knowledge, so a future edit that violates a
//! constraint fails a test instead of silently landing.
//!
//! Paths are resolved from `CARGO_MANIFEST_DIR`, never the process cwd, so
//! these tests behave the same regardless of where `cargo test` is invoked
//! from.
//!
//! The jj_lib and user-config checks strip `//` and `/* */` comments before
//! matching (see [`strip_comments`]), so doc-comment prose that merely
//! *names* a banned token cannot false-fail the build — and, the point that
//! actually matters, an import alias like `use jj_lib as j;` still gets
//! caught, because the word `jj_lib` still has to appear in code to name the
//! crate at all. The manifest check (constraint 1) deliberately does NOT go
//! through the stripper: `Cargo.toml`'s comment syntax is `#`, not `//`, and
//! a whole-file scan is a feature there — it also catches a dependency
//! under `[dev-dependencies]`, a target-specific table, or a package rename
//! such as `term = { package = "ratatui" }`.
//!
//! This file lives under `tests/`, not `src/`, so the recursive scan of
//! `src/` below never reads itself — the literal strings this file contains
//! for its own assertions (e.g. `"ConfigSource::User"`) can't trigger a
//! false positive.

use std::fs;
use std::path::Path;
use std::path::PathBuf;

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn manifest_path() -> PathBuf {
    manifest_dir().join("Cargo.toml")
}

fn read_manifest() -> String {
    let path = manifest_path();
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()))
}

/// Recursively collects every `.rs` file under `dir`. Recursion (rather than
/// a flat glob of `src/*.rs`) matters for constraint 2: `src/` has no
/// subdirectories today, but if `vcs.rs` ever becomes `vcs/mod.rs`, or a new
/// module is added in a subdirectory, a flat scan would silently stop
/// covering it and a `jj_lib` leak in a nested file would go undetected.
/// Recursing is the safer reading of "all `src/*.rs`".
fn collect_rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries =
        fs::read_dir(dir).unwrap_or_else(|e| panic!("failed to read dir {}: {e}", dir.display()));
    for entry in entries {
        let entry =
            entry.unwrap_or_else(|e| panic!("failed to read entry in {}: {e}", dir.display()));
        let path = entry.path();
        if path.is_dir() {
            collect_rs_files(&path, out);
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("rs") {
            out.push(path);
        }
    }
}

fn source_files() -> Vec<PathBuf> {
    let src = manifest_dir().join("src");
    let mut files = Vec::new();
    collect_rs_files(&src, &mut files);
    files
}

/// True if `rel_path` (relative to `src/`) belongs to the `vcs` module: the
/// top-level `vcs.rs`, or anything under a `vcs/` subdirectory (so a future
/// `vcs/mod.rs` split still counts as "the vcs module" rather than a new
/// violation).
fn is_vcs_module(rel_path: &Path) -> bool {
    let s = rel_path.to_string_lossy();
    s == "vcs.rs" || s.starts_with("vcs/") || s.starts_with("vcs\\")
}

/// Every line of `content` that contains `needle`, formatted as
/// `path:line: <trimmed line>` for use in assertion failure messages.
fn matching_lines(path: &Path, content: &str, needle: &str) -> Vec<String> {
    content
        .lines()
        .enumerate()
        .filter(|(_, line)| line.contains(needle))
        .map(|(i, line)| format!("{}:{}: {}", path.display(), i + 1, line.trim()))
        .collect()
}

/// Blanks out Rust `//` line comments and `/* */` block comments, replacing
/// comment bytes with spaces so line numbers (and column positions) in the
/// output line up exactly with `source` — [`matching_lines`] can report
/// accurate line numbers against the stripped text.
///
/// This is a line-oriented heuristic, not a Rust lexer, and its limits are
/// deliberate rather than overlooked (a real lexer/AST check — e.g. via
/// `syn` — is out of scope for this mechanical guard):
/// - It does not understand string literals, so a string containing `//` or
///   `/*` desyncs comment detection for the rest of that line (e.g. a URL
///   like `"http://…"` in a string reads as the start of a line comment).
/// - It does not support nested block comments.
/// - A string literal whose *contents* happen to spell a banned token (e.g.
///   `"jj_lib"` as a plain string, not an import) still trips the matchers
///   below, because this stripper only removes comments, not strings.
fn strip_comments(source: &str) -> String {
    #[derive(Clone, Copy)]
    enum State {
        Code,
        Line,
        Block,
    }

    let mut out = String::with_capacity(source.len());
    let mut state = State::Code;
    let mut chars = source.chars().peekable();
    while let Some(c) = chars.next() {
        match state {
            State::Code => {
                if c == '/' && chars.peek() == Some(&'/') {
                    chars.next();
                    out.push_str("  ");
                    state = State::Line;
                } else if c == '/' && chars.peek() == Some(&'*') {
                    chars.next();
                    out.push_str("  ");
                    state = State::Block;
                } else {
                    out.push(c);
                }
            }
            State::Line => {
                if c == '\n' {
                    out.push('\n');
                    state = State::Code;
                } else {
                    out.push(' ');
                }
            }
            State::Block => {
                if c == '*' && chars.peek() == Some(&'/') {
                    chars.next();
                    out.push_str("  ");
                    state = State::Code;
                } else if c == '\n' {
                    out.push('\n');
                } else {
                    out.push(' ');
                }
            }
        }
    }
    out
}

/// Guards the other three tests against passing vacuously. If path
/// resolution ever breaks (wrong crate, cwd-relative paths, a rename that
/// moves `src/`), this test fails loudly instead of leaving the constraint
/// tests to silently scan zero files and report "no violations found".
#[test]
fn guard_manifest_and_sources_were_found() {
    let manifest = read_manifest();
    assert!(
        !manifest.trim().is_empty(),
        "{} is empty — constraint checks would pass vacuously",
        manifest_path().display()
    );
    assert!(
        manifest.contains("name = \"rv-core\""),
        "{} doesn't look like rv-core's manifest (no `name = \"rv-core\"`); got:\n{manifest}",
        manifest_path().display()
    );

    let files = source_files();
    assert!(
        files.len() >= 6,
        "expected at least 6 files under {}/src, found {}: {:?} — constraint checks would pass vacuously",
        manifest_dir().display(),
        files.len(),
        files
    );

    let basenames: Vec<String> = files
        .iter()
        .filter_map(|p| p.file_name())
        .map(|n| n.to_string_lossy().into_owned())
        .collect();
    for expected in [
        "vcs.rs",
        "anchor.rs",
        "store.rs",
        "markdown.rs",
        "diff.rs",
        "model.rs",
    ] {
        assert!(
            basenames.contains(&expected.to_string()),
            "expected src/ to contain {expected}, but the recursive scan found only: {basenames:?}"
        );
    }
}

/// Constraint: `rv-core`'s manifest declares no `ratatui`, `crossterm`, or
/// `tui-textarea`. This is the terminal-free boundary that makes the anchor
/// logic testable without a TTY; those crates belong only to the `rv` binary
/// crate's UI layer.
///
/// Scans the raw manifest text, not comment-stripped: `Cargo.toml` comments
/// use `#`, not `//`/`/* */`, so [`strip_comments`] would not help here, and
/// a whole-file scan is actually what we want — it also catches a dependency
/// tucked under `[dev-dependencies]` or a target-specific table, and a
/// package rename such as `term = { package = "ratatui" }`.
#[test]
fn rv_core_manifest_has_no_tui_dependencies() {
    let manifest = read_manifest();
    let banned = ["ratatui", "crossterm", "tui-textarea"];

    let mut violations = Vec::new();
    for needle in banned {
        violations.extend(matching_lines(&manifest_path(), &manifest, needle));
    }

    assert!(
        violations.is_empty(),
        "rv-core/Cargo.toml must not depend on ratatui, crossterm, or tui-textarea \
         (they belong to rv's UI layer; rv-core must stay terminal-free so the \
         anchor logic is testable without a TTY) — found:\n{}",
        violations.join("\n")
    );
}

/// Constraint: only `vcs.rs` (the `vcs` module) imports or references
/// `jj_lib`. Every other module takes plain Rust types, so a jj-lib version
/// bump has a one-file blast radius.
///
/// Comments are stripped before matching (see [`strip_comments`]), and the
/// matcher looks for the bare word `jj_lib`, not the path form `jj_lib::`.
/// Both halves matter:
/// - Stripping comments means `diff.rs` and `lib.rs`, which legitimately
///   *mention* `jj_lib` in doc comments (e.g. "no `jj_lib` type crosses
///   this boundary"), no longer trip the matcher — that prose is gone
///   before the scan runs.
/// - Matching the bare word (rather than requiring `::`) closes an alias
///   bypass: `use jj_lib as j;` followed by aliased `j::backend::CommitId`
///   elsewhere never spells `jj_lib::`, but the `use jj_lib as j;` line
///   itself still has to name `jj_lib` to declare the alias, so the bare
///   word still catches it.
///
/// Remaining limit: a string literal containing the token `jj_lib` would
/// still trip this (see [`strip_comments`]'s doc comment) — accepted, not
/// worth a real Rust lexer for a mechanical guard test.
#[test]
fn only_vcs_module_mentions_jj_lib() {
    let src_root = manifest_dir().join("src");
    let files = source_files();

    let mut violations = Vec::new();
    for path in &files {
        let rel = path.strip_prefix(&src_root).unwrap_or(path);
        if is_vcs_module(rel) {
            continue;
        }
        let content = fs::read_to_string(path)
            .unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()));
        let code_only = strip_comments(&content);
        violations.extend(matching_lines(path, &code_only, "jj_lib"));
    }

    assert!(
        violations.is_empty(),
        "jj_lib must be imported only by the vcs module (src/vcs.rs, or a future \
         src/vcs/ directory) so that a jj-lib version bump touches one file — \
         found jj_lib usage outside vcs (comments stripped before matching):\n{}",
        violations.join("\n")
    );
}

/// Constraint: no source file reads the user's jj config. rv must behave
/// identically on a stock and a heavily customized jj install, so
/// `UserSettings` must be built entirely from config that `rv` supplies
/// itself, never from `~/.config/jj/config.toml` or repo-local config.
///
/// Comments are stripped before matching (see [`strip_comments`]), so an
/// explanatory comment that happens to name one of the banned tokens cannot
/// false-fail the build.
///
/// Deliberately does NOT match `ConfigSource::Default`, which `vcs.rs`
/// legitimately uses to build rv's own synthetic config layer.
///
/// Bound (accepted, not a gap): this is a literal-token grep over `rv`'s own
/// source. It cannot see a jj-lib-*internal* helper that reads user or repo
/// config on its own without `rv`'s calling code ever spelling
/// `config_path`, `ConfigSource::User`, or `ConfigSource::Repo`. The
/// architectural backstop for that case lives in `vcs::settings()`, which
/// builds `UserSettings` from `StackedConfig::with_defaults()` plus one
/// in-memory layer that `rv` supplies itself; Task 1's review verified
/// against jj-lib's own source that `with_defaults()` is pure and performs
/// no disk access. This test enforces that nobody undoes that by writing
/// new code with these tokens — it does not re-verify jj-lib's internals.
#[test]
fn no_source_reads_user_jj_config() {
    let files = source_files();
    let banned = ["config_path", "ConfigSource::User", "ConfigSource::Repo"];

    let mut violations = Vec::new();
    for path in &files {
        let content = fs::read_to_string(path)
            .unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()));
        let code_only = strip_comments(&content);
        for needle in banned {
            violations.extend(matching_lines(path, &code_only, needle));
        }
    }

    assert!(
        violations.is_empty(),
        "no source file may read the user's jj config (no config_path, no \
         ConfigSource::User, no ConfigSource::Repo) — rv must behave identically \
         on a stock and a heavily customized jj install; found (comments stripped \
         before matching):\n{}",
        violations.join("\n")
    );
}
