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
//! # Why the parser is forgiving
//!
//! The document is handed to a language model and to a human with an editor,
//! and both will mangle it. Nothing either can write may cost a comment, so
//! [`parse_replies`] has no error path at all: unknown prose is skipped, a
//! reply that precedes every anchor marker is dropped rather than
//! mis-attributed to a *later* anchor, a corrupt `id=` clears the binding
//! instead of silently reusing the previous comment's id, and a body that
//! runs into the next entry is truncated at the first structural line rather
//! than swallowing it. `comments.json` — not this file — remains the
//! authority on which comments exist, so the worst case here is a reply that
//! fails to attach, never a comment that disappears.
//!
//! # The `**Reply:**` body rule
//!
//! A reply body is the text after the marker on its own line, plus every
//! following line, up to (excluding) the first **structural** line: a
//! markdown heading (`#…`), an HTML comment (`<!--`, which is how every
//! `rv:anchor` marker starts), a `<details>`/`</details>`/`<summary>` tag, or
//! another `**Comment:**`/`**Reply:**` marker. Blank lines *inside* the body
//! are kept, so a multi-paragraph reply survives whole; leading and trailing
//! blank lines are trimmed. A fenced code block opened inside the body is
//! consumed through its closing fence, so `#[test]`, `# comment` or a quoted
//! `**Reply:**` inside a snippet cannot truncate the reply.
//!
//! Blank lines are not terminators because losing the tail of a reply is a
//! real loss, while the cost of the alternative is cosmetic: stray prose a
//! human leaves directly below a reply is absorbed into that reply's body.
//!
//! Fenced blocks are also skipped *outside* reply bodies, which is what lets
//! `rv` review its own source: an entry's context fence can legitimately
//! contain a line beginning `**Reply:**` or `<!-- rv:anchor id=`, and quoted
//! content must never be read as document structure.

use crate::model::Side;
use crate::store::Comment;
use crate::store::CommentState;
use crate::store::Session;

/// First line of every document: the format version, so a future `rv` can
/// recognize a `v1` file it did not write.
const VERSION_MARKER: &str = "<!-- rv:v1 -->";

/// The opening of an anchor marker, up to and including its `id=`. Anything
/// up to the next whitespace is the comment id.
const ANCHOR_MARKER_PREFIX: &str = "<!-- rv:anchor id=";

/// Lead-in of a rendered comment body, and a structural line for the parser.
const COMMENT_MARKER: &str = "**Comment:**";

/// Lead-in of a reply body: the only thing an LLM may add to the document,
/// and the only thing [`parse_replies`] extracts.
const REPLY_MARKER: &str = "**Reply:**";

/// The protocol block, addressed to the LLM that reads this file. It states
/// the one permitted edit (append a reply) and the two prohibitions that keep
/// the file machine-readable and keep verification human: markers, headings
/// and section order are `rv`'s, and only the human resolves anything — an
/// agent that grades its own homework is how bad fixes land (spec §9).
const PROTOCOL: &str = "> **For LLMs:** fix each open comment, then append a `**Reply:**` block directly\n\
     > beneath it. Do not edit `<!-- rv: -->` markers, headings, or section order.\n\
     > Do not mark anything resolved — the human verifies in the TUI.\n";

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
/// preceding `<!-- rv:anchor id=… -->` marker, in document order.
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

        // Quoted content is not structure: skip fenced blocks wholesale so a
        // context fence containing a marker-like line cannot be read as one.
        if let Some(fence) = fence_open(line) {
            index = skip_fence(&lines, index, fence, None);
            continue;
        }

        if let Some(marker) = anchor_id(line) {
            // An empty id means a corrupt marker: clear the binding rather
            // than letting a following reply land on the previous comment.
            current_id = (!marker.is_empty()).then_some(marker);
            index += 1;
            continue;
        }

        let Some(first) = line.trim_start().strip_prefix(REPLY_MARKER) else {
            index += 1;
            continue;
        };

        let mut body: Vec<&str> = vec![first.trim()];
        index += 1;
        while index < lines.len() {
            let candidate = lines[index];
            if is_structural(candidate) {
                break;
            }
            match fence_open(candidate) {
                Some(fence) => index = skip_fence(&lines, index, fence, Some(&mut body)),
                None => {
                    body.push(candidate);
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

        // A reply above every anchor marker has nothing to bind to; dropping
        // it is the only safe reading, since attaching it to a *later* id
        // would put words in a comment the writer never looked at.
        if let Some(id) = current_id {
            replies.push((id.to_owned(), body.join("\n")));
        }
    }

    replies
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
        out.push_str(&format!("\n{fence}{}\n", fence_language(&anchor.file)));
        for line in &anchor.context {
            out.push_str(line);
            out.push('\n');
        }
        out.push_str(&fence);
        out.push('\n');
    }

    render_labeled(out, COMMENT_MARKER, &comment.body);
    if let Some(reply) = &comment.reply {
        render_labeled(out, REPLY_MARKER, reply);
    }
}

/// Writes a `**Comment:**`/`**Reply:**` block as its own paragraph. An empty
/// `text` renders the bare label rather than a label plus a trailing space.
fn render_labeled(out: &mut String, label: &str, text: &str) {
    out.push('\n');
    out.push_str(label);
    if !text.is_empty() {
        out.push(' ');
        out.push_str(text);
    }
    out.push('\n');
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

/// The comment id in an anchor marker on this line, if it carries one.
///
/// The marker is found anywhere in the line (it may be indented), and the id
/// runs to the next whitespace — with a trailing `-->` stripped, so a marker
/// written without a space before its terminator still reads.
fn anchor_id(line: &str) -> Option<&str> {
    let start = line.find(ANCHOR_MARKER_PREFIX)? + ANCHOR_MARKER_PREFIX.len();
    // `split`, not `split_whitespace`: an `id=` mangled empty must yield an
    // empty id (which clears the binding), not skip ahead to the next field.
    let id = line[start..]
        .split(char::is_whitespace)
        .next()
        .unwrap_or("");
    Some(id.strip_suffix("-->").unwrap_or(id))
}

/// Whether this line ends a reply body: see the body rule in the module docs.
fn is_structural(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.starts_with('#')
        || trimmed.starts_with("<!--")
        || trimmed.starts_with("<details")
        || trimmed.starts_with("</details")
        || trimmed.starts_with("<summary")
        || trimmed.starts_with(COMMENT_MARKER)
        || trimmed.starts_with(REPLY_MARKER)
}

/// The width of the backtick fence this line opens, if it opens one.
fn fence_open(line: &str) -> Option<usize> {
    let width = line
        .trim_start()
        .chars()
        .take_while(|character| *character == '`')
        .count();
    (width >= 3).then_some(width)
}

/// Advances past the fenced block opening at `index`, collecting its lines
/// into `body` when one is given (a fence inside a reply is part of that
/// reply; a fence anywhere else is quoted content to be ignored).
///
/// An unterminated fence consumes the rest of the document — the only reading
/// that cannot mistake quoted text for structure.
fn skip_fence<'a>(
    lines: &[&'a str],
    index: usize,
    fence: usize,
    mut body: Option<&mut Vec<&'a str>>,
) -> usize {
    let mut index = index;
    if let Some(body) = &mut body {
        body.push(lines[index]);
    }
    index += 1;
    while index < lines.len() {
        let line = lines[index];
        if let Some(body) = &mut body {
            body.push(line);
        }
        index += 1;
        let closes = fence_open(line).is_some_and(|width| width >= fence)
            && line.trim().trim_start_matches('`').is_empty();
        if closes {
            break;
        }
    }
    index
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
