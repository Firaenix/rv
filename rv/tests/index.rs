//! Tests for the in-scope symbol index: what a reviewer can step to with `n`,
//! and in what order.
//!
//! [`rv::index`] is pure — no terminal, no store, no repository — so
//! everything below is a plain function call on blobs written out in the test.
//! Nothing here reads a file, which is the whole point: the model is handed
//! the bytes and the scope, so a jump list can be tested without a jj
//! workspace behind it.
//!
//! Four rules earn the module its existence, and each is pinned twice — once
//! as the example a reader can follow and once as a law over arbitrary input:
//!
//! * **Scope is the caller's and the model has no opinion.** Every changed
//!   file in the range, or one change's files; the index is told which, and
//!   the file numbers it is told are the ones it hands back.
//! * **Symbols come from the side the file will exist on** — the head blob,
//!   except for a removed file, whose symbols only exist at the base.
//! * **Stepping crosses file boundaries and does not wrap.** Silent wrapping
//!   makes a reviewer believe they have seen everything when they have
//!   looped, so past the last symbol is `None`.
//! * **Nothing invented, nothing lost.** The entries are exactly the union of
//!   the per-file symbol sets in scope — the conservation law that makes the
//!   index trustworthy enough to navigate by, and the reason a file with no
//!   grammar contributes nothing rather than failing the build of it.

use proptest::prelude::*;
use rv::app::anchored_side;
use rv::index::Entry;
use rv::index::Index;
use rv::index::Scoped;
use rv::index::indexed_side;
use rv_core::diff::LineKind;
use rv_core::model::ChangeKind;
use rv_core::model::Side;
use rv_core::symbols;
use rv_core::symbols::Symbol;
use rv_core::symbols::SymbolKind;

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

/// Two definitions, on their own lines, in the order a reviewer reads them.
const A_RS: &str = "fn alpha() {}\n\nstruct Beta;\n";

/// Two more, in a second file, so that stepping has a boundary to cross.
const B_RS: &str = "fn gamma() {}\n\nfn delta() {}\n";

/// A file no grammar claims. It has a shape a parser would find definitions
/// in, so a test that finds nothing here has found nothing for the right
/// reason.
const NOTES_TXT: &str = "fn not_code() {}\nstruct NotCode;\n";

/// A file whose head side and base side name different things, for the tests
/// about which side is indexed.
const HEAD_ONLY_RS: &str = "fn only_at_head() {}\n";
const BASE_ONLY_RS: &str = "fn only_at_base() {}\n";

/// A file that exists on both sides, indexed from the head one.
fn modified<'a>(file: usize, path: &'a str, base: &'a str, head: &'a str) -> Scoped<'a> {
    Scoped {
        file,
        path,
        kind: ChangeKind::Modified,
        change_id: None,
        base: Some(base.as_bytes()),
        head: Some(head.as_bytes()),
    }
}

/// A file the range adds: there is no base blob at all.
fn added<'a>(file: usize, path: &'a str, head: &'a str) -> Scoped<'a> {
    Scoped {
        file,
        path,
        kind: ChangeKind::Added,
        change_id: None,
        base: None,
        head: Some(head.as_bytes()),
    }
}

/// A file the range removes: there is no head blob, and its symbols are only
/// in the base one.
fn removed<'a>(file: usize, path: &'a str, base: &'a str) -> Scoped<'a> {
    Scoped {
        file,
        path,
        kind: ChangeKind::Removed,
        change_id: None,
        base: Some(base.as_bytes()),
        head: None,
    }
}

/// The bookmark view over the two Rust fixtures: file 0 is `a.rs`, file 1 is
/// `b.rs`, and both are indexed from their head blobs.
fn two_files<'a>() -> Vec<Scoped<'a>> {
    vec![added(0, "a.rs", A_RS), added(1, "b.rs", B_RS)]
}

/// Every symbol name the index holds, in index order.
fn names(index: &Index) -> Vec<&str> {
    index
        .entries()
        .iter()
        .map(|entry| entry.symbol.name.as_str())
        .collect()
}

/// The one entry named `name`, or a failure that says which name was missing.
fn entry_named<'a>(index: &'a Index, name: &str) -> &'a Entry {
    index
        .entries()
        .iter()
        .find(|entry| entry.symbol.name == name)
        .unwrap_or_else(|| panic!("{name} is indexed, got {:?}", names(index)))
}

/// Where the cursor is when it sits on `entry`: the pair `next_after` and
/// `previous_before` take.
fn cursor(entry: &Entry) -> (usize, u32) {
    (entry.file, entry.symbol.line)
}

// ---------------------------------------------------------------------------
// Scope
// ---------------------------------------------------------------------------

#[test]
fn every_file_in_scope_contributes_its_symbols() {
    let index = Index::of(&two_files());
    assert_eq!(names(&index), ["alpha", "Beta", "gamma", "delta"]);
    assert_eq!(entry_named(&index, "alpha").path, "a.rs");
    assert_eq!(entry_named(&index, "delta").path, "b.rs");
}

#[test]
fn the_entries_are_in_file_order_and_then_line_order() {
    let index = Index::of(&two_files());
    let order: Vec<(usize, u32)> = index.entries().iter().map(cursor).collect();
    let mut sorted = order.clone();
    sorted.sort_unstable();
    assert_eq!(order, sorted, "the walk order is the reading order");
}

#[test]
fn the_scope_is_the_callers_and_the_model_has_no_opinion() {
    // One change's files out of a larger review: only what was passed in is
    // indexed, and the file numbers that come back are the ones that went in
    // rather than positions in the scope.
    let scope = vec![added(7, "b.rs", B_RS)];
    let index = Index::of(&scope);
    assert_eq!(names(&index), ["gamma", "delta"]);
    assert!(
        index.entries().iter().all(|entry| entry.file == 7),
        "a file keeps the number the caller addressed it by"
    );
}

#[test]
fn a_change_rides_along_with_every_symbol_it_owns() {
    let scope = vec![
        Scoped {
            change_id: Some("ytskpxpw"),
            ..added(0, "a.rs", A_RS)
        },
        Scoped {
            change_id: Some("zmomvwzm"),
            ..added(1, "b.rs", B_RS)
        },
    ];
    let index = Index::of(&scope);
    assert_eq!(
        entry_named(&index, "alpha").change_id.as_deref(),
        Some("ytskpxpw")
    );
    assert_eq!(
        entry_named(&index, "gamma").change_id.as_deref(),
        Some("zmomvwzm"),
        "the picker can say which change a symbol came from"
    );
}

#[test]
fn an_empty_scope_has_nothing_to_step_through() {
    let index = Index::of(&[]);
    assert!(index.is_empty());
    assert_eq!(index.len(), 0);
    assert!(index.entries().is_empty());
    assert!(index.next_after(0, 0).is_none());
    assert!(index.previous_before(0, 999).is_none());
}

// ---------------------------------------------------------------------------
// Which side a file's symbols come from
// ---------------------------------------------------------------------------

#[test]
fn a_removed_file_is_indexed_from_the_base_side() {
    // You navigate code as it will exist, except where it will not exist at
    // all: the one file whose symbols are only on the side being left behind.
    let scope = vec![removed(0, "gone.rs", BASE_ONLY_RS)];
    let index = Index::of(&scope);
    assert_eq!(
        names(&index),
        ["only_at_base"],
        "its symbols come from the side that still has them"
    );
}

#[test]
fn a_removed_file_ignores_whatever_is_at_the_head_path() {
    // The side is decided by how the file changed, never by which blob the
    // caller happened to have. `App` reads both sides for every file it opens,
    // so a model that took "whichever one is there" would look right on every
    // fixture and be wrong the first time a path was removed and something
    // else stood at it.
    let scope = vec![Scoped {
        head: Some(HEAD_ONLY_RS.as_bytes()),
        ..removed(0, "gone.rs", BASE_ONLY_RS)
    }];
    assert_eq!(names(&Index::of(&scope)), ["only_at_base"]);
}

#[test]
fn a_modified_file_is_indexed_from_the_head_side() {
    // Both blobs are present and both parse, so this fails for a model that
    // takes whichever side it finds first rather than the side it was told.
    let scope = vec![modified(0, "a.rs", BASE_ONLY_RS, HEAD_ONLY_RS)];
    let index = Index::of(&scope);
    assert_eq!(names(&index), ["only_at_head"]);
}

#[test]
fn a_renamed_file_is_indexed_from_the_head_side_too() {
    let scope = vec![Scoped {
        kind: ChangeKind::Renamed,
        ..modified(0, "b.rs", BASE_ONLY_RS, HEAD_ONLY_RS)
    }];
    assert_eq!(names(&Index::of(&scope)), ["only_at_head"]);
}

#[test]
fn the_side_a_file_is_indexed_on_is_the_projects_one_side_rule() {
    // A removed file is nothing but removed lines, and everything else has a
    // head side — so this is `anchored_side` asked about a whole file, not a
    // second copy of it. If that rule ever moves, this moves with it.
    assert_eq!(
        indexed_side(ChangeKind::Removed),
        anchored_side(LineKind::Removed)
    );
    for kind in [ChangeKind::Added, ChangeKind::Modified, ChangeKind::Renamed] {
        assert_eq!(indexed_side(kind), anchored_side(LineKind::Added));
    }
    assert_eq!(indexed_side(ChangeKind::Removed), Side::Left);
    assert_eq!(indexed_side(ChangeKind::Added), Side::Right);
}

// ---------------------------------------------------------------------------
// Stepping
// ---------------------------------------------------------------------------

#[test]
fn stepping_crosses_a_file_boundary() {
    let index = Index::of(&two_files());
    let last = entry_named(&index, "Beta");
    let (file, line) = cursor(last);
    let next = index.next_after(file, line).expect("a next symbol");
    assert_ne!(
        next.file, last.file,
        "n at the last symbol of a file moves to the next file"
    );
    assert_eq!(next.symbol.name, "gamma", "and to that file's first symbol");
}

#[test]
fn stepping_does_not_wrap_around_at_either_end() {
    let index = Index::of(&two_files());
    let last = entry_named(&index, "delta");
    let (file, line) = cursor(last);
    assert!(
        index.next_after(file, line).is_none(),
        "past the last symbol is nothing, never the first one again"
    );

    let first = entry_named(&index, "alpha");
    let (file, line) = cursor(first);
    assert!(index.previous_before(file, line).is_none());
}

#[test]
fn stepping_from_between_two_symbols_lands_on_the_one_after() {
    // The cursor is a line in a file, not a symbol: most of the time it is
    // sitting on neither.
    let source = "fn one() {}\n\n\n\n\nfn two() {}\n";
    let index = Index::of(&[added(0, "a.rs", source)]);
    let one = entry_named(&index, "one");
    let two = entry_named(&index, "two");
    assert!(
        one.symbol.line < 3 && two.symbol.line > 3,
        "line 3 is between"
    );
    assert_eq!(index.next_after(0, 3).expect("forward").symbol.name, "two");
    assert_eq!(
        index.previous_before(0, 3).expect("back").symbol.name,
        "one"
    );
}

#[test]
fn previous_before_undoes_next_after() {
    let index = Index::of(&two_files());
    for entry in index.entries() {
        let (file, line) = cursor(entry);
        let Some(next) = index.next_after(file, line) else {
            continue;
        };
        let (file, line) = cursor(next);
        assert_eq!(
            index.previous_before(file, line).map(cursor),
            Some((entry.file, entry.symbol.line)),
            "stepping forward and back returns to {}",
            entry.symbol.name
        );
    }
}

#[test]
fn a_file_outside_the_scope_has_no_place_in_the_order() {
    // Not an error and not the first entry either: a file the caller did not
    // put in scope has no position in a walk over the scope, and guessing one
    // would drop the reviewer somewhere they never asked to be.
    let index = Index::of(&two_files());
    assert!(index.next_after(9, 0).is_none());
    assert!(index.previous_before(9, 999).is_none());
}

#[test]
fn two_definitions_on_one_line_are_one_stop_and_both_are_in_the_index() {
    // The cursor is a `(file, line)` pair, so two definitions sharing a line
    // are one place to stand: `n` moves the cursor or it does nothing, and a
    // jump that lands where it started reads as a broken key. Both are still
    // *in* the index — the picker lists them, and conservation is what says
    // so.
    let index = Index::of(&[added(0, "a.rs", "struct One; struct Two;\nfn after() {}\n")]);
    assert_eq!(names(&index), ["One", "Two", "after"]);
    let one = entry_named(&index, "One");
    let two = entry_named(&index, "Two");
    assert_eq!(
        one.symbol.line, two.symbol.line,
        "the fixture shares a line"
    );
    assert_eq!(
        index
            .next_after(0, one.symbol.line)
            .map(|e| e.symbol.name.as_str()),
        Some("after"),
        "one line is one stop"
    );
}

// ---------------------------------------------------------------------------
// Files that contribute nothing
// ---------------------------------------------------------------------------

#[test]
fn a_file_with_no_grammar_contributes_nothing_and_is_skipped() {
    let scope = vec![
        added(0, "a.rs", A_RS),
        added(1, "notes.txt", NOTES_TXT),
        added(2, "b.rs", B_RS),
    ];
    let index = Index::of(&scope);
    assert!(
        index
            .entries()
            .iter()
            .all(|entry| entry.path.ends_with(".rs")),
        "got {:?}",
        index.entries().iter().map(|e| &e.path).collect::<Vec<_>>()
    );
    let beta = entry_named(&index, "Beta");
    assert_eq!(
        index
            .next_after(beta.file, beta.symbol.line)
            .map(|e| e.file),
        Some(2),
        "the walk steps over it rather than stopping at it"
    );
}

#[test]
fn a_file_with_no_grammar_is_still_a_place_to_step_from() {
    // It contributes no entries, but it is in scope, so it has a position in
    // the order — a reviewer reading `notes.txt` presses `n` and arrives at
    // the next file's first symbol.
    let scope = vec![
        added(0, "a.rs", A_RS),
        added(1, "notes.txt", NOTES_TXT),
        added(2, "b.rs", B_RS),
    ];
    let index = Index::of(&scope);
    assert_eq!(
        index.next_after(1, 0).map(|e| e.symbol.name.as_str()),
        Some("gamma")
    );
    assert_eq!(
        index
            .previous_before(1, 999)
            .map(|e| e.symbol.name.as_str()),
        Some("Beta")
    );
}

#[test]
fn a_grammar_with_no_tags_query_contributes_nothing() {
    // TOML is highlighted and has no symbols. Pinned here so the next reader
    // does not go looking for the bug.
    let index = Index::of(&[added(0, "Cargo.toml", "[package]\nname = \"rv\"\n")]);
    assert!(index.entries().is_empty());
}

#[test]
fn a_file_with_no_blob_on_its_side_contributes_nothing() {
    // A binary file, an unreadable one, a symlink: the caller has no bytes to
    // give, and that is a file with no symbols rather than a failure.
    let scope = vec![
        Scoped {
            base: None,
            head: None,
            ..added(0, "a.rs", A_RS)
        },
        added(1, "b.rs", B_RS),
    ];
    assert_eq!(names(&Index::of(&scope)), ["gamma", "delta"]);
}

#[test]
fn a_file_with_no_blob_is_still_a_place_to_step_from() {
    // The same rule as for a file with no grammar, and it has to be checked
    // separately: this one is skipped a line earlier, before its path is ever
    // looked at. A reviewer sitting on a binary file presses `n` and arrives
    // at the next file's first symbol rather than at nothing.
    let scope = vec![
        added(0, "a.rs", A_RS),
        Scoped {
            base: None,
            head: None,
            ..added(1, "logo.png", "")
        },
        added(2, "b.rs", B_RS),
    ];
    let index = Index::of(&scope);
    assert_eq!(
        index.next_after(1, 0).map(|e| e.symbol.name.as_str()),
        Some("gamma")
    );
    assert_eq!(
        index
            .previous_before(1, 999)
            .map(|e| e.symbol.name.as_str()),
        Some("Beta")
    );
}

#[test]
fn bytes_that_are_not_utf8_are_not_a_panic() {
    let blob = [0xff, 0xfe, b'\n'];
    let scope = vec![Scoped {
        file: 0,
        path: "weird.rs",
        kind: ChangeKind::Added,
        change_id: None,
        base: None,
        head: Some(&blob),
    }];
    let _ = Index::of(&scope);
}

#[test]
fn every_grammar_with_a_tags_query_is_indexed() {
    // Six of the eleven grammars rv ships have a usable `tags.scm`; the index
    // asks `rv_core::symbols` and inherits exactly that set.
    let sources = [
        ("a.rs", "fn rust_one() {}\n"),
        ("b.py", "def python_one():\n    pass\n"),
        ("c.go", "package main\n\nfunc GoOne() {}\n"),
        ("d.js", "function jsOne() {}\n"),
        ("e.ts", "export function tsOne(): void {}\n"),
        ("f.tsx", "export function tsxOne() { return null; }\n"),
    ];
    let scope: Vec<Scoped<'_>> = sources
        .iter()
        .enumerate()
        .map(|(index, (path, source))| added(index, path, source))
        .collect();
    let index = Index::of(&scope);
    for (path, _) in sources {
        assert!(
            index.entries().iter().any(|entry| entry.path == path),
            "{path} has symbols, got {:?}",
            names(&index)
        );
    }
}

#[test]
fn a_symbol_keeps_the_kind_the_grammar_gave_it() {
    let index = Index::of(&two_files());
    assert_eq!(
        entry_named(&index, "alpha").symbol.kind,
        SymbolKind::Function
    );
    assert_eq!(entry_named(&index, "Beta").symbol.kind, SymbolKind::Struct);
}

// ---------------------------------------------------------------------------
// Properties
// ---------------------------------------------------------------------------

/// One generated file: which side it lives on, what language it is written in,
/// the definitions on the side that gets indexed, and the definitions on the
/// side that does not.
#[derive(Clone, Debug)]
struct Spec {
    kind: ChangeKind,
    language: Language,
    live: Vec<String>,
    dead: Vec<String>,
}

#[derive(Clone, Copy, Debug)]
enum Language {
    Rust,
    Python,
    Go,
    None,
}

impl Language {
    fn path(self, index: usize) -> String {
        match self {
            Language::Rust => format!("src/f{index}.rs"),
            Language::Python => format!("src/f{index}.py"),
            Language::Go => format!("src/f{index}.go"),
            Language::None => format!("docs/f{index}.txt"),
        }
    }

    /// A file defining each of `names`, one definition per line so that no two
    /// symbols share a line — which is what makes a `(file, line)` cursor able
    /// to address every one of them.
    fn source(self, names: &[String]) -> String {
        let mut source = match self {
            Language::Go => "package main\n".to_owned(),
            _ => String::new(),
        };
        for name in names {
            match self {
                Language::Rust => source.push_str(&format!("fn {name}() {{}}\n")),
                Language::Python => source.push_str(&format!("def {name}():\n    pass\n")),
                Language::Go => source.push_str(&format!("func {name}() {{}}\n")),
                Language::None => source.push_str(&format!("{name}\n")),
            }
        }
        source
    }
}

/// Names that can never collide with a keyword in any of the three languages,
/// and can repeat — two definitions with the same name are two places a
/// reviewer could mean, and the index keeps both.
fn a_name() -> impl Strategy<Value = String> {
    "s[a-z0-9_]{0,5}"
}

fn a_spec() -> impl Strategy<Value = Spec> {
    (
        prop_oneof![
            Just(ChangeKind::Added),
            Just(ChangeKind::Modified),
            Just(ChangeKind::Renamed),
            Just(ChangeKind::Removed),
        ],
        prop_oneof![
            Just(Language::Rust),
            Just(Language::Python),
            Just(Language::Go),
            Just(Language::None),
        ],
        prop::collection::vec(a_name(), 0..4),
        prop::collection::vec(a_name(), 0..3),
    )
        .prop_map(|(kind, language, live, dead)| Spec {
            kind,
            language,
            live,
            dead,
        })
}

/// The blobs each spec's file has, kept alive for the borrowed [`Scoped`]s
/// below: `(path, base, head)`.
fn blobs(specs: &[Spec]) -> Vec<(String, String, String)> {
    specs
        .iter()
        .enumerate()
        .map(|(index, spec)| {
            let live = spec.language.source(&spec.live);
            let dead = spec.language.source(&spec.dead);
            let (base, head) = match spec.kind {
                ChangeKind::Removed => (live, dead),
                _ => (dead, live),
            };
            (spec.language.path(index), base, head)
        })
        .collect()
}

/// The scope those blobs describe. `file` is the spec's position plus a fixed
/// offset, so a model that confused a file number with a position in the scope
/// would be caught.
fn scope_of<'a>(specs: &[Spec], blobs: &'a [(String, String, String)]) -> Vec<Scoped<'a>> {
    specs
        .iter()
        .zip(blobs)
        .enumerate()
        .map(|(index, (spec, (path, base, head)))| Scoped {
            file: index + 3,
            path,
            kind: spec.kind,
            change_id: None,
            base: Some(base.as_bytes()),
            head: Some(head.as_bytes()),
        })
        .collect()
}

/// What the index must hold, restated independently of it: for each file in
/// scope, every symbol `rv_core::symbols` finds in the blob on that file's
/// side.
fn union_of_the_symbol_sets(scope: &[Scoped<'_>]) -> Vec<(usize, Symbol)> {
    scope
        .iter()
        .flat_map(|scoped| {
            let blob = match scoped.kind {
                ChangeKind::Removed => scoped.base,
                _ => scoped.head,
            };
            let symbols = blob
                .map(|blob| symbols::of(blob, scoped.path))
                .unwrap_or_default();
            symbols.into_iter().map(|symbol| (scoped.file, symbol))
        })
        .collect()
}

proptest! {
    /// The conservation law: nothing invented, nothing lost.
    #[test]
    fn the_entries_are_exactly_the_union_of_the_symbol_sets_in_scope(
        specs in prop::collection::vec(a_spec(), 0..5),
    ) {
        let blobs = blobs(&specs);
        let scope = scope_of(&specs, &blobs);
        let index = Index::of(&scope);

        let expected = union_of_the_symbol_sets(&scope);
        let actual: Vec<(usize, Symbol)> = index
            .entries()
            .iter()
            .map(|entry| (entry.file, entry.symbol.clone()))
            .collect();

        // As a bag first, so a failure says *what* was lost or invented before
        // it says where it was in the order.
        let mut wanted: Vec<(usize, Symbol)> = expected.clone();
        let mut got = actual.clone();
        let key = |(file, symbol): &(usize, Symbol)| {
            (*file, symbol.line, symbol.name.clone(), symbol.kind)
        };
        wanted.sort_by_key(key);
        got.sort_by_key(key);
        prop_assert_eq!(&got, &wanted);
        prop_assert_eq!(&actual, &expected, "and in the caller's order");

        // Every entry says which file it came from, in the caller's numbering.
        for entry in index.entries() {
            prop_assert!(
                scope.iter().any(|scoped| scoped.file == entry.file && entry.path == scoped.path),
                "entry {:?} names a file that is not in scope", entry
            );
        }
    }

    /// The round trip: `n` from the first entry visits every entry exactly
    /// once, in order, and stops.
    #[test]
    fn stepping_visits_every_entry_exactly_once_in_order(
        specs in prop::collection::vec(a_spec(), 0..5),
    ) {
        let blobs = blobs(&specs);
        let scope = scope_of(&specs, &blobs);
        let index = Index::of(&scope);
        let Some(first) = index.entries().first() else {
            return Ok(());
        };

        let mut walk = vec![cursor(first)];
        let mut here = cursor(first);
        for _ in 1..index.len() {
            let next = index.next_after(here.0, here.1);
            prop_assert!(next.is_some(), "the walk stopped at {:?} of {}", here, index.len());
            here = cursor(next.expect("checked"));
            walk.push(here);
        }
        prop_assert!(
            index.next_after(here.0, here.1).is_none(),
            "the walk wrapped around instead of ending"
        );

        let entries: Vec<(usize, u32)> = index.entries().iter().map(cursor).collect();
        prop_assert_eq!(walk, entries);
    }

    /// And backwards, for the same reason.
    #[test]
    fn stepping_back_visits_every_entry_exactly_once_in_reverse(
        specs in prop::collection::vec(a_spec(), 0..5),
    ) {
        let blobs = blobs(&specs);
        let scope = scope_of(&specs, &blobs);
        let index = Index::of(&scope);
        let Some(last) = index.entries().last() else {
            return Ok(());
        };

        let mut walk = vec![cursor(last)];
        let mut here = cursor(last);
        for _ in 1..index.len() {
            let previous = index.previous_before(here.0, here.1);
            prop_assert!(previous.is_some(), "the walk stopped at {:?}", here);
            here = cursor(previous.expect("checked"));
            walk.push(here);
        }
        prop_assert!(index.previous_before(here.0, here.1).is_none());

        walk.reverse();
        let entries: Vec<(usize, u32)> = index.entries().iter().map(cursor).collect();
        prop_assert_eq!(walk, entries);
    }
}

/// The rule this module's headline claims, in the one shape that can catch a
/// model getting it wrong: the walk follows the **caller's scope order**, not
/// the file numbers.
///
/// Every other test hands over a scope whose numbers ascend with position, so a
/// rank derived from `Scoped::file` produces exactly the right answer and a
/// mutant that does so survives. It was measured surviving all twenty-seven of
/// them. This scope numbers its files *descending*, which is what a churn-sorted
/// review looks like — the biggest file first, whatever its position in
/// `App::files()` — and there the two orders disagree.
///
/// The consequence of getting it wrong is not a cosmetic reordering: `n` reaches
/// a file, finds no rank for the number it holds, and stops halfway through a
/// review that still has symbols in it.
#[test]
fn the_walk_follows_the_scope_order_not_the_file_numbers() {
    let first = "fn early() {}\n";
    let second = "fn later() {}\n";
    // File 9 comes first in scope and file 2 second: the caller's order is the
    // walk's order, and it is not the numeric one.
    let scope = vec![
        added(9, "first.rs", first),
        added(2, "second.rs", second),
    ];
    let index = Index::of(&scope);

    assert_eq!(
        names(&index),
        vec!["early", "later"],
        "the entries came back in file-number order rather than scope order"
    );

    // And the cursor walks them the same way: from the first file in scope to
    // the second, never stalling because 2 < 9.
    let early = entry_named(&index, "early");
    let (file, line) = cursor(early);
    let next = index
        .next_after(file, line)
        .expect("a symbol after the first one");
    assert_eq!(
        next.symbol.name, "later",
        "stepping from the first file in scope did not reach the second"
    );

    // Backwards too, which is where a rank taken from the file number strands
    // the reviewer: from file 2 there is a previous entry only if 9 ranks first.
    let later = entry_named(&index, "later");
    let (file, line) = cursor(later);
    let previous = index
        .previous_before(file, line)
        .expect("a symbol before the second one");
    assert_eq!(previous.symbol.name, "early");
}
