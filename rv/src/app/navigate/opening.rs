//! Opening the file the cursor lands on: the selection, the blobs it is read
//! from, and the diff computed over them.

use anyhow::Context as _;
use anyhow::Result;
use rv_core::diff;

use crate::app::App;
use crate::app::DiffEngine;
use crate::app::diffs::Target;

impl App {
    /// Moves the sidebar selection to `index` and loads that file's diff.
    ///
    /// Out-of-range indices are ignored, which is what makes `[` at the top and
    /// `]` at the bottom no-ops rather than errors. The file reopens where it
    /// was left, re-clamped on the way in because it was clamped against
    /// whatever the diff was when it was written.
    #[tracing::instrument(level = "debug", skip(self))]
    pub(in crate::app) fn select_file(&mut self, index: usize) -> Result<()> {
        if index >= self.review.files.len() || index == self.file_index {
            return Ok(());
        }
        let path = self.review.files.get(index).map(|f| f.path.clone());
        tracing::debug!(?path, index, "select_file");
        self.point_at_file(index);
        self.load_selected()?;
        self.set_cursor_row(self.cursor_row());
        self.resettle_sidebar();
        Ok(())
    }

    /// Moves the selection to file `index` without loading its diff — shared
    /// with the commits view, which shows the change's own diff, not the
    /// bookmark's.
    pub(in crate::app) fn point_at_file(&mut self, index: usize) {
        self.file_index = index;
        // A scroll chosen for one file's long lines is noise on the next one's.
        self.diff_hscroll = 0;
    }

    /// Computes the selected file's diff if it has not been computed yet.
    ///
    /// Both sides are read at their own path and their own commit, so a rename
    /// diffs its base-side source against its head-side target rather than
    /// against a file that does not exist.
    #[tracing::instrument(level = "debug", skip(self))]
    pub(in crate::app) fn load_selected(&mut self) -> Result<()> {
        let Some(file) = self.review.files.get(self.file_index) else {
            return Ok(());
        };
        if self.diffs[self.file_index].is_some() {
            // A fast diff whose refinement was dropped — its request replaced in
            // the slot while the reviewer scrolled past — is re-asked on return.
            // Without this, one pass through a long list left every intermediate
            // file pinned to the fast diff for the rest of the session.
            let file = self.file_index;
            if self.engine == DiffEngine::Auto
                && !self.refining.contains(&Target::File(file))
                && !self.refined.contains(&Target::File(file))
            {
                tracing::debug!(file, "load_selected: re-requesting a dropped refinement");
                self.request_refinement(file)?;
            }
            // A merge whose request was dropped by the merger's own slot
            // (see `start_merge`'s doc) is rolled back to `None`, the same
            // state a non-difftastic or empty diff leaves behind. Only the
            // dropped case is worth re-kicking: a diff that is genuinely
            // difftastic-with-lines and still `None` here was never merged
            // at all.
            let eligible = matches!(
                self.diffs[file].as_ref().map(|d| &d.source),
                Some(rv_core::diff::DiffSource::Difftastic { .. })
            ) && self.diffs[file]
                .as_ref()
                .is_some_and(|d| !d.lines.is_empty());
            if eligible && self.merges.get(file).and_then(Option::as_ref).is_none() {
                tracing::debug!(file, "load_selected: re-kicking a dropped merge");
                self.start_merge(file);
            }
            return Ok(());
        }

        let session = &self.review.session;
        let base_commit = session.base_commit.clone();
        let head_commit = session.head_commit.clone();
        let base_path = file.source_path.as_deref().unwrap_or(&file.path).to_owned();
        let head_path = file.path.clone();
        let old = self
            .review
            .repo
            .read_blob(&base_commit, &base_path)
            .with_context(|| format!("could not read {base_path} at the base of the review"))?;
        let new = self
            .review
            .repo
            .read_blob(&head_commit, &head_path)
            .with_context(|| format!("could not read {head_path} at the head of the review"))?;

        self.diffs[self.file_index] = Some(match self.engine {
            // The in-process engine first: 0.2 ms against difftastic's flat 26 ms
            // spawn, so the keystroke never waits. difftastic is asked for in the
            // background and the pane swaps when it lands — see
            // [`crate::app::diffs`].
            DiffEngine::Auto | DiffEngine::Fallback => {
                diff::compute_with(old.as_deref(), new.as_deref(), &head_path, false)
            }
            DiffEngine::Structural => diff::compute(old.as_deref(), new.as_deref(), &head_path),
        });
        // Cloned rather than moved: `refine` below takes the originals for the
        // background structural pass, and the bytes stored here are what
        // `crate::app::merges` reads to synthesize full-file context
        // — the same blobs regardless of which engine ends up answering, so
        // there is nothing to update when the structural diff later replaces
        // the fast one.
        self.blobs[self.file_index] = Some((
            old.clone().unwrap_or_default(),
            new.clone().unwrap_or_default(),
        ));
        // Parsed from the very blobs the diff was computed from, so the spans a
        // line is painted with describe the text that line came from — and parsed
        // *off* this thread, so a large file draws now and colours in a moment.
        self.parse_highlights(base_commit, base_path, old.as_deref());
        self.parse_highlights(head_commit, head_path, new.as_deref());
        // Kick the full-file merge in the background, off the very blobs the
        // diff was computed from — the pane draws the fallback view (the
        // diff's own changed-only lines) until it lands. When difftastic
        // later replaces the fast diff, `apply_refined` requeues the merge.
        self.start_merge(self.file_index);
        if self.engine == DiffEngine::Auto {
            self.refine(self.file_index, old, new);
        }
        Ok(())
    }

    /// Re-reads `file`'s blobs and asks the refiner for its structural diff.
    ///
    /// For a file whose first request was dropped by slot replacement. The blobs
    /// are re-read rather than kept from the first load: keeping every
    /// scrolled-past file's bytes alive for a maybe-return would trade a bounded
    /// re-read on selection for unbounded memory on a large review.
    fn request_refinement(&mut self, file: usize) -> Result<()> {
        let Some(entry) = self.review.files.get(file) else {
            return Ok(());
        };
        let base_path = entry
            .source_path
            .as_deref()
            .unwrap_or(&entry.path)
            .to_owned();
        let head_path = entry.path.clone();
        let base_commit = self.review.session.base_commit.clone();
        let head_commit = self.review.session.head_commit.clone();
        let old = self
            .review
            .repo
            .read_blob(&base_commit, &base_path)
            .with_context(|| format!("could not read {base_path} at the base of the review"))?;
        let new = self
            .review
            .repo
            .read_blob(&head_commit, &head_path)
            .with_context(|| format!("could not read {head_path} at the head of the review"))?;
        self.refine(file, old, new);
        Ok(())
    }
}
