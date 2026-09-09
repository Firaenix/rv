//! The request a merge is asked with, the worker that answers it, and the
//! §4.6 retry it makes before giving up.
//!
//! # Single-slot, latest-wins
//!
//! The worker holds a slot rather than a queue, mirroring
//! [`crate::app::diffs`]: a request that arrives while another waits replaces
//! it, and scrolling past ten files therefore costs one merge rather than
//! ten. The pattern is documented on [`crate::app::diffs::Refiner`]; the
//! reasoning is the same.

use std::sync::Arc;
use std::sync::Condvar;
use std::sync::Mutex;
use std::sync::mpsc::Receiver;
use std::sync::mpsc::Sender;
use std::sync::mpsc::channel;

use rv_core::diff::FileDiff;
use rv_core::diff::compute_line_oriented;
use rv_core::diff::merge_context;
use rv_core::diff::whole_file_diff;

use super::MergeOutcome;
use super::MergeState;
use super::Merged;
use crate::app::App;
use crate::app::diffs::Target;

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
    /// leaking the thread — same reasoning as [`crate::app::diffs::Refiner`].
    Shutdown,
}

/// The worker, its slot, and the channel results come back on.
///
/// A close mirror of [`crate::app::diffs::Refiner`]. The two workers stay
/// separate because they answer different questions on different lifecycles —
/// the diff refiner replaces a fast in-process diff with difftastic's, and
/// this merger fills a file's context — and one worker running two kinds of
/// job would need a discriminator on the response channel for a handful of
/// duplicated lines.
pub(in crate::app) struct Merger {
    slot: Arc<(Mutex<Option<Job>>, Condvar)>,
    pub(super) results: Receiver<Merged>,
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
    /// Shared body of [`App::start_merge`] and [`App::start_commit_merge`]:
    /// queues `target`'s merge, replacing whatever was waiting.
    pub(super) fn start_merge_for(
        &mut self,
        target: Target,
        diff: FileDiff,
        base: Vec<u8>,
        head: Vec<u8>,
    ) {
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
