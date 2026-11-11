//! The handoff bundle `rv brief` prints: every unsettled note with the code
//! around it as it stands now, and the commands that answer it.

use anyhow::Context as _;
use anyhow::Result;
use rv_core::markdown::brief::Brief;
use rv_core::markdown::brief::Excerpt;
use rv_core::markdown::brief::ExcerptSource;
use rv_core::markdown::brief::Header;
use rv_core::markdown::brief::Note;
use rv_core::markdown::brief::Protocol;
use rv_core::markdown::brief::Scope;
use rv_core::model::Confidence;
use rv_core::model::Side;
use rv_core::store::Comment;
use rv_core::store::CommentState;

use super::Review;
use crate::stale;

/// The brief as a value, before it is printed — what `rv brief --ask` and
/// the reviewer's `B` hand to a configured agent.
///
/// Reads the store and the blobs and writes nothing, `session.toml` included.
/// Settled notes are left out: a brief is the work still to answer.
pub fn assemble(review: &Review, context: u32) -> Result<Brief> {
    let mut comments = super::in_range(
        review,
        review
            .store
            .comments()
            .context("could not read the review's comments")?,
    );
    let drifts = stale::survey(review, &mut comments);
    let notes = comments
        .into_iter()
        .filter(|comment| {
            matches!(
                comment.state,
                CommentState::Open | CommentState::AwaitingVerification | CommentState::Outdated
            )
        })
        .map(|comment| {
            let confidence = drifts
                .get(&comment.id)
                .map_or(Confidence::Exact, |drift| drift.confidence);
            note(review, comment, confidence, context)
        })
        .collect();
    let root = review.store.root();
    let workspace = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let scope = if review.is_tree() {
        Scope::Tree
    } else {
        Scope::Range
    };
    Ok(Brief::new(
        Header::of(&review.session, scope, &workspace.display().to_string()),
        notes,
        Protocol::new(&invocation(review)),
    ))
}

/// `comment` as the brief shows it, its anchor already placed by
/// [`stale::survey`]. An outdated note quotes what it stored, never the text
/// now standing at its old line number.
fn note(review: &Review, comment: Comment, confidence: Confidence, context: u32) -> Note {
    let outdated = comment.state == CommentState::Outdated;
    let excerpt = if outdated {
        Excerpt::stored(&comment.anchor)
    } else {
        let source = match comment.anchor.side {
            Side::Left => ExcerptSource::Base,
            Side::Right => ExcerptSource::Head,
        };
        stale::anchored_text(review, &comment.anchor)
            .and_then(|text| Excerpt::around(&text, comment.anchor.line, context, source))
    };
    Note {
        id: comment.id,
        file: comment.anchor.file,
        line: comment.anchor.line,
        side: comment.anchor.side,
        confidence: if outdated {
            Confidence::Outdated
        } else {
            confidence
        },
        state: comment.state,
        body: comment.body,
        replies: comment.reply.into_iter().collect(),
        excerpt,
    }
}

/// `rv` with the flags that select this same review, so the protocol's
/// commands answer it rather than whichever review a bare `rv` would open.
fn invocation(review: &Review) -> String {
    let mut command = String::from("rv");
    if review.is_tree() {
        command.push_str(" --tree");
    }
    let (from, to) = &review.asked;
    for (flag, value) in [("--from", from), ("--to", to)] {
        if let Some(value) = value {
            command.push_str(&format!(" {flag} {}", shell_quoted(value)));
        }
    }
    command
}

fn shell_quoted(word: &str) -> String {
    let plain = !word.is_empty()
        && word
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/' | '@'));
    if plain {
        word.to_owned()
    } else {
        format!("'{}'", word.replace('\'', r"'\''"))
    }
}
