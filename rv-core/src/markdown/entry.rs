//! One comment as one entry of the document, expanded or collapsed.
//!
//! [`render`](super::render) lays out the page — header, protocol, sections —
//! and calls in here for each comment. Everything below writes into the
//! caller's buffer, and nothing it writes reaches column 0 except the shapes
//! the document's grammar reserves: the entry heading, the `<details>` tags,
//! the anchor marker and the two body labels.

use crate::model::Anchor;
use crate::model::Side;
use crate::store::Comment;

use super::BODY_INDENT;
use super::COMMENT_MARKER;
use super::REPLY_MARKER;

/// How many characters of a comment body are quoted in a collapsed entry's
/// `<summary>` before it is elided, so a collapsed entry is identifiable
/// without expanding it but a long comment cannot blow out the line.
const SUMMARY_BODY_CHARS: usize = 72;

/// An expanded entry: heading, anchor marker, context fence, comment, reply.
pub(super) fn expanded(out: &mut String, number: usize, comment: &Comment) {
    out.push_str(&format!(
        "### {number}. `{}:{}`\n",
        comment.anchor.file, comment.anchor.line
    ));
    body(out, comment);
}

/// A collapsed entry: the same content behind a `<details>`, with the number
/// and location moved into the `<summary>` so it reads without expanding.
///
/// The section heading stays outside, so `## Resolved (4)` is always visible.
pub(super) fn collapsed(out: &mut String, number: usize, comment: &Comment, marker: &str) {
    let excerpt = summary_excerpt(&comment.body);
    let dash = if excerpt.is_empty() { "" } else { " — " };
    // The blank line after `</summary>` closes the raw-HTML block, so the
    // entry inside is parsed as markdown rather than shown as literal text.
    out.push_str(&format!(
        "<details><summary>{marker} {number}. <code>{}:{}</code>{dash}{excerpt}</summary>\n\n",
        escape_html(&comment.anchor.file),
        comment.anchor.line,
    ));
    body(out, comment);
    out.push_str("</details>\n");
}

/// The part of an entry that is identical expanded or collapsed: anchor
/// marker, context fence, comment body, and the reply if one exists.
fn body(out: &mut String, comment: &Comment) {
    let anchor = &comment.anchor;
    // The `hash=` value is either a blake3 hex digest or the
    // `<rv:out-of-range>` sentinel `anchor::create` uses for a line that does
    // not exist. The sentinel's `<`/`>` are harmless here: only the sequence
    // `-->` terminates an HTML comment, and no marker field can contain one —
    // ids and commit/change ids are hex, `side` is an enum, `line` is a
    // number. So the sentinel is written verbatim.
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

    labeled(out, COMMENT_MARKER, &comment.body);
    if let Some(reply) = &comment.reply {
        labeled(out, REPLY_MARKER, reply);
    }
}

/// Writes a `**Comment:**`/`**Reply:**` block as its own paragraph.
///
/// The first line follows the label; every continuation line is prefixed with
/// [`BODY_INDENT`] so that body text can never occupy column 0, where the
/// document's structure lives. Blank lines stay blank rather than becoming
/// trailing whitespace, and an empty `text` renders the bare label rather than
/// a label plus a trailing space.
fn labeled(out: &mut String, label: &str, text: &str) {
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

fn side_str(side: Side) -> &'static str {
    match side {
        Side::Left => "left",
        Side::Right => "right",
    }
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
