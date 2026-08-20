//! Property-based and parameterized tests for the `REVIEW-FEEDBACK.md` render
//! (`rv_core::markdown`).
//!
//! `tests/markdown.rs` pins exact behaviours against hand-written fixtures.
//! This file attacks the same contract from the other side: it generates
//! *hostile* comment and reply bodies — prose spliced with the document's own
//! grammar — and asserts the invariants the design spec rests on, chiefly
//! that no comment is ever dropped and that nothing a model writes into a body
//! can imitate the document's structure.
//!
//! The reply parser these bodies used to be fed to is gone, and the corpus
//! that defended its mangling tolerance went with it (CLI-loop spec §5). The
//! generators stayed: what they now defend is the render, where a body that
//! reaches column 0 is a body that has become a section heading.
//!
//! # What the oracles are
//!
//! Every property here is checked against something independent of
//! `markdown.rs`:
//!
//! - [`reterminated`] re-derives, from `str::lines`'s stated behaviour, the
//!   body rewrites the page cannot see.
//! - The conservation and section properties count column-0 lines in the
//!   rendered text and compare against counts taken from the *input* comments.
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
use rv_core::model::ChangeRef;
use rv_core::model::Side;
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
        comments: Vec::new(),
    };
    rv_core::markdown::render(&empty, &[])
        .lines()
        .skip_while(|line| !line.starts_with("> "))
        .take_while(|line| line.starts_with("> "))
        .count()
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
        // `render` takes its comments as an argument; the stored array plays
        // no part in the page.
        comments: Vec::new(),
    }
}

// ---------------------------------------------------------------------------
// Oracles
// ---------------------------------------------------------------------------

/// `body` with its line terminators rewritten in the ways `render` cannot see.
///
/// Bodies are written out through `str::lines`, which splits on `\n` and
/// strips a trailing `\r` from each line. Two rewrites therefore leave
/// `lines()` — and so the page — identical: CRLF endings become LF, and a
/// body whose last line has content gains the trailing newline that line's own
/// terminator already supplies.
///
/// Both are guarded, because neither holds unconditionally. A trailing
/// newline added to a body that *already* ends in one appends an empty line
/// (`"a\n"` is one line, `"a\n\n"` is two), and a `\r\n` produced by an
/// isolated `\r` meeting an existing `\n` would swallow a character `lines()
/// keeps. Everything else about the body — blank lines interior, leading or
/// trailing, whitespace, indentation — is left exactly alone: trimming those
/// was the reply parser's rule, and it went with the parser.
fn reterminated(body: &str) -> String {
    let unix = body.replace("\r\n", "\n");
    match unix.chars().last() {
        Some('\n' | '\r') | None => unix,
        Some(_) => format!("{unix}\n"),
    }
}

/// `### <n>. …` — an entry heading, as `render_expanded` writes it.
fn is_entry_heading(line: &str) -> bool {
    line.strip_prefix("### ")
        .is_some_and(|rest| rest.starts_with(|first: char| first.is_ascii_digit()))
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
mod generator;
mod properties;

use generator::context_lines;
use generator::hostile_text;

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
