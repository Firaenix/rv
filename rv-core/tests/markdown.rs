//! Tests for the `.review/REVIEW-FEEDBACK.md` render and reply parser
//! (spec §10).
//!
//! Pure string work: no repository, no filesystem, no terminal. The property
//! the whole round trip rests on is at the bottom —
//! [`render_then_parse_replies_round_trips_every_state`] renders a document
//! containing every lifecycle state and asserts every reply comes back bound
//! to the right comment id.

use rv_core::markdown::parse_replies;
use rv_core::markdown::render;
use rv_core::model::Anchor;
use rv_core::model::ChangeRef;
use rv_core::model::Side;
use rv_core::store::Comment;
use rv_core::store::CommentState;
use rv_core::store::Session;

/// Two changes, in the order `Session.changes` fixes for rendering.
const FIRST_CHANGE: &str = "zzzzaaaabbbb";
const SECOND_CHANGE: &str = "yyyyccccdddd";

fn session() -> Session {
    Session {
        revset: "trunk()..@".to_owned(),
        base_commit: "a1b2c3d4".to_owned(),
        head_commit: "e5f6a7b8".to_owned(),
        changes: vec![
            ChangeRef {
                change_id: FIRST_CHANGE.to_owned(),
                commit_id: "1111aaaa".to_owned(),
                description: "anchor the comments".to_owned(),
            },
            ChangeRef {
                change_id: SECOND_CHANGE.to_owned(),
                commit_id: "2222bbbb".to_owned(),
                description: "render the markdown".to_owned(),
            },
        ],
        started_at: "2026-08-17T14:02Z".to_owned(),
    }
}

fn anchor(file: &str, line: u32) -> Anchor {
    Anchor {
        file: file.to_owned(),
        side: Side::Right,
        line,
        content_hash: "9e21abcd".to_owned(),
        context: vec![
            "        if let Some(hit) = idx.find(sym) {".to_owned(),
            "            return Resolution::exact(hit.start + off.unwrap());".to_owned(),
            "        }".to_owned(),
        ],
    }
}

fn comment(id: &str, change_id: &str, file: &str, line: u32, state: CommentState) -> Comment {
    Comment {
        id: id.to_owned(),
        change_id: change_id.to_owned(),
        commit_id: "a91c40de".to_owned(),
        anchor: anchor(file, line),
        body: "`unwrap()` panics for node-scoped comments, which have no offset.".to_owned(),
        state,
        reply: None,
    }
}

fn with_reply(mut comment: Comment, reply: &str) -> Comment {
    comment.reply = Some(reply.to_owned());
    comment
}

fn count(haystack: &str, needle: &str) -> usize {
    haystack.matches(needle).count()
}

/// Asserts `first` appears before `second`, reporting the whole document when
/// it does not — a positional assertion is useless without the text it failed
/// against.
fn assert_before(document: &str, first: &str, second: &str) {
    let first_at = document
        .find(first)
        .unwrap_or_else(|| panic!("{first:?} missing from:\n{document}"));
    let second_at = document
        .find(second)
        .unwrap_or_else(|| panic!("{second:?} missing from:\n{document}"));
    assert!(
        first_at < second_at,
        "{first:?} should precede {second:?} in:\n{document}"
    );
}

/// An open comment renders fully expanded: version marker, `# Review:` line,
/// base→head line, the LLM protocol block, a counted `## Open` heading, and an
/// entry made of heading, `rv:anchor` marker, context fence and comment body.
#[test]
fn open_renders_expanded_with_anchor_and_protocol() {
    let comments = [comment(
        "7f3a",
        FIRST_CHANGE,
        "rv-core/src/anchor.rs",
        128,
        CommentState::Open,
    )];

    let document = render(&session(), &comments);

    assert!(
        document.starts_with("<!-- rv:v1 -->\n"),
        "document must open with the version marker:\n{document}"
    );
    assert!(
        document.contains("# Review: `trunk()..@` — 2 changes, 1 comment\n"),
        "missing review heading:\n{document}"
    );
    assert!(
        document.contains("Base `a1b2c3d4` → head `e5f6a7b8`"),
        "missing base→head line:\n{document}"
    );
    assert!(
        document.contains("2026-08-17T14:02Z"),
        "missing session start:\n{document}"
    );

    // The protocol block: what the LLM may do, and what only the human may do.
    assert!(
        document.contains("**For LLMs:** fix each open comment"),
        "missing protocol block:\n{document}"
    );
    assert!(
        document.contains("append a `**Reply:**` block directly"),
        "protocol must ask for appended replies:\n{document}"
    );
    assert!(
        document.contains("Do not edit `<!-- rv: -->` markers, headings, or section order."),
        "protocol must forbid editing markers/headings/order:\n{document}"
    );
    assert!(
        document.contains("Do not mark anything resolved — the human verifies in the TUI."),
        "protocol must reserve resolution for the human:\n{document}"
    );

    // All four sections always appear, in order, each with its count.
    assert!(document.contains("## Open (1)"), "{document}");
    assert!(
        document.contains("## Awaiting verification (0)"),
        "{document}"
    );
    assert!(document.contains("## Resolved (0)"), "{document}");
    assert!(document.contains("## Outdated (0)"), "{document}");
    assert_before(&document, "## Open (1)", "## Awaiting verification (0)");
    assert_before(&document, "## Awaiting verification (0)", "## Resolved (0)");
    assert_before(&document, "## Resolved (0)", "## Outdated (0)");

    // The entry itself.
    assert!(
        document.contains("### 1. `rv-core/src/anchor.rs:128`\n"),
        "missing entry heading:\n{document}"
    );
    assert!(
        document.contains(
            "<!-- rv:anchor id=7f3a change=zzzzaaaabbbb commit=a91c40de side=right line=128 \
             hash=9e21abcd -->\n"
        ),
        "missing anchor marker:\n{document}"
    );
    assert!(
        document.contains("```rust\n        if let Some(hit) = idx.find(sym) {\n"),
        "missing context fence:\n{document}"
    );
    assert!(
        document.contains("**Comment:** `unwrap()` panics for node-scoped comments"),
        "missing comment body:\n{document}"
    );
    assert!(
        !document.contains("<details"),
        "open entries must not be collapsed:\n{document}"
    );
    assert!(
        !document.contains("**Reply:**\n") && !document.contains("**Reply:** "),
        "a comment with no reply must not render a reply block:\n{document}"
    );
}

/// Resolved and outdated entries are collapsed behind one `<details>` each,
/// with the section headings themselves left visible above them.
#[test]
fn resolved_and_outdated_render_collapsed() {
    let comments = [
        with_reply(
            comment(
                "aaaa",
                FIRST_CHANGE,
                "rv-core/src/store.rs",
                12,
                CommentState::Resolved,
            ),
            "Made the write write-through.",
        ),
        comment(
            "bbbb",
            FIRST_CHANGE,
            "rv-core/src/diff.rs",
            40,
            CommentState::Outdated,
        ),
    ];

    let document = render(&session(), &comments);

    assert_eq!(
        count(&document, "<details"),
        2,
        "one <details> per collapsed entry:\n{document}"
    );
    assert_eq!(
        count(&document, "</details>"),
        2,
        "every <details> must be closed:\n{document}"
    );
    assert!(document.contains("## Resolved (1)"), "{document}");
    assert!(document.contains("## Outdated (1)"), "{document}");

    // Per-entry summaries carry the presentational number and the location, so
    // a collapsed entry is identifiable without expanding it.
    assert!(
        document.contains("<summary>✅ 1. <code>rv-core/src/store.rs:12</code>"),
        "missing resolved summary:\n{document}"
    );
    assert!(
        document.contains("<summary>⚠️ 2. <code>rv-core/src/diff.rs:40</code>"),
        "missing outdated summary:\n{document}"
    );
    // Section headings stay outside the collapsed blocks.
    assert_before(&document, "## Resolved (1)", "<details>");
    assert_before(&document, "## Outdated (1)", "<summary>⚠️");
    // The anchor marker and body survive collapsing — nothing is dropped.
    assert!(
        document.contains("</summary>\n\n<!-- rv:anchor id=aaaa "),
        "a blank line after </summary> keeps the entry inside markdown:\n{document}"
    );
    assert!(document.contains("<!-- rv:anchor id=bbbb "), "{document}");
    assert!(
        document.contains("**Reply:** Made the write write-through."),
        "{document}"
    );
}

/// Each `**Reply:**` block is attributed to the id of the `rv:anchor` marker
/// above it, in document order.
#[test]
fn replies_parsed_by_id() {
    let document = "\
<!-- rv:v1 -->

## Open (2)

### 1. `a.rs:1`
<!-- rv:anchor id=7f3a change=z commit=c side=right line=1 hash=h -->

**Comment:** first question

**Reply:** fixed by using unwrap_or(0)

### 2. `b.rs:9`
<!-- rv:anchor id=2b81 change=z commit=c side=left line=9 hash=h -->

**Comment:** second question

**Reply:** added a panic hook
";

    let replies = parse_replies(document);

    assert_eq!(
        replies,
        vec![
            ("7f3a".to_owned(), "fixed by using unwrap_or(0)".to_owned()),
            ("2b81".to_owned(), "added a panic hook".to_owned()),
        ]
    );
}

/// Hand edits and LLM mangling are tolerated, never fatal: unknown prose
/// between entries is ignored, a reply written flush against its comment (no
/// blank line, as an LLM is apt to do) still parses, and a stray heading level
/// does not detach the reply from its anchor.
#[test]
fn hand_edited_prose_does_not_break_parsing() {
    let document = "\
<!-- rv:v1 -->

I stopped reviewing here on Tuesday. Ignore this note.

## Open (2)

### 1. `a.rs:1`
<!-- rv:anchor id=7f3a change=z commit=c side=right line=1 hash=h -->

NOTE(nick): I typed this straight into the file.

**Comment:** first question
**Reply:** fixed, flush against the comment

Some trailing prose nobody asked for.

#### an extra heading level someone added

### 2. `b.rs:9`
<!-- rv:anchor id=2b81 change=z commit=c side=right line=9 hash=h -->
**Comment:** second question
**Reply:** also fixed
";

    let replies = parse_replies(document);

    assert_eq!(
        replies,
        vec![
            (
                "7f3a".to_owned(),
                "fixed, flush against the comment\n\nSome trailing prose nobody asked for."
                    .to_owned()
            ),
            ("2b81".to_owned(), "also fixed".to_owned()),
        ],
        "prose must be ignored or absorbed, never fatal"
    );
}

/// The parser binds a reply to the most recent anchor marker it has seen, not
/// to the first one in the file — and a reply that precedes every marker has
/// no id to bind to, so it is dropped rather than panicking or mis-attributed.
#[test]
fn reply_binds_to_nearest_preceding_anchor() {
    let document = "\
<!-- rv:v1 -->

**Reply:** orphan reply written above every anchor

<!-- rv:anchor id=first change=z commit=c side=right line=1 hash=h -->

**Comment:** first

<!-- rv:anchor id=second change=z commit=c side=right line=2 hash=h -->

**Comment:** second

**Reply:** belongs to the second anchor
";

    let replies = parse_replies(document);

    assert_eq!(
        replies,
        vec![(
            "second".to_owned(),
            "belongs to the second anchor".to_owned()
        )],
        "a reply above every anchor must be dropped, not bound to a later id"
    );
}

/// The documented multi-line rule: a reply body runs to the next *structural*
/// line (heading, HTML comment, `<details>`/`<summary>` tag, or another
/// `**Comment:**`/`**Reply:**` marker), keeping interior blank lines, and a
/// fenced block inside the body is consumed whole so structural-looking lines
/// inside the fence cannot truncate the reply.
#[test]
fn multi_line_reply_body_runs_to_the_next_structural_line() {
    let document = "\
<!-- rv:anchor id=7f3a change=z commit=c side=right line=1 hash=h -->

**Comment:** explain the fix

**Reply:** first paragraph of the reply.

second paragraph, after a blank line.

```rust
// #[test] and **Reply:** inside a fence are body text, not structure
fn fixed() {}
```

closing thought.

### 2. `b.rs:9`

prose after the next heading must not be part of the reply
";

    let replies = parse_replies(document);

    assert_eq!(
        replies,
        vec![(
            "7f3a".to_owned(),
            "first paragraph of the reply.\n\nsecond paragraph, after a blank line.\n\n\
             ```rust\n// #[test] and **Reply:** inside a fence are body text, not structure\n\
             fn fixed() {}\n```\n\nclosing thought."
                .to_owned()
        )]
    );
}

/// Within a section, entries order by change index (per `Session.changes`),
/// then path, then line. A comment whose change is not in the session still
/// renders — last in its section — because dropping a comment is never
/// acceptable.
#[test]
fn entries_order_by_change_then_path_then_line() {
    let comments = [
        comment("d", SECOND_CHANGE, "a.rs", 5, CommentState::Open),
        comment("u", "unknownchange", "a.rs", 1, CommentState::Open),
        comment("b", FIRST_CHANGE, "b.rs", 1, CommentState::Open),
        comment("c", FIRST_CHANGE, "a.rs", 9, CommentState::Open),
        comment("a", FIRST_CHANGE, "a.rs", 2, CommentState::Open),
    ];

    let document = render(&session(), &comments);

    assert!(document.contains("## Open (5)"), "{document}");
    assert!(document.contains("### 1. `a.rs:2`\n"), "{document}");
    assert!(document.contains("### 2. `a.rs:9`\n"), "{document}");
    assert!(document.contains("### 3. `b.rs:1`\n"), "{document}");
    assert!(document.contains("### 4. `a.rs:5`\n"), "{document}");
    assert!(document.contains("### 5. `a.rs:1`\n"), "{document}");
    assert!(
        document.contains("<!-- rv:anchor id=u change=unknownchange"),
        "an unknown change must still render:\n{document}"
    );
}

/// The out-of-range sentinel hash contains `<` and `>` but not `-->`, so it
/// renders verbatim without terminating the HTML comment early, and the
/// resulting marker still parses. An empty context renders no fence at all.
#[test]
fn out_of_range_sentinel_hash_survives_the_marker() {
    let mut broken = comment("cafe", FIRST_CHANGE, "gone.rs", 4000, CommentState::Open);
    broken.anchor.content_hash = "<rv:out-of-range>".to_owned();
    broken.anchor.context = Vec::new();
    let comments = [with_reply(broken, "restored the file")];

    let document = render(&session(), &comments);

    assert!(
        document.contains("line=4000 hash=<rv:out-of-range> -->\n"),
        "sentinel must render verbatim inside the marker:\n{document}"
    );
    assert_eq!(
        count(&document, "```"),
        0,
        "an empty context must render no fence:\n{document}"
    );
    assert_eq!(
        parse_replies(&document),
        vec![("cafe".to_owned(), "restored the file".to_owned())],
        "the marker must still be readable:\n{document}"
    );
}

/// A collapsed entry's summary quotes the first line of the comment, elided
/// at a word boundary and HTML-escaped so a body containing markup cannot
/// break out of the `<summary>` element.
#[test]
fn long_summary_excerpt_elides_at_a_word_boundary() {
    let mut wordy = comment("aaaa", FIRST_CHANGE, "a.rs", 1, CommentState::Resolved);
    wordy.body = "this <b>body</b> is long enough that the summary has to elide it \
                  somewhere sensible rather than mid-word"
        .to_owned();

    let document = render(&session(), &[wordy]);

    assert!(
        document.contains(
            "— this &lt;b&gt;body&lt;/b&gt; is long enough that the summary has to elide it…\
             </summary>"
        ),
        "summary must elide at a word boundary and escape markup:\n{document}"
    );
}

/// A context fence quotes the reviewed file, so marker-like lines inside it
/// are content, not structure. This is what lets `rv` review its own source
/// without the review file parsing its own vocabulary.
#[test]
fn marker_like_lines_inside_a_context_fence_are_not_parsed() {
    let mut quoting = comment(
        "7f3a",
        FIRST_CHANGE,
        "rv-core/src/markdown.rs",
        66,
        CommentState::Open,
    );
    quoting.anchor.context = vec![
        "const REPLY_MARKER: &str = \"**Reply:**\";".to_owned(),
        "**Reply:** a line of the reviewed file, not of the review".to_owned(),
        "<!-- rv:anchor id=notreal -->".to_owned(),
    ];

    let document = render(&session(), &[quoting]);

    assert_eq!(
        parse_replies(&document),
        Vec::new(),
        "quoted content must not be read as document structure:\n{document}"
    );
}

/// An `id=` mangled down to nothing clears the binding instead of letting the
/// reply below it land on the previous comment — a dropped reply is
/// recoverable, words put in the wrong comment's mouth are not.
#[test]
fn a_corrupt_anchor_id_drops_the_reply_rather_than_misbinding_it() {
    let document = "\
<!-- rv:anchor id=good change=z commit=c side=right line=1 hash=h -->

**Comment:** first

**Reply:** belongs to good

<!-- rv:anchor id= change=z commit=c side=right line=2 hash=h -->

**Comment:** second

**Reply:** belongs to nobody
";

    let replies = parse_replies(document);

    assert_eq!(
        replies,
        vec![("good".to_owned(), "belongs to good".to_owned())]
    );
}

/// The property the milestone rests on: render a document holding every
/// lifecycle state, feed it straight back to the parser, and every reply comes
/// back bound to the id it was rendered under — collapsed sections included.
#[test]
fn render_then_parse_replies_round_trips_every_state() {
    let comments = [
        comment("0pen", FIRST_CHANGE, "open.rs", 1, CommentState::Open),
        with_reply(
            comment(
                "wait",
                FIRST_CHANGE,
                "await.rs",
                2,
                CommentState::AwaitingVerification,
            ),
            "single line reply",
        ),
        with_reply(
            comment("done", SECOND_CHANGE, "done.rs", 3, CommentState::Resolved),
            "reply with\n\ntwo paragraphs",
        ),
        with_reply(
            comment("olds", SECOND_CHANGE, "old.rs", 4, CommentState::Outdated),
            "reply on an outdated anchor",
        ),
    ];

    let document = render(&session(), &comments);
    let replies = parse_replies(&document);

    assert_eq!(
        replies,
        vec![
            ("wait".to_owned(), "single line reply".to_owned()),
            ("done".to_owned(), "reply with\n\ntwo paragraphs".to_owned()),
            ("olds".to_owned(), "reply on an outdated anchor".to_owned()),
        ],
        "round trip lost or mis-bound a reply:\n{document}"
    );
}
