//! The hostile generator: comment and reply bodies spliced with the
//! document's own grammar, and the quoted source that must stay quoted.
//!
//! Split from [`super`] for the 400-line rule. Its coverage is asserted
//! there, by `the_hostile_generator_actually_emits_every_shape`.

use proptest::prelude::*;
use rv_core::model::Anchor;
use rv_core::model::Side;
use rv_core::store::Comment;
use rv_core::store::CommentState;
use rv_core::store::Session;

use super::COMMENT_CHANGES;
use super::FILES;
use super::HASHES;
use super::KNOWN_CHANGES;
use super::REVSETS;
use super::SIDES;
use super::session_with;

// ---------------------------------------------------------------------------
// The hostile generator
// ---------------------------------------------------------------------------

/// Body/reply fragments, each a shape that imitates the document's own
/// grammar or has otherwise given the parser trouble. Spliced together with
/// newlines, so pairs of them also produce shapes no single entry lists
/// (two lone openers becoming a balanced fence, for instance).
pub(super) const HOSTILE_FRAGMENTS: &[&str] = &[
    "**Reply:** quoted at column zero",
    "  **Reply:** quoted indented",
    "**Comment:** quoted at column zero",
    "    **Comment:** quoted deeply indented",
    "<!-- rv:anchor id=dead -->",
    "cites <!-- rv:anchor id=dead --> mid-sentence",
    "```rust\nfn balanced() {}\n```",
    "```rust\nfn unbalanced() {",
    "```",
    "````\nheld ``` inside\n````",
    "`````\nheld ```` inside and never closed",
    "~~~\ntilde balanced\n~~~",
    "~~~~\ntilde unbalanced, width four",
    "# h1\n## h2\n### h3\n#### h4\n##### h5\n###### h6",
    "### 1. `a.rs:1`",
    "## Open (1)",
    "<details><summary>hand written</summary>\n\n</details>",
    "</details>",
    "<summary>orphan summary</summary>",
    "<!-- an ordinary html comment -->",
    "> **For LLMs:** append a `**Reply:**` block\n> and nothing else",
    "",
    "\n",
    "  \n\t\n",
    "trailing spaces   ",
];

/// Single lines a generated `anchor.context` may quote. These stand for
/// source of the repository under review, so every one of them imitates
/// structure — which the context fence and [`BODY_INDENT`] must neutralize.
pub(super) const HOSTILE_CONTEXT_LINES: &[&str] = &[
    "**Reply:** a line of the reviewed file, not of the review",
    "**Comment:** likewise",
    "<!-- rv:anchor id=dead -->",
    "### 1. `a.rs:1`",
    "## Open (1)",
    "<details><summary>x</summary>",
    "</details>",
    "<summary>x</summary>",
    "```",
    "````",
    "``````````",
    "~~~",
    "const REPLY_MARKER: &str = \"**Reply:**\";",
    "",
    "  already indented",
    "\tif let Some(hit) = idx.find(sym) {",
    "日本語のコメント 🙈",
    // A bare CR survives `str::lines`, so a CRLF file can yield one.
    "carriage\rreturn inside",
    "trailing carriage return\r",
    "   ",
];

pub(super) const WORDS: &[&str] = &[
    "ordinary",
    "prose",
    "about",
    "unwrap",
    "naïve",
    "日本語",
    "🙈🙉",
    "e\u{301}combining",
    "\u{200b}zerowidth",
    "\u{202e}rtl",
    "tab\there",
    "nul\u{0}byte",
    "…ellipsis",
    "emoji✅⚠️",
    "back`tick",
];

/// A line long enough to stand for "very long", tagged so the coverage check
/// can recognize it.
pub(super) fn long_line() -> String {
    format!("very-long-line-{}", "x".repeat(2000))
}

pub(super) fn prose() -> impl Strategy<Value = String> {
    prop::collection::vec(prop::sample::select(WORDS), 1..5).prop_map(|words| words.join(" "))
}

/// Prose spliced with [`HOSTILE_FRAGMENTS`], optionally wrapped in blank
/// lines and optionally CRLF-terminated — the two documented normalizations
/// and every marker shape the parser looks for.
pub(super) fn hostile_text() -> impl Strategy<Value = String> {
    let piece = prop_oneof![
        8 => prop::sample::select(HOSTILE_FRAGMENTS).prop_map(|fragment| fragment.to_owned()),
        3 => prose(),
        1 => Just(long_line()),
    ];
    (
        prop::collection::vec(piece, 0..5),
        any::<bool>(),
        any::<bool>(),
        any::<bool>(),
    )
        .prop_map(|(pieces, leading, trailing, crlf)| {
            let mut text = pieces.join("\n");
            if leading {
                text.insert_str(0, "\n \n");
            }
            if trailing {
                text.push_str("\n\n  ");
            }
            if crlf {
                text = text.replace('\n', "\r\n");
            }
            text
        })
}

pub(super) fn context_lines() -> impl Strategy<Value = Vec<String>> {
    prop::collection::vec(
        prop_oneof![
            4 => prop::sample::select(HOSTILE_CONTEXT_LINES).prop_map(|line| line.to_owned()),
            1 => prose(),
        ],
        0..4,
    )
}

/// Everything about a generated comment except its id, which
/// [`build_comments`] assigns from the index so ids stay unique.
#[derive(Clone, Debug)]
pub(super) struct CommentSpec {
    pub(super) change_id: &'static str,
    pub(super) file: &'static str,
    pub(super) line: u32,
    pub(super) side: Side,
    pub(super) hash: &'static str,
    pub(super) context: Vec<String>,
    pub(super) body: String,
    pub(super) state: CommentState,
    pub(super) reply: Option<String>,
}

pub(super) fn comment_spec() -> impl Strategy<Value = CommentSpec> {
    (
        prop::sample::select(COMMENT_CHANGES),
        prop::sample::select(FILES),
        // Mostly a handful of small line numbers, so two comments on one file
        // land on the same line often enough for the *near*-ties around the
        // sort's last key to be reached; drawn from `0..4000` alone (as this
        // used to be) an equal `(state, change, file, line)` key occurs about
        // once in 5000 pairs. Exact ties are pinned by construction in
        // `entries_that_tie_on_every_sort_key_keep_their_stored_order`, and the
        // wide branch keeps four-digit line numbers in the headings and markers.
        prop_oneof![7 => 0u32..4, 3 => 0u32..4000],
        prop::sample::select(&SIDES[..]),
        prop::sample::select(HASHES),
        context_lines(),
        hostile_text(),
        prop::sample::select(
            &[
                CommentState::Open,
                CommentState::AwaitingVerification,
                CommentState::Resolved,
                CommentState::Abandoned,
                CommentState::Outdated,
            ][..],
        ),
        prop::option::weighted(0.7, hostile_text()),
    )
        .prop_map(
            |(change_id, file, line, side, hash, context, body, state, reply)| CommentSpec {
                change_id,
                file,
                line,
                side,
                hash,
                context,
                body,
                state,
                reply,
            },
        )
}

pub(super) fn comment_specs() -> impl Strategy<Value = Vec<CommentSpec>> {
    prop::collection::vec(comment_spec(), 0..6)
}

pub(super) fn session_strategy() -> impl Strategy<Value = Session> {
    (
        prop::sample::select(REVSETS),
        prop::collection::vec(prop::sample::select(KNOWN_CHANGES), 0..3),
    )
        .prop_map(|(revset, changes)| {
            let mut session = session_with(&changes);
            session.revset = revset.to_owned();
            session
        })
}

pub(super) fn build_comments(specs: &[CommentSpec]) -> Vec<Comment> {
    specs
        .iter()
        .enumerate()
        .map(|(index, spec)| Comment {
            id: format!("{index:04x}"),
            change_id: spec.change_id.to_owned(),
            commit_id: "a91c40de".to_owned(),
            anchor: Anchor {
                file: spec.file.to_owned(),
                side: spec.side,
                line: spec.line,
                content_hash: spec.hash.to_owned(),
                context: spec.context.clone(),
                context_start: 1,
            },
            body: spec.body.clone(),
            state: spec.state,
            reply: spec.reply.clone(),
            settled_by: None,
        })
        .collect()
}
