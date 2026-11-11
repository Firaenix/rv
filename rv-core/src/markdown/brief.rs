//! `rv brief`: the review's unsettled notes as one self-contained prompt.
//!
//! Pure string work like [`super::render`]: the caller resolves each anchor and
//! slices its excerpt out of the blob; this lays the page out and states how to
//! answer. The [`Brief`] value is also what `rv brief --json` serializes, so
//! the field names below are a contract.

use serde::Serialize;
use serde::Serializer;

use crate::model::Anchor;
use crate::model::ChangeRef;
use crate::model::Confidence;
use crate::model::Side;
use crate::store::CommentState;
use crate::store::Session;

use super::BODY_INDENT;
use super::COMMENT_MARKER;
use super::DEGRADED;
use super::REPLY_MARKER;
use super::degraded_base;
use super::entry::fenced;
use super::entry::labeled;
use super::entry::side_str;
use super::plural;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Brief {
    pub session: Header,
    pub notes: Vec<Note>,
    pub protocol: Protocol,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Header {
    pub kind: Scope,
    pub revset: String,
    pub base: String,
    pub head: String,
    pub workspace: String,
    pub started_at: String,
    pub changes: Vec<ChangeRef>,
    pub degraded_base: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Scope {
    Range,
    Tree,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Note {
    pub id: String,
    pub file: String,
    pub line: u32,
    #[serde(serialize_with = "side_name")]
    pub side: Side,
    #[serde(serialize_with = "confidence_name")]
    pub confidence: Confidence,
    pub state: CommentState,
    pub body: String,
    pub replies: Vec<String>,
    pub excerpt: Option<Excerpt>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Excerpt {
    pub source: ExcerptSource,
    /// `0` for a stored excerpt from an anchor written before `context_start`
    /// existed: the numbering is unknown, not guessed.
    pub start_line: u32,
    pub lines: Vec<String>,
}

/// Where an excerpt's lines come from: the side's current blob, or — for an
/// outdated note, whose current text would be a bogus quote — the context the
/// anchor stored when the note was written.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ExcerptSource {
    Head,
    Base,
    Stored,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Protocol {
    pub reply: String,
    pub resolve: String,
    pub abandon: String,
    pub comments: String,
    pub check: String,
}

impl Brief {
    /// Orders the notes by file, then line — the grouping the page renders.
    #[must_use]
    pub fn new(session: Header, mut notes: Vec<Note>, protocol: Protocol) -> Self {
        notes.sort_by(|a, b| a.file.cmp(&b.file).then(a.line.cmp(&b.line)));
        Self {
            session,
            notes,
            protocol,
        }
    }
}

impl Header {
    #[must_use]
    pub fn of(session: &Session, kind: Scope, workspace: &str) -> Self {
        Self {
            kind,
            revset: session.revset.clone(),
            base: session.base_commit.clone(),
            head: session.head_commit.clone(),
            workspace: workspace.to_owned(),
            started_at: session.started_at.clone(),
            changes: session.changes.clone(),
            degraded_base: degraded_base(session).is_some(),
        }
    }
}

impl Excerpt {
    /// `context` lines either side of `line` in `text`, or `None` where `text`
    /// has no such line.
    #[must_use]
    pub fn around(text: &str, line: u32, context: u32, source: ExcerptSource) -> Option<Self> {
        let lines: Vec<&str> = text.lines().collect();
        let count = u32::try_from(lines.len()).unwrap_or(u32::MAX);
        if line == 0 || line > count {
            return None;
        }
        let start = line.saturating_sub(context).max(1);
        let end = line.saturating_add(context).min(count);
        Some(Self {
            source,
            start_line: start,
            lines: lines[start as usize - 1..end as usize]
                .iter()
                .map(|line| (*line).to_owned())
                .collect(),
        })
    }

    #[must_use]
    pub fn stored(anchor: &Anchor) -> Option<Self> {
        (!anchor.context.is_empty()).then(|| Self {
            source: ExcerptSource::Stored,
            start_line: anchor.context_start,
            lines: anchor.context.clone(),
        })
    }

    fn end_line(&self) -> u32 {
        let count = u32::try_from(self.lines.len()).unwrap_or(u32::MAX);
        self.start_line.saturating_add(count.saturating_sub(1))
    }
}

impl Protocol {
    /// The answering commands, each led by `invocation`: `rv` plus whichever
    /// flags select this review, since a bare `rv reply` would reach another.
    #[must_use]
    pub fn new(invocation: &str) -> Self {
        Self {
            reply: format!("{invocation} reply <id> -m <answer>"),
            resolve: format!("{invocation} resolve <id>"),
            abandon: format!("{invocation} abandon <id>"),
            comments: format!("{invocation} comments --json"),
            check: format!("{invocation} status --check"),
        }
    }
}

#[must_use]
pub fn render(brief: &Brief) -> String {
    let mut out = String::new();
    header(&mut out, &brief.session, brief.notes.len());
    if brief.notes.is_empty() {
        out.push_str("\nNo note is waiting on an answer.\n");
    }
    let mut file = None;
    for note in &brief.notes {
        if file != Some(&note.file) {
            out.push_str(&format!("\n## `{}`\n", note.file));
            file = Some(&note.file);
        }
        entry(&mut out, note);
    }
    protocol(&mut out, &brief.protocol, &brief.session.workspace);
    out
}

fn header(out: &mut String, session: &Header, notes: usize) {
    out.push_str(&format!("# Review brief: `{}`\n\n", session.revset));
    let noted = format!("{notes} note{} to answer", plural(notes));
    match session.kind {
        Scope::Range => out.push_str(&format!(
            "Range review, base `{}` → head `{}` · {} change{} · {noted}\n",
            session.base,
            session.head,
            session.changes.len(),
            plural(session.changes.len()),
        )),
        Scope::Tree => out.push_str(&format!(
            "Tree review of every file as it stands at `{}` · {noted}\n",
            session.head
        )),
    }
    out.push_str(&format!(
        "Workspace `{}` · rv {} · started {}\n",
        session.workspace,
        env!("CARGO_PKG_VERSION"),
        session.started_at,
    ));
    if session.degraded_base {
        out.push_str(&format!("\n**Note:** {DEGRADED}\n"));
    }
    if !session.changes.is_empty() {
        out.push_str("\nChanges:\n\n");
        for change in &session.changes {
            let summary = match change.description.lines().next() {
                Some(first) if !first.trim().is_empty() => first,
                _ => "(no description set)",
            };
            out.push_str(&format!("- `{}` {summary}\n", change.change_id));
        }
    }
}

fn entry(out: &mut String, note: &Note) {
    out.push_str(&format!(
        "\n### `{}:{}` — note `{}`\n\nState `{}` · anchor `{}` · {} side\n",
        note.file,
        note.line,
        note.id,
        state_name(note.state),
        note.confidence.as_str(),
        side_str(note.side),
    ));
    let outdated = note.confidence == Confidence::Outdated;
    match &note.excerpt {
        Some(excerpt) => {
            let caption = match excerpt.source {
                ExcerptSource::Stored if excerpt.start_line == 0 => {
                    "Outdated: the code it was written against is gone. As it read then:".to_owned()
                }
                ExcerptSource::Stored => format!(
                    "Outdated: the code it was written against is gone. As it read then, \
                     lines {}–{}:",
                    excerpt.start_line,
                    excerpt.end_line()
                ),
                ExcerptSource::Head | ExcerptSource::Base => format!(
                    "Lines {}–{} at {}; the note is on line {}.",
                    excerpt.start_line,
                    excerpt.end_line(),
                    if excerpt.source == ExcerptSource::Base {
                        "base"
                    } else {
                        "head"
                    },
                    note.line
                ),
            };
            out.push_str(&format!("\n{BODY_INDENT}{caption}\n"));
            fenced(out, &note.file, &excerpt.lines);
        }
        None if outdated => out.push_str(&format!(
            "\n{BODY_INDENT}Outdated: the code it was written against is gone, and no excerpt \
             of it was stored.\n"
        )),
        None => {}
    }
    labeled(out, COMMENT_MARKER, &note.body);
    for reply in &note.replies {
        labeled(out, REPLY_MARKER, reply);
    }
}

fn protocol(out: &mut String, protocol: &Protocol, workspace: &str) {
    out.push_str(&format!(
        "\n## How to answer\n\n\
         Run these from the workspace root, `{workspace}`:\n\n\
         - `{}` stores your answer on a note; `-m -` reads it from stdin. A second reply \
         replaces the first, and replying settles nothing.\n\
         - `{}` settles a note the code now addresses; `{}` drops it unfixed.\n\
         - `{}` re-reads every note; `{}` exits 1 while any is still open.\n\n\
         Line numbers are 1-based in the file at head, or at base for a `left`-side note. \
         An outdated note's code has moved on: answer from its stored excerpt, or abandon it.\n",
        protocol.reply, protocol.resolve, protocol.abandon, protocol.comments, protocol.check,
    ));
}

fn state_name(state: CommentState) -> &'static str {
    match state {
        CommentState::Open => "open",
        CommentState::AwaitingVerification => "awaiting-verification",
        CommentState::Resolved => "resolved",
        CommentState::Abandoned => "abandoned",
        CommentState::Outdated => "outdated",
    }
}

fn side_name<S: Serializer>(side: &Side, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(side_str(*side))
}

fn confidence_name<S: Serializer>(
    confidence: &Confidence,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(confidence.as_str())
}
