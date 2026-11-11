//! The whole tree of one commit, for a review of the code as it stands.

use jj_lib::backend::TreeValue;
use jj_lib::object_id::ObjectId as _;

use super::Error;
use super::Repository;
use super::WORKING_COPY;
use super::chain;
use crate::model::ChangeRef;

/// One plain file in a commit's tree.
///
/// Nothing here comes from reading the blob: a tree of thousands of files is
/// listed without opening one, and whether a file is binary is learned when it
/// is opened.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TreeEntry {
    pub path: String,
    /// The blob's id, as hex.
    pub file_id: String,
}

impl Repository {
    /// The commit `revision` names, as hex — the working copy when `None`.
    pub fn resolve(&self, revision: Option<&str>) -> Result<String, Error> {
        let expression = self.parse_or_default(revision, self.working_copy_expression());
        let id = self.single_commit(revision.unwrap_or(WORKING_COPY), &expression)?;
        Ok(id.hex())
    }

    /// Every plain file in `commit_id`'s tree, in path order.
    ///
    /// Symlinks, submodules and conflicted paths are left out: none of them
    /// has a single text to show.
    pub fn tree(&self, commit_id: &str) -> Result<Vec<TreeEntry>, Error> {
        let commit = self.commit_by_hex(commit_id)?;
        let mut entries = Vec::new();
        for (path, value) in commit.tree().entries() {
            let value = value.map_err(|error| Error::Jj(chain(&error)))?;
            if let Some(TreeValue::File { id, .. }) = value.as_normal() {
                entries.push(TreeEntry {
                    path: path.as_internal_file_string().to_owned(),
                    file_id: id.hex(),
                });
            }
        }
        entries.sort_by(|left, right| left.path.cmp(&right.path));
        Ok(entries)
    }

    /// The change `commit_id` belongs to, named the way [`Repository::stack`]
    /// names one.
    pub fn change_of(&self, commit_id: &str) -> Result<ChangeRef, Error> {
        let commit = self.commit_by_hex(commit_id)?;
        Ok(ChangeRef {
            change_id: commit.change_id().reverse_hex(),
            commit_id: commit.id().hex(),
            description: commit.description().trim_end().to_owned(),
        })
    }

    /// Whether `head_commit` is this workspace's working-copy commit, so its
    /// files on disk are the files under review.
    pub fn head_is_working_copy(&self, head_commit: &str) -> bool {
        self.repo
            .view()
            .get_wc_commit_id(&self.workspace_name)
            .is_some_and(|id| id.hex() == head_commit)
    }
}
