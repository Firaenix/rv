//! `.review/REVIEW-FEEDBACK.md`: the review rendered as a page.
//!
//! Pure string work — no filesystem, no jj-lib, no terminal. [`render`] turns a
//! [`Session`] and its [`Comment`]s into the document. The document is a
//! **one-way view** (CLI-loop spec, 2026-08-19): agents read the review with
//! `rv comments --json` and answer with `rv reply`, and nothing reads this file
//! back. The reply parser this module used to carry, and the hostile-input
//! corpus defending it, were deleted with the migration they served.
//!
//! One constraint outlives the parser: **every column-0 line of a rendered
//! document is structure**, because [`render`] indents everything it did not
//! author itself by [`BODY_INDENT`]. It is why a comment quoting the
//! document's own grammar reads as a quotation rather than as a section. The
//! reasoning is in
//! `docs/superpowers/specs/2026-08-17-rv-storage-model-design.md` §10.

use crate::store::Comment;
use crate::store::CommentState;
use crate::store::Session;

mod entry;

/// First line of every document: the format version, so a future `rv` can
/// recognize a `v1` file it did not write.
const VERSION_MARKER: &str = "<!-- rv:v1 -->";

/// Lead-in of a rendered comment body.
const COMMENT_MARKER: &str = "**Comment:**";

/// Lead-in of a rendered reply body.
const REPLY_MARKER: &str = "**Reply:**";

/// What [`render`] prefixes to every continuation line of a comment or reply
/// body, so no interpolated text can occupy column 0.
///
/// Two spaces: enough to push body text off column 0 (where structure lives)
/// and few enough that markdown treats the line as lazy paragraph
/// continuation, an indented fence (up to three spaces is still a fence) or
/// list content — never as an indented code block, which needs four.
const BODY_INDENT: &str = "  ";

/// The one line addressed to a program that finds this file: the document is a
/// **view**, and the CLI is where the review is read and answered.
///
/// It replaces the old `For LLMs:` protocol block whole. That block taught the
/// column-0 `**Reply:**` convention, because appending to this file used to be
/// the reply channel; the CLI-loop amendment made `rv reply` the channel and
/// this file write-only, so the only useful thing to tell a reader is where
/// the real interface lives. The `<!-- rv:anchor -->` markers stay as
/// provenance for the ids they name.
const PROTOCOL: &str = "> This file is a rendered view — nothing reads it back. Read the review with\n\
     > `rv comments --json`, answer with `rv reply <id> -m`, settle with\n\
     > `rv resolve <id>` / `rv abandon <id>`.\n";

/// The five sections, in the fixed order that makes them the state machine:
/// heading title, the state it holds, and the marker prefixed to a collapsed
/// entry's summary (`None` for the expanded sections).
///
/// Every section is rendered even when empty, so the document's shape does not
/// depend on which states happen to be occupied. Every **state** has a section:
/// abandoned comments were silently absent from the document for a while, which
/// is precisely the "dropping a comment is never an acceptable outcome" failure
/// the storage spec forbids — a decision *against* a finding is still part of
/// what the review concluded.
const SECTIONS: [(&str, CommentState, Option<&str>); 5] = [
    ("Open", CommentState::Open, None),
    (
        "Awaiting verification",
        CommentState::AwaitingVerification,
        None,
    ),
    ("Resolved", CommentState::Resolved, Some("✅")),
    ("Abandoned", CommentState::Abandoned, Some("🚫")),
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
/// Body text is never written at column 0: a comment or reply that quotes the
/// document's own grammar is indented past it, so the page a reviewer reads
/// says what the store holds rather than what the text imitates.
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
                None => entry::expanded(&mut out, number, comment),
                Some(marker) => entry::collapsed(&mut out, number, comment, marker),
            }
            number += 1;
        }
    }

    out
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

fn plural(count: usize) -> &'static str {
    if count == 1 { "" } else { "s" }
}
