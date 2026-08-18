//! The sentences the status line refuses with.
//!
//! Gathered so that one fact about a review, or about a line, reads the same way
//! whichever key asked.

/// One bar row is the whole budget: a key and one word each, 75 columns, which
/// fits an 80-column terminal.
pub(super) const HELP: &str =
    "↓↑ line  [/] file  c comment  enter stack  d delete  s fold  ? help  q quit";

pub(super) const DELETE_NEEDS_A_COMMENT: &str =
    "the file list selects files, not comments: tab for those, right for the diff";

pub(super) const SETTLE_NEEDS_A_COMMENT: &str =
    "resolving and abandoning are about comments: tab for those, right for the diff";

/// About the review rather than about a line: the browser is not showing a line,
/// so "no comments on this line" would send the reviewer to the diff.
pub(super) const NO_COMMENTS_IN_REVIEW: &str = "no comments in this review yet";

pub(super) const VIEW_KEYS_ARE_FOR_THE_FILE_LIST: &str =
    "the shape and the order are the file list's: tab for it";

pub(super) const NO_COMMENTS: &str = "no comments on this line";
