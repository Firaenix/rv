//! Plain data shared across `rv`. No jj-lib types cross this boundary.

use serde::Deserialize;
use serde::Serialize;

/// One change in the reviewed stack.
///
/// `change_id` is the review's identity and is always the `reverse_hex` (`z`-`k`)
/// form that `jj log` displays. `commit_id` is advisory: it moves whenever the
/// change is rewritten, so it must never be used to decide staleness.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChangeRef {
    pub change_id: String,
    pub commit_id: String,
    pub description: String,
}

/// Which side of a diff a location refers to: the base (`Left`) or the head
/// (`Right`) revision.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum Side {
    Left,
    Right,
}

/// How a file changed between the two endpoints of the review.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum ChangeKind {
    Added,
    Modified,
    Removed,
    Renamed,
}

/// A file that differs between the two endpoints of the review.
///
/// `path` is the head-side path; `source_path` is the base-side path and is only
/// set when it differs, i.e. for a rename.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FileChange {
    pub path: String,
    pub source_path: Option<String>,
    pub kind: ChangeKind,
    pub binary: bool,
}
