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
//! * The merge is computed **once per target**, cached in `App::merges`
//!   (parallel to `App::blobs`, indexed by file) or `App::commit_merges`
//!   (parallel to `App::commit_blobs`, keyed by pair) — one worker handles
//!   both, the target riding along in the request the same way
//!   [`super::diffs::Refiner`] carries it.
//! * While the merge is [`MergeState::Pending`] — the sub-second between
//!   requesting it and its answer landing — [`App::displayed_lines`] returns
//!   the diff's own changed-only lines. That is the shipped-before-this-
//!   feature view, which is a fallback the reviewer has already seen and
//!   which by construction always exists.
//! * The status bar draws a `preparing full view` segment while the shown
//!   target is [`MergeState::Pending`]; on [`MergeState::Ready`] or
//!   [`MergeState::ReadyFallback`] the pane swaps to the full view without a
//!   keystroke, labelled by which engine built it. [`MergeState::Bailed`]
//!   remains as the honest "nothing worked" answer, believed unreachable in
//!   practice now that [`MergeState::ReadyFallback`] exists — see its doc.
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
use rv_core::diff::whole_file_diff;

use super::App;
use super::diffs::Target;

/// What `App::merges[file]` (or `App::commit_merges[pair]`) holds while the
/// merge is inflight, done, or declined.
///
/// See the module doc for how the three states drive the reviewer's view.
#[derive(Debug)]
pub(super) enum MergeState {
    /// A merge for this file has been requested; the worker has not returned
    /// yet. The pane draws the diff's own changed-only lines until it does.
    Pending,
    /// The syntax-aware answer merged the whole file on its own, or
    /// difftastic's own `--byte-limit 0` retry (§4.6) did after the
    /// syntax-aware pass declined. Which one is recorded on the file's
    /// [`DiffSource::Difftastic`] (`line_oriented`), which
    /// [`crate::ui::diff::title`] reads to label the pane.
    Ready(Vec<DiffLine>),
    /// Even difftastic's own retry could not pair a region 1:1 — both
    /// difftastic-based attempts inferred an unreported gap from two
    /// anchor points and the two sides disagreed on the gap's length
    /// (design spec §3). `similar`'s whole-file line diff (the same engine
    /// [`DiffSource::Similar`] names elsewhere) merged instead: it never
    /// infers a gap, so it cannot hit the same ambiguity. The pane still
    /// shows the whole file; the title says which engine built it.
    ReadyFallback(Vec<DiffLine>),
    /// The merge was attempted, the retry was attempted, and both declined,
    /// with no further recourse. Believed unreachable for any file with
    /// real text on both sides — `similar`'s diff (above) always succeeds
    /// where it is tried — kept rather than removed because a future
    /// change to either engine could reopen it, and "we give up" should
    /// stay representable if it ever legitimately recurs.
    Bailed,
}

/// A finished merge on its way back to the main thread.
///
/// Two shapes rather than a `Result` because "declined honestly" is not an
/// error: it is one of three legitimate outcomes the reviewer needs told
/// apart from the other two.
pub(super) struct Merged {
    pub(super) target: Target,
    pub(super) outcome: MergeOutcome,
}

pub(super) enum MergeOutcome {
    /// The merge completed and produced lines, difftastic-based either way.
    Ready {
        lines: Vec<DiffLine>,
        line_oriented: bool,
    },
    /// Neither difftastic-based attempt could pair the file; `similar`'s
    /// whole-file diff did.
    ReadyFallback(Vec<DiffLine>),
    /// Both difftastic-based attempts declined and, at the point this is
    /// constructed, so did the whole-file fallback — see [`MergeState::Bailed`].
    Bailed,
}

/// A single-slot merge request.
struct Request {
    target: Target,
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
    ///
    /// A dropped-in-the-slot request also rolls its target back to `None`
    /// rather than leaving it `Pending`: the worker holds one slot (below),
    /// and a request queued for target A that gets replaced by target B
    /// before the worker grabs it never runs — A's `Pending` would
    /// otherwise lie forever, since [`super::navigate::load_selected`] and
    /// [`super::commit_diff::App::select_commit_file`] only re-kick a
    /// dropped *refinement* on return, not a dropped *merge*. Rolling back
    /// to `None` makes the return-to-target re-kick this too. Mirrors
    /// [`super::diffs::App::refine_target`]'s handling of its own dropped
    /// requests.
    #[tracing::instrument(level = "debug", skip(self))]
    pub(super) fn start_merge(&mut self, file: usize) {
        let path = self.review.files.get(file).map(|f| f.path.clone());
        let Some(diff) = self.diffs.get(file).and_then(Option::as_ref) else {
            tracing::debug!(?path, "start_merge: diff not loaded yet, skipping");
            return;
        };
        // The merge is only meaningful for a difftastic answer with lines to
        // walk from — mirrors the guard in [`super::context::build`].
        if !matches!(diff.source, DiffSource::Difftastic { .. }) || diff.lines.is_empty() {
            tracing::debug!(
                ?path,
                source = ?diff.source,
                lines = diff.lines.len(),
                "start_merge: not a mergeable difftastic diff"
            );
            self.set_merge_state(Target::File(file), None);
            return;
        }
        let (base, head) = match self.blobs.get(file).and_then(Option::as_ref) {
            Some((base, head)) => (base.clone(), head.clone()),
            None => (Vec::new(), Vec::new()),
        };
        let diff = diff.clone();
        self.start_merge_for(Target::File(file), diff, base, head);
    }

    /// The commits-view counterpart of [`App::start_merge`] — same guards,
    /// same slot, same drop-rollback, keyed by pair instead of file index.
    #[tracing::instrument(level = "debug", skip(self))]
    pub(super) fn start_commit_merge(&mut self, pair: usize) {
        let Some(diff) = self.commit_diffs.get(&pair) else {
            tracing::debug!(pair, "start_commit_merge: diff not loaded yet, skipping");
            return;
        };
        if !matches!(diff.source, DiffSource::Difftastic { .. }) || diff.lines.is_empty() {
            tracing::debug!(
                pair,
                source = ?diff.source,
                lines = diff.lines.len(),
                "start_commit_merge: not a mergeable difftastic diff"
            );
            self.set_merge_state(Target::Commit(pair), None);
            return;
        }
        let (base, head) = match self.commit_blobs.get(&pair) {
            Some((base, head)) => (base.clone(), head.clone()),
            None => (Vec::new(), Vec::new()),
        };
        let diff = diff.clone();
        self.start_merge_for(Target::Commit(pair), diff, base, head);
    }

    /// Shared body of [`App::start_merge`] and [`App::start_commit_merge`]:
    /// queues `target`'s merge, replacing whatever was waiting.
    fn start_merge_for(&mut self, target: Target, diff: FileDiff, base: Vec<u8>, head: Vec<u8>) {
        self.set_merge_state(target, Some(MergeState::Pending));

        self.start_merger();
        // The dropped target, if any, is read out of the lock scope and
        // rolled back after: `set_merge_state` needs `&mut self` as a whole,
        // which the borrow on `self.merger.slot` below would otherwise
        // still be alive for.
        let dropped_target = {
            let (slot, waiting) = &*self.merger.slot;
            let Ok(mut held) = slot.lock() else {
                return;
            };
            let previous = held.replace(Job::Merge(Request {
                target,
                diff,
                base,
                head,
            }));
            waiting.notify_one();
            match previous {
                Some(Job::Merge(dropped)) if dropped.target != target => Some(dropped.target),
                _ => None,
            }
        };
        match dropped_target {
            Some(dropped_target) => {
                tracing::debug!(
                    ?dropped_target,
                    "start_merge: replaced a queued request the worker never grabbed; \
                     rolling its merge state back to None so a return re-kicks it"
                );
                self.set_merge_state(dropped_target, None);
            }
            None => tracing::debug!(?target, "start_merge: queued"),
        }
    }

    /// Writes `state` for `target` into whichever storage it belongs to —
    /// `merges[file]`, indexed, or `commit_merges[pair]`, a map. `None`
    /// means the same thing in both: not attempted, or rolled back.
    fn set_merge_state(&mut self, target: Target, state: Option<MergeState>) {
        match target {
            Target::File(file) => {
                if let Some(slot) = self.merges.get_mut(file) {
                    *slot = state;
                }
            }
            Target::Commit(pair) => match state {
                Some(state) => {
                    self.commit_merges.insert(pair, state);
                }
                None => {
                    self.commit_merges.remove(&pair);
                }
            },
        }
    }

    /// Reads whatever [`App::set_merge_state`] last wrote for `target`. Read
    /// by [`super::diffview`] to decide what the pane draws.
    pub(super) fn merge_state_of(&self, target: Target) -> Option<&MergeState> {
        match target {
            Target::File(file) => self.merges.get(file).and_then(Option::as_ref),
            Target::Commit(pair) => self.commit_merges.get(&pair),
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
                    // line-oriented engine and try to merge the result.
                    None => match retry_line_oriented(&request, &old_text, &new_text) {
                        outcome @ MergeOutcome::Ready { .. } => outcome,
                        // Even the retry could not pair a region 1:1: fall
                        // back to a whole-file diff of the same two blobs,
                        // which cannot hit this ambiguity because it never
                        // infers a gap — see `MergeState::ReadyFallback`.
                        _ => MergeOutcome::ReadyFallback(whole_file_diff(
                            Some(&request.base),
                            Some(&request.head),
                        )),
                    },
                };
                if sender
                    .send(Merged {
                        target: request.target,
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

    #[tracing::instrument(level = "debug", skip(self, merged), fields(target = ?merged.target))]
    fn apply_merged(&mut self, merged: Merged) {
        let outcome = merged.outcome;
        let line_oriented = matches!(
            &outcome,
            MergeOutcome::Ready {
                line_oriented: true,
                ..
            }
        );
        match &outcome {
            MergeOutcome::Ready {
                lines,
                line_oriented,
            } => {
                tracing::debug!(lines = lines.len(), line_oriented, "apply_merged: Ready");
            }
            MergeOutcome::ReadyFallback(lines) => {
                tracing::debug!(lines = lines.len(), "apply_merged: ReadyFallback");
            }
            MergeOutcome::Bailed => tracing::debug!("apply_merged: Bailed"),
        }
        // Copy the retry-succeeded flag onto the diff's DiffSource so
        // [`crate::ui::diff::title`] can label the pane as line-diff-composed
        // (§4.6). Only mutate the flag; the `language` is the first
        // invocation's answer and stays put — see the enum's own doc. Not
        // done for `ReadyFallback`: that state carries its own provenance in
        // `MergeState` directly, not on `DiffSource` — see its doc.
        match merged.target {
            Target::File(file) => {
                if line_oriented
                    && let Some(Some(diff)) = self.diffs.get_mut(file)
                    && let DiffSource::Difftastic {
                        line_oriented: flag,
                        ..
                    } = &mut diff.source
                {
                    *flag = true;
                }
            }
            Target::Commit(pair) => {
                if line_oriented
                    && let Some(diff) = self.commit_diffs.get_mut(&pair)
                    && let DiffSource::Difftastic {
                        line_oriented: flag,
                        ..
                    } = &mut diff.source
                {
                    *flag = true;
                }
            }
        }
        self.set_merge_state(
            merged.target,
            Some(match outcome {
                MergeOutcome::Ready { lines, .. } => MergeState::Ready(lines),
                MergeOutcome::ReadyFallback(lines) => MergeState::ReadyFallback(lines),
                MergeOutcome::Bailed => MergeState::Bailed,
            }),
        );
    }

    /// Blocks until the selected file's merge has landed. The reviewer never
    /// calls this; the event loop swaps as results arrive. Tests use it to
    /// look at a finished merge without racing the worker.
    pub fn finish_merging(&mut self) {
        while matches!(
            self.merge_state_of(self.shown_target()),
            Some(MergeState::Pending)
        ) {
            match self.merger.results.recv() {
                Ok(merged) => self.apply_merged(merged),
                Err(_) => return,
            }
        }
    }

    /// Whether the diff on screen's merge is still running — the file's or,
    /// in the commits view, the selected pair's ([`App::shown_target`]).
    /// Read by the status bar and by the event loop's paint poll.
    #[must_use]
    pub fn merging(&self) -> bool {
        matches!(
            self.merge_state_of(self.shown_target()),
            Some(MergeState::Pending)
        )
    }
}

/// Runs the §4.6 `--byte-limit 0` retry against the same blobs, re-parses,
/// and asks [`merge_context`] again. Returns [`MergeOutcome::Ready`] with
/// `line_oriented: true` on success, [`MergeOutcome::Bailed`] on any
/// failure — a difftastic that could not be run, output that did not parse,
/// or a merge that still declined. The caller does not treat `Bailed` as
/// final: it is the signal to fall back further, to the whole-file diff.
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
