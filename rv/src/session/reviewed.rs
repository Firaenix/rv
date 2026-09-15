//! Ticking a file off, and telling whether a tick still describes the code.
//!
//! One construction path for the TUI's `x` and `rv review`, as for comments:
//! the hash a tick is made against and the hash it is checked against must be
//! taken the same way or every tick goes stale the moment it is written.

use anyhow::Context as _;
use anyhow::Result;
use rv_core::model::FileChange;
use rv_core::store::ReviewedFile;
use rv_core::store::reviewed_hash;

use super::Review;

/// What a stored tick says about the file as it stands now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Freshness {
    /// The diff is the one that was reviewed.
    Current,
    /// The file changed under the tick.
    Changed,
}

/// The commits a scope's diff runs between: the range for `None`, or the
/// change and its parent in the stack.
pub fn endpoints(review: &Review, change_id: Option<&str>) -> Result<(String, String)> {
    let Some(change_id) = change_id else {
        return Ok((
            review.session.base_commit.clone(),
            review.session.head_commit.clone(),
        ));
    };
    let changes = &review.session.changes;
    let position = changes
        .iter()
        .position(|change| change.change_id == change_id)
        .with_context(|| format!("no change {change_id} in this review"))?;
    let from = changes
        .get(position + 1)
        .map_or(review.session.base_commit.as_str(), |older| {
            older.commit_id.as_str()
        });
    Ok((from.to_owned(), changes[position].commit_id.clone()))
}

/// The file `path` names in the scope's diff, or an error naming the scope.
fn file_in_scope(review: &Review, path: &str, change_id: Option<&str>) -> Result<FileChange> {
    let (from, to) = endpoints(review, change_id)?;
    let files = match change_id {
        None => review.files.clone(),
        Some(_) => review.repo.files(&from, &to)?,
    };
    files
        .into_iter()
        .find(|file| file.path == path)
        .with_context(|| match change_id {
            None => format!(
                "{path} is not in this review's range ({})",
                review.session.revset
            ),
            Some(change) => format!("{path} is not touched by change {change}"),
        })
}

/// The identity of `file`'s diff between `from` and `to`.
pub fn hash_of(review: &Review, file: &FileChange, from: &str, to: &str) -> Result<String> {
    let old_path = file.source_path.as_deref().unwrap_or(&file.path);
    let old = review.repo.read_blob(from, old_path)?.unwrap_or_default();
    let new = review.repo.read_blob(to, &file.path)?.unwrap_or_default();
    Ok(reviewed_hash(&old, &new))
}

pub fn mark(review: &Review, path: &str, change_id: Option<&str>) -> Result<ReviewedFile> {
    let file = file_in_scope(review, path, change_id)?;
    let (from, to) = endpoints(review, change_id)?;
    let reviewed = ReviewedFile {
        file: file.path.clone(),
        change_id: change_id.map(str::to_owned),
        content_hash: hash_of(review, &file, &from, &to)?,
    };
    review
        .store
        .mark_reviewed(&reviewed)
        .context("could not record the file as reviewed")?;
    Ok(reviewed)
}

pub fn unmark(review: &Review, path: &str, change_id: Option<&str>) -> Result<bool> {
    review
        .store
        .unmark_reviewed(path, change_id)
        .context("could not clear the reviewed mark")
}

/// Whether `reviewed` still describes the diff it was made against. A file
/// no longer in its scope reads as changed: the tick was made against
/// something that is not there.
pub fn freshness(review: &Review, reviewed: &ReviewedFile) -> Freshness {
    let current =
        file_in_scope(review, &reviewed.file, reviewed.change_id.as_deref()).and_then(|file| {
            let (from, to) = endpoints(review, reviewed.change_id.as_deref())?;
            hash_of(review, &file, &from, &to)
        });
    match current {
        Ok(hash) if hash == reviewed.content_hash => Freshness::Current,
        _ => Freshness::Changed,
    }
}
