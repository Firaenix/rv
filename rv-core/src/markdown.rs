//! `.review/REVIEW-FEEDBACK.md`: the round-trip surface between a human
//! reviewer and an LLM (spec §10).
//!
//! This module is pure string work — no filesystem, no jj-lib, no terminal.
//! [`render`] turns a [`Session`] plus its [`Comment`]s into the markdown
//! document, and [`parse_replies`] reads back the one thing an LLM is allowed
//! to add to that document: `**Reply:**` blocks. Ownership is disjoint by
//! design (`rv` writes entries and anchors, the LLM only appends replies), so
//! the parser deliberately extracts *only* replies and ignores everything
//! else it meets.
//!
//! # Structure lives at column 0
//!
//! The document interpolates reviewer- and LLM-written prose, and quoted
//! source from the repository under review, into a structure-sensitive
//! grammar — so content must not be able to imitate structure. The
//! separation is positional: **in a rendered document, every column-0 line is
//! structure**, because [`render`] indents everything it did not author
//! itself by [`BODY_INDENT`] — continuation lines of comment and reply
//! bodies, and whole context fences.
//!
//! Two spaces is enough to leave column 0 and few enough that markdown
//! renders the result identically (lazy paragraph continuation; a fenced
//! block strips its opener's indent from each line). A reviewer quoting the
//! protocol (`**Reply:** …` as the second line of a comment — exactly what
//! happens when `rv` reviews this file) therefore cannot fabricate a reply,
//! and a body or a quoted line reading `<!-- rv:anchor id=… -->` cannot
//! rebind the parser to a comment that does not exist. [`parse_replies`]
//! removes the indent again, so a rendered body parses back byte-identical.
//!
//! # Why the parser is forgiving
//!
//! The document is handed to a language model and to a human with an editor,
//! and both will mangle it. Nothing either can write may cost a comment, so
//! [`parse_replies`] has no error path at all. Its rules, in the order they
//! matter:
//!
//! - **A reply binds only within its own entry.** Every entry boundary — a
//!   `### <n>.` entry heading, a `## ` section heading, a `<details>` — clears
//!   the binding. A marker that was deleted, indented, or garbled beyond
//!   recognition therefore leaves the following reply bound to *nothing*, and
//!   it is dropped rather than silently attributed to the entry above it.
//! - **Marker fields are read by name, not position.** `<!--rv:anchor id=…`,
//!   `<!-- rv:anchor  id=…` and a reordered `<!-- rv:anchor change=… id=… -->`
//!   all read. A line that announces itself as `rv:anchor` but carries no
//!   readable `id=` clears the binding, for the same reason as above.
//! - **A reply above every marker is dropped**, never bound to a *later* id.
//! - **An unbalanced fence is text, not a fence.** A fence only counts when
//!   its closing partner sits in the same region — no entry boundary and no
//!   column-0 `**Comment:**`/`**Reply:**` marker in between (see
//!   [`bounds_fence`]). Otherwise the opener is an ordinary line. One stray
//!   ` ``` ` in a comment body can therefore neither swallow the entry's own
//!   reply by pairing with a fence *inside* it, nor reach across an entry
//!   boundary to pair with the next entry's context fence.
//!
//! `comments.json` — not this file — remains the authority on which comments
//! exist, so the worst case here is a reply that fails to attach, never a
//! comment that disappears.
//!
//! M2: nothing in this milestone consumes [`parse_replies`] (spec §14 puts
//! reply consumption and state transitions in Milestone 2). When it does, M2
//! needs two things this signature cannot express: a diagnostics channel
//! alongside the replies (so a marker the parser had to give up on is
//! reported rather than silently dropped), and an "Unattached replies"
//! section that [`render`] preserves, so an LLM's work is not erased by the
//! next render when it fails to bind.
//!
//! # The `**Reply:**` body rule
//!
//! A reply body is the text after the marker on its own line, plus every
//! following line, up to (excluding) the first **structural** line — see
//! [`is_structural`]: an entry or section heading, an HTML comment, a
//! `<details>`/`</details>`/`<summary>` tag, or another
//! `**Comment:**`/`**Reply:**` marker, each at column 0. Blank lines *inside*
//! the body are kept, so a multi-paragraph reply survives whole; leading and
//! trailing blank lines are trimmed. A balanced fenced block inside the body
//! is consumed whole, so structural-looking lines inside a snippet cannot
//! truncate the reply.
//!
//! Only the heading levels [`render`] actually emits terminate a body: `## `
//! and a numbered `### <n>.`. An LLM writing `### What I changed` or a
//! `# shell comment` inside its reply keeps them, because truncating a reply
//! loses work that goes nowhere — the tail would be attributed to nothing and
//! erased by the next render.
//!
//! Blank lines are not terminators for the same reason, and the cost of that
//! choice is cosmetic: stray prose a human leaves directly below a reply is
//! absorbed into that reply's body.

use crate::model::Side;
use crate::store::Comment;
use crate::model::Anchor;
use crate::store::CommentState;
use crate::store::Session;

/// First line of every document: the format version, so a future `rv` can
/// recognize a `v1` file it did not write.
const VERSION_MARKER: &str = "<!-- rv:v1 -->";

/// What a line must contain, on top of opening an HTML comment at column 0,
/// to be an anchor marker. The `id=` field is then found by name, so
/// reordered or reformatted fields still read.
const ANCHOR_MARKER_TAG: &str = "rv:anchor";

/// Lead-in of a rendered comment body, and a structural line for the parser.
const COMMENT_MARKER: &str = "**Comment:**";

/// Lead-in of a reply body: the only thing an LLM may add to the document,
/// and the only thing [`parse_replies`] extracts.
const REPLY_MARKER: &str = "**Reply:**";

/// What [`render`] prefixes to every continuation line of a comment or reply
/// body, and what [`parse_replies`] removes again.
///
/// Two spaces: enough to push body text off column 0 (where structure lives)
/// and few enough that markdown treats the line as lazy paragraph
/// continuation, an indented fence (up to three spaces is still a fence) or
/// list content — never as an indented code block, which needs four.
const BODY_INDENT: &str = "  ";

/// The protocol block, addressed to the LLM that reads this file: the one edit
/// it may make, and the one it may not.
///
/// **Replies are appended; structure is `rv`'s.** Markers, headings and section
/// order are what make the file machine-readable, and a reply is the only thing
/// the parser goes looking for.
///
/// It no longer forbids resolving. It used to, on the grounds that an agent
/// grading its own homework is how bad fixes land — but the ban only pushed the
/// act into prose nobody reads. The storage model now records **who** settled a
/// comment and shows it, so an agent may resolve its own finding and the file
/// and the screen both say it was the agent (storage spec §3). That is the safe
/// half of the original rule kept and the unenforceable half dropped: the danger
/// was never the resolving, it was the resolving going unnoticed. Editing the
/// state *here* is still out — `session.toml` is the authority, and a state
/// written into the export would be overwritten by the next render.
const PROTOCOL: &str = "> **For LLMs:** fix each open comment, then append a `**Reply:**` block directly\n\
     > beneath it, with the `**Reply:**` marker at the start of the line — never\n\
     > indented, never inside a list item, or the reply is not read.\n\
     > Do not edit `<!-- rv: -->` markers, headings, or section order, and do not\n\
     > write a state into this file — `rv` resolves and abandons, and records who\n\
     > did.\n";

/// How many characters of a comment body are quoted in a collapsed entry's
/// `<summary>` before it is elided, so a collapsed entry is identifiable
/// without expanding it but a long comment cannot blow out the line.
const SUMMARY_BODY_CHARS: usize = 72;

/// The four sections, in the fixed order that makes them the state machine:
/// heading title, the state it holds, and the marker prefixed to a collapsed
/// entry's summary (`None` for the expanded sections).
///
/// Every section is rendered even when empty, so the document's shape — and
/// therefore the instruction "do not edit … section order" — does not depend
/// on which states happen to be occupied.
const SECTIONS: [(&str, CommentState, Option<&str>); 4] = [
    ("Open", CommentState::Open, None),
    (
        "Awaiting verification",
        CommentState::AwaitingVerification,
        None,
    ),
    ("Resolved", CommentState::Resolved, Some("✅")),
    ("Outdated", CommentState::Outdated, Some("⚠️")),
];

/// Renders the whole `REVIEW-FEEDBACK.md` document.
///
/// The header states the session's revset, its change and comment counts, the
/// base→head pair, the crate version and `session.started_at` (rendered
/// verbatim — the store treats it as an opaque string). The second count is
/// comments rather than the changed-file count of spec §10's example, since a
/// [`Session`] carries no file list and an invented figure is worse than an
/// honest one.
///
/// Sections come in [`SECTIONS`] order; within a section, entries are ordered
/// by the comment's change index in `session.changes`, then path, then line
/// (spec §10). A comment whose `change_id` is not in `session.changes` — a
/// change abandoned or rewritten out of the session, say — sorts *last within
/// its section* and still renders: dropping a comment is never an acceptable
/// outcome. Entries are numbered `1..` across the whole document in render
/// order; that number is presentational, and the `id=` in the anchor marker
/// is the stable identity.
///
/// Body text is never written at column 0 (see the module docs), so
/// `parse_replies(&render(session, comments))` returns exactly the replies
/// `comments` carried, in the order rendered, with byte-identical bodies —
/// including bodies holding fenced code, markers, headings, or an unbalanced
/// fence.
///
/// The claim holds for bodies that are already line-normalized, which is
/// exactly what `parse_replies` itself produces. Two normalizations happen on
/// the first pass through and are then stable: leading and trailing *blank
/// lines* of a body are dropped (the document uses a blank line as the
/// separator before the next structural element, so they are not
/// recoverable), and `\r\n` line endings come back as `\n`, since the parser
/// splits with `str::lines`.
pub fn render(session: &Session, comments: &[Comment]) -> String {
    let mut out = String::new();

    out.push_str(VERSION_MARKER);
    out.push('\n');
    out.push_str(&format!(
        "# Review: `{}` — {} change{}, {} comment{}\n",
        session.revset,
        session.changes.len(),
        plural(session.changes.len()),
        comments.len(),
        plural(comments.len()),
    ));
    out.push_str(&format!(
        "Base `{}` → head `{}` · rv {} · {}\n",
        session.base_commit,
        session.head_commit,
        env!("CARGO_PKG_VERSION"),
        session.started_at,
    ));
    if let Some(note) = degraded_base(session) {
        out.push_str(&note);
    }
    out.push('\n');
    out.push_str(PROTOCOL);

    let mut number = 1;
    for (title, state, collapsed_marker) in SECTIONS {
        let mut section: Vec<&Comment> = comments
            .iter()
            .filter(|comment| comment.state == state)
            .collect();
        // Stable sort: comments that tie on (change, path, line) keep the
        // order they were stored in.
        section.sort_by(|a, b| {
            change_index(session, &a.change_id)
                .cmp(&change_index(session, &b.change_id))
                .then_with(|| a.anchor.file.cmp(&b.anchor.file))
                .then_with(|| a.anchor.line.cmp(&b.anchor.line))
        });

        out.push_str(&format!("\n## {title} ({})\n", section.len()));
        for comment in section {
            out.push('\n');
            match collapsed_marker {
                None => render_expanded(&mut out, number, comment),
                Some(marker) => render_collapsed(&mut out, number, comment, marker),
            }
            number += 1;
        }
    }

    out
}

/// Extracts every `**Reply:**` block, paired with the id of the nearest
/// preceding `<!-- rv:anchor id=… -->` marker *within the same entry*, in
/// document order.
///
/// A comment with two reply blocks yields two pairs, both under its id and in
/// the order written — this function reports what the document says and
/// leaves precedence to the caller. Never panics, never errors: see the
/// module docs for the tolerated-mangling contract and the body rule.
pub fn parse_replies(document: &str) -> Vec<(String, String)> {
    let lines: Vec<&str> = document.lines().collect();
    let mut replies = Vec::new();
    let mut current_id: Option<&str> = None;
    let mut index = 0;

    while index < lines.len() {
        let line = lines[index];

        // Quoted content is not structure: skip balanced fenced blocks
        // wholesale so a context fence containing a marker-like line cannot
        // be read as one. An *unbalanced* fence is left to fall through as
        // ordinary text rather than eating the rest of the document.
        if let Some(closing) = balanced_fence(&lines, index) {
            index = closing + 1;
            continue;
        }

        // An entry boundary ends the previous entry's binding, so a reply can
        // never attach across it however mangled the markers in between are.
        if is_entry_boundary(line) {
            current_id = None;
            index += 1;
            continue;
        }

        if let Some(id) = anchor_id(line) {
            // A marker with no readable id is corrupt: clear the binding
            // rather than letting a following reply land on the entry above.
            current_id = (!id.is_empty()).then_some(id);
            index += 1;
            continue;
        }

        let Some(first) = line.strip_prefix(REPLY_MARKER) else {
            index += 1;
            continue;
        };

        // Exactly the single space `render` writes after the marker, so a
        // body whose first line is itself indented round-trips unchanged.
        let mut body: Vec<&str> = vec![first.strip_prefix(' ').unwrap_or(first)];
        index += 1;
        while index < lines.len() {
            let candidate = lines[index];
            if is_structural(candidate) {
                break;
            }
            match balanced_fence(&lines, index) {
                Some(closing) => {
                    body.extend(lines[index..=closing].iter().map(|line| dedent(line)));
                    index = closing + 1;
                }
                None => {
                    body.push(dedent(candidate));
                    index += 1;
                }
            }
        }
        while body.first().is_some_and(|line| line.trim().is_empty()) {
            body.remove(0);
        }
        while body.last().is_some_and(|line| line.trim().is_empty()) {
            body.pop();
        }

        // A reply outside every entry has nothing to bind to; dropping it is
        // the only safe reading, since attaching it to another id would put
        // words in a comment the writer never looked at.
        if let Some(id) = current_id {
            replies.push((id.to_owned(), body.join("\n")));
        }
    }

    replies
}

/// What a degraded `trunk()` means, in one sentence, for whoever is reading.
pub const DEGRADED: &str = "`trunk()` resolved to the repository root — this repo has no \
     `origin`/`upstream` main, master or trunk bookmark — so the range is the whole history \
     and every file reads as an addition.";

/// A line naming the case where the range is not what the revset suggests.
///
/// `trunk()` is a union of the usual remote bookmarks *and the repository root*,
/// so in a repo with no remote it resolves to the root and `trunk()..@` becomes
/// the whole history. The header then reads `trunk()..@` over an all-zero base
/// with every file marked added, and a model handed that document cannot tell a
/// whole-repo dump from a real branch review — nor can a reviewer tell why
/// everything is a `+`.
///
/// The revset records what the user *typed*; this names what it *resolved to*,
/// which is the difference the finding was about.
#[must_use]
pub fn degraded_base(session: &Session) -> Option<String> {
    let root = session.base_commit.chars().all(|c| c == '0');
    let asked_for_trunk = session.revset.starts_with("trunk()");
    (root && asked_for_trunk).then(|| {
        // Not a blockquote: the protocol block is the *only* quoted run at column
        // 0, which is a property the parser's shape rules rest on and worth more
        // than the indentation.
        format!("**Note:** {DEGRADED}\n")
    })
}

/// Which lines the excerpt below covers, and which of them the comment is about.
///
/// The excerpt is up to eleven lines and its target is **not** reliably the
/// middle one: [`crate::anchor::snapshot_of`] clamps at the edges, so the
/// commented line is the sixth row in the middle of a file and the third near the
/// top. Nothing in the document said which, and a reviewer reading the finished
/// export reported that as the one thing they could not resolve from the file
/// alone — which matters most in exactly the case the excerpt exists for, where
/// the file has moved on and cannot be consulted.
///
/// # Why a caption rather than a numbered gutter
///
/// Numbering each row inside the fence would answer the same question and cost
/// two things worth more: the quoted text would stop being verbatim, and it is
/// stored precisely so a later reader can check it against the revision it came
/// from; and a fence whose every line carries `238 │ ` is no longer code anyone
/// can copy or any viewer can highlight. The caption states the mapping and
/// leaves the snapshot alone.
///
/// `None` for an anchor written before `context_start` existed, where the mapping
/// is genuinely unknown — an unnumbered excerpt is honest, and a guessed number
/// is worse than none.
fn excerpt_caption(anchor: &Anchor) -> Option<String> {
    let start = anchor.context_start;
    if start == 0 {
        return None;
    }
    let count = u32::try_from(anchor.context.len()).unwrap_or(u32::MAX);
    let end = start.saturating_add(count.saturating_sub(1));
    let row = anchor.line.checked_sub(start)?.saturating_add(1);
    Some(format!(
        "Lines {start}–{end}; the comment is on line {} — row {row} of {count} below.",
        anchor.line
    ))
}

/// An expanded entry: heading, anchor marker, context fence, comment, reply.
fn render_expanded(out: &mut String, number: usize, comment: &Comment) {
    out.push_str(&format!(
        "### {number}. `{}:{}`\n",
        comment.anchor.file, comment.anchor.line
    ));
    render_body(out, comment);
}

/// A collapsed entry: the same content behind a `<details>`, with the number
/// and location moved into the `<summary>` so it reads without expanding.
///
/// The section heading stays outside, so `## Resolved (4)` is always visible.
fn render_collapsed(out: &mut String, number: usize, comment: &Comment, marker: &str) {
    let excerpt = summary_excerpt(&comment.body);
    let dash = if excerpt.is_empty() { "" } else { " — " };
    // The blank line after `</summary>` closes the raw-HTML block, so the
    // entry inside is parsed as markdown rather than shown as literal text.
    out.push_str(&format!(
        "<details><summary>{marker} {number}. <code>{}:{}</code>{dash}{excerpt}</summary>\n\n",
        escape_html(&comment.anchor.file),
        comment.anchor.line,
    ));
    render_body(out, comment);
    out.push_str("</details>\n");
}

/// The part of an entry that is identical expanded or collapsed: anchor
/// marker, context fence, comment body, and the reply if one exists.
fn render_body(out: &mut String, comment: &Comment) {
    let anchor = &comment.anchor;
    // The `hash=` value is either a blake3 hex digest or the
    // `<rv:out-of-range>` sentinel `anchor::create` uses for a line that does
    // not exist. The sentinel's `<`/`>` are harmless here: only the sequence
    // `-->` terminates an HTML comment, and no marker field can contain one —
    // ids and commit/change ids are hex, `side` is an enum, `line` is a
    // number. So the sentinel is written verbatim, which keeps the marker
    // byte-identical to what `parse_replies` reads back.
    out.push_str(&format!(
        "<!-- rv:anchor id={} change={} commit={} side={} line={} hash={} -->\n",
        comment.id,
        comment.change_id,
        comment.commit_id,
        side_str(anchor.side),
        anchor.line,
        anchor.content_hash,
    ));

    if !anchor.context.is_empty() {
        if let Some(caption) = excerpt_caption(anchor) {
            out.push_str(&format!("\n{BODY_INDENT}{caption}\n"));
        }
        // One backtick longer than the longest run in the context (never
        // shorter than three), so quoted code that itself contains a fence
        // cannot close this one early.
        let longest = anchor
            .context
            .iter()
            .map(|line| longest_backtick_run(line))
            .max()
            .unwrap_or(0);
        let fence = "`".repeat((longest + 1).max(3));
        // The whole block — opener, quoted lines and closer — is indented by
        // BODY_INDENT, for the same reason bodies are: nothing quoted may
        // reach column 0. Markdown strips indentation equal to the opening
        // fence's from each content line, so the code renders unchanged.
        out.push_str(&format!(
            "\n{BODY_INDENT}{fence}{}\n",
            fence_language(&anchor.file)
        ));
        for line in &anchor.context {
            if !line.is_empty() {
                out.push_str(BODY_INDENT);
            }
            out.push_str(line);
            out.push('\n');
        }
        out.push_str(BODY_INDENT);
        out.push_str(&fence);
        out.push('\n');
    }

    render_labeled(out, COMMENT_MARKER, &comment.body);
    if let Some(reply) = &comment.reply {
        render_labeled(out, REPLY_MARKER, reply);
    }
}

/// Writes a `**Comment:**`/`**Reply:**` block as its own paragraph.
///
/// The first line follows the label; every continuation line is prefixed with
/// [`BODY_INDENT`] so that body text can never occupy column 0, where the
/// parser looks for structure (see the module docs). Blank lines stay blank
/// rather than becoming trailing whitespace, and an empty `text` renders the
/// bare label rather than a label plus a trailing space.
fn render_labeled(out: &mut String, label: &str, text: &str) {
    out.push('\n');
    out.push_str(label);
    for (position, line) in text.lines().enumerate() {
        if position > 0 {
            out.push('\n');
            if !line.is_empty() {
                out.push_str(BODY_INDENT);
            }
        } else if !line.is_empty() {
            out.push(' ');
        }
        out.push_str(line);
    }
    out.push('\n');
}

/// Removes the one [`BODY_INDENT`] [`render_labeled`] adds, and nothing more:
/// a hand-written line that never had it is returned untouched.
fn dedent(line: &str) -> &str {
    line.strip_prefix(BODY_INDENT).unwrap_or(line)
}

/// Where `change_id` sits in the session's change order, or [`usize::MAX`] for
/// a change the session does not list — which sorts such comments last
/// instead of dropping them.
fn change_index(session: &Session, change_id: &str) -> usize {
    session
        .changes
        .iter()
        .position(|change| change.change_id == change_id)
        .unwrap_or(usize::MAX)
}

fn side_str(side: Side) -> &'static str {
    match side {
        Side::Left => "left",
        Side::Right => "right",
    }
}

fn plural(count: usize) -> &'static str {
    if count == 1 { "" } else { "s" }
}

/// The comment id carried by an anchor marker on this line, or `Some("")` if
/// the line announces itself as one but has no readable `id=` (which clears
/// the parser's binding). `None` means this is not a marker line at all.
///
/// The marker must open an HTML comment at column 0 — indented or quoted text
/// is content, not structure — but everything after that is read leniently:
/// fields are matched **by name**, so `<!--rv:anchor id=7f3a-->` and a
/// reordered `<!-- rv:anchor change=z id=7f3a -->` both read, and a trailing
/// `-->` stuck to the value is stripped.
fn anchor_id(line: &str) -> Option<&str> {
    if !line.starts_with("<!--") || !line.contains(ANCHOR_MARKER_TAG) {
        return None;
    }
    let id = line
        .split_whitespace()
        .find_map(|field| field.strip_prefix("id="))
        .map(|id| id.strip_suffix("-->").unwrap_or(id))
        .unwrap_or("");
    Some(id)
}

/// Whether this line starts a new entry (or section), which clears the
/// parser's current binding: no reply may ever attach across one.
fn is_entry_boundary(line: &str) -> bool {
    is_entry_heading(line) || is_section_heading(line) || line.starts_with("<details")
}

/// Whether a fence may not span this line, because it opens a new region the
/// fence cannot belong to: an entry boundary, or the `**Comment:**` /
/// `**Reply:**` label that starts a body.
///
/// Both halves prevent a *loss*. Without the entry-boundary half, an
/// unbalanced fence pairs with some later entry's context fence, skips the
/// headings and markers in between, and carries a stale binding into the next
/// entry. Without the body-label half, an unbalanced fence in a comment body
/// pairs with the closing fence of its *own* entry's reply, swallowing the
/// `**Reply:**` marker and losing that reply — a reviewer pasting a partial
/// snippet into a comment is ordinary, and fenced code in replies is the
/// common case.
///
/// Rendered documents keep every quoted and authored line off column 0, so
/// none of this can fire on a fence [`render`] wrote. In a hand-edited
/// document a fence whose *contents* hold a column-0 body label is read as
/// text instead — truncation inside one entry, never a reply attached to the
/// wrong comment.
fn bounds_fence(line: &str) -> bool {
    is_entry_boundary(line) || line.starts_with(COMMENT_MARKER) || line.starts_with(REPLY_MARKER)
}

/// `### <n>. …` — the shape [`render_expanded`] writes. A heading an LLM
/// writes inside a reply (`### What I changed`) is deliberately not one.
fn is_entry_heading(line: &str) -> bool {
    line.strip_prefix("### ")
        .is_some_and(|rest| rest.starts_with(|first: char| first.is_ascii_digit()))
}

/// `## <title> (<n>)` — the shape [`render`] writes for a section.
fn is_section_heading(line: &str) -> bool {
    line.starts_with("## ")
}

/// Whether this line ends a reply body: see the body rule in the module docs.
///
/// Every test is anchored at column 0. Body text is indented past it by
/// [`render_labeled`], so a comment or reply quoting any of these markers
/// cannot terminate — or fabricate — anything.
fn is_structural(line: &str) -> bool {
    line.starts_with("<!--")
        || line.starts_with("<details")
        || line.starts_with("</details")
        || line.starts_with("<summary")
        || line.starts_with(COMMENT_MARKER)
        || line.starts_with(REPLY_MARKER)
        || is_entry_heading(line)
        || is_section_heading(line)
}

/// A fence marker character and the number of times it repeats.
#[derive(Clone, Copy)]
struct Fence {
    marker: char,
    width: usize,
}

/// The fence this line opens, if it opens one. Both markdown fence
/// characters count: an unrecognized `~~~` fence would let its contents be
/// read as structure, which is the same hazard as an unbalanced one.
fn fence_open(line: &str) -> Option<Fence> {
    let trimmed = line.trim_start();
    let marker = trimmed.chars().next()?;
    if marker != '`' && marker != '~' {
        return None;
    }
    let width = trimmed.chars().take_while(|char| *char == marker).count();
    (width >= 3).then_some(Fence { marker, width })
}

/// The index of the line closing the fence opened at `open`, or `None` if the
/// line opens no fence or nothing closes it before the end of the document.
///
/// Scanning ahead before committing is what keeps one unbalanced fence — in a
/// hand-written comment or a truncated LLM reply — from swallowing every
/// anchor and reply after it. With no closing partner the opener is simply
/// ordinary text, and the document past it still parses.
fn balanced_fence(lines: &[&str], open: usize) -> Option<usize> {
    let fence = fence_open(lines[open])?;
    for (index, line) in lines.iter().enumerate().skip(open + 1) {
        if bounds_fence(line) {
            return None;
        }
        let trimmed = line.trim();
        let width = trimmed
            .chars()
            .take_while(|char| *char == fence.marker)
            .count();
        // A closing fence is at least as long as its opener and holds
        // nothing else — no info string.
        if width >= fence.width && trimmed.chars().count() == width {
            return Some(index);
        }
    }
    None
}

/// The longest run of consecutive backticks anywhere in `line` — what a
/// context fence has to out-length to stay closed.
fn longest_backtick_run(line: &str) -> usize {
    let mut longest = 0;
    let mut run = 0;
    for character in line.chars() {
        if character == '`' {
            run += 1;
            longest = longest.max(run);
        } else {
            run = 0;
        }
    }
    longest
}

/// The info string for an entry's context fence, by file extension, or `""`
/// for an extension with no obvious highlighting language. Presentational
/// only — nothing parses it.
fn fence_language(file: &str) -> &'static str {
    let name = file.rsplit('/').next().unwrap_or(file);
    match name.rsplit_once('.').map(|(_, extension)| extension) {
        Some("rs") => "rust",
        Some("toml") => "toml",
        Some("md") => "markdown",
        Some("json") => "json",
        Some("yaml" | "yml") => "yaml",
        Some("py") => "python",
        Some("ts") => "typescript",
        Some("tsx") => "tsx",
        Some("js" | "mjs" | "cjs") => "javascript",
        Some("go") => "go",
        Some("sh" | "bash" | "zsh") => "bash",
        Some("sql") => "sql",
        Some("html") => "html",
        Some("css") => "css",
        _ => "",
    }
}

/// The first non-blank line of `body`, HTML-escaped and elided past
/// [`SUMMARY_BODY_CHARS`], for a collapsed entry's `<summary>`.
fn summary_excerpt(body: &str) -> String {
    let first = body.lines().map(str::trim).find(|line| !line.is_empty());
    let Some(first) = first else {
        return String::new();
    };
    if first.chars().count() <= SUMMARY_BODY_CHARS {
        return escape_html(first);
    }
    let kept: String = first.chars().take(SUMMARY_BODY_CHARS).collect();
    // Elide at the last word boundary inside the budget, so a summary never
    // ends mid-word; a single long word has none and is cut where it lands.
    let kept = match kept.rsplit_once(char::is_whitespace) {
        Some((head, _)) if !head.is_empty() => head,
        _ => kept.trim_end(),
    };
    escape_html(&format!("{kept}…"))
}

/// Escapes the three characters that would otherwise be read as markup where
/// text is interpolated into an HTML element (`<summary>`, `<code>`).
fn escape_html(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}
