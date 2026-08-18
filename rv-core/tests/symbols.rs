//! Tests for `rv_core::symbols`: the tree-sitter-tags layer that turns a
//! file's bytes into the named definitions a reviewer can jump to.
//!
//! Two halves, the same shape as `tests/highlight.rs`. The hand-written cases
//! pin one example per documented behaviour — what a Rust file's items come
//! back as, that a method and a free function of the same name are both
//! reported, that a reference is not a definition, and one characteristic
//! definition per language that has tags — plus the languages that have
//! *none*, stated explicitly, because "this language has no symbols" is a fact
//! a user will hit and should not have to discover twice.
//!
//! The properties go after the contract a caller relies on: totality over
//! arbitrary bytes and arbitrary paths, every reported line inside the
//! source's own line count, and an answer that depends on `(source, path)` and
//! nothing else — the last stated as "a call in between changes nothing",
//! because the caller of this module is about to cache its results.
//!
//! The line-count oracle ([`ref_line_count`]) is a plain `split('\n')` walk
//! written independently of the module, so "inside the file" is checked
//! against a second opinion rather than against a copy of the code under test.

use proptest::prelude::*;
use rstest::rstest;
use rv_core::highlight::Highlights;
use rv_core::symbols::Symbol;
use rv_core::symbols::SymbolKind;
use rv_core::symbols::of;

// ---------------------------------------------------------------------------
// Oracles and helpers
// ---------------------------------------------------------------------------

/// How many lines `source` has, as the rest of this codebase counts them: the
/// text between newlines, with no final empty line opened by a trailing `\n`.
/// A `split('\n')` walk on purpose — an independent opinion, not a second copy
/// of the module's arithmetic.
fn ref_line_count(source: &[u8]) -> u32 {
    let text = String::from_utf8_lossy(source);
    let mut pieces: Vec<&str> = text.split('\n').collect();
    if text.ends_with('\n') {
        pieces.pop();
    }
    u32::try_from(pieces.len()).expect("line count fits")
}

/// `(name, kind, line)` for every symbol, the shape the assertions read in.
fn triples(symbols: &[Symbol]) -> Vec<(&str, SymbolKind, u32)> {
    symbols
        .iter()
        .map(|symbol| (symbol.name.as_str(), symbol.kind, symbol.line))
        .collect()
}

fn names(symbols: &[Symbol]) -> Vec<&str> {
    symbols.iter().map(|symbol| symbol.name.as_str()).collect()
}

/// One path, language and minimal sample per language that has a tags query,
/// so the per-grammar properties reach every parser rv actually extracts
/// symbols from. A language added to the module without a row here would never
/// be fuzzed, and [`the_tagged_paths_cover_every_language_with_symbols`] is
/// what notices.
const TAGGED_PATHS: &[(&str, &str, &str)] = &[
    ("prop.rs", "rust", "fn f() {}\n"),
    ("prop.go", "go", "package p\nfunc f() {}\n"),
    ("prop.py", "python", "def f():\n    pass\n"),
    ("prop.js", "javascript", "function f() {}\n"),
    ("prop.ts", "typescript", "function f(): void {}\n"),
    ("prop.tsx", "tsx", "function f() { return <b/>; }\n"),
];

/// The other half of the shipped grammars: highlighted, but with no `tags.scm`
/// to extract definitions from. Each is pinned with content that *does* have
/// something a reader would call a definition — a bash function, a TOML table,
/// a markdown heading — so that "no symbols" is a stated fact about the
/// grammar rather than an accident of an empty sample.
const UNTAGGED_PATHS: &[(&str, &str)] = &[
    ("Cargo.toml", "[package]\nname = \"rv\"\n"),
    ("Cargo.lock", "[[package]]\nname = \"rv\"\n"),
    ("README.md", "# Title\n\nSome prose.\n"),
    ("ci.yml", "jobs:\n  build:\n    runs-on: ubuntu\n"),
    ("package.json", "{\"name\": \"rv\", \"version\": \"1\"}\n"),
    ("run.sh", "main() {\n  echo hi\n}\nmain\n"),
];

// ---------------------------------------------------------------------------
// Rust: the language rv is reviewed in
// ---------------------------------------------------------------------------

/// The case from the plan. A struct, a method and a free function, each with
/// its kind and the 1-based line its *name* sits on.
#[test]
fn rust_items_are_found_with_their_kinds_and_lines() {
    let source = br#"
struct Anchor { line: u32 }

impl Anchor {
    fn resolve(&self, text: &str) -> Option<u32> { None }
}

fn parse(s: &str) -> Anchor { Anchor { line: 0 } }
"#;
    let symbols = of(source, "anchor.rs");
    let named = triples(&symbols);

    assert!(
        named.contains(&("Anchor", SymbolKind::Struct, 2)),
        "got {named:?}"
    );
    assert!(
        named
            .iter()
            .any(|(n, k, _)| *n == "resolve" && *k == SymbolKind::Function),
        "got {named:?}"
    );
    assert!(
        named
            .iter()
            .any(|(n, k, _)| *n == "parse" && *k == SymbolKind::Function),
        "got {named:?}"
    );
}

/// Navigation must not silently drop one of two things a reviewer could mean.
#[test]
fn a_method_and_a_free_function_of_the_same_name_are_both_reported() {
    let source = b"fn write(x: u8) {}\nstruct S;\nimpl S { fn write(&self) {} }\n";

    let count = of(source, "s.rs")
        .iter()
        .filter(|s| s.name == "write")
        .count();

    assert_eq!(count, 2, "both are jump targets");
}

/// Every Rust item kind rv distinguishes, in one file. The shipped `tags.scm`
/// calls a struct, an enum, a union and a type alias all `class`; rv splits
/// them, and this is the test that says so.
#[test]
fn every_rust_item_kind_gets_its_own_symbol_kind() {
    let source = b"struct S;\n\
                   enum E { A }\n\
                   union U { a: u8 }\n\
                   type T = u8;\n\
                   trait Tr { fn m(&self); }\n\
                   mod m {}\n\
                   macro_rules! mac { () => {} }\n\
                   fn f() {}\n\
                   impl S { fn method(&self) {} }\n\
                   impl<X> Tr for Vec<X> { fn m(&self) {} }\n";
    let found = of(source, "kinds.rs");
    let named = triples(&found);

    for expected in [
        ("S", SymbolKind::Struct, 1),
        ("E", SymbolKind::Enum, 2),
        ("U", SymbolKind::Struct, 3),
        ("T", SymbolKind::Type, 4),
        ("Tr", SymbolKind::Trait, 5),
        ("m", SymbolKind::Module, 6),
        ("mac", SymbolKind::Macro, 7),
        ("f", SymbolKind::Function, 8),
        ("S", SymbolKind::Impl, 9),
        ("method", SymbolKind::Function, 9),
        ("Vec", SymbolKind::Impl, 10),
    ] {
        assert!(
            named.contains(&expected),
            "missing {expected:?} in {named:?}"
        );
    }
}

/// A call is not a definition. `tags.scm` reports both — that is what makes it
/// useful for a code-search index — and rv keeps only the definitions, because
/// a jump list full of call sites is a jump list nobody can step through.
#[test]
fn a_call_site_is_not_a_symbol() {
    let source = b"fn parse() {}\nfn go() { parse(); helper(); }\n";

    let found = of(source, "calls.rs");

    assert_eq!(names(&found), ["parse", "go"], "got {found:?}");
}

// ---------------------------------------------------------------------------
// One characteristic definition per language that has tags
// ---------------------------------------------------------------------------

#[test]
fn go_definitions_are_found() {
    let source = b"package main\n\
                   type Anchor struct{ Line int }\n\
                   func Parse(s string) *Anchor { return nil }\n\
                   func (a *Anchor) Resolve() int { return 0 }\n";
    let found = of(source, "anchor.go");
    let named = triples(&found);

    assert!(
        named.contains(&("Anchor", SymbolKind::Type, 2)),
        "got {named:?}"
    );
    assert!(
        named.contains(&("Parse", SymbolKind::Function, 3)),
        "got {named:?}"
    );
    assert!(
        named.contains(&("Resolve", SymbolKind::Function, 4)),
        "got {named:?}"
    );
}

#[test]
fn python_definitions_are_found() {
    let source = b"VERSION = 3\n\
                   class Anchor:\n\
                   \x20   def resolve(self):\n\
                   \x20       return None\n";
    let found = of(source, "anchor.py");
    let named = triples(&found);

    assert!(
        named.contains(&("VERSION", SymbolKind::Constant, 1)),
        "got {named:?}"
    );
    assert!(
        named.contains(&("Anchor", SymbolKind::Struct, 2)),
        "got {named:?}"
    );
    assert!(
        named.contains(&("resolve", SymbolKind::Function, 3)),
        "got {named:?}"
    );
}

#[test]
fn javascript_definitions_are_found() {
    let source = b"function parse(s) { return s; }\n\
                   class Anchor { resolve() { return 1; } }\n\
                   const render = (x) => x;\n";
    let found = of(source, "anchor.js");
    let named = triples(&found);

    assert!(
        named.contains(&("parse", SymbolKind::Function, 1)),
        "got {named:?}"
    );
    assert!(
        named.contains(&("Anchor", SymbolKind::Struct, 2)),
        "got {named:?}"
    );
    assert!(
        named.contains(&("resolve", SymbolKind::Function, 2)),
        "got {named:?}"
    );
    assert!(
        named.contains(&("render", SymbolKind::Function, 3)),
        "got {named:?}"
    );
}

/// TypeScript needs both halves of the query, exactly as highlighting does:
/// `interface` and `function_signature` come from TypeScript's own `tags.scm`,
/// while a plain `function` or `class` declaration is only in JavaScript's.
/// A configuration built from the TypeScript half alone finds the interface
/// and nothing else.
#[test]
fn typescript_definitions_need_both_halves_of_the_query() {
    let source = b"interface Anchor { resolve(): number; }\n\
                   export function parse(s: string): Anchor | null { return null; }\n\
                   class Store { write(): void {} }\n";
    let found = of(source, "anchor.ts");
    let named = triples(&found);

    assert!(
        named.contains(&("Anchor", SymbolKind::Trait, 1)),
        "got {named:?}"
    );
    assert!(
        named.contains(&("resolve", SymbolKind::Function, 1)),
        "got {named:?}"
    );
    assert!(
        named.contains(&("parse", SymbolKind::Function, 2)),
        "got {named:?}"
    );
    assert!(
        named.contains(&("Store", SymbolKind::Struct, 3)),
        "got {named:?}"
    );
    assert!(
        named.contains(&("write", SymbolKind::Function, 3)),
        "got {named:?}"
    );
}

/// `.tsx` is a separate parser, not a separate query — JSX does not parse as
/// TypeScript — so it gets its own case.
#[test]
fn tsx_definitions_are_found() {
    let source = b"interface Props { title: string; }\n\
                   export function Panel(p: Props) { return <div>{p.title}</div>; }\n";
    let found = of(source, "panel.tsx");
    let named = triples(&found);

    assert!(
        named.contains(&("Props", SymbolKind::Trait, 1)),
        "got {named:?}"
    );
    assert!(
        named.contains(&("Panel", SymbolKind::Function, 2)),
        "got {named:?}"
    );
}

// ---------------------------------------------------------------------------
// The languages with no symbols, stated rather than discovered
// ---------------------------------------------------------------------------

/// A file whose name selects no grammar at all yields nothing rather than
/// guessing from the content.
#[rstest]
#[case("notes.txt")]
#[case("Makefile")]
#[case("data.json")]
#[case("")]
#[case(".rs")]
#[case("archive.tar.gz")]
fn a_file_with_no_grammar_yields_no_symbols_rather_than_guessing(#[case] path: &str) {
    assert!(of(b"anything at all\n", path).is_empty(), "path {path:?}");
}

/// Five of rv's eleven grammars ship no `tags.scm`: TOML, JSON, YAML,
/// markdown and bash. Those files are highlighted and have no symbols, and
/// that is the answer a user gets — not a partial list, and not a crash. Each
/// case asserts both halves, so the pin cannot pass because detection quietly
/// stopped claiming the file.
#[test]
fn a_highlighted_language_with_no_tags_query_has_no_symbols() {
    for (path, sample) in UNTAGGED_PATHS {
        assert!(
            Highlights::of(sample.as_bytes(), path).language().is_some(),
            "{path:?} is supposed to be a highlighted language"
        );
        assert!(
            of(sample.as_bytes(), path).is_empty(),
            "{path:?} has no tags query, so it has no symbols"
        );
    }
}

/// Every path in [`TAGGED_PATHS`] really does produce symbols, and the list
/// covers every language rv extracts them from. Without this the per-grammar
/// properties could silently stop exercising a parser.
#[test]
fn the_tagged_paths_cover_every_language_with_symbols() {
    let mut languages: Vec<&str> = TAGGED_PATHS
        .iter()
        .map(|(path, language, sample)| {
            assert_eq!(
                names(&of(sample.as_bytes(), path)),
                ["f"],
                "path {path:?} did not report the one definition in {sample:?}"
            );
            *language
        })
        .collect();
    languages.sort_unstable();
    assert_eq!(
        languages,
        ["go", "javascript", "python", "rust", "tsx", "typescript"],
        "every language with a usable tags.scm has a path the properties can reach it by"
    );
}

// ---------------------------------------------------------------------------
// Detection: by name, and only by name
// ---------------------------------------------------------------------------

/// The same detection `highlight.rs` uses, reached through the same table:
/// the last path segment, matched case-insensitively, directories ignored.
#[rstest]
#[case("parse.rs")]
#[case("src/parse.rs")]
#[case("a/b/c/parse.rs")]
#[case("parse.RS")]
#[case("weird.name.rs")]
#[case("crates\\rv\\parse.rs")]
fn a_rust_name_selects_the_rust_tags_query(#[case] path: &str) {
    assert_eq!(names(&of(b"fn a() {}\n", path)), ["a"], "path {path:?}");
}

/// Names that look like they might be claimed but are not — the same list
/// `highlight.rs` pins, restated here so that symbols and colour cannot drift
/// apart. `zsh` is not bash, `.mdx` is not markdown, `Gemfile.lock` is not
/// TOML and `Dockerfile` is nothing rv ships; none of them has symbols either.
#[rstest]
#[case("build.zsh")]
#[case("notes.mdx")]
#[case("Gemfile.lock")]
#[case("Dockerfile")]
#[case("yarn.lock")]
#[case("bashrc")]
#[case("Cargo.lock/inner.txt")]
fn a_name_the_detection_table_does_not_claim_has_no_symbols(#[case] path: &str) {
    assert!(
        of(b"fn a() {}\nfunction a() {}\n", path).is_empty(),
        "path {path:?}"
    );
}

/// Detection reads the path and nothing else: Rust under a `.txt` name has no
/// symbols, and prose under a `.rs` name is handed to the Rust grammar (and
/// simply has no definitions in it). Content sniffing is what this avoids.
#[test]
fn detection_ignores_the_content_entirely() {
    assert!(of(b"fn parse() {}\n", "notes.txt").is_empty());
    assert!(of(b"just some prose, honestly\n", "prose.rs").is_empty());
}

// ---------------------------------------------------------------------------
// Totality and ordering
// ---------------------------------------------------------------------------

/// Source that does not parse still returns what tree-sitter recovered, and
/// never panics. A reviewer looking at a work-in-progress branch sees exactly
/// this.
#[test]
fn source_that_does_not_parse_still_returns_what_it_can() {
    let _ = of(b"fn a( { struct \n", "broken.rs");
    let _ = of(b"fn (((( unterminated\n", "broken.rs");
    let _ = of(b"", "empty.rs");
}

/// A blob that is not UTF-8 at all — a stray binary under a `.rs` name, or a
/// file in some other encoding — is an answer, not a panic.
#[test]
fn invalid_utf8_is_not_a_panic() {
    let _ = of(&[0xff, 0xfe, b'\n'], "weird.rs");
    let _ = of(&[b'f', b'n', b' ', 0x80, 0x80, b'\n'], "weird.rs");
    let _ = of(&[b'f', b'n', b' ', 0xc3], "weird.rs");
}

/// Callers step through these in order, so they come back in order.
#[test]
fn symbols_come_back_in_line_order() {
    let source = b"fn c() {}\nfn a() {}\nfn b() {}\n";

    let lines: Vec<u32> = of(source, "x.rs").iter().map(|s| s.line).collect();

    let mut sorted = lines.clone();
    sorted.sort_unstable();
    assert_eq!(lines, sorted, "callers step through these in order");
}

/// Lines are 1-based, like every other line number in this codebase — the
/// first line of a file is line 1, never line 0.
#[test]
fn lines_are_one_based() {
    assert_eq!(of(b"fn first() {}\n", "x.rs")[0].line, 1);
}

// ---------------------------------------------------------------------------
// Generators
// ---------------------------------------------------------------------------

/// Fragments that produce real definitions when pasted together, mixed with
/// fragments that break the parse and fragments with multi-byte characters —
/// so the properties see both well-formed source and the wreckage of a
/// half-typed edit.
fn source_fragment() -> impl Strategy<Value = &'static str> {
    prop::sample::select(vec![
        "fn go(x: u32) -> Result<Ast> {",
        "struct Ünïcödé { field: String }",
        "impl Trait for Ünïcödé {}",
        "enum E { A, B }",
        "mod inner {",
        "macro_rules! mac { () => {} }",
        "type Alias = u8;",
        "func Parse(s string) error {",
        "type Anchor struct{}",
        "class Anchor:",
        "    def resolve(self):",
        "function parse(s) {",
        "interface Props { title: string; }",
        "    // a comment",
        "}",
        // Half-typed items, with no `{` and no `;` after the name. Joined
        // without a trailing newline, one of these as the last fragment puts a
        // definition's *name* on the final byte of the blob — the case an
        // off-by-one in the name slice panics on, and the case a partial read
        // or a mid-keystroke buffer really produces.
        "fn parse",
        "struct Anchor",
        "mod inner",
        "func Parse",
        "class Anchor",
        "function parse",
        "fn ((((",
        "\" unterminated",
        "",
        "\t",
        "мир мир мир",
        "}}}}",
    ])
}

/// Plausible-to-hostile source text: joined fragments, or arbitrary
/// characters, with either kind of line ending.
fn source_text() -> impl Strategy<Value = String> {
    prop_oneof![
        4 => prop::collection::vec(source_fragment(), 0..12)
            .prop_map(|lines| lines.join("\n")),
        2 => prop::collection::vec(source_fragment(), 0..12)
            .prop_map(|lines| format!("{}\n", lines.join("\r\n"))),
        1 => ".{0,120}",
    ]
}

/// Blobs, not strings: valid sources, sources chopped off mid-identifier — a
/// truncated blob is what a partial read or a corrupt object looks like, and
/// it is the case where a definition's *name* runs right up to the end of the
/// file — and uniformly random bytes.
fn source_bytes() -> impl Strategy<Value = Vec<u8>> {
    prop_oneof![
        3 => source_text().prop_map(String::into_bytes),
        3 => (source_text(), 0usize..200)
            .prop_map(|(text, at)| {
                let bytes = text.into_bytes();
                let at = at.min(bytes.len());
                bytes[..at].to_vec()
            }),
        2 => prop::collection::vec(any::<u8>(), 0..160),
    ]
}

/// Path-shaped strings, including the ones that trip a naive extension parse:
/// dotfiles, double extensions, directories with dots, and no extension at
/// all. Every language rv ships has a row, so the totality property runs every
/// parser.
fn path_text() -> impl Strategy<Value = String> {
    prop_oneof![
        3 => prop::sample::select(vec![
            "a.rs", "src/a.rs", "a.RS", "a.tar.gz", "Makefile", "", ".rs", "a.rs/b", "dir.rs/b.txt",
            "a.", "a.txt", "weird .rs", "a.rs ", "мир.rs",
            "a.toml", "Cargo.lock", "cargo.lock", "yarn.lock", "a.md", "a.yml",
            "a.json", "a.py", "a.go", "a.ts", "a.tsx", "a.js", "a.sh", ".bashrc", "Dockerfile",
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
    // More cases than the highlight properties run, because the interesting
    // inputs here are narrow: a definition whose name lands on the very last
    // byte of a truncated blob is a small target, and it is the one an
    // off-by-one in the name slice panics on.
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// Totality over arbitrary bytes and arbitrary paths. `of` is documented
    /// as never failing: a reviewer opening a file must never be the thing
    /// that kills the process, whatever the bytes and whatever the name.
    #[test]
    fn of_is_total_over_arbitrary_bytes_and_paths(
        bytes in source_bytes(),
        path in path_text(),
    ) {
        for symbol in of(&bytes, &path) {
            let _ = symbol.name.len();
            let _ = symbol.kind.label();
            let _ = symbol.line;
        }
    }

    /// Every grammar with a tags query, over the same hostile bytes. Each one
    /// is a separate C parser with its own external scanner, and a truncated
    /// blob — a definition whose name runs to the last byte of the file — is
    /// exactly where an off-by-one in the name slice or the line number
    /// shows up.
    #[test]
    fn every_tagged_grammar_is_total_over_arbitrary_bytes(source in source_bytes()) {
        for (path, _, _) in TAGGED_PATHS {
            for symbol in of(&source, path) {
                prop_assert!(!symbol.name.is_empty(), "{path}: an empty name is not a jump target");
            }
        }
    }

    /// Every reported line is a real line of the source: at least 1, never
    /// past the end. A jump to a line the file does not have is a cursor
    /// nobody can place.
    #[test]
    fn every_line_is_inside_the_source(source in source_bytes()) {
        let count = ref_line_count(&source);
        for (path, _, _) in TAGGED_PATHS {
            for symbol in of(&source, path) {
                prop_assert!(
                    symbol.line >= 1 && symbol.line <= count,
                    "{path}: {:?} is not one of the {count} lines of {source:?}",
                    symbol
                );
            }
        }
    }

    /// Symbols come back in line order, whatever the source. The caller steps
    /// through them with `n`, so an out-of-order entry is a jump that goes
    /// backwards.
    #[test]
    fn symbols_are_always_in_line_order(source in source_bytes()) {
        for (path, _, _) in TAGGED_PATHS {
            let lines: Vec<u32> = of(&source, path).iter().map(|s| s.line).collect();
            let mut sorted = lines.clone();
            sorted.sort_unstable();
            prop_assert_eq!(&lines, &sorted, "{} went backwards", path);
        }
    }

    /// The answer depends on the source and on the *grammar* the path
    /// selects, and on nothing else — not on the rest of the name, and not on
    /// what was parsed before. Two calls in a row agree, and so do two calls
    /// under different names of the same language with another file parsed in
    /// between.
    ///
    /// The second half is the one with teeth. The caller of this module is an
    /// index that caches by `(commit, path)`, and a cache keyed by the path
    /// alone — the easy mistake — is invisible to "the same call twice":
    /// it answers consistently, just with the previous file's symbols.
    #[test]
    fn symbols_depend_only_on_the_source_and_the_grammar(
        first in source_bytes(),
        second in source_bytes(),
        stem in "[a-z]{6}",
    ) {
        // Two names never seen before, so that a cache built up over earlier
        // cases cannot make this one pass by accident.
        let one = format!("{stem}_one.rs");
        let two = format!("{stem}_two.rs");

        let once = of(&first, &one);
        let twice = of(&first, &one);
        prop_assert_eq!(&once, &twice, "the same call twice disagreed");

        let _ = of(&second, &two);
        let elsewhere = of(&first, &two);
        prop_assert_eq!(&once, &elsewhere, "the file's name changed its symbols");
    }
}
