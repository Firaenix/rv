//! The named definitions in a file, as plain data (navigation spec §4).
//! [`of`] runs a tree-sitter *tags* query over a file's bytes and returns the
//! [`Symbol`]s a reviewer can jump to: a name, a [`SymbolKind`], and the
//! 1-based line the name sits on.
//!
//! No terminal, no ratatui, no rect. `rv` decides how a symbol is drawn and
//! how a jump is animated; this module only says what is in the file.
//!
//! **One grammar serves both; each runs its own parse.** The navigation spec
//! says a single parse serves both highlighting and symbols. It does not, and
//! saying so is better than leaving the next reader hunting for a shared parse
//! that was never there. `tree-sitter-highlight` and `tree-sitter-tags` each
//! own a `Parser` and each parse internally; there is no public seam that
//! takes a `Tree` and produces either. Unifying them would mean
//! reimplementing the highlighter's injection and locals handling on top of a
//! tree rv parsed itself, to save a few milliseconds on files the size a code
//! review deals in. What *is* shared is the thing that matters: the grammar
//! and the detection table. A file is Rust here for exactly the reason it is
//! Rust in [`highlight`](crate::highlight) — [`highlight::language_of`] is the
//! one function that decides — so colour and navigation can never disagree
//! about what a file is.
//!
//! Three rules shape the rest of this module.
//!
//! **Definitions only; a reference is not a symbol.** A `tags.scm` reports
//! both, because its original job is a code-search index that answers "who
//! calls this". A reviewer stepping through a diff wants the other half: every
//! call site in the list would bury the definitions under them.
//!
//! **Nothing here fails.** A blob that is not UTF-8, a half-typed function
//! that does not parse, a language with no tags query, a path that is not a
//! path — each gives back a `Vec`, possibly empty. A reviewer opening a file
//! must never be the thing that kills the process. Six of the eleven grammars
//! rv ships have a usable `tags.scm` (Rust, Go, Python, JavaScript,
//! TypeScript and TSX); TOML, JSON, YAML, markdown and bash have none, so
//! those files are coloured and have no symbols — a fact `tests/symbols.rs`
//! pins rather than leaving a user to discover twice.
//!
//! **Every definition is reported, including two with the same name.** A
//! method and a free function both called `write` are two different places a
//! reviewer could mean, and dropping either is the kind of silent loss that
//! makes a jump list untrustworthy.

use std::sync::OnceLock;

use tree_sitter_tags::TagsConfiguration;
use tree_sitter_tags::TagsContext;

use crate::highlight;

/// What a definition is, as far as a reviewer choosing where to jump needs to
/// care. Deliberately small and terminal-free: `rv` decides what glyph or
/// colour each becomes.
///
/// The vocabulary is Rust's, because that is the language rv is reviewed in,
/// and every other grammar maps onto it. The mapping is stated in
/// [`kind_of`], and the two lossy spots are worth knowing when reading the
/// picker: a Python or JavaScript **class** is a [`Struct`](SymbolKind::Struct)
/// — the nearest thing to "a named aggregate this language defines" — and a Go
/// **type** is a [`Type`](SymbolKind::Type) whether it is a struct, an
/// interface or an alias, because Go's own `tags.scm` does not distinguish
/// them and rv follows each grammar rather than second-guessing it.
///
/// [`Other`](SymbolKind::Other) means "the grammar called this a definition,
/// but not one of the kinds rv names" — a new syntax type appearing in an
/// upgraded grammar lands here rather than being dropped.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum SymbolKind {
    Function,
    Struct,
    Enum,
    Trait,
    Impl,
    Module,
    Constant,
    Type,
    Macro,
    Other,
}

impl SymbolKind {
    /// A short lowercase word for this kind, for a picker row or a status
    /// line. Rust's own keyword wherever Rust has one, so `fn write` in a
    /// status bar reads as the line it came from.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            SymbolKind::Function => "fn",
            SymbolKind::Struct => "struct",
            SymbolKind::Enum => "enum",
            SymbolKind::Trait => "trait",
            SymbolKind::Impl => "impl",
            SymbolKind::Module => "mod",
            SymbolKind::Constant => "const",
            SymbolKind::Type => "type",
            SymbolKind::Macro => "macro",
            SymbolKind::Other => "item",
        }
    }
}

/// One definition: its name, what kind of thing it is, and the **1-based**
/// line its *name* appears on — not the line the item starts on, which for an
/// attributed or documented item is several lines earlier. Jumping to the name
/// puts the cursor on the word the reviewer searched for.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct Symbol {
    pub name: String,
    pub kind: SymbolKind,
    pub line: u32,
}

/// Every definition in `source`, in line order, using whichever grammar
/// `path`'s name selects.
///
/// Never fails. A path no grammar claims, a language with no tags query, a
/// query that does not compile, bytes that are not UTF-8 and source that does
/// not parse all give back a `Vec` — empty in the first three cases, and in
/// the last two whatever tree-sitter recovered, which for a work-in-progress
/// branch is most of the file.
///
/// The answer depends on `(source, path)` and nothing else: no cache, no
/// parser reused between calls, nothing carried from one file to the next. The
/// caller is expected to cache by `(commit, path)`, and it can only do that
/// safely if this is a function.
#[must_use]
pub fn of(source: &[u8], path: &str) -> Vec<Symbol> {
    let Some(language) = highlight::language_of(path) else {
        return Vec::new();
    };
    let Some(config) = configuration(language) else {
        return Vec::new();
    };

    let mut context = TagsContext::new();
    let Ok((tags, _)) = context.generate_tags(config, source, None) else {
        return Vec::new();
    };

    // Sorted by where the *name* starts, which is line order and, within a
    // line, column order. tree-sitter-tags emits tags through a small
    // reordering queue, so the iterator's own order is close to this but not
    // guaranteed to be it, and a caller stepping with `n` needs the guarantee.
    let mut found: Vec<(usize, usize, Symbol)> = Vec::new();
    for tag in tags {
        // A mid-stream error means the parse was cancelled or a query
        // predicate blew up; keep what has been collected rather than
        // throwing away a nearly complete file's symbols.
        let Ok(tag) = tag else { break };
        if !tag.is_definition {
            continue;
        }
        let Some(bytes) = source.get(tag.name_range.clone()) else {
            continue;
        };
        if bytes.is_empty() {
            continue;
        }
        // A name is bytes from a blob that was never validated, so it is
        // decoded lossily rather than skipped: a file in some other encoding
        // should still be navigable, with a replacement character in the one
        // name that needs it.
        let name = String::from_utf8_lossy(bytes).into_owned();
        // `span` is the name node's position; `row` is 0-based, lines are
        // 1-based everywhere in this codebase.
        let Ok(line) = u32::try_from(tag.span.start.row + 1) else {
            continue;
        };
        let kind = kind_of(config.syntax_type_name(tag.syntax_type_id));
        found.push((
            tag.name_range.start,
            tag.name_range.end,
            Symbol { name, kind, line },
        ));
    }

    found.sort_by_key(|(start, end, _)| (*start, *end));
    found.into_iter().map(|(_, _, symbol)| symbol).collect()
}

/// The [`SymbolKind`] for a tags syntax type — the word after `definition.` in
/// a `tags.scm` capture.
///
/// The vocabulary is not rv's to choose: each grammar's query names its own
/// types, and this is the one place they are collapsed onto rv's. Unknown
/// words become [`SymbolKind::Other`] so that a grammar upgrade that
/// introduces a syntax type shows up as an unlabelled symbol rather than as a
/// symbol that vanished.
fn kind_of(syntax_type: &str) -> SymbolKind {
    match syntax_type {
        // A method is a function for navigation purposes: a reviewer looking
        // for `write` does not know or care which one they will land on.
        "function" | "method" => SymbolKind::Function,
        // `class` is what Python and JavaScript call theirs; `struct` is
        // rv's own refinement of the Rust query (see [`RUST_KINDS_QUERY`]).
        "struct" | "class" => SymbolKind::Struct,
        "enum" => SymbolKind::Enum,
        // A Rust trait and a TypeScript interface are the same idea and the
        // Rust query already calls a trait an `interface`.
        "interface" => SymbolKind::Trait,
        "impl" => SymbolKind::Impl,
        "module" => SymbolKind::Module,
        "constant" => SymbolKind::Constant,
        "type" => SymbolKind::Type,
        "macro" => SymbolKind::Macro,
        _ => SymbolKind::Other,
    }
}

// ---------------------------------------------------------------------------
// Queries
// ---------------------------------------------------------------------------

/// The tags configuration for a language name from
/// [`highlight::language_of`], or `None` for a language that has no usable
/// `tags.scm` — TOML, JSON, YAML, markdown and bash, which are highlighted and
/// have no symbols.
///
/// Matched on the language name rather than re-deriving it from the path, so
/// there is still exactly one detection table. A grammar added to
/// `highlight.rs` and not added here simply has no symbols, which is the
/// correct default: most grammar crates ship no tags query at all.
fn configuration(language: &str) -> Option<&'static TagsConfiguration> {
    match language {
        "rust" => rust_tags(),
        "go" => go_tags(),
        "python" => python_tags(),
        "javascript" => javascript_tags(),
        "typescript" => typescript_tags(),
        "tsx" => tsx_tags(),
        _ => None,
    }
}

/// rv's refinement of the Rust `tags.scm`, prepended to it.
///
/// The shipped query captures a struct, an enum, a union *and* a type alias
/// all as `@definition.class`, and an `impl` block as
/// `@reference.implementation` — sensible for a code-search index, useless for
/// a reviewer who wants to know whether the thing they are about to jump to is
/// an enum. These patterns give each its own syntax type.
///
/// Prepending is load-bearing. `tree-sitter-tags` keeps exactly one tag per
/// name node and, when two patterns claim the same one, keeps the tag from the
/// **earlier** pattern — so these win over the shipped query's `class` and
/// `reference.implementation` for the same node, and every item the shipped
/// query finds and these do not is still found by it. Nothing is added to the
/// set of symbols except the `impl` blocks, which are promoted from reference
/// to definition deliberately: "where is the impl of `Anchor`" is a jump a
/// reviewer makes constantly, and it is a different place from `struct
/// Anchor`.
///
/// The two `impl_item` patterns are the plain and the generic form —
/// `impl Anchor` and `impl<T> Anchor<T>` — named after the *type*, not the
/// trait, in both `impl Anchor` and `impl Display for Anchor`, because the
/// type is what a reviewer looks for. The trait name is still reported by the
/// shipped query, as a reference, and dropped.
const RUST_KINDS_QUERY: &str = r"
(struct_item name: (type_identifier) @name) @definition.struct
(union_item name: (type_identifier) @name) @definition.struct
(enum_item name: (type_identifier) @name) @definition.enum
(type_item name: (type_identifier) @name) @definition.type
(impl_item type: (type_identifier) @name) @definition.impl
(impl_item type: (generic_type type: (type_identifier) @name)) @definition.impl
";

/// Rust: the shipped query with [`RUST_KINDS_QUERY`] in front of it, falling
/// back to the shipped query alone if the refinement does not compile.
///
/// The fallback is the version-skew case, and it is why the refinement is
/// worth having rather than dangerous: a grammar upgrade that renames
/// `union_item` costs Rust its `enum`-versus-`struct` distinction for one
/// release, not its symbols. `every_rust_item_kind_gets_its_own_symbol_kind`
/// is what notices that the fallback was taken.
fn rust_tags() -> Option<&'static TagsConfiguration> {
    static CONFIG: OnceLock<Option<TagsConfiguration>> = OnceLock::new();
    CONFIG
        .get_or_init(|| {
            let refined = format!("{RUST_KINDS_QUERY}{}", tree_sitter_rust::TAGS_QUERY);
            TagsConfiguration::new(tree_sitter_rust::LANGUAGE.into(), &refined, "")
                .or_else(|_| {
                    TagsConfiguration::new(
                        tree_sitter_rust::LANGUAGE.into(),
                        tree_sitter_rust::TAGS_QUERY,
                        "",
                    )
                })
                .ok()
        })
        .as_ref()
}

fn go_tags() -> Option<&'static TagsConfiguration> {
    static CONFIG: OnceLock<Option<TagsConfiguration>> = OnceLock::new();
    CONFIG
        .get_or_init(|| {
            TagsConfiguration::new(
                tree_sitter_go::LANGUAGE.into(),
                tree_sitter_go::TAGS_QUERY,
                "",
            )
            .ok()
        })
        .as_ref()
}

fn python_tags() -> Option<&'static TagsConfiguration> {
    static CONFIG: OnceLock<Option<TagsConfiguration>> = OnceLock::new();
    CONFIG
        .get_or_init(|| {
            TagsConfiguration::new(
                tree_sitter_python::LANGUAGE.into(),
                tree_sitter_python::TAGS_QUERY,
                "",
            )
            .ok()
        })
        .as_ref()
}

fn javascript_tags() -> Option<&'static TagsConfiguration> {
    static CONFIG: OnceLock<Option<TagsConfiguration>> = OnceLock::new();
    CONFIG
        .get_or_init(|| {
            TagsConfiguration::new(
                tree_sitter_javascript::LANGUAGE.into(),
                tree_sitter_javascript::TAGS_QUERY,
                "",
            )
            .ok()
        })
        .as_ref()
}

/// The tags query both TypeScript parsers use: JavaScript's, then
/// TypeScript's own — the same two halves, for the same reason, as
/// `highlight::typescript_query`.
///
/// `tree-sitter-typescript`'s `TAGS_QUERY` is the TypeScript-specific half
/// only: `interface`, `module`, and the *signature* forms that appear inside
/// them. On its own it finds no plain `function`, no `class` and no method
/// body — a TypeScript file would come back with its interfaces and nothing
/// else. The TypeScript grammar is a superset of JavaScript's, so JavaScript's
/// query is valid against it and supplies the other half.
///
/// The order is not load-bearing: the two halves match disjoint node types
/// (`method_signature` and `method_definition`, `interface_declaration` and
/// `class_declaration`), so no node is claimed by both and the earlier-pattern
/// tie-break never fires. Written this way round because it is the order the
/// halves are named in.
fn typescript_tags_query() -> &'static str {
    static QUERY: OnceLock<String> = OnceLock::new();
    QUERY.get_or_init(|| {
        format!(
            "{}\n{}",
            tree_sitter_javascript::TAGS_QUERY,
            tree_sitter_typescript::TAGS_QUERY
        )
    })
}

fn typescript_tags() -> Option<&'static TagsConfiguration> {
    static CONFIG: OnceLock<Option<TagsConfiguration>> = OnceLock::new();
    CONFIG
        .get_or_init(|| {
            TagsConfiguration::new(
                tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
                typescript_tags_query(),
                "",
            )
            .ok()
        })
        .as_ref()
}

fn tsx_tags() -> Option<&'static TagsConfiguration> {
    static CONFIG: OnceLock<Option<TagsConfiguration>> = OnceLock::new();
    CONFIG
        .get_or_init(|| {
            TagsConfiguration::new(
                tree_sitter_typescript::LANGUAGE_TSX.into(),
                typescript_tags_query(),
                "",
            )
            .ok()
        })
        .as_ref()
}
