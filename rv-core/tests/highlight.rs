//! Tests for `rv_core::highlight`: the tree-sitter layer that turns a file's
//! bytes into per-line, per-column [`Span`]s naming a [`Capture`] class.
//!
//! Two halves. The hand-written cases pin one example per documented
//! behaviour — what a Rust file's keywords and functions come back as, what a
//! file with no grammar comes back as, what a broken parse comes back as. The
//! properties go after the *contract* a consumer relies on to render without
//! crashing: totality over arbitrary bytes and arbitrary paths, spans that are
//! ordered, disjoint and inside their own line, byte offsets that are always
//! char boundaries, and a deterministic answer for the same input.
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
