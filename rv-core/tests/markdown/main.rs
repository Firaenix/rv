//! Tests for the `.review/REVIEW-FEEDBACK.md` render (spec §10).
//!
//! Pure string work: no repository, no filesystem, no terminal. The document
//! is a one-way view — the reply parser and its hostile-input corpus were
//! deleted with the migration they served (CLI-loop spec §5) — so what these
//! pin is what the page says: every comment present exactly once, sections
//! ordered and counted, and interpolated prose kept off column 0.

mod excerpt;

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
        // `render` takes its comments as an argument, so these fixtures leave
        // the stored array out: the page must not depend on it.
        comments: Vec::new(),
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
        context_start: 1,
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
        settled_by: None,
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

    // The one line addressed to a program that finds this file: the document
    // is a view, and the CLI is the real interface.
    assert!(
        document.contains("rendered view") && document.contains("nothing reads it back"),
        "the view must say it is one:\n{document}"
    );
    assert!(
        document.contains("rv comments --json")
            && document.contains("rv reply")
            && document.contains("rv resolve"),
        "the view must name the CLI that replaced the round trip:\n{document}"
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
    // The context block is indented with the rest of the quoted content, so
    // nothing `rv` did not author itself sits at column 0.
    assert!(
        document.contains("  ```rust\n          if let Some(hit) = idx.find(sym) {\n"),
        "missing indented context fence:\n{document}"
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
/// renders verbatim without terminating the HTML comment early. An empty
/// context renders no fence at all.
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
}

/// A context fence quotes the reviewed file, and everything inside it is
/// indented past column 0 — where the document's structure lives — so a
/// quoted line that imitates the grammar stays a quotation. This is what lets
/// `rv` review its own source without the review page reading its own
/// vocabulary as structure.
#[test]
fn marker_like_lines_inside_a_context_fence_stay_quoted() {
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

    for quoted in [
        "**Reply:** a line of the reviewed file, not of the review",
        "<!-- rv:anchor id=notreal -->",
    ] {
        assert!(
            document.contains(&format!("  {quoted}\n")),
            "quoted source must be indented off column 0:\n{document}"
        );
        assert!(
            !document.contains(&format!("\n{quoted}\n")),
            "quoted source reached column 0, where structure lives:\n{document}"
        );
    }
    assert_eq!(
        count(&document, "\n<!-- rv:anchor "),
        1,
        "only the entry's own marker may sit at column 0:\n{document}"
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

/// An abandoned comment renders, collapsed, in its own section: *dropped
/// unfixed* is part of what the review concluded, and for a while the render
/// silently omitted it — the one outcome the storage spec forbids.
#[test]
fn an_abandoned_comment_renders_in_its_own_section() {
    let dropped = comment(
        "cccc",
        FIRST_CHANGE,
        "rv-core/src/diff.rs",
        7,
        CommentState::Abandoned,
    );
    let document = render(&session(), &[dropped]);

    assert!(
        document.contains("## Abandoned (1)"),
        "no abandoned section:\n{document}"
    );
    assert!(
        document.contains("<details>"),
        "an abandoned entry renders expanded rather than collapsed:\n{document}"
    );
    // The order is fixed: abandoned sits between resolved and outdated.
    assert_before(&document, "## Resolved (0)", "## Abandoned (1)");
    assert_before(&document, "## Abandoned (1)", "## Outdated (0)");
}
