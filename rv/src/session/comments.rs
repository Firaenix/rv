//! Saving a comment: the one construction path and the one attribution rule.
//!
//! The TUI resolves its location from the selected diff line and the CLI from
//! its arguments; everything after "which line is this about" happens here
//! once. The project has already shipped one bug from two places deciding the
//! same fact.

use anyhow::Context as _;
use anyhow::Result;
use rv_core::anchor;
use rv_core::model::ChangeRef;
use rv_core::model::Side;
use rv_core::store::Comment;
use rv_core::store::CommentState;

use super::Review;
use super::write_markdown;

/// The change a comment on `path` belongs to: the newest change in the range
/// whose own diff touches it.
///
/// One rule for the TUI and the CLI. The CLI used `changes.first()`, which is
/// the newest entry and as often as not the *empty working-copy change* — so a
/// comment on code an older change introduced was filed under a change that
/// touched nothing. Falls back to the newest change where no diff claims the
/// path, which is also the answer for an empty stack's error path.
pub fn owning_change<'a>(review: &'a Review, path: &str) -> Result<&'a ChangeRef> {
    let changes = &review.session.changes;
    for (position, change) in changes.iter().enumerate() {
        let base = changes
            .get(position + 1)
            .map_or(review.session.base_commit.as_str(), |older| {
                older.commit_id.as_str()
            });
        let Ok(files) = review.repo.files(base, &change.commit_id) else {
            continue;
        };
        if files
            .iter()
            .any(|file| file.path == path || file.source_path.as_deref() == Some(path))
        {
            return Ok(change);
        }
    }
    changes
        .first()
        .context("the review covers no change to comment on")
}

/// Builds and saves a comment, given the side-resolved location.
///
/// The one construction path: the TUI resolves its location from the selected
/// diff line and the CLI from its arguments, and everything after that — the
/// blob read, the anchor, the id seed, the assembly, the save, the export
/// refresh — happens here once. The project has already shipped one bug from
/// two places deciding the same fact, and a second copy of this policy would be
/// a two-file migration lying in wait.
pub fn save_comment(
    review: &Review,
    path: &str,
    side: Side,
    line: u32,
    commit: &str,
    body: &str,
) -> Result<Comment> {
    let body = body.trim();
    if body.is_empty() {
        anyhow::bail!("an empty comment says nothing — nothing saved");
    }
    let change = owning_change(review, path)?;

    // The anchor hashes the line as it stands in the file, not as the diff
    // rendered it, so it resolves against the file's own future text.
    let blob = review
        .repo
        .read_blob(commit, path)
        .with_context(|| format!("could not read {path} to anchor the comment"))?;
    let text = blob.map(|bytes| String::from_utf8_lossy(&bytes).into_owned());

    let comment = Comment {
        id: crate::app::comment_id(&change.change_id, path, side, line, body),
        change_id: change.change_id.clone(),
        commit_id: commit.to_owned(),
        anchor: anchor::create(path, side, line, text.as_deref().unwrap_or_default()),
        body: body.to_owned(),
        state: CommentState::Open,
        reply: None,
        settled_by: None,
    };
    review
        .store
        .append_comment(&comment)
        .context("could not save the comment")?;
    write_markdown(review)?;
    Ok(comment)
}

/// `rv comment`: resolves the CLI's arguments to a side-specific location and
/// saves through [`save_comment`].
pub fn add_comment(
    review: &Review,
    path: &str,
    side: Side,
    line: u32,
    body: &str,
) -> Result<Comment> {
    let file = review
        .files
        .iter()
        .find(|file| file.path == path || file.source_path.as_deref() == Some(path))
        .with_context(|| {
            format!(
                "{path} is not in this review's range ({})",
                review.session.revset
            )
        })?;
    let (anchored_path, commit) = match side {
        Side::Left => (
            file.source_path.as_deref().unwrap_or(&file.path),
            review.session.base_commit.as_str(),
        ),
        Side::Right => (file.path.as_str(), review.session.head_commit.as_str()),
    };
    // A refusal a program can act on beats an anchor that never resolves.
    let blob = review.repo.read_blob(commit, anchored_path)?.with_context(|| {
        let where_ = match side {
            Side::Left => "the base",
            Side::Right => "the head",
        };
        format!("{anchored_path} does not exist at {where_} of this review")
    })?;
    let text = String::from_utf8(blob)
        .with_context(|| format!("{anchored_path} is not text on that side"))?;
    let lines = u32::try_from(text.lines().count()).unwrap_or(u32::MAX);
    if line == 0 || line > lines {
        anyhow::bail!("{anchored_path} has lines 1..={lines}, not {line}");
    }
    save_comment(review, anchored_path, side, line, commit, body)
}

