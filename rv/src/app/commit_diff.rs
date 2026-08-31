//! Loading the diff a commits-view row names, on the same fast-then-refined
//! path the bookmark view uses.
//!
//! Split from [`super::commits`] at the 400-line rule: that half builds the
//! change index and its tree; this half is specifically about turning a
//! selected row into the diff, blobs and highlights the renderer reads — the
//! commits-view counterpart of [`super::navigate::App::load_selected`].

use anyhow::Context as _;
use anyhow::Result;

use super::App;
use super::SidebarTab;

impl App {
    /// Selects the file a commits-view row names, and shows **that change's**
    /// diff of it.
    ///
    /// Two selections, because the row means two things. The bookmark file is
    /// still chosen — comments, the sidebar's cursor and the status line all
    /// address a file by its position in `App::files()` — while the diff on
    /// screen, and the commits a comment written on it is anchored between, come
    /// from the change the row sits under.
    ///
    /// A pair whose path is not in the bookmark's own list is passed over: it
    /// was touched by a change and undone by a later one, so the range has no
    /// file to select.
    pub(super) fn select_commit_file(&mut self, pair: usize) -> Result<()> {
        let Some(path) = self.commit_path(pair).map(str::to_owned) else {
            return Ok(());
        };
        // Point at the file without the bookmark's diff of it — the change's own
        // diff, loaded below, is what this view shows.
        if let Some(index) = self.review.files.iter().position(|file| file.path == path)
            && index != self.file_index
        {
            self.point_at_file(index);
            self.set_cursor_row(self.cursor_row());
            self.resettle_sidebar();
        }
        self.commit_pair = Some(pair);
        self.load_commit_diff(pair)
    }

    /// Computes the `pair`th row's own diff, unless it is already cached.
    ///
    /// The same engine and the same blobs-to-highlights path the bookmark view
    /// uses — see `load_selected` — because a line of this diff is painted by the
    /// same renderer and must be able to answer the same questions about itself.
    /// That includes the background refinement: the fast in-process diff is
    /// drawn at once and difftastic is asked for off-thread, so landing on a
    /// commit-view file for the first time costs 0.2 ms, not difftastic's flat
    /// 26 ms spawn — the swap arrives through `apply_refined` a moment later.
    fn load_commit_diff(&mut self, pair: usize) -> Result<()> {
        if self.commit_diffs.contains_key(&pair) {
            if self.engine() == super::DiffEngine::Auto
                && !self.refining.contains(&super::diffs::Target::Commit(pair))
                && !self.refined.contains(&super::diffs::Target::Commit(pair))
            {
                self.refine_commit(pair)?;
            }
            return Ok(());
        }
        let Some((from, to, base_path, head_path)) = self.commit_blob_keys(pair) else {
            return Ok(());
        };
        let old = self
            .review
            .repo
            .read_blob(&from, &base_path)
            .with_context(|| format!("could not read {base_path} at {from}"))?;
        let new = self
            .review
            .repo
            .read_blob(&to, &head_path)
            .with_context(|| format!("could not read {head_path} at {to}"))?;

        let diff = match self.engine() {
            super::DiffEngine::Structural => {
                rv_core::diff::compute(old.as_deref(), new.as_deref(), &head_path)
            }
            _ => rv_core::diff::compute_with(old.as_deref(), new.as_deref(), &head_path, false),
        };
        self.commit_blobs.insert(
            pair,
            (
                old.clone().unwrap_or_default(),
                new.clone().unwrap_or_default(),
            ),
        );
        self.commit_diffs.insert(pair, diff);
        self.parse_highlights(from, base_path.clone(), old.as_deref());
        self.parse_highlights(to, head_path.clone(), new.as_deref());
        if self.engine() == super::DiffEngine::Auto {
            self.refine_target(super::diffs::Target::Commit(pair), head_path, old, new);
        }
        Ok(())
    }

    /// Re-asks for `pair`'s structural diff, off freshly read blobs.
    ///
    /// The commits-view mirror of `request_refinement`: a request dropped by the
    /// worker's single slot while the reviewer scrolled past leaves the fast
    /// fallback in place, and this puts it back in the queue on return.
    fn refine_commit(&mut self, pair: usize) -> Result<()> {
        let Some((from, to, base_path, head_path)) = self.commit_blob_keys(pair) else {
            return Ok(());
        };
        let old = self.review.repo.read_blob(&from, &base_path).ok().flatten();
        let new = self.review.repo.read_blob(&to, &head_path).ok().flatten();
        self.refine_target(super::diffs::Target::Commit(pair), head_path, old, new);
        Ok(())
    }

    /// The two commits and two paths the `pair`th row's diff is read from.
    fn commit_blob_keys(&self, pair: usize) -> Option<(String, String, String, String)> {
        let index = self.commit_index();
        let (change, file) = index.pair(pair)?;
        let (from, to) = index.endpoints_of(change)?;
        Some((
            from.to_owned(),
            to.to_owned(),
            file.source_path.as_deref().unwrap_or(&file.path).to_owned(),
            file.path.clone(),
        ))
    }

    /// The two commits the diff on screen is between.
    ///
    /// The review's own endpoints in the bookmark view, and the selected
    /// change's in the commits view. This is what a comment is anchored between,
    /// and it has to be the pair the text on screen was read from: a comment
    /// whose commit names a revision its quoted text cannot be read back from is
    /// a comment that cannot be verified, which is that field's only job.
    pub(super) fn shown_endpoints(&self) -> (String, String) {
        let session = &self.review.session;
        let fallback = || (session.base_commit.clone(), session.head_commit.clone());
        if self.sidebar_tab() != SidebarTab::Commits {
            return fallback();
        }
        let Some(pair) = self.commit_pair else {
            return fallback();
        };
        // See `selected_diff`: a pair that does not name the selected file is
        // stale, and the review's own endpoints are what the shown diff is
        // between.
        if self.commit_path(pair) != self.selected_file().map(|file| file.path.as_str()) {
            return fallback();
        }
        let index = self.commit_index();
        index
            .pair(pair)
            .and_then(|(change, _)| index.endpoints_of(change))
            .map_or_else(fallback, |(from, to)| (from.to_owned(), to.to_owned()))
    }
}
