//! Syntax highlighting as plain data (spec §7). [`Highlights::of`] runs a
//! tree-sitter grammar over a file's bytes and returns [`Span`]s: for each
//! line, the byte ranges that belong to a [`Capture`] class.
//!
//! No colours live here. `rv-core` is terminal-free, so `Capture` is a
//! vocabulary of *kinds* — keyword, string, comment — and the mapping from a
//! kind to an actual terminal colour lives in the `rv` crate, which is the
//! only place that knows what a terminal is.
//!
//! Three rules shape the rest of this module.
//!
//! **Detection is by name, and only by name.** Nothing sniffs the content. A
//! wrong guess paints a file as a language it is not, which reads worse than
//! no colour at all, so a file whose name selects no grammar comes back with
//! [`language`](Highlights::language) `None` and no spans and is rendered
//! plain. Two tables do the selecting: an extension table, and a short
//! filename table for names whose extension does not name their language —
//! `Cargo.lock` is TOML, and it is the first file in a Rust repository
//! alphabetically, so an rv that cannot colour it looks like an rv whose
//! highlighting does not work. A name earns a filename row only if it is
//! unambiguous; guessing there is content sniffing by another spelling.
//!
//! **Nothing here fails.** A blob that is not UTF-8, a half-typed function
//! that does not parse, a path that is not a path — each produces an answer.
//! A reviewer opening a file must never be the thing that kills the process,
//! and tree-sitter recovers from parse errors on its own, so a broken file
//! still gets most of its colour.
//!
//! **Every span is clamped to its own line.** Offsets are relative to the
//! start of the line, never to the start of the file; a construct that
//! crosses a newline is cut into one span per line; and a span never reaches
//! past the end of the line's text, so a consumer indexing by column cannot
//! panic on a range the grammar reported past the end. [`Span::slice`] is the
//! safe way to take a span's text when the string in hand is not byte-for-byte
//! the blob line the span was measured against — a diff line, for instance.

use std::sync::OnceLock;

use tree_sitter_highlight::HighlightConfiguration;
use tree_sitter_highlight::HighlightEvent;
use tree_sitter_highlight::Highlighter;

/// What a run of source text is, as far as a renderer needs to care. A small,
/// deliberately terminal-free vocabulary: `rv` maps each variant to a colour.
///
/// [`Other`](Capture::Other) means "the grammar captured this, but it is not
/// one of the kinds rv paints" — Rust attributes land here — and is rendered
/// in the default foreground, exactly as an unhighlighted file is.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Capture {
    Keyword,
    Function,
    Type,
    String,
    Number,
    Comment,
    Punctuation,
    Variable,
    Constant,
    Other,
}

/// One highlighted run: `start..end` are byte offsets **within** 1-based
/// `line`, not within the file, and both are guaranteed to land on a character
/// boundary of that line's text.
///
/// Within a line, spans are ordered by `start` and never overlap, so a
/// renderer can walk them in a single pass and every column belongs to at most
/// one capture.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Span {
    pub line: u32,
    pub start: u32,
    pub end: u32,
    pub capture: Capture,
}

impl Span {
    /// The text this span covers in `text`, clamped so that it is always a
    /// substring and never a panic.
    ///
    /// The span was measured against a line of the blob that was highlighted.
    /// A caller often holds something close to but not identical to that line
    /// — a `DiffLine`'s text, say, which may have been produced by a different
    /// tool. Rather than make every such caller repeat the same bounds and
    /// character-boundary checks, this clamps `end` to the length of `text`
    /// and walks both ends back to a character boundary, yielding a shorter
    /// slice (possibly empty) instead of slicing out of range or through the
    /// middle of a multi-byte character.
    #[must_use]
    pub fn slice<'a>(&self, text: &'a str) -> &'a str {
        let end = floor_char_boundary(text, (self.end as usize).min(text.len()));
        let start = floor_char_boundary(text, (self.start as usize).min(end));
        &text[start..end]
    }
}

/// The largest index `i <= at` that is a character boundary of `text`.
fn floor_char_boundary(text: &str, at: usize) -> usize {
    let mut at = at.min(text.len());
    while at > 0 && !text.is_char_boundary(at) {
        at -= 1;
    }
    at
}

/// Every highlighted run in one file, ordered by line and then by column.
///
/// The default value — no language, no spans — is what a file with no grammar
/// gets, and is what `rv` renders plain.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Highlights {
    spans: Vec<Span>,
    language: Option<&'static str>,
}

impl Highlights {
    /// Highlights `source` using whichever grammar `path`'s name selects.
    ///
    /// Never fails: a path with no known name, a grammar that cannot be
    /// built, bytes that are not UTF-8 and source that does not parse all
    /// produce a value rather than an error. The first two report no language
    /// (so the caller can say *why* a file is plain); the last two report the
    /// language and whatever spans tree-sitter managed to recover.
    #[must_use]
    pub fn of(source: &[u8], path: &str) -> Highlights {
        let Some(grammar) = grammar_for_path(path) else {
            return Highlights::default();
        };
        let Some(config) = (grammar.configuration)() else {
            // The grammar's query failed to compile — a version skew between
            // the grammar crate and tree-sitter. Report no language rather
            // than a language with no spans, so the UI says "plain" honestly.
            return Highlights::default();
        };

        let language = Some(grammar.name);
        let lines = LineIndex::of(source);
        let mut highlighter = Highlighter::new();
        let injection = grammar.injection;
        let Ok(events) = highlighter.highlight(config, source, None, injection) else {
            return Highlights {
                spans: Vec::new(),
                language,
            };
        };

        let mut spans = Vec::new();
        let mut stack: Vec<Capture> = Vec::new();
        for event in events {
            // A mid-stream error means the parse was cancelled or the layer
            // gave up; keep the spans collected so far rather than throwing
            // away a nearly complete file's colour.
            let Ok(event) = event else { break };
            match event {
                HighlightEvent::HighlightStart(highlight) => {
                    stack.push(capture_at(highlight.0));
                }
                HighlightEvent::HighlightEnd => {
                    stack.pop();
                }
                HighlightEvent::Source { start, end } => {
                    if let Some(&capture) = stack.last() {
                        lines.push_spans(&mut spans, source, start, end, capture);
                    }
                }
            }
        }

        tidy(&mut spans);
        Highlights { spans, language }
    }

    /// The language whose grammar produced these spans, or `None` when the
    /// file's extension names no grammar rv ships. `rv` shows this in the
    /// pane's title so a reader knows why a file is plain.
    #[must_use]
    pub fn language(&self) -> Option<&'static str> {
        self.language
    }

    /// The spans on 1-based `line`, in column order. A line with no captures,
    /// line `0`, and any line past the end of the file all give back an empty
    /// slice — a viewport scrolled past the end of a file asks for those
    /// constantly.
    #[must_use]
    pub fn line(&self, line: u32) -> &[Span] {
        let start = self.spans.partition_point(|span| span.line < line);
        let end = self.spans.partition_point(|span| span.line <= line);
        &self.spans[start..end]
    }
}

/// Sorts spans into (line, column) order, drops any that overlap the one
/// before them, and merges neighbours that touch and share a capture.
///
/// The event stream is already ordered and disjoint — tree-sitter-highlight
/// emits `Source` events that partition the file — so the sort and the overlap
/// check are defence in depth: they make [`Highlights::line`]'s binary search
/// and the type's disjointness guarantee hold no matter what a future grammar
/// or a future version of the highlighter emits. The merge is not defensive:
/// adjacent runs with the same capture (a `::` next to a `(`, both
/// punctuation) are genuinely one styled run, and merging them keeps the span
/// list roughly as short as the eye says it should be.
fn tidy(spans: &mut Vec<Span>) {
    spans.sort_by_key(|span| (span.line, span.start));

    let mut tidied: Vec<Span> = Vec::with_capacity(spans.len());
    for span in spans.iter().copied() {
        match tidied.last_mut() {
            Some(last) if last.line == span.line && span.start < last.end => {} // overlap: drop
            Some(last)
                if last.line == span.line
                    && last.end == span.start
                    && last.capture == span.capture =>
            {
                last.end = span.end;
            }
            _ => tidied.push(span),
        }
    }
    *spans = tidied;
}

// ---------------------------------------------------------------------------
// Lines
// ---------------------------------------------------------------------------

/// Where each line of a blob starts and where its *text* ends. The end
/// excludes the `\n` that terminates the line and the `\r` of a CRLF ending,
/// because neither is a column a renderer paints — clamping to this end is
/// what keeps a span inside the text a consumer actually has.
///
/// A trailing newline does not open a final empty line: `"a\n"` is one line.
struct LineIndex {
    /// `(start, text_end)` byte offsets, one entry per line, ascending.
    lines: Vec<(usize, usize)>,
}

impl LineIndex {
    fn of(source: &[u8]) -> LineIndex {
        let mut lines = Vec::new();
        let mut start = 0usize;
        for (offset, &byte) in source.iter().enumerate() {
            if byte == b'\n' {
                lines.push((start, text_end(source, start, offset)));
                start = offset + 1;
            }
        }
        if start < source.len() {
            lines.push((start, text_end(source, start, source.len())));
        }
        LineIndex { lines }
    }

    /// The index of the line containing byte offset `at`, or `None` when the
    /// blob has no lines at all.
    fn line_at(&self, at: usize) -> Option<usize> {
        let after = self.lines.partition_point(|(start, _)| *start <= at);
        after.checked_sub(1)
    }

    /// Cuts the byte range `start..end` of `source` into one [`Span`] per line
    /// it touches, each clamped to that line's text and snapped to character
    /// boundaries, and appends them to `out`.
    fn push_spans(
        &self,
        out: &mut Vec<Span>,
        source: &[u8],
        start: usize,
        end: usize,
        capture: Capture,
    ) {
        if start >= end {
            return;
        }
        let Some(mut index) = self.line_at(start) else {
            return;
        };
        while let Some(&(line_start, text_end)) = self.lines.get(index) {
            if line_start >= end {
                break;
            }
            let from = start.max(line_start);
            let to = end.min(text_end);
            // `to` can sit at or before `from` when the range covers only a
            // line's newline (or its `\r`), which is not a paintable column.
            if from < to
                && let Some(span) = span_of(source, index, line_start, from, to, capture)
            {
                out.push(span);
            }
            index += 1;
        }
    }
}

/// The end of a line's text: `end` with a CRLF's `\r` removed.
fn text_end(source: &[u8], start: usize, end: usize) -> usize {
    if end > start && source[end - 1] == b'\r' {
        end - 1
    } else {
        end
    }
}

/// Builds the span for `from..to` on the line starting at `line_start`,
/// snapping both ends inward to character boundaries of `source` so that the
/// resulting range never splits a multi-byte character. Snapping only ever
/// shrinks the range, so it cannot make two spans overlap.
///
/// Gives back `None` for a range that snapping emptied, and for the absurd
/// case of a file so large that a line number or column does not fit in a
/// `u32` — dropping the span is better than reporting a wrapped offset.
fn span_of(
    source: &[u8],
    index: usize,
    line_start: usize,
    from: usize,
    to: usize,
    capture: Capture,
) -> Option<Span> {
    let from = ceil_char_boundary(source, from, to);
    let to = floor_byte_boundary(source, to, from);
    if from >= to {
        return None;
    }
    Some(Span {
        line: u32::try_from(index + 1).ok()?,
        start: u32::try_from(from - line_start).ok()?,
        end: u32::try_from(to - line_start).ok()?,
        capture,
    })
}

/// True for a UTF-8 continuation byte — the bytes that are *not* the start of
/// a character. Works on bytes that are not valid UTF-8 at all, which is the
/// point: this module clamps blobs it has not validated.
fn is_continuation(byte: u8) -> bool {
    byte & 0b1100_0000 == 0b1000_0000
}

/// The smallest index `i` in `at..=ceil` that starts a character.
fn ceil_char_boundary(source: &[u8], mut at: usize, ceil: usize) -> usize {
    while at < ceil && source.get(at).copied().is_some_and(is_continuation) {
        at += 1;
    }
    at
}

/// The largest index `i` in `floor..=at` that starts a character (or is the
/// end of the blob, which always is one).
fn floor_byte_boundary(source: &[u8], mut at: usize, floor: usize) -> usize {
    while at > floor && source.get(at).copied().is_some_and(is_continuation) {
        at -= 1;
    }
    at
}

// ---------------------------------------------------------------------------
// Grammars
// ---------------------------------------------------------------------------

/// One language rv can highlight. Adding a grammar is one row in [`GRAMMARS`]
/// plus the function that row points at; nothing else in this module knows
/// about a particular language.
#[derive(Clone, Copy)]
struct Grammar {
    /// What [`Highlights::language`] reports.
    name: &'static str,
    /// The extensions that select it, without the dot. Matched
    /// case-insensitively, so these are written lowercase.
    extensions: &'static [&'static str],
    /// Whole file names that select it, for files whose extension does not
    /// name their language (`Cargo.lock`) or which have none (`.bashrc`).
    /// Matched case-insensitively against the path's last segment.
    filenames: &'static [&'static str],
    /// The compiled highlight configuration, built once per process.
    ///
    /// A function pointer rather than a name matched in a `match`, so a
    /// grammar cannot be listed here and then silently produce no
    /// highlighting because nothing dispatched to it.
    configuration: fn() -> Option<&'static HighlightConfiguration>,
    /// Resolves a language name this grammar's *injections* query asks for.
    ///
    /// Almost every grammar answers [`no_injection`]: rv highlights a file as
    /// one language, and resolving, say, the `rust` a markdown fence declares
    /// would be reading the content to decide what it is — the thing the
    /// module's first rule forbids. Markdown is the exception, and it is not
    /// really an exception: its inline content is parsed by a *second parser
    /// of the same language*, not by a language guessed from the text.
    injection: fn(&str) -> Option<&'static HighlightConfiguration>,
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
        configuration: rust_configuration,
        injection: no_injection,
    },
    Grammar {
        name: "toml",
        extensions: &["toml"],
        // `Cargo.lock` is TOML under a `.lock` extension, and it sorts first
        // in a Rust repository — the single most valuable row in this table.
        filenames: &["Cargo.lock"],
        configuration: toml_configuration,
        injection: no_injection,
    },
    Grammar {
        name: "markdown",
        extensions: &["md", "markdown"],
        filenames: &[],
        configuration: markdown_configuration,
        injection: markdown_injection,
    },
    Grammar {
        name: "yaml",
        extensions: &["yaml", "yml"],
        filenames: &[],
        configuration: yaml_configuration,
        injection: no_injection,
    },
    Grammar {
        name: "json",
        // The grammar has a `comment` rule, so `.jsonc` parses as well as
        // `.json` does.
        extensions: &["json", "jsonc"],
        filenames: &[],
        configuration: json_configuration,
        injection: no_injection,
    },
    Grammar {
        name: "python",
        extensions: &["py", "pyi"],
        filenames: &[],
        configuration: python_configuration,
        injection: no_injection,
    },
    Grammar {
        name: "go",
        extensions: &["go"],
        filenames: &[],
        configuration: go_configuration,
        injection: no_injection,
    },
    Grammar {
        name: "typescript",
        extensions: &["ts", "mts", "cts"],
        filenames: &[],
        configuration: typescript_configuration,
        injection: no_injection,
    },
    Grammar {
        name: "tsx",
        // A separate parser, not a separate query: JSX does not parse as
        // TypeScript, which is why the crate ships two languages.
        extensions: &["tsx"],
        filenames: &[],
        configuration: tsx_configuration,
        injection: no_injection,
    },
    Grammar {
        name: "javascript",
        // The JavaScript grammar parses JSX itself, so `.jsx` needs no
        // second parser the way `.tsx` does.
        extensions: &["js", "jsx", "mjs", "cjs"],
        filenames: &[],
        configuration: javascript_configuration,
        injection: no_injection,
    },
    Grammar {
        name: "bash",
        extensions: &["sh", "bash"],
        filenames: &[".bashrc", ".bash_profile", ".bash_aliases", ".bash_logout"],
        configuration: bash_configuration,
        injection: no_injection,
    },
];

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
fn configured<E>(built: Result<HighlightConfiguration, E>) -> Option<HighlightConfiguration> {
    let mut config = built.ok()?;
    let names: Vec<&str> = CAPTURES.iter().map(|(name, _)| *name).collect();
    config.configure(&names);
    Some(config)
}

/// The answer for every grammar that does not highlight a second language
/// inside itself, which is all of them but markdown.
fn no_injection(_: &str) -> Option<&'static HighlightConfiguration> {
    None
}

/// The Rust grammar's configuration, or `None` if its query does not compile
/// against the linked tree-sitter — a version skew, which shows up as a file
/// rendered plain rather than as a crash.
fn rust_configuration() -> Option<&'static HighlightConfiguration> {
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

fn toml_configuration() -> Option<&'static HighlightConfiguration> {
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

fn yaml_configuration() -> Option<&'static HighlightConfiguration> {
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

fn json_configuration() -> Option<&'static HighlightConfiguration> {
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

fn python_configuration() -> Option<&'static HighlightConfiguration> {
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

fn go_configuration() -> Option<&'static HighlightConfiguration> {
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
fn bash_configuration() -> Option<&'static HighlightConfiguration> {
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

fn javascript_configuration() -> Option<&'static HighlightConfiguration> {
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
fn typescript_query() -> &'static str {
    static QUERY: OnceLock<String> = OnceLock::new();
    QUERY.get_or_init(|| {
        format!(
            "{}\n{}",
            tree_sitter_javascript::HIGHLIGHT_QUERY,
            tree_sitter_typescript::HIGHLIGHTS_QUERY
        )
    })
}

fn typescript_configuration() -> Option<&'static HighlightConfiguration> {
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

fn tsx_configuration() -> Option<&'static HighlightConfiguration> {
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
fn markdown_configuration() -> Option<&'static HighlightConfiguration> {
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
fn markdown_inline_configuration() -> Option<&'static HighlightConfiguration> {
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
fn markdown_injection(name: &str) -> Option<&'static HighlightConfiguration> {
    (name == "markdown_inline")
        .then(markdown_inline_configuration)
        .flatten()
}

/// The grammar `path`'s name selects, or `None` when neither table claims it.
///
/// The filename table is consulted first: it exists precisely for names whose
/// extension says something else (`Cargo.lock`) or nothing at all
/// (`.bashrc`), so an exact name beats an extension.
fn grammar_for_path(path: &str) -> Option<Grammar> {
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

/// The highlight names rv recognizes, each paired with the [`Capture`] it
/// becomes. This doubles as the list handed to
/// `HighlightConfiguration::configure`, so a `Highlight(i)` coming back from
/// the highlighter indexes straight into it.
///
/// tree-sitter matches a query's dotted capture name against these by parts,
/// preferring the most specific match, so listing `variable` and
/// `variable.builtin` separately is how `self` gets coloured as a keyword
/// while a parameter stays a variable, and listing the bare roots is what
/// makes `comment.documentation`, `punctuation.bracket` and `function.macro`
/// land on the right kind without naming each one.
///
/// One consequence worth knowing when reading a Rust file: tree-sitter-rust
/// captures integer and float literals as `constant.builtin`, the same as
/// `true` and `false`, so Rust numbers arrive as [`Capture::Constant`] and
/// never as [`Capture::Number`]. That variant is for grammars that do
/// distinguish them, and TOML, YAML and JSON all do; rv follows each grammar
/// rather than second-guessing it, which is why the same literal can be a
/// different kind in two languages.
///
/// The `text.*` rows are markdown's, which is the only grammar here whose
/// subject is prose rather than code. `text` itself is [`Capture::Other`] so
/// that the parts of the vocabulary rv has no colour for — `text.emphasis`,
/// `text.strong`, since a terminal-free span carries no italic or bold — fall
/// back to being rendered plain rather than being painted some arbitrary
/// colour that means nothing.
const CAPTURES: &[(&str, Capture)] = &[
    ("attribute", Capture::Other),
    // TOML and YAML give booleans their own name; without this row `true` in
    // a `Cargo.toml` is the one word on the line with no colour.
    ("boolean", Capture::Constant),
    ("comment", Capture::Comment),
    ("constant", Capture::Constant),
    ("constructor", Capture::Type),
    ("escape", Capture::String),
    ("function", Capture::Function),
    ("keyword", Capture::Keyword),
    ("label", Capture::Variable),
    ("number", Capture::Number),
    ("operator", Capture::Punctuation),
    ("property", Capture::Variable),
    ("punctuation", Capture::Punctuation),
    ("string", Capture::String),
    ("tag", Capture::Other),
    ("text", Capture::Other),
    // A code span or a fenced block: literal text, the same as a string.
    ("text.literal", Capture::String),
    // The visible text of a link, which names something else.
    ("text.reference", Capture::Variable),
    // A heading, the strongest structural marker a markdown file has.
    ("text.title", Capture::Keyword),
    ("text.uri", Capture::String),
    ("type", Capture::Type),
    ("variable", Capture::Variable),
    ("variable.builtin", Capture::Keyword),
];

/// The capture kind for a highlight index, falling back to
/// [`Capture::Other`] for an index outside [`CAPTURES`] — which cannot happen
/// while the same table configures the highlighter, but is not worth a panic
/// if it ever does.
fn capture_at(index: usize) -> Capture {
    CAPTURES
        .get(index)
        .map_or(Capture::Other, |(_, capture)| *capture)
}
