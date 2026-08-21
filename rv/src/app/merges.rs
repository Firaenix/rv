//! The full-file-context merge, computed once per file in the background.
//!
//! This exists because [`rv_core::diff::merge_context`] walks the whole file
//! and clones every context line — cheap once, ruinous per frame — and the
//! shipped `App::displayed_lines` was calling it several times per paint. A
//! `wc -l` of the current review measured *126* calls per frame on a 100+
//! -line diff, at 5-90ms each; the biggest file peaked at ~90ms per frame,
//! which is the ~10 fps ceiling reviewers reported as "extremely laggy to
//! scroll on."
//!
//! # Design
//!
//! * The merge is computed **once per file**, cached in `App::merges`
//!   parallel to `App::blobs`.
//! * While the merge is [`MergeState::Pending`] — the sub-second between
//!   requesting it and its answer landing — [`App::displayed_lines`] returns
//!   the diff's own changed-only lines. That is the shipped-before-this-
//!   feature view, which is a fallback the reviewer has already seen and
//!   which by construction always exists.
//! * The status bar draws a `preparing full view` segment while the selected
//!   file is [`MergeState::Pending`]; on [`MergeState::Ready`] the pane swaps
//!   to the full view without a keystroke, and on [`MergeState::Bailed`] the
//!   title-suffix "context unavailable" (§4.4) reports the decline.
//!
//! # Single-slot, latest-wins
//!
//! The worker holds a slot rather than a queue, mirroring [`super::diffs`]:
//! a request that arrives while another waits replaces it, and scrolling
//! past ten files therefore costs one merge rather than ten. The pattern is
//! documented on [`super::diffs::Refiner`]; the reasoning is the same.

use std::sync::Arc;
use std::sync::Condvar;
use std::sync::Mutex;
use std::sync::mpsc::Receiver;
use std::sync::mpsc::Sender;
use std::sync::mpsc::channel;

use rv_core::diff::DiffLine;
use rv_core::diff::DiffSource;
use rv_core::diff::FileDiff;
use rv_core::diff::compute_line_oriented;
use rv_core::diff::merge_context;

use super::App;

/// What `App::merges[file]` holds while the merge is inflight, done, or
/// declined.
///
/// See the module doc for how the three states drive the reviewer's view.
#[derive(Debug)]
pub(super) enum MergeState {
    /// A merge for this file has been requested; the worker has not returned
    /// yet. The pane draws the diff's own changed-only lines until it does.
    Pending,
    /// The merge succeeded. These are the lines the pane draws. Whether the
    /// merge came from the syntax-aware answer or the §4.6 `--byte-limit 0`
    /// retry is recorded on the file's [`DiffSource::Difftastic`]
    /// (`line_oriented`), which [`crate::ui::diff::title`] reads to label
    /// the pane — the `Ready` variant carries only the lines because a
    /// second copy on the state and the source would be one thing to keep
    /// in step.
    Ready(Vec<DiffLine>),
    /// The merge was attempted, the retry was attempted, and both declined.
    /// The pane draws the diff's own changed-only lines and the title carries
    /// "context unavailable" — the same fallback `Pending` shows, but
    /// permanent.
    Bailed,
}

/// A finished merge on its way back to the main thread.
///
/// Two shapes rather than a `Result` because "declined honestly" is not an
/// error: it is one of three legitimate outcomes the reviewer needs told
/// apart from the other two.
pub(super) struct Merged {
    pub(super) file: usize,
    pub(super) outcome: MergeOutcome,
}

pub(super) enum MergeOutcome {
    /// The merge completed and produced lines.
    Ready {
        lines: Vec<DiffLine>,
        line_oriented: bool,
    },
    /// The merge was attempted, and — where the syntax-aware answer
    /// returned `None` — the `--byte-limit 0` retry (§4.6) was attempted
    /// too, and both declined. This is the honest "no line-for-line
    /// pairing" answer, not a failure to try.
    Bailed,
}

/// A single-slot merge request.
struct Request {
    file: usize,
    /// The diff whose changed-lines drive the walk. Cloned into the request
    /// because the worker runs off the main thread and does not borrow from
    /// [`App`], and [`FileDiff`]'s `Vec<DiffLine>` is what the walk reads —
    /// re-computing the diff on the worker would spawn a difftastic process
    /// per merge, which is not a cost the shipped model pays.
    diff: FileDiff,
    /// The `(old, new)` bytes the diff was computed from. The reviewer's
    /// blobs are the source of truth for [`merge_context`]'s text; the
    /// worker never re-reads the repository.
    base: Vec<u8>,
    head: Vec<u8>,
}

enum Job {
    Merge(Request),
    /// Wake the parked worker so the owning [`Merger`] can drop without
    /// leaking the thread — same reasoning as [`super::diffs::Refiner`].
    Shutdown,
}

/// The worker, its slot, and the channel results come back on.
///
/// A close mirror of [`super::diffs::Refiner`]. The two workers stay separate
/// because they answer different questions on different lifecycles — the
/// diff refiner replaces a fast in-process diff with difftastic's, and this
/// merger fills a file's context — and one worker running two kinds of job
/// would need a discriminator on the response channel for a handful of
/// duplicated lines.
pub(super) struct Merger {
    slot: Arc<(Mutex<Option<Job>>, Condvar)>,
    results: Receiver<Merged>,
    sender: Sender<Merged>,
    started: bool,
}

impl Default for Merger {
    fn default() -> Self {
        let (sender, results) = channel();
        Self {
            slot: Arc::new((Mutex::new(None), Condvar::new())),
            results,
            sender,
            started: false,
        }
    }
}

impl Drop for Merger {
    fn drop(&mut self) {
        if !self.started {
            return;
        }
        let (slot, waiting) = &*self.slot;
        if let Ok(mut held) = slot.lock() {
            *held = Some(Job::Shutdown);
        }
        waiting.notify_one();
    }
}

impl App {
    /// Kicks a merge for `file`, replacing any previous request in the slot.
    ///
    /// Sets `App::merges[file]` to [`MergeState::Pending`] before the worker
    /// wakes, so the status bar's segment shows immediately rather than on
    /// the first frame after the request round-trips.
    ///
    /// A no-op for a source the merge does not run against —
    /// non-difftastic diffs (the `similar` fallback already emits full
    /// context) and empty ones (nothing to anchor from, §4.5): those files
    /// stay `merges[file] = None`, which [`App::displayed_lines`] treats the
    /// same as `Pending`, so the pane still draws the diff's own lines.
    pub(super) fn start_merge(&mut self, file: usize) {
        let Some(diff) = self.diffs.get(file).and_then(Option::as_ref) else {
            return;
        };
        // The merge is only meaningful for a difftastic answer with lines to
        // walk from — mirrors the guard in [`super::context::build`].
        if !matches!(diff.source, DiffSource::Difftastic { .. }) || diff.lines.is_empty() {
            if let Some(slot) = self.merges.get_mut(file) {
                *slot = None;
            }
            return;
        }
        let (base, head) = match self.blobs.get(file).and_then(Option::as_ref) {
            Some((base, head)) => (base.clone(), head.clone()),
            None => (Vec::new(), Vec::new()),
        };
        let diff = diff.clone();

        if let Some(slot) = self.merges.get_mut(file) {
            *slot = Some(MergeState::Pending);
        }

        self.start_merger();
        let (slot, waiting) = &*self.merger.slot;
        if let Ok(mut held) = slot.lock() {
            // Replaced, not queued: the reviewer has scrolled past whatever
            // was in there. Its `Pending` flag stays: the worker was
            // finishing it anyway, and the reviewer may scroll back — a
            // dropped request whose flag was cleared would show the fallback
            // forever on the returned-to file.
            *held = Some(Job::Merge(Request {
                file,
                diff,
                base,
                head,
            }));
            waiting.notify_one();
        }
    }

    fn start_merger(&mut self) {
        if self.merger.started {
            return;
        }
        self.merger.started = true;
        let slot = Arc::clone(&self.merger.slot);
        let sender = self.merger.sender.clone();
        std::thread::spawn(move || {
            let (held, waiting) = &*slot;
            loop {
                let job = {
                    let Ok(mut guard) = held.lock() else {
                        return;
                    };
                    while guard.is_none() {
                        let Ok(next) = waiting.wait(guard) else {
                            return;
                        };
                        guard = next;
                    }
                    guard.take()
                };
                let request = match job {
                    Some(Job::Merge(request)) => request,
                    Some(Job::Shutdown) => return,
                    None => continue,
                };
                let old_text = String::from_utf8_lossy(&request.base).into_owned();
                let new_text = String::from_utf8_lossy(&request.head).into_owned();
                let outcome = match merge_context(&request.diff.lines, &old_text, &new_text) {
                    Some(lines) => MergeOutcome::Ready {
                        lines,
                        line_oriented: false,
                    },
                    // §4.6: the syntax-aware merge declined (§3's
                    // reformatted-region case). Ask difftastic again with its
                    // line-oriented engine and try to merge the result; only
                    // bail if *that* also cannot pair. One extra difftastic
                    // spawn per file, cached like everything else.
                    None => retry_line_oriented(&request, &old_text, &new_text),
                };
                if sender
                    .send(Merged {
                        file: request.file,
                        outcome,
                    })
                    .is_err()
                {
                    return;
                }
            }
        });
    }

    /// Drains finished merges into `App::merges`. Returns whether anything
    /// arrived — the event loop uses that to decide whether the frame it
    /// just painted is now out of date.
    pub(super) fn collect_merged(&mut self) -> bool {
        let mut arrived = false;
        while let Ok(merged) = self.merger.results.try_recv() {
            self.apply_merged(merged);
            arrived = true;
        }
        arrived
    }

    fn apply_merged(&mut self, merged: Merged) {
        let outcome = merged.outcome;
        let line_oriented = matches!(
            &outcome,
            MergeOutcome::Ready {
                line_oriented: true,
                ..
            }
        );
        // Copy the retry-succeeded flag onto the file's DiffSource so
        // [`crate::ui::diff::title`] can label the pane as line-diff-composed
        // (§4.6). Only mutate the flag; the `language` is the first
        // invocation's answer and stays put — see the enum's own doc.
        if line_oriented
            && let Some(Some(diff)) = self.diffs.get_mut(merged.file)
            && let DiffSource::Difftastic {
                line_oriented: flag,
                ..
            } = &mut diff.source
        {
            *flag = true;
        }
        if let Some(slot) = self.merges.get_mut(merged.file) {
            *slot = Some(match outcome {
                MergeOutcome::Ready { lines, .. } => MergeState::Ready(lines),
                MergeOutcome::Bailed => MergeState::Bailed,
            });
        }
    }

    /// Blocks until the selected file's merge has landed. The reviewer never
    /// calls this; the event loop swaps as results arrive. Tests use it to
    /// look at a finished merge without racing the worker.
    pub fn finish_merging(&mut self) {
        while matches!(
            self.merges.get(self.file_index).and_then(Option::as_ref),
            Some(MergeState::Pending)
        ) {
            match self.merger.results.recv() {
                Ok(merged) => self.apply_merged(merged),
                Err(_) => return,
            }
        }
    }

    /// Whether the selected file's merge is still running. Read by the
    /// status bar and by the event loop's paint poll.
    #[must_use]
    pub fn merging(&self) -> bool {
        matches!(
            self.merges.get(self.file_index).and_then(Option::as_ref),
            Some(MergeState::Pending)
        )
    }
}

/// Runs the §4.6 `--byte-limit 0` retry against the same blobs, re-parses,
/// and asks [`merge_context`] again. Returns [`MergeOutcome::Ready`] with
/// `line_oriented: true` on success, [`MergeOutcome::Bailed`] on any
/// failure — a difftastic that could not be run, output that did not parse,
/// or a merge that still declined.
///
/// Free function rather than a method because it does not touch [`App`] —
/// it runs on the worker thread, off the request's own bytes, and the retry
/// is an implementation detail of "compute the merge" rather than a
/// separate concern the App should orchestrate.
fn retry_line_oriented(request: &Request, old_text: &str, new_text: &str) -> MergeOutcome {
    let Some((retry_lines, _suppressed)) =
        compute_line_oriented(Some(&request.base), Some(&request.head), &request.diff.path)
    else {
        return MergeOutcome::Bailed;
    };
    if retry_lines.is_empty() {
        // The line-oriented engine saw nothing to change: this is the
        // §4.6 "still no anchor" case. The syntax-aware answer was already
        // Bailed, so there is nothing new to say.
        return MergeOutcome::Bailed;
    }
    match merge_context(&retry_lines, old_text, new_text) {
        Some(lines) => MergeOutcome::Ready {
            lines,
            line_oriented: true,
        },
        None => MergeOutcome::Bailed,
    }
}
