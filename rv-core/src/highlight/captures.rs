//! The kinds of token rv distinguishes, and the query names that map to them.
//!
//! Deliberately small. Every grammar names its captures differently — one
//! calls a type `type` and another `constructor` — and this is the one place
//! those names are reduced to a vocabulary a renderer can hold in its head.

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
pub(super) const CAPTURES: &[(&str, Capture)] = &[
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
pub(super) fn capture_at(index: usize) -> Capture {
    CAPTURES
        .get(index)
        .map_or(Capture::Other, |(_, capture)| *capture)
}
