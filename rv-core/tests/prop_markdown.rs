//! Property-based and parameterized tests for the `REVIEW-FEEDBACK.md`
//! round-trip surface (`rv_core::markdown`).
//!
//! `tests/markdown.rs` pins exact behaviours against hand-written fixtures.
//! This file attacks the same contract from the other side: it generates
//! *hostile* comment and reply bodies — prose spliced with the document's own
//! grammar — and asserts the invariants the design spec rests on, chiefly
//! "nothing a model can write to this file causes comment loss".
//!
//! # What the oracles are
//!
//! Every property here is checked against something independent of
//! `markdown.rs`:
//!
//! - [`normalized`] re-derives the module's two documented normalizations from
//!   their stated causes (`str::lines` for CRLF, blank-line trimming for the
//!   body rule) rather than by calling the parser.
//! - The conservation and section properties count column-0 lines in the
//!   rendered text and compare against counts taken from the *input* comments.
//! - The anti-misbinding properties tag each generated reply with a sentinel
//!   naming its own comment id, so a reply landing on the wrong entry is
//!   visible without trusting the parser's own bookkeeping.
//! - [`hostile_text`] is checked for coverage by
//!   [`the_hostile_generator_actually_emits_every_shape`], because a hostile
//!   generator that never emits the interesting case is how this kind of test
//!   becomes theatre.
//!
//! # What the generators deliberately hold well-formed, and why
//!
//! Comment ids, change/commit ids, file paths, the revset and `started_at` are
//! generated newline-free. They are not model-writable: ids are hex `rv`
//! generates, paths and revsets come from jj, and `started_at` is `rv`'s own
//! string. A newline inside any of them would inject a column-0 line into the
//! header or an entry heading, which is a corrupt *input* rather than a
//! tolerated *edit* — the module's positional grammar only claims to defend
//! the interpolated prose and quoted source. `anchor.context` is likewise
//! generated newline-free: its elements are file *lines* by construction (see
//! `anchor::create`), and everything else about them is hostile.

use proptest::prelude::*;
use proptest::strategy::ValueTree;
use proptest::test_runner::TestRunner;
use rstest::rstest;
use rv_core::markdown::parse_replies;
use rv_core::markdown::render;
use rv_core::model::Anchor;
use rv_core::model::ChangeRef;
use rv_core::model::Side;
use rv_core::store::Comment;
use rv_core::store::CommentState;
use rv_core::store::Session;

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

/// Changes the generated sessions may list, in this order.
const KNOWN_CHANGES: &[&str] = &["zzzzaaaabbbb", "yyyyccccdddd"];

/// Change ids a generated comment may carry — including one no session lists,
/// which must still render (last in its section) rather than be dropped.
const COMMENT_CHANGES: &[&str] = &["zzzzaaaabbbb", "yyyyccccdddd", "abandonedchg"];

const FILES: &[&str] = &[
    "a.rs",
    "b.rs",
    "rv-core/src/markdown.rs",
    "docs/notes.md",
    "a file with spaces.toml",
    "no-extension",
    "日本語/ファイル.rs",
];

const HASHES: &[&str] = &["9e21abcd", "<rv:out-of-range>", ""];

const REVSETS: &[&str] = &["trunk()..@", "@", "all()", "x | y"];

static SIDES: [Side; 2] = [Side::Left, Side::Right];

/// The four sections in the fixed order the spec assigns them: the ordering
/// oracle for entries, and the expected heading order.
const SECTION_ORDER: [(&str, CommentState); 5] = [
    ("Open", CommentState::Open),
    ("Awaiting verification", CommentState::AwaitingVerification),
    ("Resolved", CommentState::Resolved),
    ("Abandoned", CommentState::Abandoned),
    ("Outdated", CommentState::Outdated),
];

/// The states rendered expanded (`### <n>.` heading), in the same order.
const EXPANDED_STATES: [CommentState; 2] = [CommentState::Open, CommentState::AwaitingVerification];

/// The number of `> ` lines in the protocol block.
///
/// Read off a rendered document rather than written down: the two properties
/// below are about the block being **contiguous** and being the **only** quoted
/// run at column 0, and neither is a claim about its length. Hard-coding the
/// length made both fail the day a line was added to the protocol, which is a
/// test failing on its own bookkeeping rather than on the code.
fn protocol_lines() -> usize {
    let empty = Session {
        revset: "trunk()..@".to_owned(),
        base_commit: "0".repeat(40),
        head_commit: "1".repeat(40),
        changes: Vec::new(),
        started_at: "epoch:0".to_owned(),
    };
    rv_core::markdown::render(&empty, &[])
        .lines()
        .skip_while(|line| !line.starts_with("> "))
        .take_while(|line| line.starts_with("> "))
        .count()
}

fn base_session() -> Session {
    session_with(KNOWN_CHANGES)
}

fn session_with(changes: &[&str]) -> Session {
    Session {
        revset: "trunk()..@".to_owned(),
        base_commit: "a1b2c3d4".to_owned(),
        head_commit: "e5f6a7b8".to_owned(),
        changes: changes
            .iter()
            .map(|change_id| ChangeRef {
                change_id: (*change_id).to_owned(),
                commit_id: "1111aaaa".to_owned(),
                description: "a change".to_owned(),
            })
            .collect(),
        started_at: "epoch:1755440000".to_owned(),
    }
}

fn plain_comment(id: &str, state: CommentState) -> Comment {
    Comment {
        id: id.to_owned(),
        change_id: KNOWN_CHANGES[0].to_owned(),
        commit_id: "a91c40de".to_owned(),
        anchor: Anchor {
            file: "a.rs".to_owned(),
            side: Side::Right,
            line: 1,
            content_hash: "9e21abcd".to_owned(),
            context: vec!["fn main() {".to_owned(), "}".to_owned()],
            context_start: 1,
        },
        body: "an ordinary comment".to_owned(),
        state,
        reply: None,
        settled_by: None,
    }
}

// ---------------------------------------------------------------------------
// Oracles
// ---------------------------------------------------------------------------

/// The documented normal form of a body, derived from its two stated causes
/// rather than from the parser.
///
/// 1. `render` splits the body with `str::lines` and rejoins with `\n`, and the
///    document's own `\n` after each line means a second `str::lines` in
///    `parse_replies` eats a line's trailing bare `\r` too — so every CR that
///    sits at a line ending disappears.
/// 2. Leading and trailing *blank* lines of the body are dropped: the document
///    uses a blank line as the separator before the next structural element,
///    so they are not recoverable. Interior blank lines survive.
fn normalized(body: &str) -> String {
    let mut lines: Vec<&str> = body
        .lines()
        .map(|line| line.strip_suffix('\r').unwrap_or(line))
        .collect();
    while lines.first().is_some_and(|line| line.trim().is_empty()) {
        lines.remove(0);
    }
    while lines.last().is_some_and(|line| line.trim().is_empty()) {
        lines.pop();
    }
    lines.join("\n")
}

/// Where `change_id` sits in the session's change list — the spec's primary
/// entry sort key, with a change the session does not list sorting last.
fn change_position(session: &Session, change_id: &str) -> usize {
    session
        .changes
        .iter()
        .position(|change| change.change_id == change_id)
        .unwrap_or(usize::MAX)
}

/// The order the spec fixes for entries: sections in [`SECTION_ORDER`], then
/// change index, then path, then line, with ties keeping input order.
fn expected_order<'a>(session: &Session, comments: &'a [Comment]) -> Vec<&'a Comment> {
    let mut ordered = Vec::new();
    for (_, state) in SECTION_ORDER {
        let mut section: Vec<&Comment> = comments
            .iter()
            .filter(|comment| comment.state == state)
            .collect();
        section.sort_by(|a, b| {
            change_position(session, &a.change_id)
                .cmp(&change_position(session, &b.change_id))
                .then_with(|| a.anchor.file.cmp(&b.anchor.file))
                .then_with(|| a.anchor.line.cmp(&b.anchor.line))
        });
        ordered.extend(section);
    }
    ordered
}

/// The `(id, body)` pairs a round trip must return: every comment carrying a
/// reply, in render order, with the reply in normal form.
fn expected_replies(session: &Session, comments: &[Comment]) -> Vec<(String, String)> {
    expected_order(session, comments)
        .into_iter()
        .filter_map(|comment| {
            comment
                .reply
                .as_ref()
                .map(|reply| (comment.id.clone(), normalized(reply)))
        })
        .collect()
}

/// `### <n>. …` — an entry heading, as `render_expanded` writes it.
fn is_entry_heading(line: &str) -> bool {
    line.strip_prefix("### ")
        .is_some_and(|rest| rest.starts_with(|first: char| first.is_ascii_digit()))
}

/// A line that starts a new region, which is what the parser's binding, and
/// the mangling this file tolerates, are defined against.
fn is_boundary(line: &str) -> bool {
    is_entry_heading(line) || line.starts_with("## ") || line.starts_with("<details")
}

fn count_lines(document: &str, predicate: impl Fn(&str) -> bool) -> usize {
    document.lines().filter(|line| predicate(line)).count()
}

/// How many `<details>` are open when each line begins.
fn details_depths(lines: &[&str]) -> Vec<usize> {
    let mut depth = 0usize;
    let mut depths = Vec::with_capacity(lines.len());
    for line in lines {
        if *line == "</details>" {
            depth = depth.saturating_sub(1);
        }
        depths.push(depth);
        if line.starts_with("<details") {
            depth += 1;
        }
    }
    depths
}

/// The presentational entry numbers in document order, taken from both entry
/// shapes: the `### <n>.` heading of an expanded entry and the `<summary>`
/// of a collapsed one.
fn entry_numbers(document: &str) -> Vec<usize> {
    let mut numbers = Vec::new();
    for line in document.lines() {
        let candidate = if let Some(rest) = line.strip_prefix("### ") {
            Some(rest)
        } else if let Some(rest) = line.strip_prefix("<details><summary>") {
            // `<summary>✅ 3. <code>…` — the marker glyph, then the number.
            rest.split_once(' ').map(|(_, tail)| tail)
        } else {
            None
        };
        let Some(candidate) = candidate else { continue };
        let digits: String = candidate.chars().take_while(char::is_ascii_digit).collect();
        if let Ok(number) = digits.parse::<usize>() {
            numbers.push(number);
        }
    }
    numbers
}

// ---------------------------------------------------------------------------
// The hostile generator
// ---------------------------------------------------------------------------

/// Body/reply fragments, each a shape that imitates the document's own
/// grammar or has otherwise given the parser trouble. Spliced together with
/// newlines, so pairs of them also produce shapes no single entry lists
/// (two lone openers becoming a balanced fence, for instance).
const HOSTILE_FRAGMENTS: &[&str] = &[
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
const HOSTILE_CONTEXT_LINES: &[&str] = &[
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

const WORDS: &[&str] = &[
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
fn long_line() -> String {
    format!("very-long-line-{}", "x".repeat(2000))
}

fn prose() -> impl Strategy<Value = String> {
    prop::collection::vec(prop::sample::select(WORDS), 1..5).prop_map(|words| words.join(" "))
}

/// Prose spliced with [`HOSTILE_FRAGMENTS`], optionally wrapped in blank
/// lines and optionally CRLF-terminated — the two documented normalizations
/// and every marker shape the parser looks for.
fn hostile_text() -> impl Strategy<Value = String> {
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

fn context_lines() -> impl Strategy<Value = Vec<String>> {
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
struct CommentSpec {
    change_id: &'static str,
    file: &'static str,
    line: u32,
    side: Side,
    hash: &'static str,
    context: Vec<String>,
    body: String,
    state: CommentState,
    reply: Option<String>,
}

fn comment_spec() -> impl Strategy<Value = CommentSpec> {
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

fn comment_specs() -> impl Strategy<Value = Vec<CommentSpec>> {
    prop::collection::vec(comment_spec(), 0..6)
}

fn session_strategy() -> impl Strategy<Value = Session> {
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

fn build_comments(specs: &[CommentSpec]) -> Vec<Comment> {
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

// ---------------------------------------------------------------------------
// The generator's own coverage
// ---------------------------------------------------------------------------

/// Whether `text` has a line beginning with `prefix` at column 0.
fn has_column_zero(text: &str, prefix: &str) -> bool {
    text.starts_with(prefix) || text.contains(&format!("\n{prefix}"))
}

/// Every shape [`hostile_text`] is required to emit. A property over a
/// generator that never produces the interesting case protects nothing, so
/// these are asserted rather than assumed.
#[allow(clippy::type_complexity)]
const REQUIRED_SHAPES: &[(&str, fn(&str) -> bool)] = &[
    ("column-0 **Reply:** marker", |t| {
        has_column_zero(t, "**Reply:**")
    }),
    ("indented **Reply:** marker", |t| {
        t.contains("\n  **Reply:**")
    }),
    ("column-0 **Comment:** marker", |t| {
        has_column_zero(t, "**Comment:**")
    }),
    ("indented **Comment:** marker", |t| {
        t.contains("\n    **Comment:**")
    }),
    ("quoted anchor marker", |t| {
        t.contains("<!-- rv:anchor id=dead -->")
    }),
    ("balanced backtick fence", |t| {
        t.contains("```rust\nfn balanced() {}\n```") || t.contains("```rust\r\nfn balanced()")
    }),
    ("unbalanced backtick fence", |t| {
        t.contains("fn unbalanced() {")
    }),
    ("balanced tilde fence", |t| t.contains("tilde balanced")),
    ("unbalanced tilde fence", |t| t.contains("tilde unbalanced")),
    ("fence of width four", |t| has_column_zero(t, "````")),
    ("fence of width five", |t| has_column_zero(t, "`````")),
    ("heading level 1", |t| has_column_zero(t, "# h1")),
    ("heading level 2", |t| has_column_zero(t, "## h2")),
    ("heading level 3", |t| has_column_zero(t, "### h3")),
    ("heading level 4", |t| has_column_zero(t, "#### h4")),
    ("heading level 5", |t| has_column_zero(t, "##### h5")),
    ("heading level 6", |t| has_column_zero(t, "###### h6")),
    ("numbered entry heading", |t| {
        has_column_zero(t, "### 1. `a.rs:1`")
    }),
    ("section heading", |t| has_column_zero(t, "## Open (1)")),
    ("<details> open tag", |t| has_column_zero(t, "<details>")),
    ("</details> close tag", |t| has_column_zero(t, "</details>")),
    ("<summary> tag", |t| has_column_zero(t, "<summary>")),
    ("leading blank lines", |t| {
        t.starts_with("\n \n") || t.starts_with("\r\n \r\n")
    }),
    ("trailing blank lines", |t| {
        t.ends_with("\n\n  ") || t.ends_with("\r\n\r\n  ")
    }),
    ("CRLF line endings", |t| t.contains("\r\n")),
    ("the empty string", |t| t.is_empty()),
    ("a non-ASCII character", |t| !t.is_ascii()),
    ("a very long line", |t| {
        t.lines().any(|line| line.chars().count() >= 1000)
    }),
];

/// The generator emits every shape the properties below are meant to cover.
/// Deterministic RNG, so this can only fail when the generator changes.
#[test]
fn the_hostile_generator_actually_emits_every_shape() {
    let strategy = hostile_text();
    let mut runner = TestRunner::deterministic();
    let mut hits = vec![0usize; REQUIRED_SHAPES.len()];

    for _ in 0..1500 {
        let sample = strategy
            .new_tree(&mut runner)
            .expect("the hostile strategy must produce a value")
            .current();
        for (index, (_, predicate)) in REQUIRED_SHAPES.iter().enumerate() {
            if predicate(&sample) {
                hits[index] += 1;
            }
        }
    }

    let missing: Vec<&str> = REQUIRED_SHAPES
        .iter()
        .zip(&hits)
        .filter(|(_, count)| **count == 0)
        .map(|((name, _), _)| *name)
        .collect();
    assert!(
        missing.is_empty(),
        "the hostile generator never emitted: {missing:?}"
    );
}

/// The same for the context generator: quoted source must actually imitate
/// structure, or the context-fence properties prove nothing.
#[test]
fn the_context_generator_actually_emits_marker_like_lines() {
    let strategy = context_lines();
    let mut runner = TestRunner::deterministic();
    let required: &[&str] = &[
        "**Reply:** a line of the reviewed file, not of the review",
        "**Comment:** likewise",
        "<!-- rv:anchor id=dead -->",
        "### 1. `a.rs:1`",
        "## Open (1)",
        "</details>",
        "<details><summary>x</summary>",
        "``````````",
        "~~~",
        "",
    ];
    let mut seen = vec![false; required.len()];
    let mut saw_empty_vec = false;

    for _ in 0..1200 {
        let sample = strategy
            .new_tree(&mut runner)
            .expect("the context strategy must produce a value")
            .current();
        saw_empty_vec |= sample.is_empty();
        for (index, needle) in required.iter().enumerate() {
            seen[index] |= sample.iter().any(|line| line == needle);
        }
    }

    let missing: Vec<&str> = required
        .iter()
        .zip(&seen)
        .filter(|(_, hit)| !**hit)
        .map(|(needle, _)| *needle)
        .collect();
    assert!(
        missing.is_empty(),
        "the context generator never emitted: {missing:?}"
    );
    assert!(saw_empty_vec, "an empty context must also be generated");
}

/// The normalization oracle is itself a normal form: applying it twice changes
/// nothing.
///
/// This one guards the *oracle*, not the module — without it,
/// [`the_document_is_a_fixpoint_from_the_second_pass`] could report stability
/// that came from a sloppy oracle rather than from `markdown.rs`.
#[test]
fn the_normalization_oracle_is_idempotent() {
    let strategy = hostile_text();
    let mut runner = TestRunner::deterministic();
    for _ in 0..600 {
        let sample = strategy
            .new_tree(&mut runner)
            .expect("the hostile strategy must produce a value")
            .current();
        let once = normalized(&sample);
        assert_eq!(normalized(&once), once, "not a normal form: {sample:?}");
    }
}

// ---------------------------------------------------------------------------
// Round-trip closure
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(192))]

    /// The headline guarantee: `parse_replies(render(session, comments))` is
    /// exactly the `(id, reply)` pairs of the comments carrying a reply, in
    /// render order, with bodies byte-identical to their documented normal
    /// form. Bodies, replies and quoted context are all hostile at once.
    #[test]
    fn render_then_parse_returns_exactly_the_stored_replies(
        session in session_strategy(),
        specs in comment_specs(),
    ) {
        let comments = build_comments(&specs);
        let document = render(&session, &comments);

        prop_assert_eq!(
            parse_replies(&document),
            expected_replies(&session, &comments),
            "round trip lost, fabricated, mis-bound or mangled a reply"
        );
    }

    /// Order is not an accident of the sort's implementation: the ids come
    /// back in the order their anchor markers appear in the text.
    #[test]
    fn parsed_replies_follow_document_order(
        session in session_strategy(),
        specs in comment_specs(),
    ) {
        let comments = build_comments(&specs);
        let document = render(&session, &comments);
        let replies = parse_replies(&document);

        let mut previous = None;
        for (id, _) in &replies {
            let at = document
                .find(&format!("<!-- rv:anchor id={id} "))
                .or_else(|| document.find(&format!("<!-- rv:anchor id={id} -->")));
            let at = at.expect("every parsed id must come from a rendered anchor marker");
            if let Some(previous) = previous {
                prop_assert!(
                    previous < at,
                    "replies came back out of document order at id {}",
                    id
                );
            }
            previous = Some(at);
        }
    }

    /// The last clause of the documented entry order, and the only one a random
    /// generator cannot be relied on to reach: comments that tie on *every* sort
    /// key — same section, same change, same file, same line — keep the order
    /// they were stored in. `render` says so in a comment on its `sort_by`, and
    /// [`expected_order`] mirrors it with a stable sort, but with
    /// `comment_spec()`'s original `0u32..4000` line strategy an exact
    /// `(state, change, file, line)` tie arose about once in 5000 pairs, so
    /// nothing in this file exercised the clause.
    ///
    /// Built by construction, and asserted against the *document* rather than
    /// against the sort: `n` comments identical in every key, differing only in
    /// the id `build_comments` assigns from the index and in their hostile
    /// bodies, must lay their anchor markers down in ascending id order.
    #[test]
    fn entries_that_tie_on_every_sort_key_keep_their_stored_order(
        session in session_strategy(),
        template in comment_spec(),
        bodies in prop::collection::vec(
            (hostile_text(), prop::option::weighted(0.7, hostile_text())),
            2..6,
        ),
    ) {
        let specs: Vec<CommentSpec> = bodies
            .into_iter()
            .map(|(body, reply)| CommentSpec {
                body,
                reply,
                ..template.clone()
            })
            .collect();
        let comments = build_comments(&specs);
        let document = render(&session, &comments);

        // `every_comment_renders_exactly_once` pins that a column-0
        // `<!-- rv:anchor ` line is one `render` wrote, so reading the ids off
        // them is reading the document, not re-running the sort.
        let laid_down: Vec<&str> = document
            .lines()
            .filter_map(|line| line.strip_prefix("<!-- rv:anchor id="))
            .filter_map(|rest| rest.split_whitespace().next())
            .collect();
        let stored: Vec<&str> = comments.iter().map(|comment| comment.id.as_str()).collect();
        prop_assert_eq!(
            &laid_down,
            &stored,
            "tied entries were reordered: every comment is on {}:{} of change {}",
            template.file, template.line, template.change_id
        );

        prop_assert_eq!(
            parse_replies(&document),
            expected_replies(&session, &comments),
            "a document of nothing but tied entries lost or mis-bound a reply"
        );
    }

    /// One normalization on the first pass, then a fixed point: writing the
    /// parsed replies back into the comments and re-rendering produces a
    /// byte-identical document forever after. This is the real `rv` loop —
    /// render, hand to a model, read replies back, re-render — so drift here
    /// would compound on every cycle.
    ///
    /// Pass one is pinned to [`expected_replies`] first, because everything
    /// after it is a comparison of the module against itself: a `parse_replies`
    /// that returned nothing at all would satisfy `second == first` (`[] == []`)
    /// and `third == second` too, and only the `already_normal` branch below —
    /// which needs *every* stored reply to be in normal form, so about one case
    /// in eight — would notice. With the first line in place the property stands
    /// on an independent oracle and then adds stability to it.
    #[test]
    fn the_document_is_a_fixpoint_from_the_second_pass(
        session in session_strategy(),
        specs in comment_specs(),
    ) {
        let first_comments = build_comments(&specs);
        let first = render(&session, &first_comments);
        let first_replies = parse_replies(&first);

        prop_assert_eq!(
            &first_replies,
            &expected_replies(&session, &first_comments),
            "pass one must already be the stored replies in normal form"
        );

        let second_comments = with_parsed_replies(&first_comments, &first_replies);
        let second = render(&session, &second_comments);
        let second_replies = parse_replies(&second);

        prop_assert_eq!(
            &second_replies,
            &first_replies,
            "the parse result must not keep changing"
        );

        let third = render(&session, &with_parsed_replies(&second_comments, &second_replies));
        prop_assert_eq!(&third, &second, "the document must be stable from pass two");

        // And pass one only normalizes: a document whose replies were already
        // in normal form is byte-identical on the first pass too.
        let already_normal = first_comments
            .iter()
            .all(|comment| comment.reply.as_ref().is_none_or(|reply| normalized(reply) == *reply));
        if already_normal {
            prop_assert_eq!(&second, &first, "normalized replies must not be renormalized");
        }
    }
}

/// Replaces each comment's reply with the one parsed back for its id, which is
/// what `rv` stores after reading the markdown file.
fn with_parsed_replies(comments: &[Comment], replies: &[(String, String)]) -> Vec<Comment> {
    comments
        .iter()
        .map(|comment| {
            let mut updated = comment.clone();
            updated.reply = replies
                .iter()
                .find(|(id, _)| *id == comment.id)
                .map(|(_, body)| body.clone());
            updated
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Conservation and structure
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(192))]

    /// No comment is ever dropped, whatever its state, content, or whether the
    /// session still lists its change: one entry and one anchor marker per
    /// comment, numbered `1..=n` in document order.
    #[test]
    fn every_comment_renders_exactly_once(
        session in session_strategy(),
        specs in comment_specs(),
    ) {
        let comments = build_comments(&specs);
        let document = render(&session, &comments);

        let expanded = comments
            .iter()
            .filter(|comment| EXPANDED_STATES.contains(&comment.state))
            .count();
        let collapsed = comments.len() - expanded;

        prop_assert_eq!(
            count_lines(&document, is_entry_heading),
            expanded,
            "one `### <n>.` heading per expanded comment"
        );
        prop_assert_eq!(
            count_lines(&document, |line| line.starts_with("<details><summary>")),
            collapsed,
            "one collapsed entry per resolved/outdated comment"
        );
        prop_assert_eq!(
            count_lines(&document, |line| line == "</details>"),
            collapsed,
            "every <details> must be closed"
        );
        prop_assert_eq!(
            count_lines(&document, |line| line.starts_with("<!-- rv:anchor ")),
            comments.len(),
            "one anchor marker per comment"
        );
        prop_assert_eq!(
            count_lines(&document, |line| line.starts_with("**Comment:**")),
            comments.len(),
            "one comment body per comment"
        );

        // Every id is present, once, in a marker of its own.
        for comment in &comments {
            let marker = format!("<!-- rv:anchor id={} ", comment.id);
            prop_assert_eq!(
                count_lines(&document, |line| line.starts_with(&marker)),
                1,
                "id {} must appear in exactly one anchor marker",
                comment.id
            );
        }

        let numbers = entry_numbers(&document);
        prop_assert_eq!(
            numbers,
            (1..=comments.len()).collect::<Vec<usize>>(),
            "entries must be numbered 1..=n in document order"
        );
    }

    /// The four sections come in fixed order, each heading's count is the
    /// number of comments actually in that state, and every comment's anchor
    /// marker sits under its own section heading — collapsed inside a
    /// `<details>` for Resolved/Outdated, at depth zero otherwise.
    #[test]
    fn sections_are_ordered_counted_and_correctly_collapsed(
        session in session_strategy(),
        specs in comment_specs(),
    ) {
        let comments = build_comments(&specs);
        let document = render(&session, &comments);
        let lines: Vec<&str> = document.lines().collect();
        let depths = details_depths(&lines);

        prop_assert_eq!(
            count_lines(&document, |line| line.starts_with("## ")),
            SECTION_ORDER.len(),
            "exactly five section headings, whatever the bodies contain"
        );
        prop_assert_eq!(
            depths.last().copied().unwrap_or(0)
                + usize::from(lines.last().is_some_and(|line| line.starts_with("<details"))),
            0,
            "every <details> must be balanced"
        );

        // Heading positions, in the order they appear.
        let headings: Vec<(usize, &str)> = lines
            .iter()
            .enumerate()
            .filter(|(_, line)| line.starts_with("## "))
            .map(|(index, line)| (index, *line))
            .collect();
        for (position, (_, heading)) in headings.iter().enumerate() {
            let (title, state) = SECTION_ORDER[position];
            let in_state = comments
                .iter()
                .filter(|comment| comment.state == state)
                .count();
            let expected = format!("## {title} ({in_state})");
            prop_assert_eq!(
                *heading,
                expected.as_str(),
                "section {} is wrong",
                position
            );
        }

        for comment in &comments {
            let marker = format!("<!-- rv:anchor id={} ", comment.id);
            let at = lines
                .iter()
                .position(|line| line.starts_with(&marker))
                .expect("every comment must render an anchor marker");
            let section = SECTION_ORDER
                .iter()
                .position(|(_, state)| *state == comment.state)
                .expect("every state is a section");
            let start = headings[section].0;
            let end = headings
                .get(section + 1)
                .map_or(lines.len(), |(index, _)| *index);
            prop_assert!(
                start < at && at < end,
                "id {} landed outside its own section",
                comment.id
            );

            let collapsed = !EXPANDED_STATES.contains(&comment.state);
            prop_assert_eq!(
                depths[at],
                usize::from(collapsed),
                "id {} has the wrong <details> nesting",
                comment.id
            );
        }
    }

    /// Structure lives at column 0, and only `render` writes there: every
    /// non-empty line of a rendered document is either indented out of column
    /// 0 or one of the shapes `render` authors — and the number of each kind
    /// of authored line matches the input, so interpolated prose cannot add
    /// one.
    #[test]
    fn only_render_writes_at_column_zero(
        session in session_strategy(),
        specs in comment_specs(),
    ) {
        let comments = build_comments(&specs);
        let document = render(&session, &comments);
        let with_reply = comments.iter().filter(|c| c.reply.is_some()).count();

        for line in document.lines() {
            let authored = line == "<!-- rv:v1 -->"
                || line.starts_with("# Review: `")
                || line.starts_with("Base `")
                || line.starts_with("> ")
                || line.starts_with("## ")
                || is_entry_heading(line)
                || line.starts_with("<details><summary>")
                || line == "</details>"
                || line.starts_with("<!-- rv:anchor ")
                || line.starts_with("**Comment:**")
                || line.starts_with("**Reply:**")
                // Names a `trunk()` that resolved to the root — see
                // `markdown::degraded_base`.
                || line.starts_with("**Note:**");
            prop_assert!(
                line.is_empty() || line.starts_with("  ") || authored,
                "line at column 0 that render did not author: {:?}",
                line
            );
        }

        prop_assert_eq!(
            count_lines(&document, |line| line.starts_with("**Reply:**")),
            with_reply,
            "exactly one column-0 reply marker per stored reply"
        );
        prop_assert_eq!(
            count_lines(&document, |line| line.starts_with("> ")),
            protocol_lines(),
            "the protocol block must be the only quoted block at column 0"
        );
        prop_assert_eq!(
            count_lines(&document, |line| line == "<!-- rv:v1 -->"),
            1,
            "one version marker"
        );
    }

    /// The header is unconditional: version marker first, then the review
    /// heading with correctly pluralized counts, the base→head line, and one
    /// protocol block — no matter what the comments contain.
    #[test]
    fn the_header_and_protocol_survive_any_content(
        session in session_strategy(),
        specs in comment_specs(),
    ) {
        let comments = build_comments(&specs);
        let document = render(&session, &comments);

        prop_assert!(
            document.starts_with("<!-- rv:v1 -->\n"),
            "the version marker must be the first line"
        );

        let changes = session.changes.len();
        let plural = |count: usize| if count == 1 { "" } else { "s" };
        let heading = format!(
            "# Review: `{}` — {} change{}, {} comment{}\n",
            session.revset,
            changes,
            plural(changes),
            comments.len(),
            plural(comments.len()),
        );
        prop_assert!(
            document.contains(&heading),
            "missing or miscounted review heading: {:?}",
            heading
        );
        prop_assert!(
            document.contains(&format!(
                "Base `{}` → head `{}`",
                session.base_commit, session.head_commit
            )),
            "missing base→head line"
        );
        prop_assert!(
            document.contains(&session.started_at),
            "missing session start"
        );

        // The protocol block is one contiguous run of `> ` lines.
        let lines: Vec<&str> = document.lines().collect();
        let first_quote = lines
            .iter()
            .position(|line| line.starts_with("> "))
            .expect("the protocol block must be rendered");
        let run = lines[first_quote..]
            .iter()
            .take_while(|line| line.starts_with("> "))
            .count();
        prop_assert_eq!(run, protocol_lines(), "the protocol block must be contiguous");
        prop_assert!(
            lines[first_quote].contains("rendered view"),
            "the note must say the document is a view"
        );
        prop_assert!(
            lines[first_quote..first_quote + run]
                .iter()
                .any(|line| line.contains("rv comments --json")),
            "the note must name the CLI that replaced the round trip"
        );
    }
}

// ---------------------------------------------------------------------------
// No fabrication, no misbinding
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(192))]

    /// The anti-misbinding invariant, from a generator that leans on hostile
    /// *comment* bodies: whatever a body quotes — a reply marker, an anchor
    /// marker, a heading, a fence — the set of ids that come back is exactly
    /// the set of comments that genuinely carried a reply, with the right
    /// body each. Never an invented reply, never one bound to the wrong entry.
    #[test]
    fn no_reply_is_fabricated_or_bound_to_the_wrong_comment(
        session in session_strategy(),
        specs in prop::collection::vec(comment_spec(), 1..5),
        replying in prop::collection::vec(any::<bool>(), 1..5),
    ) {
        // Keep the hostile bodies, but decide independently (and often
        // negatively) which comments actually have a reply, so a fabricated
        // reply has somewhere to show up.
        let mut comments = build_comments(&specs);
        for (index, comment) in comments.iter_mut().enumerate() {
            if !replying.get(index % replying.len()).copied().unwrap_or(false) {
                comment.reply = None;
            }
        }
        let document = render(&session, &comments);
        let replies = parse_replies(&document);

        for (id, body) in &replies {
            let owner = comments.iter().find(|comment| comment.id == *id);
            let owner = owner.unwrap_or_else(|| {
                panic!("parse_replies invented the id {id:?}");
            });
            let stored = owner
                .reply
                .as_ref()
                .unwrap_or_else(|| panic!("a reply was fabricated for {id:?}, which had none"));
            prop_assert_eq!(body, &normalized(stored), "wrong body bound to {}", id);
        }
        prop_assert_eq!(
            replies.len(),
            comments.iter().filter(|comment| comment.reply.is_some()).count(),
            "every stored reply must come back exactly once"
        );
    }

    /// Quoted source is content, never structure: a document whose entries
    /// quote `**Reply:**`, anchor markers and fences inside their context
    /// blocks yields no replies at all when no comment carries one.
    #[test]
    fn hostile_context_never_yields_a_reply(
        session in session_strategy(),
        specs in prop::collection::vec(comment_spec(), 1..5),
    ) {
        let mut comments = build_comments(&specs);
        for comment in &mut comments {
            comment.reply = None;
            // Guarantee a non-empty context, so the fence is always rendered.
            if comment.anchor.context.is_empty() {
                comment.anchor.context = vec![
                    "**Reply:** a line of the reviewed file".to_owned(),
                    "<!-- rv:anchor id=dead -->".to_owned(),
                ];
            }
        }

        let document = render(&session, &comments);

        prop_assert_eq!(
            parse_replies(&document),
            Vec::new(),
            "quoted source was read as document structure"
        );
    }
}

// ---------------------------------------------------------------------------
// Tolerated mangling
// ---------------------------------------------------------------------------

/// Edits a model or a human with an editor could plausibly make to a rendered
/// document.
///
/// Deliberately excluded, because the module does not claim to survive them:
/// removing or indenting an entry boundary (`### <n>.`, `## `, `<details`),
/// which is documented to be what clears a binding, and inserting a *column-0*
/// anchor marker, which is by construction indistinguishable from one `rv`
/// wrote. Both are `rv`'s to write and the protocol block tells the model so.
#[derive(Clone, Debug)]
enum Mangle {
    /// Push a line out of column 0, where the parser stops seeing it.
    Indent(usize),
    /// Delete a line outright — an anchor marker, a body line, a fence.
    Delete(usize),
    /// Garble an anchor marker's `id=` field past recognition.
    GarbleAnchor(usize),
    /// Splice hostile text in, indented.
    InsertIndented(usize, &'static str),
    /// Splice hostile text in at column 0, where it can imitate structure.
    InsertColumnZero(usize, &'static str),
    /// Cut the document off at a line boundary.
    Truncate(usize),
}

/// Column-0 insertions: everything the grammar recognizes except an anchor
/// marker (see [`Mangle`]).
const COLUMN_ZERO_INSERTIONS: &[&str] = &[
    "**Reply:** injected at column zero",
    "**Comment:** injected at column zero",
    "## Injected (0)",
    "### 9. `injected.rs:1`",
    "<details><summary>injected</summary>",
    "<details>",
    "</details>",
    "<summary>injected</summary>",
    "```",
    "```rust",
    "~~~",
    "prose at column zero",
    "",
];

fn mangle() -> impl Strategy<Value = Mangle> {
    prop_oneof![
        3 => any::<usize>().prop_map(Mangle::Indent),
        3 => any::<usize>().prop_map(Mangle::Delete),
        2 => any::<usize>().prop_map(Mangle::GarbleAnchor),
        2 => (any::<usize>(), prop::sample::select(HOSTILE_FRAGMENTS))
            .prop_map(|(at, text)| Mangle::InsertIndented(at, text)),
        2 => (any::<usize>(), prop::sample::select(COLUMN_ZERO_INSERTIONS))
            .prop_map(|(at, text)| Mangle::InsertColumnZero(at, text)),
        1 => any::<usize>().prop_map(Mangle::Truncate),
    ]
}

/// The first non-boundary line at or after `at`, cycling; `None` if the
/// document is nothing but boundaries.
fn non_boundary(lines: &[String], at: usize) -> Option<usize> {
    (0..lines.len())
        .map(|offset| (at + offset) % lines.len())
        .find(|index| !is_boundary(&lines[*index]))
}

fn apply_mangles(document: &str, mangles: &[Mangle]) -> String {
    let mut lines: Vec<String> = document.lines().map(str::to_owned).collect();
    for mangle in mangles {
        if lines.is_empty() {
            break;
        }
        match mangle {
            Mangle::Indent(at) => {
                if let Some(index) = non_boundary(&lines, at % lines.len()) {
                    lines[index] = format!("  {}", lines[index]);
                }
            }
            Mangle::Delete(at) => {
                if let Some(index) = non_boundary(&lines, at % lines.len()) {
                    lines.remove(index);
                }
            }
            Mangle::GarbleAnchor(at) => {
                let start = at % lines.len();
                let found = (0..lines.len())
                    .map(|offset| (start + offset) % lines.len())
                    .find(|index| lines[*index].starts_with("<!-- rv:anchor "));
                if let Some(index) = found {
                    lines[index] = lines[index].replacen("id=", "xd=", 1);
                }
            }
            Mangle::InsertIndented(at, text) => {
                let where_at = at % (lines.len() + 1);
                let inserted: Vec<String> = text.lines().map(|line| format!("  {line}")).collect();
                lines.splice(where_at..where_at, inserted);
            }
            Mangle::InsertColumnZero(at, text) => {
                let where_at = at % (lines.len() + 1);
                let inserted: Vec<String> = text.lines().map(str::to_owned).collect();
                lines.splice(where_at..where_at, inserted);
            }
            Mangle::Truncate(at) => lines.truncate(at % (lines.len() + 1)),
        }
    }
    let mut out = lines.join("\n");
    out.push('\n');
    out
}

/// A reply body that names its own comment, so a misbinding is visible without
/// trusting the parser's bookkeeping.
fn sentinel(id: &str) -> String {
    format!("SENTINEL-{id}-REPLY")
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// The load-bearing tolerance claim: however a rendered document is
    /// mangled (short of rewriting `rv`'s own boundaries), a reply body never
    /// lands under another comment's id, and no id is invented. A reply may be
    /// dropped — that is recoverable, `comments.json` is the authority — but
    /// words must never be put in a comment nobody wrote them under.
    #[test]
    fn mangling_never_binds_a_reply_to_another_comment(
        specs in prop::collection::vec(comment_spec(), 1..5),
        mangles in prop::collection::vec(mangle(), 0..5),
    ) {
        let session = base_session();
        let mut comments = build_comments(&specs);
        for comment in &mut comments {
            let tail = comment.reply.clone().unwrap_or_default();
            comment.reply = Some(format!("{}\n{tail}", sentinel(&comment.id)));
        }

        let document = render(&session, &comments);
        let mangled = apply_mangles(&document, &mangles);
        let replies = parse_replies(&mangled);

        for (id, body) in &replies {
            prop_assert!(
                mangled.contains(&format!("id={id}")),
                "invented the id {:?}",
                id
            );
            for comment in &comments {
                if comment.id != *id {
                    prop_assert!(
                        !body.contains(&sentinel(&comment.id)),
                        "comment {}'s reply was bound to {}",
                        comment.id,
                        id
                    );
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Totality: parse_replies has no error path
// ---------------------------------------------------------------------------

/// Structural lines to build fuzz documents out of. Only three ids appear in a
/// column-0 anchor marker — `aaa`, `bbb` and `ccc` — which makes the returned
/// id set independently checkable.
const SOUP_LINES: &[&str] = &[
    "<!-- rv:v1 -->",
    "# Review: `trunk()..@` — 1 change, 1 comment",
    "## Open (1)",
    "## Resolved (0)",
    "### 1. `a.rs:1`",
    "### 99. `b.rs:2`",
    "#### not an entry heading",
    "<!-- rv:anchor id=aaa change=z commit=c side=right line=1 hash=h -->",
    "<!--rv:anchor id=bbb-->",
    "<!-- rv:anchor id= -->",
    "<!-- rv:anchor change=z commit=c -->",
    "<!-- rv:anchor id=ccc",
    "  <!-- rv:anchor id=indented -->",
    "**Comment:** a comment body",
    "**Reply:** a reply body",
    "**Reply:**",
    "**Reply:**no space after the marker",
    "  **Reply:** an indented reply",
    "<details><summary>x</summary>",
    "<details>",
    "</details>",
    "<summary>x</summary>",
    "```",
    "```rust",
    "````",
    "~~~",
    "~~~~",
    "  ```",
    "",
    "  indented prose",
    "prose at column zero",
    "> protocol quote",
    "中文 🙈 \u{202e}\u{200b}",
];

/// The only ids [`SOUP_LINES`] puts in a column-0 anchor marker.
const SOUP_IDS: &[&str] = &["aaa", "bbb", "ccc"];

fn soup() -> impl Strategy<Value = String> {
    prop::collection::vec(prop::sample::select(SOUP_LINES), 0..25)
        .prop_map(|lines| lines.join("\n"))
}

/// Text with no well-formedness guarantee whatsoever, for the totality
/// property.
///
/// Two shapes, because they cover different halves of the claim. Pure `char`
/// noise is what drives `fence_open`'s leading-character inspection over
/// non-ASCII bytes en masse — the shape that would panic on a `trimmed[..1]`
/// slice. But pure noise never produces *structure*: measured over 5000 samples
/// it contained `<!--` zero times and returned a non-empty `Vec` zero times, so
/// the assertions about the ids that come back never executed. Splicing the
/// grammar's own lines between the noise fixes that without giving up the first
/// shape — every other line is still an arbitrary `char`, so the char-boundary
/// paths are exercised on real markers rather than on noise alone.
fn arbitrary_utf8() -> impl Strategy<Value = String> {
    prop_oneof![
        1 => prop::collection::vec(any::<char>(), 0..160)
            .prop_map(|characters| characters.into_iter().collect::<String>()),
        1 => prop::collection::vec(
                prop_oneof![
                    3 => any::<char>().prop_map(String::from),
                    1 => prop::sample::select(SOUP_LINES).prop_map(str::to_owned),
                ],
                0..40,
            )
            .prop_map(|pieces| pieces.join("\n")),
    ]
}

/// The totality generator reaches the parser's reply path, so the id assertions
/// in [`parse_replies_never_panics_on_arbitrary_utf8`] are live code rather than
/// a claim the test does not check. Deterministic RNG, so this can only fail
/// when the generator changes.
#[test]
fn the_totality_generator_reaches_the_reply_path() {
    let strategy = arbitrary_utf8();
    let mut runner = TestRunner::deterministic();
    let mut with_marker = 0usize;
    let mut with_replies = 0usize;
    let mut with_control_char = 0usize;

    for _ in 0..2000 {
        let sample = strategy
            .new_tree(&mut runner)
            .expect("the totality strategy must produce a value")
            .current();
        if sample.contains("<!--") {
            with_marker += 1;
        }
        if !parse_replies(&sample).is_empty() {
            with_replies += 1;
        }
        if sample.chars().any(char::is_control) {
            with_control_char += 1;
        }
    }

    assert!(with_marker > 0, "no sample contained an anchor marker");
    assert!(
        with_replies > 0,
        "no sample ever produced a reply, so the id assertions are dead code"
    );
    assert!(
        with_control_char > 0,
        "no sample contained a control character"
    );
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    /// `parse_replies` has no error path and no panic path: arbitrary UTF-8 —
    /// any char, any length, no structure at all — must come back as a `Vec`.
    #[test]
    fn parse_replies_never_panics_on_arbitrary_utf8(text in arbitrary_utf8()) {
        let replies = parse_replies(&text);
        for (id, _) in &replies {
            prop_assert!(!id.is_empty(), "an empty id must never bind a reply");
            prop_assert!(
                text.contains(&format!("id={id}")),
                "the id {:?} was not read out of the input",
                id
            );
        }
    }

    /// Fuzz built from the grammar's own lines, which reaches far more of the
    /// parser than random characters do. Ids are never invented, and the
    /// number of replies is bounded by the number of column-0 `**Reply:**`
    /// lines — one marker can never yield two replies.
    #[test]
    fn structural_soup_invents_nothing(document in soup()) {
        let replies = parse_replies(&document);

        for (id, _) in &replies {
            prop_assert!(
                SOUP_IDS.contains(&id.as_str()),
                "id {:?} is not one the input put in a column-0 marker",
                id
            );
        }
        prop_assert!(
            replies.len() <= count_lines(&document, |line| line.starts_with("**Reply:**")),
            "more replies ({}) than column-0 reply markers ({})",
            replies.len(),
            count_lines(&document, |line| line.starts_with("**Reply:**"))
        );
    }

    /// The same bound and no-invention rule over mangled *rendered*
    /// documents, where the lines are real and the structure is nearly valid.
    #[test]
    fn mangled_documents_stay_within_their_bounds(
        specs in prop::collection::vec(comment_spec(), 1..4),
        mangles in prop::collection::vec(mangle(), 0..6),
    ) {
        let comments = build_comments(&specs);
        let document = render(&base_session(), &comments);
        let mangled = apply_mangles(&document, &mangles);
        let replies = parse_replies(&mangled);

        prop_assert!(
            replies.len() <= count_lines(&mangled, |line| line.starts_with("**Reply:**")),
            "more replies than column-0 reply markers"
        );
        for (id, _) in &replies {
            prop_assert!(!id.is_empty(), "an empty id must never bind a reply");
            prop_assert!(
                mangled.contains(&format!("id={id}")),
                "the id {:?} was not read out of the document",
                id
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Named hostile shapes
// ---------------------------------------------------------------------------

/// One case per hostile shape, so a regression names the shape that broke
/// rather than reporting a shrunk blob. Each shape is used as the reply (which
/// must round-trip to its normal form) and, separately, as the comment body of
/// a comment with no reply (which must fabricate nothing).
///
/// These shapes are the ones `tests/markdown.rs`'s table does not cover: the
/// two documented normalizations, unicode, control characters, extreme line
/// lengths, and the tag and fence variants it leaves out.
#[rstest]
#[case::empty("")]
#[case::only_blank_lines("\n\n  \n")]
#[case::leading_blank_lines("\n\nafter two blank lines")]
#[case::trailing_blank_lines("before two blank lines\n\n")]
#[case::trailing_whitespace_only_line("body\n   ")]
#[case::crlf("first\r\nsecond\r\n\r\nthird")]
#[case::bare_cr_at_line_end("first\r\nsecond\r")]
#[case::interior_blank_line("one\n\ntwo")]
#[case::interior_whitespace_only_line("one\n \ntwo")]
#[case::indented_reply_marker("  **Reply:** indented, so the parser must not see it")]
#[case::indented_comment_marker("    **Comment:** indented four")]
#[case::bare_anchor_marker("<!-- rv:anchor id=dead -->")]
#[case::all_heading_levels("# one\n## two\n### three\n#### four\n##### five\n###### six")]
#[case::tilde_unbalanced("~~~\nnever closed")]
#[case::tilde_width_four("~~~~\nheld ~~~ inside\n~~~~")]
#[case::backtick_width_five_unbalanced("`````\nheld ``` inside")]
#[case::mixed_fence_markers("```\ntext\n~~~\nmore\n```")]
#[case::fence_holding_details("```\n</details>\n<details>\n```")]
#[case::details_never_closed("<details><summary>never closed</summary>")]
#[case::orphan_summary("<summary>orphan</summary>")]
#[case::plain_html_comment("<!-- just a comment -->")]
#[case::unicode("naïve 日本語 🙈 \u{200b} e\u{301} \u{202e}rtl …")]
#[case::control_characters("before\u{0}after\u{7}bell\u{b}vtab")]
#[case::blank_first_line("\n**Reply:** on the second line")]
#[case::tabs("\tfirst\n\t\tsecond")]
fn hostile_shape_round_trips(
    #[case] shape: &str,
    #[values(
        CommentState::Open,
        CommentState::AwaitingVerification,
        CommentState::Resolved,
        CommentState::Outdated
    )]
    state: CommentState,
) {
    let session = base_session();

    // As a reply: comes back byte-identical to its normal form.
    let mut replying = plain_comment("7f3a", state);
    replying.reply = Some(shape.to_owned());
    let document = render(&session, &[replying]);
    assert_eq!(
        parse_replies(&document),
        vec![("7f3a".to_owned(), normalized(shape))],
        "reply shape {shape:?} did not round-trip:\n{document}"
    );

    // As a comment body with no reply: fabricates nothing.
    let mut quoting = plain_comment("7f3a", state);
    quoting.body = shape.to_owned();
    let document = render(&session, &[quoting]);
    assert_eq!(
        parse_replies(&document),
        Vec::new(),
        "comment shape {shape:?} fabricated a reply:\n{document}"
    );
}

/// Very long lines, kept out of the table above so the case names stay
/// readable: a single 8000-character line, and eighty lines of prose.
#[rstest]
#[case::one_very_long_line(1, 8000)]
#[case::many_lines(80, 40)]
fn extreme_line_shapes_round_trip(#[case] lines: usize, #[case] width: usize) {
    let shape: Vec<String> = (0..lines)
        .map(|index| format!("{index}-{}", "x".repeat(width)))
        .collect();
    let shape = shape.join("\n");

    let mut replying = plain_comment("7f3a", CommentState::AwaitingVerification);
    replying.reply = Some(shape.clone());
    let document = render(&base_session(), &[replying]);

    assert_eq!(
        parse_replies(&document),
        vec![("7f3a".to_owned(), normalized(&shape))],
        "a {lines}x{width} body did not round-trip"
    );
}

/// One case per hostile *context* line: quoted source imitating the grammar
/// must stay quoted, whether or not the entry carries a real reply.
#[rstest]
#[case::reply_marker("**Reply:** a line of the reviewed file")]
#[case::comment_marker("**Comment:** a line of the reviewed file")]
#[case::anchor_marker("<!-- rv:anchor id=dead -->")]
#[case::entry_heading("### 1. `a.rs:1`")]
#[case::section_heading("## Open (1)")]
#[case::details_open("<details><summary>x</summary>")]
#[case::details_close("</details>")]
#[case::summary("<summary>x</summary>")]
#[case::short_fence("```")]
#[case::wide_fence("``````````")]
#[case::tilde_fence("~~~")]
#[case::fence_in_source("const F: &str = \"```\";")]
#[case::empty_line("")]
#[case::already_indented("  indented in the source file")]
fn hostile_context_line_stays_quoted(#[case] context_line: &str) {
    let session = base_session();

    let mut quiet = plain_comment("7f3a", CommentState::Open);
    quiet.anchor.context = vec![
        "before".to_owned(),
        context_line.to_owned(),
        "after".to_owned(),
    ];
    let document = render(&session, &[quiet.clone()]);
    assert_eq!(
        parse_replies(&document),
        Vec::new(),
        "context line {context_line:?} fabricated a reply:\n{document}"
    );

    let mut replying = quiet;
    replying.state = CommentState::AwaitingVerification;
    replying.reply = Some("the real reply".to_owned());
    let document = render(&session, &[replying]);
    assert_eq!(
        parse_replies(&document),
        vec![("7f3a".to_owned(), "the real reply".to_owned())],
        "context line {context_line:?} disturbed the real reply:\n{document}"
    );
}
