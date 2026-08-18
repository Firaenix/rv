//! The sentences the status line refuses with.
//!
//! Gathered here because a refusal is a fact about the *review* or about the
//! *line*, and the same fact must read the same way whichever key asked.

/// The status line shown before the reviewer has done anything.
///
/// One bar row is the whole budget, so each entry is a key and one word — 75
/// columns, which fits an 80-column terminal.
pub(super) const HELP: &str =
    "↓↑ line  [/] file  c comment  enter stack  d delete  s fold  ? help  q quit";

/// What `d` says from the sidebar's **Files** tab, where there is no comment
/// under the cursor to delete.
pub(super) const DELETE_NEEDS_A_COMMENT: &str =
    "the file list selects files, not comments: tab for those, right for the diff";

/// What `d` and `s` say from the sidebar's **Comments** tab when the review has
/// no comments at all.
///
/// About the *review* rather than about a line: the browser is not showing a
/// line, so "no comments on this line" would send the reviewer to the diff.
pub(super) const NO_COMMENTS_IN_REVIEW: &str = "no comments in this review yet";

/// What `t` and `o` say from the sidebar's **Comments** tab.
pub(super) const VIEW_KEYS_ARE_FOR_THE_FILE_LIST: &str =
    "the shape and the order are the file list's: tab for it";

/// What `Enter`, `d` and `s` say when the selected line carries no comments.
///
/// One sentence for all three because it is one fact about the line.
pub(super) const NO_COMMENTS: &str = "no comments on this line";
