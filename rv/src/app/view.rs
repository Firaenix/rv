//! The display preferences: every toggle that describes the reviewer's
//! screen rather than the review — what `Settings` seeds, the `v` leader
//! flips, and a refresh carries over whole.
//!
//! One struct rather than a dozen fields on [`App`](super::App), because a
//! refresh builds a fresh app and copies the preferences across, and a list
//! of fields to copy is a list that rots: `v #` and `v c` were lost on every
//! `v r` for as long as that list was written by hand.

use super::viewside::ViewSide;
use crate::config::Settings;
use crate::layout::Split;
use crate::statusbar;
use crate::tree::Sort;

#[derive(Clone, Copy, Debug)]
pub struct View {
    /// Whether the reviewer wants full-file context (spec §5, walked back to
    /// let a toggle stand — see [`crate::app::context`]). Reviewer default is
    /// `true`, and `f` flips it. Not persisted: this is a display preference
    /// scoped to one run, not a review artefact.
    pub full_context: bool,
    /// Whether the file list is drawn as a directory tree rather than as a flat
    /// list of whole paths.
    pub tree: bool,
    /// The order the file list's rows are in.
    pub sort: Sort,
    /// Whether a sidebar row's name is tinted by its change's proportion —
    /// green through the seam to red, across the text itself.
    pub tint: bool,
    /// Whether the sidebar shows the `+n -n` column at all.
    pub counts: bool,
    /// Whether a commit row in the commits list wraps its hash and subject
    /// onto as many rows as it takes, rather than clipping the subject to
    /// one. Commits-only, unlike `tint`/`counts`: a file or directory row
    /// has no subject to wrap.
    pub wrap_commit_subjects: bool,
    /// Whether the status bar draws its separators in ASCII, read from
    /// `RV_ASCII` **once** at startup: the renderer runs on every keystroke and
    /// the environment cannot change under a running process.
    pub ascii: bool,
    /// How the width is divided between the two panes.
    pub split: Split,
    /// Whether the diff pane groups each hunk's removals before its additions,
    /// the way a unified diff prints — rather than difftastic's interleaving of
    /// the two sides. Session-only, `v g` flips it. See [`crate::app::regroup`].
    pub grouped: bool,
    /// Which side of the change the diff pane shows: both (the default), the
    /// base alone, or the head alone. Session-only, `v b` cycles it. See
    /// [`crate::app::viewside`].
    pub view_side: ViewSide,
    /// Whether `i` has put the change tooltip away.
    pub info_dismissed: bool,
    /// Whether the reviewer has put the sidebar away with `z`.
    ///
    /// What they asked for, not what they get: a terminal narrow enough hides
    /// it regardless, and that decision belongs to [`crate::layout`], which is
    /// the only place that knows how wide the screen is.
    pub sidebar_hidden: bool,
}

impl View {
    /// The preferences a review opens with: the settings file where it says,
    /// the reviewer default where it does not.
    pub fn from_settings(settings: &Settings) -> Self {
        Self {
            full_context: settings.full_context.unwrap_or(true),
            tree: settings.tree.unwrap_or(false),
            sort: settings.sort.map_or_else(Sort::default, Sort::from),
            tint: settings.tint.unwrap_or(true),
            counts: settings.counts.unwrap_or(true),
            wrap_commit_subjects: settings.wrap_commit_subjects.unwrap_or(false),
            // `RV_ASCII` set still wins — an environment override outranks a
            // settings file the way a flag outranks both.
            ascii: statusbar::ascii_from_env() || settings.ascii.unwrap_or(false),
            split: settings.split.map_or_else(Split::default, Split::new),
            grouped: settings.grouped.unwrap_or(false),
            view_side: ViewSide::default(),
            info_dismissed: false,
            sidebar_hidden: settings.sidebar_hidden.unwrap_or(false),
        }
    }
}
