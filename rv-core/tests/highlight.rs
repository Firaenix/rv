//! Tests for `rv_core::highlight`: the tree-sitter layer that turns a file's
//! bytes into per-line, per-column [`Span`]s naming a [`Capture`] class.
//!
//! Two halves. The hand-written cases pin one example per documented
//! behaviour — what a Rust file's keywords and functions come back as, what a
//! file with no grammar comes back as, what a broken parse comes back as, and
//! one characteristic construct per shipped grammar, so a grammar that stops
//! working is a failing test rather than a screen that quietly loses its
//! colour. The properties go after the *contract* a consumer relies on to
//! render without crashing: totality over arbitrary bytes and arbitrary paths,
//! spans that are ordered, disjoint and inside their own line, byte offsets
//! that are always char boundaries, and a deterministic answer for the same
//! input — and they run over *every* grammar, not only Rust, because each
//! grammar is a separate C parser with its own opinion about malformed input.
//!
//! The line oracle ([`ref_line`]) is written independently of the module's own
//! line indexing — a plain `split('\n')` — so the "inside its line" property
//! is checked against a second opinion rather than against a copy of the code
//! under test.

use proptest::prelude::*;
use rstest::rstest;
use rv_core::highlight::Capture;
use rv_core::highlight::Highlights;
use rv_core::highlight::Span;

// ---------------------------------------------------------------------------
// Oracles
// ---------------------------------------------------------------------------

/// The text of 1-based `line` in `source`, as the module defines a line: the
/// bytes between newlines, with a `\r` of a CRLF ending removed. Deliberately
/// a `split('\n')` walk rather than a reimplementation of the module's line
/// index, so it is an independent opinion about where a line ends.
///
/// A `source` that ends in `\n` has no final empty line: `"a\n"` is one line.
fn ref_line(source: &str, line: u32) -> Option<&str> {
    if line == 0 {
        return None;
    }
    let mut pieces: Vec<&str> = source.split('\n').collect();
    if source.ends_with('\n') {
        pieces.pop();
    }
    pieces
        .get(line as usize - 1)
        .map(|piece| piece.strip_suffix('\r').unwrap_or(piece))
}

/// Every line number that could hold a span in `source`, plus a few past the
/// end — the range a consumer might ask about while scrolling.
fn probe_lines(source: &str) -> Vec<u32> {
    let count = ref_line_count(source);
    (0..=count + 2).collect()
}

fn ref_line_count(source: &str) -> u32 {
    let mut pieces: Vec<&str> = source.split('\n').collect();
    if source.ends_with('\n') {
        pieces.pop();
    }
    u32::try_from(pieces.len()).unwrap_or(u32::MAX)
}

/// The byte offset each line of `source` starts at, computed from the raw
/// bytes so it works on a blob that is not UTF-8. A line starts at 0 and after
/// every newline that is not the last byte; an empty blob has no lines.
fn ref_line_starts(source: &[u8]) -> Vec<usize> {
    if source.is_empty() {
        return Vec::new();
    }
    let mut starts = vec![0usize];
    for (offset, &byte) in source.iter().enumerate() {
        if byte == b'\n' && offset + 1 < source.len() {
            starts.push(offset + 1);
        }
    }
    starts
}

/// True for a UTF-8 continuation byte: the bytes that sit *inside* a character
/// rather than starting one. Stated here as the definition from the encoding,
/// independently of the module's own copy.
fn is_continuation(byte: u8) -> bool {
    byte & 0b1100_0000 == 0b1000_0000
}

/// Collects every span the highlights hold, by walking lines rather than by
/// reading a private field.
fn all_spans(highlights: &Highlights, source: &str) -> Vec<Span> {
    probe_lines(source)
        .into_iter()
        .flat_map(|line| highlights.line(line).to_vec())
        .collect()
}

/// Every path that must select a grammar, one per language rv ships. The
/// per-grammar properties walk this, so adding a grammar without adding its
/// path here is the one way to get a grammar the properties never fuzz — and
/// [`the_grammar_paths_cover_every_shipped_language`] is what catches that.
const GRAMMAR_PATHS: &[(&str, &str)] = &[
    ("a.rs", "rust"),
    ("a.toml", "toml"),
    ("Cargo.lock", "toml"),
    ("a.md", "markdown"),
    ("a.yaml", "yaml"),
    ("a.json", "json"),
    ("a.py", "python"),
    ("a.go", "go"),
    ("a.ts", "typescript"),
    ("a.tsx", "tsx"),
    ("a.js", "javascript"),
    ("a.sh", "bash"),
];

/// The [`Capture`] covering `token` on 1-based `line` of `source`, or `None`
/// when no span covers it.
///
/// This is how the per-grammar cases below are stated: not "some span on this
/// line is a keyword", which a grammar could satisfy by accident, but "*this
/// token* came back as this kind". The column is found in the line's own text
/// via [`ref_line`], the same independent oracle the properties use, so the
/// lookup does not borrow the module's idea of where a line starts.
fn capture_of(source: &str, path: &str, line: u32, token: &str) -> Option<Capture> {
    let highlights = Highlights::of(source.as_bytes(), path);
    let text = ref_line(source, line)?;
    let at = u32::try_from(text.find(token)?).ok()?;
    highlights
        .line(line)
        .iter()
        .find(|span| span.start <= at && at < span.end)
        .map(|span| span.capture)
}

/// Asserts that `token` on `line` came back as `want`, reporting what it
/// actually got — which is almost always the useful half of the message, since
/// "the grammar captured this as something else" and "the grammar did not
/// capture this at all" are different bugs.
#[track_caller]
fn assert_capture(source: &str, path: &str, line: u32, token: &str, want: Capture) {
    let got = capture_of(source, path, line, token);
    assert_eq!(
        got,
        Some(want),
        "in {path}, `{token}` on line {line} should be {want:?}, got {got:?}"
    );
}

// ---------------------------------------------------------------------------
// Rust: what the grammar actually gives back
// ---------------------------------------------------------------------------

/// The headline case: a Rust file's `fn`/`let` come back as keywords and the
/// function's own name comes back as a function, so the diff pane has more
/// than one colour to paint with.
#[test]
fn rust_keywords_and_types_are_captured() {
    let source = b"fn parse(s: &str) -> Result<Ast> {\n    let raw = s.trim();\n}\n";
    let highlights = Highlights::of(source, "parse.rs");

    assert_eq!(highlights.language(), Some("rust"));
    let first = highlights.line(1);
    assert!(
        first.iter().any(|s| s.capture == Capture::Keyword),
        "fn is a keyword, got {first:?}"
    );
    assert!(
        first.iter().any(|s| s.capture == Capture::Function),
        "parse is a function name, got {first:?}"
    );
    assert!(
        first.iter().any(|s| s.capture == Capture::Type),
        "Result and Ast are types, got {first:?}"
    );
    assert!(
        highlights
            .line(2)
            .iter()
            .any(|s| s.capture == Capture::Keyword),
        "let is a keyword, got {:?}",
        highlights.line(2)
    );
}

/// Spans are disjoint, ordered by column, and never reach past the end of the
/// line they belong to — the three things a renderer that indexes by column
/// depends on to not panic or paint over itself.
#[test]
fn spans_never_overlap_and_stay_inside_their_line() {
    let source = b"fn a() { let x = \"s\"; }\n";
    let highlights = Highlights::of(source, "a.rs");

    let line = highlights.line(1);
    let text_len = 23u32;
    assert!(!line.is_empty(), "the line has captures to check");
    for pair in line.windows(2) {
        assert!(
            pair[0].end <= pair[1].start,
            "spans are disjoint and ordered: {pair:?}"
        );
    }
    for span in line {
        assert!(
            span.start < span.end && span.end <= text_len,
            "a span stays inside its line: {span:?}"
        );
    }
}

/// The column offsets are relative to the *line*, not to the file. This is the
/// bug that makes highlighting look plausible on line 1 and slide further right
/// on every line after it, so it gets its own test: the keyword on line 3 is at
/// column 0, not at its byte offset in the blob.
#[test]
fn columns_are_relative_to_their_own_line_not_the_file() {
    let source = b"struct A;\nstruct B;\nfn go() {}\n";
    let highlights = Highlights::of(source, "m.rs");

    let third = highlights.line(3);
    let first = third.first().expect("line 3 has captures");
    assert_eq!(
        (first.line, first.start, first.end),
        (3, 0, 2),
        "`fn` is columns 0..2 of line 3, got {third:?}"
    );
}

/// Every span reports the line it was found on, so a caller that took the
/// slice from `line(n)` can still trust `span.line`.
#[test]
fn every_span_reports_its_own_line() {
    let source = b"fn a() {}\nfn b() {}\nfn c() {}\n";
    let highlights = Highlights::of(source, "m.rs");

    for line in 1..=3 {
        for span in highlights.line(line) {
            assert_eq!(span.line, line, "span {span:?} came back from line {line}");
        }
    }
}

/// Two runs of the same capture with plain text between them stay two spans.
/// Adjacent runs of one kind are merged into one, and the boundary of that
/// rule matters: merging across a gap would swallow the uncaptured identifiers
/// between the brackets and paint them as punctuation.
#[test]
fn spans_of_one_kind_are_not_merged_across_a_gap() {
    let text = "fn a() { let v = (one, two); }";
    let highlights = Highlights::of(text.as_bytes(), "a.rs");

    for token in ["one", "two"] {
        let at = u32::try_from(text.find(token).expect("token is on the line")).expect("fits");
        assert!(
            !highlights
                .line(1)
                .iter()
                .any(|span| span.start <= at && at < span.end),
            "`{token}` at column {at} is plain text, but a span covers it: {:?}",
            highlights.line(1)
        );
    }
}

/// A construct that crosses a line boundary — a raw string literal — is cut at
/// the newline into one span per line rather than one span whose end is past
/// the end of its line.
#[test]
fn a_construct_spanning_lines_is_cut_at_the_newline() {
    let source = b"fn a() {\n    let s = r#\"one\ntwo\"#;\n}\n";
    let highlights = Highlights::of(source, "a.rs");

    let second = highlights.line(2);
    let third = highlights.line(3);
    assert!(
        second.iter().any(|s| s.capture == Capture::String),
        "the string starts on line 2, got {second:?}"
    );
    assert!(
        third.iter().any(|s| s.capture == Capture::String),
        "and continues on line 3, got {third:?}"
    );
    for span in third {
        assert!(
            span.end <= 6,
            "line 3 is the six bytes `two\"#;` — no span reaches past it: {span:?}"
        );
    }
}

/// Comments come back as comments, including doc comments, which the Rust
/// grammar captures as `comment.documentation` — a name the mapping has to
/// reach by prefix rather than by exact match.
#[test]
fn comments_including_doc_comments_are_comments() {
    let source = b"/// docs\n// plain\nfn a() {}\n";
    let highlights = Highlights::of(source, "a.rs");

    for line in [1u32, 2] {
        assert!(
            highlights
                .line(line)
                .iter()
                .any(|s| s.capture == Capture::Comment),
            "line {line} is a comment, got {:?}",
            highlights.line(line)
        );
    }
}

/// A file whose extension names a grammar but which happens to be empty is
/// still that language — there is simply nothing to colour.
#[test]
fn an_empty_rust_file_is_still_rust() {
    let highlights = Highlights::of(b"", "empty.rs");

    assert_eq!(highlights.language(), Some("rust"));
    assert!(highlights.line(1).is_empty());
}

// ---------------------------------------------------------------------------
// The rest of the grammars: one characteristic construct each
// ---------------------------------------------------------------------------
//
// Each case names a token a reader would point at and says what kind it must
// come back as. These are deliberately about the *grammar's* opinion, not
// rv's: where a grammar disagrees with the obvious guess — TOML captures a
// table header as a type, YAML captures a key as a property (and so a
// variable) — the test records the grammar's answer, because that is what a
// reviewer will actually see on screen.

/// TOML is first because it is what a reviewer opening this repository sees
/// first: `Cargo.lock` sorts before every source file, so an rv that cannot
/// colour TOML looks like an rv whose highlighting does not work at all.
#[test]
fn toml_tables_keys_and_values_are_captured() {
    let source = "# a comment\n[package]\nname = \"rv\"\nedition = 2024\nok = true\n";

    assert_eq!(
        Highlights::of(source.as_bytes(), "Cargo.toml").language(),
        Some("toml")
    );
    assert_capture(source, "Cargo.toml", 1, "# a comment", Capture::Comment);
    assert_capture(source, "Cargo.toml", 2, "package", Capture::Type);
    assert_capture(source, "Cargo.toml", 3, "\"rv\"", Capture::String);
    assert_capture(source, "Cargo.toml", 4, "2024", Capture::Number);
    // `true` is `@boolean`, a capture name no grammar rv shipped before used.
    assert_capture(source, "Cargo.toml", 5, "true", Capture::Constant);
}

/// `Cargo.lock` is TOML with an extension that says `lock`. It is the first
/// file in this repository alphabetically, so the filename table exists mostly
/// for it: without this row the headline case of the whole feature is a blank
/// screen.
#[test]
fn cargo_lock_is_toml_by_filename() {
    let source = "[[package]]\nname = \"rv\"\nversion = \"0.1.0\"\n";

    assert_eq!(
        Highlights::of(source.as_bytes(), "Cargo.lock").language(),
        Some("toml")
    );
    assert_capture(source, "Cargo.lock", 2, "\"rv\"", Capture::String);
}

/// Markdown's block structure: a heading is the construct a reader looks for,
/// and a fenced code block is the other thing a spec is made of.
#[test]
fn markdown_headings_and_code_fences_are_captured() {
    let source = "# Title\n\nprose\n\n```\nfn a() {}\n```\n";

    assert_eq!(
        Highlights::of(source.as_bytes(), "notes.md").language(),
        Some("markdown")
    );
    assert_capture(source, "notes.md", 1, "Title", Capture::Keyword);
    assert_capture(source, "notes.md", 1, "#", Capture::Punctuation);
    assert_capture(source, "notes.md", 6, "fn a() {}", Capture::String);
    assert!(
        capture_of(source, "notes.md", 3, "prose").is_none(),
        "ordinary prose is not captured as anything"
    );
}

/// Markdown's *inline* content — emphasis, code spans, links — comes from a
/// second grammar that the block grammar injects. It is a separate parser, and
/// wiring it is the difference between a markdown file with three coloured
/// tokens and one that reads the way a reader expects, so it gets its own
/// case rather than riding along with the block test above.
#[test]
fn markdown_inline_emphasis_and_code_spans_are_captured() {
    let source = "Some *emph* and `code` and [text](http://example.com).\n";

    assert_capture(source, "notes.md", 1, "`", Capture::Punctuation);
    assert_capture(source, "notes.md", 1, "code", Capture::String);
    assert_capture(source, "notes.md", 1, "text", Capture::Variable);
    assert_capture(source, "notes.md", 1, "http://example.com", Capture::String);
    assert!(
        capture_of(source, "notes.md", 1, "Some").is_none(),
        "ordinary prose between the inline constructs stays plain"
    );
}

/// YAML: the key is what a reader scans down the left-hand side of a CI
/// workflow, and the grammar calls it a property — which rv maps to a
/// variable, the same as a struct field.
#[test]
fn yaml_keys_comments_and_scalars_are_captured() {
    let source = "# a comment\nname: build\ncount: 3\n";

    assert_eq!(
        Highlights::of(source.as_bytes(), "ci.yml").language(),
        Some("yaml")
    );
    assert_capture(source, "ci.yml", 1, "# a comment", Capture::Comment);
    assert_capture(source, "ci.yml", 2, "name", Capture::Variable);
    assert_capture(source, "ci.yml", 2, ":", Capture::Punctuation);
    assert_capture(source, "ci.yml", 2, "build", Capture::String);
    assert_capture(source, "ci.yml", 3, "3", Capture::Number);
}

/// JSON: the grammar makes no distinction between a key and any other string
/// that rv's vocabulary can express — both are strings — but numbers and
/// `true` are their own kinds, which is what makes a lock file readable.
#[test]
fn json_strings_numbers_and_literals_are_captured() {
    let source = "{\n  \"name\": \"rv\",\n  \"count\": 3,\n  \"ok\": true\n}\n";

    assert_eq!(
        Highlights::of(source.as_bytes(), "package.json").language(),
        Some("json")
    );
    assert_capture(source, "package.json", 2, "\"name\"", Capture::String);
    assert_capture(source, "package.json", 3, "3", Capture::Number);
    assert_capture(source, "package.json", 4, "true", Capture::Constant);
}

/// `.jsonc` is claimed by the JSON row on the strength of the grammar having a
/// `comment` rule. That is a claim about a dependency, so it is checked rather
/// than asserted in a comment: a `tsconfig.jsonc` whose comments came back
/// uncaptured would mean the row should not exist.
#[test]
fn jsonc_comments_are_comments() {
    let source = "{\n  // a comment\n  \"strict\": true\n}\n";

    assert_capture(
        source,
        "tsconfig.jsonc",
        2,
        "// a comment",
        Capture::Comment,
    );
    assert_capture(source, "tsconfig.jsonc", 3, "true", Capture::Constant);
}

/// Python: `def` and the name that follows it, the pair a reader uses to find
/// their way down a file.
#[test]
fn python_definitions_and_comments_are_captured() {
    let source = "# a comment\ndef parse(text):\n    return len(text)\n";

    assert_eq!(
        Highlights::of(source.as_bytes(), "parse.py").language(),
        Some("python")
    );
    assert_capture(source, "parse.py", 1, "# a comment", Capture::Comment);
    assert_capture(source, "parse.py", 2, "def", Capture::Keyword);
    assert_capture(source, "parse.py", 2, "parse", Capture::Function);
    assert_capture(source, "parse.py", 3, "return", Capture::Keyword);
    assert_capture(source, "parse.py", 3, "len", Capture::Function);
}

/// Go: `func` and the built-in types, which is most of what a Go signature is
/// made of.
#[test]
fn go_declarations_and_types_are_captured() {
    let source = "package main\n\n// a comment\nfunc parse(text string) int {\n\treturn 1\n}\n";

    assert_eq!(
        Highlights::of(source.as_bytes(), "parse.go").language(),
        Some("go")
    );
    assert_capture(source, "parse.go", 1, "package", Capture::Keyword);
    assert_capture(source, "parse.go", 3, "// a comment", Capture::Comment);
    assert_capture(source, "parse.go", 4, "func", Capture::Keyword);
    assert_capture(source, "parse.go", 4, "string", Capture::Type);
    assert_capture(source, "parse.go", 4, "int", Capture::Type);
    assert_capture(source, "parse.go", 5, "1", Capture::Number);
}

/// TypeScript. The interesting part is not the keyword but the *annotation*:
/// `tree-sitter-typescript` ships only the TypeScript-specific half of its
/// highlight query, so a configuration built from that alone captures the
/// types and nothing else — no comments, no strings, no function names. This
/// case fails if the JavaScript half is ever dropped.
#[test]
fn typescript_captures_both_its_types_and_the_javascript_underneath() {
    let source = "// a comment\nfunction parse(text: string): Ast {\n  return text.length;\n}\n";

    assert_eq!(
        Highlights::of(source.as_bytes(), "parse.ts").language(),
        Some("typescript")
    );
    // The TypeScript half.
    assert_capture(source, "parse.ts", 2, "string", Capture::Type);
    assert_capture(source, "parse.ts", 2, "Ast", Capture::Type);
    // The JavaScript half, which is where everything else comes from.
    assert_capture(source, "parse.ts", 1, "// a comment", Capture::Comment);
    assert_capture(source, "parse.ts", 2, "function", Capture::Keyword);
    assert_capture(source, "parse.ts", 2, "parse", Capture::Function);
    assert_capture(source, "parse.ts", 3, "length", Capture::Variable);
}

/// TSX is a different parser from TypeScript — JSX does not parse as
/// TypeScript — so it is a separate row with its own name.
///
/// Asserting only that `const` is a keyword and `"x"` a string would not show
/// that: the TypeScript parser error-recovers through JSX and still gets those
/// right, so such a test passes just as happily with the wrong parser. The
/// assertions below are the ones that actually separate the two. Fed
/// `<div …>`, the TypeScript parser reads `<div>` as a *type assertion* and
/// calls `div` a type; the TSX parser reads it as an element name. And the
/// element's body is text to TSX, while TypeScript, having lost the thread,
/// takes `hi` for an identifier.
#[test]
fn tsx_parses_jsx_that_typescript_alone_cannot() {
    let source = "const el = <div className=\"x\">hi</div>;\n";

    assert_eq!(
        Highlights::of(source.as_bytes(), "app.tsx").language(),
        Some("tsx")
    );
    assert_capture(source, "app.tsx", 1, "const", Capture::Keyword);
    assert_capture(source, "app.tsx", 1, "className", Capture::Variable);
    assert_capture(source, "app.tsx", 1, "\"x\"", Capture::String);
    // The two that need the JSX parser.
    assert_capture(source, "app.tsx", 1, "div", Capture::Variable);
    assert!(
        capture_of(source, "app.tsx", 1, "hi").is_none(),
        "the element's body is text, not an identifier: {:?}",
        capture_of(source, "app.tsx", 1, "hi")
    );
}

/// JavaScript rides along: its grammar has to be linked anyway to give
/// TypeScript a usable query, so claiming `.js` costs one row and leaves one
/// fewer file rendered plain.
#[test]
fn javascript_functions_and_strings_are_captured() {
    let source = "// a comment\nfunction go(a) {\n  return \"x\";\n}\n";

    assert_eq!(
        Highlights::of(source.as_bytes(), "go.js").language(),
        Some("javascript")
    );
    assert_capture(source, "go.js", 1, "// a comment", Capture::Comment);
    assert_capture(source, "go.js", 2, "function", Capture::Keyword);
    assert_capture(source, "go.js", 2, "go", Capture::Function);
    assert_capture(source, "go.js", 3, "\"x\"", Capture::String);
}

/// Bash: the shape of a shell script is its keywords and its comments, and the
/// crate spells its constant `HIGHLIGHT_QUERY` rather than `HIGHLIGHTS_QUERY`
/// — a difference that is invisible until a file comes back plain.
#[test]
fn bash_keywords_and_comments_are_captured() {
    let source = "#!/bin/bash\n# a comment\nif [ -n \"$x\" ]; then\n  echo hi\nfi\n";

    assert_eq!(
        Highlights::of(source.as_bytes(), "run.sh").language(),
        Some("bash")
    );
    assert_capture(source, "run.sh", 2, "# a comment", Capture::Comment);
    assert_capture(source, "run.sh", 3, "if", Capture::Keyword);
    assert_capture(source, "run.sh", 3, "then", Capture::Keyword);
    assert_capture(source, "run.sh", 4, "echo", Capture::Function);
    assert_capture(source, "run.sh", 5, "fi", Capture::Keyword);
}

/// The exact bytes that
/// [tree-sitter/tree-sitter-bash#337][1] segfaults on: an ASCII `{` followed
/// later by a four-byte UTF-8 codepoint. Under 0.25.1 on Linux this crashes
/// the process; the guard in [`Highlights::of`] catches the pattern and
/// reports the language with no spans, the same shape as a grammar that
/// gave up mid-parse.
///
/// [1]: https://github.com/tree-sitter/tree-sitter-bash/issues/337
#[test]
fn bash_guards_against_tree_sitter_bash_337_crash() {
    let source = "{\u{31860}".as_bytes();
    let highlights = Highlights::of(source, "run.sh");

    assert_eq!(highlights.language(), Some("bash"));
    assert!(
        highlights.line(1).is_empty(),
        "the guard yields no spans, got {:?}",
        highlights.line(1)
    );
}

// ---------------------------------------------------------------------------
// Language detection: extension only
// ---------------------------------------------------------------------------

/// No grammar for the extension means no language and no spans. rv renders the
/// file plain and says so, rather than guessing from the content — a wrong
/// guess colours a file as something it is not, which reads worse than no
/// colour at all.
#[rstest]
#[case("notes.txt")]
#[case("Makefile")]
#[case("archive.tar.gz")]
#[case("")]
#[case(".rs")]
#[case("src/.rs")]
#[case("rs")]
#[case("dir.rs/file.txt")]
fn a_file_with_no_grammar_reports_none_rather_than_guessing(#[case] path: &str) {
    let highlights = Highlights::of(b"anything at all\n", path);

    assert_eq!(highlights.language(), None, "path {path:?}");
    assert!(highlights.line(1).is_empty(), "path {path:?}");
}

/// The extension is read off the last path segment, whatever the directories
/// above it look like, and it is matched case-insensitively so a `README.RS`
/// off a case-insensitive filesystem is still Rust.
#[rstest]
#[case("parse.rs")]
#[case("src/parse.rs")]
#[case("a/b/c/parse.rs")]
#[case("parse.RS")]
#[case("weird.name.rs")]
#[case("crates\\rv\\parse.rs")]
fn a_rust_extension_selects_the_rust_grammar(#[case] path: &str) {
    let highlights = Highlights::of(b"fn a() {}\n", path);

    assert_eq!(highlights.language(), Some("rust"), "path {path:?}");
    assert!(!highlights.line(1).is_empty(), "path {path:?}");
}

/// The whole extension table, one case per extension rv claims. This is the
/// list a user reads as "which files get colour", so it is stated once,
/// explicitly, rather than inferred from the grammar rows.
#[rstest]
#[case("Cargo.toml", "toml")]
#[case("rustfmt.toml", "toml")]
#[case("README.md", "markdown")]
#[case("notes.markdown", "markdown")]
#[case("ci.yaml", "yaml")]
#[case("ci.yml", "yaml")]
#[case("package.json", "json")]
#[case("tsconfig.jsonc", "json")]
#[case("parse.py", "python")]
#[case("types.pyi", "python")]
#[case("main.go", "go")]
#[case("app.ts", "typescript")]
#[case("app.mts", "typescript")]
#[case("app.cts", "typescript")]
#[case("app.tsx", "tsx")]
#[case("app.js", "javascript")]
#[case("app.jsx", "javascript")]
#[case("app.mjs", "javascript")]
#[case("app.cjs", "javascript")]
#[case("run.sh", "bash")]
#[case("run.bash", "bash")]
#[case("src/deep/run.SH", "bash")]
fn an_extension_selects_its_grammar(#[case] path: &str, #[case] language: &str) {
    assert_eq!(
        Highlights::of(b"", path).language(),
        Some(language),
        "path {path:?}"
    );
}

/// The filename table: names whose extension does not name their language, or
/// which have no extension at all. It is deliberately short — a name has to be
/// unambiguous to earn a row, because a wrong guess here is the same failure
/// as content sniffing, just spelled differently.
#[rstest]
#[case("Cargo.lock", "toml")]
#[case("rv/Cargo.lock", "toml")]
#[case("cargo.lock", "toml")]
#[case(".bashrc", "bash")]
#[case("home/.bash_profile", "bash")]
fn a_known_filename_selects_its_grammar(#[case] path: &str, #[case] language: &str) {
    assert_eq!(
        Highlights::of(b"", path).language(),
        Some(language),
        "path {path:?}"
    );
}

/// Names that look like they might be claimed but are not. `Gemfile.lock` is
/// not TOML, `foo.lock` is nothing in particular, and a directory called
/// `Cargo.lock` is not a file — matching any of them would paint a file as a
/// language it is not.
#[rstest]
#[case("Gemfile.lock")]
#[case("yarn.lock")]
#[case("foo.lock")]
#[case("Cargo.lock/inner.txt")]
#[case("bashrc")]
#[case("notes.mdx")]
fn a_name_the_tables_do_not_claim_stays_plain(#[case] path: &str) {
    assert_eq!(
        Highlights::of(b"x\n", path).language(),
        None,
        "path {path:?}"
    );
}

/// Every grammar in the table is reachable by some path, and every path in
/// [`GRAMMAR_PATHS`] names the language it claims to. This is what keeps the
/// per-grammar properties honest: a grammar added to the module without a path
/// here would never be fuzzed, and the count check is what notices.
#[test]
fn the_grammar_paths_cover_every_shipped_language() {
    let mut languages: Vec<&str> = GRAMMAR_PATHS
        .iter()
        .map(|(path, language)| {
            assert_eq!(
                Highlights::of(b"", path).language(),
                Some(*language),
                "path {path:?}"
            );
            *language
        })
        .collect();
    languages.sort_unstable();
    languages.dedup();
    assert_eq!(
        languages,
        [
            "bash",
            "go",
            "javascript",
            "json",
            "markdown",
            "python",
            "rust",
            "toml",
            "tsx",
            "typescript",
            "yaml",
        ],
        "every shipped grammar has a path the properties can reach it by"
    );
}

/// Detection looks at the path and nothing else: a file full of Rust under a
/// `.txt` name stays plain, and a file full of prose under a `.rs` name is
/// handed to the Rust grammar. Content sniffing is exactly what this avoids.
#[test]
fn detection_ignores_the_content_entirely() {
    let rust_source = b"fn main() { println!(\"hi\"); }\n";

    assert_eq!(Highlights::of(rust_source, "notes.txt").language(), None);
    assert_eq!(
        Highlights::of(b"just some prose, honestly\n", "prose.rs").language(),
        Some("rust")
    );
}

// ---------------------------------------------------------------------------
// Totality: the inputs a real repository will hand this
// ---------------------------------------------------------------------------

/// Source that does not parse still comes back with something. tree-sitter
/// recovers from errors, and a half-highlighted broken file is what a reviewer
/// looking at a work-in-progress branch has to see.
#[test]
fn source_that_does_not_parse_still_returns_something() {
    let highlights = Highlights::of(b"fn (((( unterminated\n", "broken.rs");

    let _ = highlights.line(1); // must not panic; tree-sitter recovers
    assert_eq!(highlights.language(), Some("rust"));
}

/// A blob that is not UTF-8 at all — a stray binary under a `.rs` name, or a
/// file in some other encoding — is an answer, not a panic.
#[test]
fn invalid_utf8_is_not_a_panic() {
    let _ = Highlights::of(&[0xff, 0xfe, b'\n'], "weird.rs");
    let _ = Highlights::of(&[0xff, 0xfe, b'\n'], "weird.txt");
    let _ = Highlights::of(&[b'f', b'n', b' ', 0x80, 0x80, b'\n'], "weird.rs");
}

/// Asking for a line that does not exist — line 0, or a line past the end —
/// gives back nothing instead of panicking. A viewport that scrolls past the
/// end of a file does this constantly.
#[test]
fn a_line_outside_the_file_is_empty_not_a_panic() {
    let highlights = Highlights::of(b"fn a() {}\n", "a.rs");

    assert!(highlights.line(0).is_empty());
    assert!(highlights.line(2).is_empty());
    assert!(highlights.line(u32::MAX).is_empty());
}

// ---------------------------------------------------------------------------
// Slicing: the consumer's text is not always the blob's line
// ---------------------------------------------------------------------------

/// [`Span::slice`] is the safe way to get a span's text, because the string a
/// caller holds (a diff line's text) is not always byte-identical to the blob
/// line the span was measured against. It clamps rather than panicking, and it
/// never splits a multi-byte character.
#[test]
fn slicing_a_span_against_a_shorter_string_clamps() {
    let span = Span {
        line: 1,
        start: 2,
        end: 10,
        capture: Capture::Keyword,
    };

    assert_eq!(span.slice("abcdefghij"), "cdefghij");
    assert_eq!(span.slice("abcd"), "cd");
    assert_eq!(span.slice("ab"), "");
    assert_eq!(span.slice(""), "");
}

/// Clamping lands on a character boundary, so a span whose end falls inside a
/// multi-byte character yields the shorter valid slice instead of panicking on
/// a byte index that is not a boundary.
#[test]
fn slicing_never_splits_a_character() {
    let text = "aé😀b"; // 1 + 2 + 4 + 1 bytes
    for start in 0..=10u32 {
        for end in 0..=10u32 {
            let span = Span {
                line: 1,
                start,
                end,
                capture: Capture::Other,
            };
            let slice = span.slice(text);
            assert!(
                text.contains(slice),
                "{start}..{end} of {text:?} gave {slice:?}"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Generators
// ---------------------------------------------------------------------------

/// Fragments that produce real captures when pasted together, mixed with
/// fragments that break the parse and fragments with multi-byte characters —
/// so the properties see both well-formed Rust and the wreckage of a
/// half-typed edit.
fn rust_fragment() -> impl Strategy<Value = &'static str> {
    prop::sample::select(vec![
        "fn go(x: u32) -> Result<Ast> {",
        "    let mut total = 0usize;",
        "    // a comment",
        "    /// a doc comment",
        "    let s = \"a string with a é in it\";",
        "    let r = r#\"raw",
        "over two lines\"#;",
        "    total += x as usize;",
        "}",
        "#[derive(Debug, Clone)]",
        "struct Ünïcödé { field: String }",
        "impl Trait for Ünïcödé {}",
        "fn ((((",
        "\" unterminated",
        "",
        "\t",
        "мир мир мир",
        "}}}}",
    ])
}

/// Plausible-to-hostile source text: joined Rust fragments, or arbitrary
/// characters, with either kind of line ending.
fn source_text() -> impl Strategy<Value = String> {
    prop_oneof![
        4 => prop::collection::vec(rust_fragment(), 0..12)
            .prop_map(|lines| lines.join("\n")),
        2 => prop::collection::vec(rust_fragment(), 0..12)
            .prop_map(|lines| format!("{}\n", lines.join("\r\n"))),
        1 => ".{0,120}",
    ]
}

/// Byte sequences a blob under a `.rs` name might really be: Rust-shaped
/// punctuation, valid multi-byte characters, stray continuation bytes, a
/// truncated lead byte, and bytes that are never valid UTF-8 anywhere. The
/// last two are the point — a grammar handed bytes it cannot decode can report
/// a range that starts inside a character, and the module has to walk that
/// range back to a boundary before a consumer sees it.
fn byte_chunk() -> impl Strategy<Value = &'static [u8]> {
    prop::sample::select(vec![
        b"fn ".as_slice(),
        b"let ",
        b"\"",
        b"//",
        b"/*",
        b"{",
        b"}",
        b"(",
        b")",
        b";",
        b"::",
        b"\n",
        b"\r\n",
        b"\t",
        b" ",
        b"0",
        b"a",
        b"Zz",
        b"\xc3\xa9",         // é
        b"\xf0\x9f\x98\x80", // an emoji
        b"\x80",             // a lone continuation byte
        b"\xbf",
        b"\xc3", // a lead byte with nothing following it
        b"\xff",
        b"\xfe",
    ])
}

/// Blobs, not strings: valid UTF-8 sources, chunk-built byte soup, and
/// uniformly random bytes.
fn source_bytes() -> impl Strategy<Value = Vec<u8>> {
    prop_oneof![
        3 => prop::collection::vec(byte_chunk(), 0..40).prop_map(|chunks| chunks.concat()),
        1 => prop::collection::vec(any::<u8>(), 0..120),
        1 => source_text().prop_map(String::into_bytes),
    ]
}

/// Path-shaped strings, including the ones that trip a naive extension parse:
/// dotfiles, double extensions, directories with dots, and no extension at all.
fn path_text() -> impl Strategy<Value = String> {
    prop_oneof![
        3 => prop::sample::select(vec![
            "a.rs", "src/a.rs", "a.RS", "a.tar.gz", "Makefile", "", ".rs", "a.rs/b", "dir.rs/b.txt",
            "a.", "a.txt", "weird .rs", "a.rs ", "мир.rs",
            // One per shipped grammar, plus the near-misses the filename table
            // must not claim, so the totality properties see every parser.
            "a.toml", "Cargo.lock", "cargo.lock", "Cargo.lock/b", "yarn.lock", "a.md", "a.yml",
            "a.json", "a.py", "a.go", "a.ts", "a.tsx", "a.js", "a.sh", ".bashrc", "bashrc",
        ])
        .prop_map(String::from),
        1 => "[a-zA-Z0-9._/\\\\-]{0,24}",
        1 => ".{0,24}",
    ]
}

// ---------------------------------------------------------------------------
// Properties
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(96))]

    /// Totality over arbitrary bytes and arbitrary paths. `of` is documented
    /// as never failing, and a reviewer opening a file must never be the
    /// thing that kills the process — so neither the call nor any lookup
    /// afterwards is allowed to panic, whatever the bytes and whatever the
    /// name.
    #[test]
    fn of_is_total_over_arbitrary_bytes_and_paths(
        bytes in prop::collection::vec(any::<u8>(), 0..256),
        path in path_text(),
    ) {
        let highlights = Highlights::of(&bytes, &path);
        let _ = highlights.language();
        for line in [0u32, 1, 2, 7, 4096, u32::MAX] {
            let _ = highlights.line(line);
        }
    }

    /// Whatever the source, every span is a non-empty range that lies inside
    /// the line it claims, measured against the independent [`ref_line`]
    /// oracle. This is the clamp the module promises: a grammar that reports
    /// a range past the end of a line must not reach a consumer.
    #[test]
    fn every_span_lies_inside_its_own_line(source in source_text()) {
        let highlights = Highlights::of(source.as_bytes(), "prop.rs");

        for line in probe_lines(&source) {
            for span in highlights.line(line) {
                prop_assert_eq!(span.line, line);
                prop_assert!(span.start < span.end, "empty span {:?}", span);
                let text = ref_line(&source, line).unwrap_or("");
                prop_assert!(
                    span.end as usize <= text.len(),
                    "span {:?} reaches past line {:?}",
                    span,
                    text
                );
            }
        }
    }

    /// Within a line, spans are ordered by column and never overlap, so a
    /// renderer can walk them in one pass and each column is claimed by at
    /// most one capture.
    #[test]
    fn spans_within_a_line_are_ordered_and_disjoint(source in source_text()) {
        let highlights = Highlights::of(source.as_bytes(), "prop.rs");

        for line in probe_lines(&source) {
            for pair in highlights.line(line).windows(2) {
                prop_assert!(
                    pair[0].end <= pair[1].start,
                    "overlapping or unordered spans {:?}",
                    pair
                );
            }
        }
    }

    /// A span's byte offsets always land on character boundaries of the line
    /// they were measured against, so `&line[span.start..span.end]` on the
    /// blob's own text is a valid slice rather than a panic.
    #[test]
    fn span_offsets_are_character_boundaries(source in source_text()) {
        let highlights = Highlights::of(source.as_bytes(), "prop.rs");

        for line in probe_lines(&source) {
            let Some(text) = ref_line(&source, line) else { continue };
            for span in highlights.line(line) {
                prop_assert!(
                    text.is_char_boundary(span.start as usize)
                        && text.is_char_boundary(span.end as usize),
                    "span {:?} is not on a character boundary of {:?}",
                    span,
                    text
                );
                let _ = &text[span.start as usize..span.end as usize];
            }
        }
    }

    /// The same guarantee stated over raw bytes, which is where it has teeth:
    /// handed a blob that is not valid UTF-8, tree-sitter really does report
    /// ranges that begin or end inside a multi-byte character, so a span edge
    /// landing on a continuation byte is a bug this catches rather than a
    /// hypothetical. The `.rs` blob in a repository that turns out to be
    /// latin-1, or a binary, is the case.
    #[test]
    fn span_edges_never_land_inside_a_character_even_in_a_blob_that_is_not_utf8(
        source in source_bytes(),
    ) {
        let highlights = Highlights::of(&source, "prop.rs");

        for (index, &line_start) in ref_line_starts(&source).iter().enumerate() {
            let line = u32::try_from(index + 1).expect("line fits");
            for span in highlights.line(line) {
                for edge in [span.start, span.end] {
                    let at = line_start + edge as usize;
                    prop_assert!(
                        !source.get(at).copied().is_some_and(is_continuation),
                        "span {:?} on line {} has an edge at byte {} = {:#04x}, inside a character",
                        span,
                        line,
                        at,
                        source.get(at).copied().unwrap_or(0)
                    );
                }
            }
        }
    }

    /// Every grammar, over the same hostile bytes: no panic, a language
    /// always reported, and the same ordering, disjointness and
    /// character-boundary guarantees Rust already has.
    ///
    /// This is the property that earns its keep once there is more than one
    /// grammar. Each one is a separate C parser with its own external
    /// scanner, and the bytes here — lone continuation bytes, truncated lead
    /// bytes, `\xff` — are exactly what a mis-encoded file in a repository
    /// looks like. A parser that reports a range starting inside a character
    /// is a panic in any consumer that indexes by column, and only running
    /// every grammar over this generator finds the one that does.
    #[test]
    fn every_grammar_is_total_over_arbitrary_bytes(source in source_bytes()) {
        for (path, language) in GRAMMAR_PATHS {
            let highlights = Highlights::of(&source, path);
            prop_assert_eq!(highlights.language(), Some(*language), "path {}", path);

            let starts = ref_line_starts(&source);
            for (index, &line_start) in starts.iter().enumerate() {
                let line = u32::try_from(index + 1).expect("line fits");
                let line_end = starts.get(index + 1).copied().unwrap_or(source.len());
                let spans = highlights.line(line);
                for pair in spans.windows(2) {
                    prop_assert!(
                        pair[0].end <= pair[1].start,
                        "{}: overlapping or unordered spans {:?}",
                        path,
                        pair
                    );
                }
                for span in spans {
                    prop_assert!(span.start < span.end, "{}: empty span {:?}", path, span);
                    prop_assert!(
                        line_start + span.end as usize <= line_end,
                        "{}: span {:?} reaches past line {}",
                        path,
                        span,
                        line
                    );
                    for edge in [span.start, span.end] {
                        let at = line_start + edge as usize;
                        prop_assert!(
                            !source.get(at).copied().is_some_and(is_continuation),
                            "{}: span {:?} has an edge at byte {} inside a character",
                            path,
                            span,
                            at
                        );
                    }
                }
            }
        }
    }

    /// Highlighting the same bytes twice gives exactly the same spans. The
    /// diff pane re-renders constantly and caches by `(commit, path)`; a
    /// second parse that disagreed with the first would make colours flicker
    /// between frames.
    #[test]
    fn highlighting_is_deterministic(source in source_text()) {
        let once = Highlights::of(source.as_bytes(), "prop.rs");
        let twice = Highlights::of(source.as_bytes(), "prop.rs");

        prop_assert_eq!(once.language(), twice.language());
        prop_assert_eq!(all_spans(&once, &source), all_spans(&twice, &source));
    }

    /// The language depends on the path and nothing else: the same path over
    /// two different blobs reports the same language. Together with
    /// [`detection_ignores_the_content_entirely`] this is the "no content
    /// sniffing" rule stated as a law.
    #[test]
    fn language_depends_only_on_the_path(
        path in path_text(),
        first in prop::collection::vec(any::<u8>(), 0..128),
        second in prop::collection::vec(any::<u8>(), 0..128),
    ) {
        prop_assert_eq!(
            Highlights::of(&first, &path).language(),
            Highlights::of(&second, &path).language()
        );
    }

    /// [`Span::slice`] is total: any span against any string gives back a
    /// substring, never a panic. The caller's text is a diff line, which is
    /// not guaranteed to be the blob line the span was measured against.
    #[test]
    fn slice_is_total(
        start in 0u32..64,
        end in 0u32..64,
        text in ".{0,40}",
    ) {
        let span = Span { line: 1, start, end, capture: Capture::Other };
        let slice = span.slice(&text);
        prop_assert!(text.contains(slice));
    }
}
