//! The selected file's diff, the row plan it flattens into, and what the
//! cursor and the highlighter read off both.
//!
//! Split out of [`super::query`] at the 400-line rule: this half is
//! specifically about diff *content* — which lines, at which row, coloured
//! how — and the other half is everything else the renderer and keyboard
//! read off [`App`].

use std::borrow::Cow;

use rv_core::diff::DiffLine;
use rv_core::diff::FileDiff;
use rv_core::highlight::Highlights;
use rv_core::model::Side;

use super::App;
use super::ViewSide;
use super::merges::MergeState;
use crate::rows;
use crate::rows::Plan;

impl App {
    /// The selected file's diff, once it has been loaded.
    pub fn selected_diff(&self) -> Option<&FileDiff> {
        // The commits view shows the change's own diff of the file, not the
        // branch's — see `select_commit_file`. Everything downstream of here
        // reads one diff and does not care which, which is what keeps the row
        // plan, the cursor and the renderer from needing two code paths.
        //
        // **Only where the pair names the selected file.** A pair outlives the
        // tab it was chosen in: walking through the commits tab on the way to
        // the comment browser sets one, and coming back later would otherwise
        // pair that stale diff with whatever file is selected now — one file's
        // lines under another file's name, which a property caught in one
        // keystroke.
        if self.sidebar_tab == super::SidebarTab::Commits
            && let Some(pair) = self.commit_pair
            && let Some(diff) = self.commit_diffs.get(&pair)
            && self.commit_path(pair) == self.selected_file().map(|file| file.path.as_str())
        {
            return Some(diff);
        }
        self.diffs.get(self.file_index).and_then(Option::as_ref)
    }

    /// The lines to draw for the selected file: full-file context where
    /// [`rv_core::diff::merge_context`] built one and the reviewer has left
    /// the `f` toggle on, the engine's own (changed-only or already-full)
    /// lines otherwise — see the design spec (`docs/superpowers/specs/
    /// 2026-08-21-rv-full-file-context-design.md`) §4.3/§5 and Appendix A.
    ///
    /// This is the one place the row stream is decided: [`App::plan`], the
    /// hunk-navigation module and comment matching all read from it rather
    /// than from [`App::selected_diff`]'s own `lines`, so a comment box, a
    /// row index and a hunk jump can never disagree about which lines exist.
    ///
    /// # Cache and fallback
    ///
    /// The full-file merge is computed once per file by a background worker
    /// and cached in `App::merges`. While it is inflight — or while the
    /// reviewer has turned `f` off, or the diff is one the merger does not
    /// run over at all — this returns the diff's own `lines` directly,
    /// which is the changed-only view that shipped before the full-file
    /// feature and is guaranteed to exist. The pane swaps to the full view
    /// when the merge lands, with no keystroke.
    ///
    /// The commit-view diffs are not cached in `App::merges` (that Vec is
    /// parallel to `App::diffs`, and the commits view keys by pair, not by
    /// file). They fall back to the diff's own lines while the merge would
    /// have to be computed synchronously here — a follow-on if commit-view
    /// perf becomes a complaint, which the shipped version does not report.
    pub fn base_lines(&self) -> &[DiffLine] {
        let Some(diff) = self.selected_diff() else {
            return &[];
        };
        if !self.full_context {
            return &diff.lines;
        }
        if self.showing_commit_view() {
            return &diff.lines;
        }
        match self.merges.get(self.file_index).and_then(Option::as_ref) {
            Some(MergeState::Ready(lines)) => lines,
            // Pending, Bailed, or not-yet-requested: the changed-only view
            // is the fallback the module doc names.
            _ => &diff.lines,
        }
    }

    /// The lines to draw, after the `v g` grouping and `v b` side toggles —
    /// applied at this one place so the plan, hunk navigation and comment
    /// matching agree on which lines exist and in what order.
    ///
    /// Borrows the base lines when neither toggle is on, since [`App::line_index`]
    /// reaches this per keystroke; only a live transform owns a fresh stream.
    pub fn displayed_lines(&self) -> Cow<'_, [DiffLine]> {
        let base = self.base_lines();
        if !self.grouped && self.view_side == ViewSide::Diffed {
            return Cow::Borrowed(base);
        }
        let grouped = if self.grouped {
            super::regroup::group(base.to_vec())
        } else {
            base.to_vec()
        };
        Cow::Owned(self.view_side.filter(grouped))
    }

    /// Whether full-file context was attempted for the selected file **and
    /// declined** — §4.4's title suffix reads this, and only this, so the
    /// suffix can never appear on a file the merge was never asked about,
    /// nor on one the reviewer turned the merge off for with `f`.
    pub fn context_bailed(&self) -> bool {
        if !self.full_context || self.showing_commit_view() {
            return false;
        }
        matches!(
            self.merges.get(self.file_index).and_then(Option::as_ref),
            Some(MergeState::Bailed)
        )
    }

    /// Whether the branch-view diff is displaced by a commit-view one for
    /// the selected file — the check `selected_diff` and `selected_blobs`
    /// share, spelled once.
    fn showing_commit_view(&self) -> bool {
        self.sidebar_tab == super::SidebarTab::Commits
            && self.commit_pair.is_some_and(|pair| {
                self.commit_diffs.contains_key(&pair)
                    && self.commit_path(pair) == self.selected_file().map(|file| file.path.as_str())
            })
    }

    /// Whether the reviewer has the `f` toggle set to show full-file context.
    #[must_use]
    pub fn full_context(&self) -> bool {
        self.full_context
    }

    /// Flips the `f` toggle. The next [`App::displayed`] read observes the
    /// change immediately — there is no cache to invalidate, because the
    /// toggle is checked at read time.
    pub fn set_full_context(&mut self, on: bool) {
        self.full_context = on;
    }

    /// Which **row** of the selected file's plan the cursor is on.
    pub fn cursor_row(&self) -> usize {
        self.cursor_rows.get(self.file_index).copied().unwrap_or(0)
    }

    /// The row plan for the selected file, at the width the pane last drew a
    /// comment box's text in.
    ///
    /// The one place a plan is *made*: [`crate::ui::visible`] draws from this,
    /// so the rows the keyboard walks and the rows the pane shows are the same
    /// list.
    pub fn plan(&self) -> Plan<'_> {
        let lines = self.displayed_lines();
        // The closure reads the line off `lines` rather than through
        // `comments_for_line`, which would rebuild the whole stream once per row.
        rows::plan(
            &lines,
            &|index| match lines.get(index) {
                Some(line) => self.comments_anchored_at(line),
                None => Vec::new(),
            },
            &|comment| self.drift.get(&comment.id),
            &self.collapsed,
            self.body_width.get(),
        )
    }

    /// Records how many columns of body text the pane drew a comment box with.
    ///
    /// Called by [`crate::ui::visible`] and nowhere else; the renderer is the
    /// only thing that knows how wide it drew a box.
    pub fn note_body_width(&self, width: usize) {
        self.body_width.set(width);
    }

    /// How many rows the selected file's plan has.
    pub(super) fn row_count(&self) -> usize {
        self.plan().rows.len()
    }

    /// Which line of the selected diff is highlighted: the line that **owns**
    /// the row under the cursor.
    ///
    /// **Derived, never stored.** A stored copy would be a second cursor to
    /// keep in step with [`App::cursor_row`], which is the defect spec §10
    /// describes. Zero where there is nothing to point at, because a clamp
    /// belongs on the way in and a `None` here would reach every caller for a
    /// case none of them can do anything about.
    pub fn line_index(&self) -> usize {
        self.plan().line_of_row(self.cursor_row()).unwrap_or(0)
    }

    pub(super) fn selected_line(&self) -> Option<DiffLine> {
        self.displayed_lines().get(self.line_index()).cloned()
    }

    /// The highlight spans for the selected file's blob **on `side`**.
    ///
    /// `None` for a side the commit has no plain file at, and for a file whose
    /// extension names no grammar rv ships.
    ///
    /// Callers choose `side` through [`super::anchored_side`] and nothing else:
    /// a removed line looked up on the head side would be painted with the
    /// colours of whatever now stands at its number, which is a lie told in a
    /// colour rather than in words.
    pub fn highlights(&self, side: Side) -> Option<&Highlights> {
        let file = self.selected_file()?;
        // The endpoints the diff on screen was read from — the selected
        // change's, in the commits view — exactly as the anchor resolves them.
        // Looking up against the review's endpoints painted a change-scoped
        // line with the colours of whatever text stands at its number in a
        // different version of the file.
        let (base, head) = self.shown_endpoints();
        let (commit, path) = match side {
            Side::Left => (base, file.source_path.as_deref().unwrap_or(&file.path)),
            Side::Right => (head, file.path.as_str()),
        };
        self.highlights.get(&(commit, path.to_owned()))
    }

    /// The 1-based number of the line the cursor is on, on the side a comment
    /// there would anchor to — the number the pane's gutter shows.
    pub fn cursor_line_number(&self) -> Option<u32> {
        let line = self.selected_line()?;
        match super::anchored_side(line.kind) {
            Side::Left => line.left.or(line.right),
            Side::Right => line.right.or(line.left),
        }
    }

    /// The symbol enclosing the cursor — the nearest definition at or above
    /// its line in the selected file — where the symbol index already knows.
    ///
    /// "Already knows" is the whole contract: the bar is painted per keystroke
    /// and must never build an index, so this reads the cache `n`/`N`/`/`
    /// filled and answers nothing until they have.
    pub fn enclosing_symbol(&self) -> Option<String> {
        if self.indexed_scope.as_ref() != Some(&self.scope()) {
            return None;
        }
        let path = &self.selected_file()?.path;
        let line = self.cursor_line_number()?;
        self.symbol_index
            .entries()
            .iter()
            .filter(|entry| &entry.path == path && entry.symbol.line <= line)
            .max_by_key(|entry| entry.symbol.line)
            .map(|entry| format!("in {} {}", entry.symbol.kind.label(), entry.symbol.name))
    }
}
