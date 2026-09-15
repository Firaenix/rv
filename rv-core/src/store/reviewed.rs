//! Which files the reviewer has ticked off, and in which scope (flags spec
//! §6): a file's whole-range diff, or its diff under one change of the stack.
//! The two are different claims, so neither rolls up into the other.

use serde::Deserialize;
use serde::Serialize;

use super::Error;
use super::Store;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReviewedFile {
    pub file: String,
    /// The change whose diff of `file` was reviewed; `None` for the whole
    /// range.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub change_id: Option<String>,
    /// [`reviewed_hash`] of the two blobs the tick was made against, so a tick
    /// on a file that has since changed can be shown as such rather than
    /// claiming a review of code nobody read.
    pub content_hash: String,
}

impl ReviewedFile {
    #[must_use]
    pub fn is(&self, file: &str, change_id: Option<&str>) -> bool {
        self.file == file && self.change_id.as_deref() == change_id
    }
}

/// The blake3 hex digest of both sides of a file's diff, in order: the
/// identity of "what was reviewed" for a tick.
#[must_use]
pub fn reviewed_hash(old: &[u8], new: &[u8]) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(&(old.len() as u64).to_le_bytes());
    hasher.update(old);
    hasher.update(new);
    hasher.finalize().to_hex().to_string()
}

impl Store {
    pub fn reviewed(&self) -> Result<Vec<ReviewedFile>, Error> {
        Ok(self.read_review()?.reviewed)
    }

    /// Records `reviewed`, replacing any tick on the same file and scope.
    pub fn mark_reviewed(&self, reviewed: &ReviewedFile) -> Result<(), Error> {
        let mut review = self.read_review()?;
        match review
            .reviewed
            .iter_mut()
            .find(|existing| existing.is(&reviewed.file, reviewed.change_id.as_deref()))
        {
            Some(existing) => *existing = reviewed.clone(),
            None => review.reviewed.push(reviewed.clone()),
        }
        self.write_review(&review)
    }

    /// Clears the tick on `file` in the given scope, returning whether one
    /// was there. Idempotent, like every removal in this store.
    pub fn unmark_reviewed(&self, file: &str, change_id: Option<&str>) -> Result<bool, Error> {
        let mut review = self.read_review()?;
        let before = review.reviewed.len();
        review
            .reviewed
            .retain(|existing| !existing.is(file, change_id));
        if review.reviewed.len() == before {
            return Ok(false);
        }
        self.write_review(&review)?;
        Ok(true)
    }
}
