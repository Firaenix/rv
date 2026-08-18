//! Which grammar a path selects, and how each one is built.
//!
//! One table, asked by `Highlights::of` and by `language_of`. Detection lives
//! here and nowhere else: a second table would drift, and the day it drifted a
//! file would be coloured as one language and searched as another.


use tree_sitter_highlight::HighlightConfiguration;

use super::configs;

/// One language rv can highlight. Adding a grammar is one row in [`GRAMMARS`]
/// plus the function that row points at; nothing else in this module knows
/// about a particular language.
#[derive(Clone, Copy)]
pub(super) struct Grammar {
    /// What [`Highlights::language`] reports.
    pub(super) name: &'static str,
    /// The extensions that select it, without the dot. Matched
    /// case-insensitively, so these are written lowercase.
    pub(super) extensions: &'static [&'static str],
    /// Whole file names that select it, for files whose extension does not
    /// name their language (`Cargo.lock`) or which have none (`.bashrc`).
    /// Matched case-insensitively against the path's last segment.
    pub(super) filenames: &'static [&'static str],
    /// The compiled highlight configuration, built once per process.
    ///
    /// A function pointer rather than a name matched in a `match`, so a
    /// grammar cannot be listed here and then silently produce no
    /// highlighting because nothing dispatched to it.
    pub(super) configuration: fn() -> Option<&'static HighlightConfiguration>,
    /// Resolves a language name this grammar's *injections* query asks for.
    ///
    /// Almost every grammar answers [`configs::no_injection`]: rv highlights a file as
    /// one language, and resolving, say, the `rust` a markdown fence declares
    /// would be reading the content to decide what it is — the thing the
    /// module's first rule forbids. Markdown is the exception, and it is not
    /// really an exception: its inline content is parsed by a *second parser
    /// of the same language*, not by a language guessed from the text.
    pub(super) injection: fn(&str) -> Option<&'static HighlightConfiguration>,
}

/// Every grammar rv ships, in the order a reviewer of a Rust repository meets
/// them. The lists are what a user reads as "which files get colour", and
/// `tests/highlight.rs` states every entry again as a case, so a row added
/// here without a test is a failing test rather than a silent claim.
///
/// What is *not* here matters too. `zsh` is not an alias for `bash`: bash is a
/// superset of POSIX `sh`, so the bash grammar over a `.sh` file is safe,
/// while zsh has syntax bash does not and would parse as errors. `.mdx` is not
/// markdown, `Gemfile.lock` and `yarn.lock` are not TOML, and `.eslintrc` is
/// as often YAML as JSON — each of those renders plain instead.
const GRAMMARS: &[Grammar] = &[
    Grammar {
        name: "rust",
        extensions: &["rs"],
        filenames: &[],
        configuration: configs::rust_configuration,
        injection: configs::no_injection,
    },
    Grammar {
        name: "toml",
        extensions: &["toml"],
        // `Cargo.lock` is TOML under a `.lock` extension, and it sorts first
        // in a Rust repository — the single most valuable row in this table.
        filenames: &["Cargo.lock"],
        configuration: configs::toml_configuration,
        injection: configs::no_injection,
    },
    Grammar {
        name: "markdown",
        extensions: &["md", "markdown"],
        filenames: &[],
        configuration: configs::markdown_configuration,
        injection: configs::markdown_injection,
    },
    Grammar {
        name: "yaml",
        extensions: &["yaml", "yml"],
        filenames: &[],
        configuration: configs::yaml_configuration,
        injection: configs::no_injection,
    },
    Grammar {
        name: "json",
        // The grammar has a `comment` rule, so `.jsonc` parses as well as
        // `.json` does.
        extensions: &["json", "jsonc"],
        filenames: &[],
        configuration: configs::json_configuration,
        injection: configs::no_injection,
    },
    Grammar {
        name: "python",
        extensions: &["py", "pyi"],
        filenames: &[],
        configuration: configs::python_configuration,
        injection: configs::no_injection,
    },
    Grammar {
        name: "go",
        extensions: &["go"],
        filenames: &[],
        configuration: configs::go_configuration,
        injection: configs::no_injection,
    },
    Grammar {
        name: "typescript",
        extensions: &["ts", "mts", "cts"],
        filenames: &[],
        configuration: configs::typescript_configuration,
        injection: configs::no_injection,
    },
    Grammar {
        name: "tsx",
        // A separate parser, not a separate query: JSX does not parse as
        // TypeScript, which is why the crate ships two languages.
        extensions: &["tsx"],
        filenames: &[],
        configuration: configs::tsx_configuration,
        injection: configs::no_injection,
    },
    Grammar {
        name: "javascript",
        // The JavaScript grammar parses JSX itself, so `.jsx` needs no
        // second parser the way `.tsx` does.
        extensions: &["js", "jsx", "mjs", "cjs"],
        filenames: &[],
        configuration: configs::javascript_configuration,
        injection: configs::no_injection,
    },
    Grammar {
        name: "bash",
        extensions: &["sh", "bash"],
        filenames: &[".bashrc", ".bash_profile", ".bash_aliases", ".bash_logout"],
        configuration: configs::bash_configuration,
        injection: configs::no_injection,
    },
];

/// The name of the language `path` selects, or `None` when neither table
/// claims it — the same answer [`Highlights::of`] would report, without
/// parsing anything.
///
/// This exists so that [`symbols`](crate::symbols) can ask *which language is
/// this file* through the one table that already answers it. Detection lives
/// here and nowhere else: a second table would drift, and the day it drifted a
/// file would be coloured as one language and searched as another. It is also
/// how a caller that only wants the language avoids paying for a whole
/// highlight parse to learn it — which is also what the diff pane's title needs.
/// Highlighting runs off the drawing thread, so for the first frames of a large
/// file there are no spans to ask; a title deciding "no highlighting" from an
/// empty cache would tell a reviewer their Rust file has no grammar.
#[must_use]
pub fn language_of(path: &str) -> Option<&'static str> {
    grammar_for_path(path).map(|grammar| grammar.name)
}

/// The grammar `path`'s name selects, or `None` when neither table claims it.
///
/// The filename table is consulted first: it exists precisely for names whose
/// extension says something else (`Cargo.lock`) or nothing at all
/// (`.bashrc`), so an exact name beats an extension.
pub(super) fn grammar_for_path(path: &str) -> Option<Grammar> {
    let name = file_name(path)?;
    let by_filename = GRAMMARS.iter().find(|grammar| {
        grammar
            .filenames
            .iter()
            .any(|candidate| candidate.eq_ignore_ascii_case(name))
    });
    if let Some(grammar) = by_filename {
        return Some(*grammar);
    }

    let extension = extension_of(path)?;
    GRAMMARS
        .iter()
        .find(|grammar| {
            grammar
                .extensions
                .iter()
                .any(|candidate| candidate.eq_ignore_ascii_case(extension))
        })
        .copied()
}

/// The last segment of `path`, or `None` for a path that has no last segment
/// at all. An empty path has none; a path ending in a separator yields the
/// empty string, which no table row matches.
///
/// Written by hand rather than through `std::path::Path` so that it answers
/// the same way on every platform for the `/`-separated repository paths rv
/// deals in — and, on Windows, so that a `\` in a jj path is still a
/// separator.
fn file_name(path: &str) -> Option<&str> {
    path.rsplit(['/', '\\']).next()
}

/// The extension of `path`'s last segment, without the dot.
///
/// The edge cases are visible on purpose: a name with no dot has no extension,
/// a dotfile (`.rs`) is a name and not an extension, and a name with several
/// dots (`archive.tar.gz`) has only the last one.
fn extension_of(path: &str) -> Option<&str> {
    let name = file_name(path)?;
    let (stem, extension) = name.rsplit_once('.')?;
    if stem.is_empty() {
        return None;
    }
    Some(extension)
}

// ---------------------------------------------------------------------------
// Captures
// ---------------------------------------------------------------------------

