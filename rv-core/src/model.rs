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

/// How confidently [`crate::anchor::resolve`] re-located an [`Anchor`] in a
/// new version of its file.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum Confidence {
    /// The anchored line's content hash matched at its original line number.
    Exact,
    /// The anchored line's content hash matched, but at a different line
    /// number.
    Moved,
    /// No content-hash match; the anchor fell back to its original line
    /// number, which the file still has.
    Weak,
    /// No content-hash match anywhere in the new text, and no line to fall
    /// back to.
    Outdated,
}

impl Confidence {
    pub fn as_str(&self) -> &'static str {
        match self {
            Confidence::Exact => "exact",
            Confidence::Moved => "moved",
            Confidence::Weak => "weak",
            Confidence::Outdated => "outdated",
        }
    }
}

/// A comment's anchor to one line of one side of a diff.
///
/// Anchors are designed to survive history rewrites: [`crate::anchor::resolve`]
/// re-locates `line` in a new version of the file text by matching
/// `content_hash` rather than trusting the line number to have stayed put.
/// `context` is a snapshot of the surrounding lines at anchor creation time,
/// for a reviewer to orient by when resolution can only manage `Outdated`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Anchor {
    pub file: String,
    pub side: Side,
    pub line: u32,
    pub content_hash: String,
    pub context: Vec<String>,
    /// Which line of the file `context[0]` is, 1-based.
    ///
    /// Without it the excerpt in `REVIEW-FEEDBACK.md` cannot say which of its
    /// (up to) eleven lines the comment is about: the target is the sixth line in
    /// the middle of a file but the third near the top, because
    /// [`crate::anchor::snapshot_of`] clamps at the edges. A reviewer reading the
    /// finished document reported that as the one thing they could not resolve
    /// from the file alone — and the excerpt exists precisely for the case where
    /// the file has moved on and cannot be consulted.
    ///
    /// Defaulted on read, so a `.review/` written before it existed still loads;
    /// `0` means "unknown", and the export falls back to an unnumbered fence
    /// rather than numbering from a guess.
    #[serde(default)]
    pub context_start: u32,
}
