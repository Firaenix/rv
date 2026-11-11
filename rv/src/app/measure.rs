//! What every file in the review costs, measured before the first frame.
//!
//! The sidebar's counts and its change bar are facts about the *whole* review,
//! so they cannot wait for a file to be opened. Measured through the in-process
//! engine with difftastic off: this runs over every file before anything is
//! drawn, and a subprocess per file would be a hundred spawns between the
//! reviewer pressing enter and seeing a screen.

use rv_core::diff;
use rv_core::diff::LineKind;

use super::App;
use crate::gradient::Stat;
use crate::session::Review;

impl App {
    /// How many lines every file in the review adds and removes, in sidebar
    /// order, and what could not be read.
    ///
    /// Through [`diff::compute_with`] with difftastic **off**, always. It is a
    /// subprocess per file and this runs over *every* file before the first
    /// frame: a hundred files is a hundred process spawns between the reviewer
    /// pressing enter and seeing anything. The `similar` path is in-process and
    /// its line counts answer the same question about the same two blobs.
    ///
    /// A file whose blobs cannot be read measures zero rather than failing the
    /// whole review, and **says so**: measuring it as zero in silence draws the
    /// row exactly like a file nobody touched.
    pub(super) fn measure(review: &Review) -> (Vec<Stat>, Vec<String>) {
        if review.is_tree() {
            return (Self::measure_tree(review), Vec::new());
        }
        let mut unreadable = Vec::new();
        let stats = review
            .files
            .iter()
            .map(|file| {
                let base = file.source_path.as_deref().unwrap_or(&file.path);
                let old = Self::measured_blob(
                    review,
                    &review.session.base_commit,
                    base,
                    "the base",
                    &mut unreadable,
                );
                let new = Self::measured_blob(
                    review,
                    &review.session.head_commit,
                    &file.path,
                    "the head",
                    &mut unreadable,
                );
                let diff = diff::compute_with(old.as_deref(), new.as_deref(), &file.path, false);
                diff.lines
                    .iter()
                    .fold(Stat::default(), |stat, line| match line.kind {
                        LineKind::Added => Stat {
                            added: stat.added.saturating_add(1),
                            ..stat
                        },
                        LineKind::Removed => Stat {
                            removed: stat.removed.saturating_add(1),
                            ..stat
                        },
                        LineKind::Context => stat,
                    })
            })
            .collect();
        (stats, unreadable)
    }

    /// A tree review's sizes, from the working copy on disk where it is the
    /// tree under review: a stat per file, where reading every blob to learn
    /// its length would stall the first frame of a large repository. Any
    /// other revision measures zero.
    fn measure_tree(review: &Review) -> Vec<Stat> {
        let root = review.store.root();
        let on_disk = review
            .repo
            .head_is_working_copy(&review.session.head_commit);
        review
            .files
            .iter()
            .map(|file| Stat {
                bytes: if on_disk {
                    std::fs::metadata(root.join(&file.path)).map_or(0, |meta| meta.len())
                } else {
                    0
                },
                ..Stat::default()
            })
            .collect()
    }

    /// One side's blob for [`App::measure`], with a failure recorded rather
    /// than swallowed.
    ///
    /// A side the commit has no plain file at reads as `Ok(None)` — an add has
    /// no base, a delete has no head — and is not a failure; only an `Err` is.
    pub(super) fn measured_blob(
        review: &Review,
        commit: &str,
        path: &str,
        end: &str,
        unreadable: &mut Vec<String>,
    ) -> Option<Vec<u8>> {
        match review.repo.read_blob(commit, path) {
            Ok(blob) => blob,
            Err(_) => {
                unreadable.push(format!("could not read {path} at {end} of the review"));
                None
            }
        }
    }
}
