//! One builder per grammar: its query set, compiled once and kept.
//!
//! A query that fails to compile — a version skew between a grammar crate and
//! tree-sitter — reports `None` rather than panicking, so one unbuildable
//! grammar costs that language its colour and costs the others nothing.

use std::sync::OnceLock;

use tree_sitter_highlight::HighlightConfiguration;

use super::captures::CAPTURES;

/// Applies rv's capture vocabulary to a freshly built configuration, and
/// turns a query that did not compile into `None`.
///
/// A query fails to compile when a grammar crate and the linked tree-sitter
/// disagree about a node name — a version skew. That has to show up as a file
/// rendered plain, never as a crash, which is why every configuration below
/// funnels through here.
///
/// Generic over the error type only so that this module can name it without
/// naming `tree_sitter::QueryError`, and so keep `tree-sitter` out of
/// `rv-core`'s direct dependencies.
pub(super) fn configured<E>(built: Result<HighlightConfiguration, E>) -> Option<HighlightConfiguration> {
    let mut config = built.ok()?;
    let names: Vec<&str> = CAPTURES.iter().map(|(name, _)| *name).collect();
    config.configure(&names);
    Some(config)
}

/// The answer for every grammar that does not highlight a second language
/// inside itself, which is all of them but markdown.
pub(super) fn no_injection(_: &str) -> Option<&'static HighlightConfiguration> {
    None
}

/// The Rust grammar's configuration, or `None` if its query does not compile
/// against the linked tree-sitter — a version skew, which shows up as a file
/// rendered plain rather than as a crash.
pub(super) fn rust_configuration() -> Option<&'static HighlightConfiguration> {
    static CONFIG: OnceLock<Option<HighlightConfiguration>> = OnceLock::new();
    CONFIG
        .get_or_init(|| {
            // No locals query: tree-sitter-rust ships none, and rv does not
            // need scope-aware highlighting to colour a diff.
            configured(HighlightConfiguration::new(
                tree_sitter_rust::LANGUAGE.into(),
                "rust",
                tree_sitter_rust::HIGHLIGHTS_QUERY,
                tree_sitter_rust::INJECTIONS_QUERY,
                "",
            ))
        })
        .as_ref()
}

pub(super) fn toml_configuration() -> Option<&'static HighlightConfiguration> {
    static CONFIG: OnceLock<Option<HighlightConfiguration>> = OnceLock::new();
    CONFIG
        .get_or_init(|| {
            configured(HighlightConfiguration::new(
                tree_sitter_toml_ng::LANGUAGE.into(),
                "toml",
                tree_sitter_toml_ng::HIGHLIGHTS_QUERY,
                "",
                "",
            ))
        })
        .as_ref()
}

pub(super) fn yaml_configuration() -> Option<&'static HighlightConfiguration> {
    static CONFIG: OnceLock<Option<HighlightConfiguration>> = OnceLock::new();
    CONFIG
        .get_or_init(|| {
            configured(HighlightConfiguration::new(
                tree_sitter_yaml::LANGUAGE.into(),
                "yaml",
                tree_sitter_yaml::HIGHLIGHTS_QUERY,
                "",
                "",
            ))
        })
        .as_ref()
}

pub(super) fn json_configuration() -> Option<&'static HighlightConfiguration> {
    static CONFIG: OnceLock<Option<HighlightConfiguration>> = OnceLock::new();
    CONFIG
        .get_or_init(|| {
            configured(HighlightConfiguration::new(
                tree_sitter_json::LANGUAGE.into(),
                "json",
                tree_sitter_json::HIGHLIGHTS_QUERY,
                "",
                "",
            ))
        })
        .as_ref()
}

pub(super) fn python_configuration() -> Option<&'static HighlightConfiguration> {
    static CONFIG: OnceLock<Option<HighlightConfiguration>> = OnceLock::new();
    CONFIG
        .get_or_init(|| {
            configured(HighlightConfiguration::new(
                tree_sitter_python::LANGUAGE.into(),
                "python",
                tree_sitter_python::HIGHLIGHTS_QUERY,
                "",
                "",
            ))
        })
        .as_ref()
}

pub(super) fn go_configuration() -> Option<&'static HighlightConfiguration> {
    static CONFIG: OnceLock<Option<HighlightConfiguration>> = OnceLock::new();
    CONFIG
        .get_or_init(|| {
            configured(HighlightConfiguration::new(
                tree_sitter_go::LANGUAGE.into(),
                "go",
                tree_sitter_go::HIGHLIGHTS_QUERY,
                "",
                "",
            ))
        })
        .as_ref()
}

/// `tree-sitter-bash` spells its constant `HIGHLIGHT_QUERY`, singular, where
/// every other grammar here spells it `HIGHLIGHTS_QUERY`. Worth a line,
/// because reaching for the wrong name is a compile error today and would be
/// an uncoloured file if the crate ever added the other spelling.
pub(super) fn bash_configuration() -> Option<&'static HighlightConfiguration> {
    static CONFIG: OnceLock<Option<HighlightConfiguration>> = OnceLock::new();
    CONFIG
        .get_or_init(|| {
            configured(HighlightConfiguration::new(
                tree_sitter_bash::LANGUAGE.into(),
                "bash",
                tree_sitter_bash::HIGHLIGHT_QUERY,
                "",
                "",
            ))
        })
        .as_ref()
}

pub(super) fn javascript_configuration() -> Option<&'static HighlightConfiguration> {
    static CONFIG: OnceLock<Option<HighlightConfiguration>> = OnceLock::new();
    CONFIG
        .get_or_init(|| {
            configured(HighlightConfiguration::new(
                tree_sitter_javascript::LANGUAGE.into(),
                "javascript",
                tree_sitter_javascript::HIGHLIGHT_QUERY,
                tree_sitter_javascript::INJECTIONS_QUERY,
                tree_sitter_javascript::LOCALS_QUERY,
            ))
        })
        .as_ref()
}

/// The highlight query both TypeScript parsers use: JavaScript's, then
/// TypeScript's own.
///
/// `tree-sitter-typescript`'s `HIGHLIGHTS_QUERY` is thirty-five lines — the
/// TypeScript-specific half only. On its own it captures type annotations and
/// `interface`, and *nothing else*: no comments, no strings, no function
/// names. The TypeScript grammar is a superset of JavaScript's, so
/// JavaScript's query is valid against it and supplies the other half.
///
/// The order is not load-bearing, which is worth stating because it looks as
/// though it should be. The two halves overlap on exactly one thing — a
/// capitalised identifier in expression position, `Foo` in `Foo.bar()`, in
/// `new Foo()`, in `extends Foo` — and they disagree only about whether to
/// call it `type` or `constructor`. [`CAPTURES`] maps both to
/// [`Capture::Type`], so the two orders produce identical spans in every
/// construct that has been put through them. Written this way round because
/// it is the order the query halves are named in, not because it wins.
pub(super) fn typescript_query() -> &'static str {
    static QUERY: OnceLock<String> = OnceLock::new();
    QUERY.get_or_init(|| {
        format!(
            "{}\n{}",
            tree_sitter_javascript::HIGHLIGHT_QUERY,
            tree_sitter_typescript::HIGHLIGHTS_QUERY
        )
    })
}

pub(super) fn typescript_configuration() -> Option<&'static HighlightConfiguration> {
    static CONFIG: OnceLock<Option<HighlightConfiguration>> = OnceLock::new();
    CONFIG
        .get_or_init(|| {
            configured(HighlightConfiguration::new(
                tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
                "typescript",
                typescript_query(),
                "",
                "",
            ))
        })
        .as_ref()
}

pub(super) fn tsx_configuration() -> Option<&'static HighlightConfiguration> {
    static CONFIG: OnceLock<Option<HighlightConfiguration>> = OnceLock::new();
    CONFIG
        .get_or_init(|| {
            configured(HighlightConfiguration::new(
                tree_sitter_typescript::LANGUAGE_TSX.into(),
                "tsx",
                typescript_query(),
                "",
                "",
            ))
        })
        .as_ref()
}

/// Markdown's *block* grammar: headings, fences, list markers, block quotes.
///
/// The injections query is rv's own rather than the crate's, and it is one
/// pattern. Two reasons, both load-bearing.
///
/// The first is a bug in the shipped query. `tree-sitter-md` asks for the
/// `markdown_inline` parser over each `(inline)` node without setting
/// `injection.include-children`, and `tree-sitter-highlight` reads the absence
/// of that flag as "highlight the node *except* its children" — so it hands
/// the inline parser the ranges an `(inline)` node's children do not cover,
/// which for a paragraph is nothing at all. The result is a markdown file with
/// its headings coloured and every emphasis, code span and link left plain.
/// Setting the flag is the fix, and it is why this string is written out here
/// instead of using `INJECTION_QUERY_BLOCK`.
///
/// The second is what is left out. The crate's query also injects whatever
/// language a fenced code block's info string names, plus `html` for HTML
/// blocks and `yaml`/`toml` for frontmatter. rv resolves none of those on
/// purpose: reading ```` ```rust ```` and parsing the block as Rust is
/// deciding what content is by looking at it, which is the one thing this
/// module refuses to do. `markdown_inline` is not that — it is the same
/// language's own second parser, the way markdown is specified to be read.
pub(super) fn markdown_configuration() -> Option<&'static HighlightConfiguration> {
    static CONFIG: OnceLock<Option<HighlightConfiguration>> = OnceLock::new();
    CONFIG
        .get_or_init(|| {
            configured(HighlightConfiguration::new(
                tree_sitter_md::LANGUAGE.into(),
                "markdown",
                tree_sitter_md::HIGHLIGHT_QUERY_BLOCK,
                "((inline) @injection.content \
                  (#set! injection.language \"markdown_inline\") \
                  (#set! injection.include-children))",
                "",
            ))
        })
        .as_ref()
}

/// Markdown's *inline* grammar: emphasis, code spans, links, escapes. Reached
/// only through [`markdown_injection`] — no file extension selects it, because
/// no file is written in it.
pub(super) fn markdown_inline_configuration() -> Option<&'static HighlightConfiguration> {
    static CONFIG: OnceLock<Option<HighlightConfiguration>> = OnceLock::new();
    CONFIG
        .get_or_init(|| {
            configured(HighlightConfiguration::new(
                tree_sitter_md::INLINE_LANGUAGE.into(),
                "markdown_inline",
                tree_sitter_md::HIGHLIGHT_QUERY_INLINE,
                "",
                "",
            ))
        })
        .as_ref()
}

/// The one injection rv resolves. Anything else the block grammar might ask
/// for is `None`, which leaves that text to the block grammar's own captures.
pub(super) fn markdown_injection(name: &str) -> Option<&'static HighlightConfiguration> {
    (name == "markdown_inline")
        .then(markdown_inline_configuration)
        .flatten()
}

