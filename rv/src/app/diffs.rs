//! Loading a file's diff without making the keystroke wait for it.
//!
//! Measured, release build: difftastic costs a **flat ~26 ms per file** whatever
//! the file's size — that is a process spawn, not diff work — and the in-process
//! `similar` engine answers the same question about the same two blobs in 0.2 ms.
//! Loading synchronously on selection therefore charged 26 ms to every press of
//! `↓` in the file list, which is what made scrolling a long change list feel like
//! it was catching up with itself.
//!
//! So the fast diff is computed inline and drawn at once, and difftastic is asked
//! for in the background. When it lands the pane swaps to it.
//!
//! # Abandoning work rather than queueing it
//!
//! One worker, and it holds a **slot rather than a queue**: a request that arrives
//! while another is waiting replaces it. Scrolling past ten files therefore costs
//! one spawn, not ten, because nine of the requests are dropped before anything is
//! spawned for them. The one already running is left to finish — its result is
//! cached and correct, and killing a child mid-diff would buy nothing a dropped
//! request does not.
//!
//! # The swap keeps your place, not your row
//!
//! The two engines do not produce the same lines: only `similar` emits
//! [`LineKind::Context`], so a file's plan is longer under it and row 7 is not the
//! same code in both. The cursor is therefore re-settled onto the **source line**
//! it was on, not the row index — a swap that moved the reviewer's place would be
//! worse than the wait it saves.

use std::sync::Arc;
use std::sync::Condvar;
use std::sync::Mutex;
use std::sync::mpsc::Receiver;
use std::sync::mpsc::Sender;
use std::sync::mpsc::channel;

use rv_core::diff;
use rv_core::diff::FileDiff;

use super::App;

/// A file whose structural diff has not been computed yet.
struct Request {
    file: usize,
    base: Option<Vec<u8>>,
    head: Option<Vec<u8>>,
    path: String,
}

/// A structural diff on its way back to the main thread.
pub(super) struct Refined {
    file: usize,
    diff: FileDiff,
}

/// The worker, its slot, and the channel results come back on.
pub(super) struct Refiner {
    slot: Arc<(Mutex<Option<Request>>, Condvar)>,
    results: Receiver<Refined>,
    sender: Sender<Refined>,
    started: bool,
}

impl Default for Refiner {
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

impl App {
    /// Asks for `file`'s structural diff, replacing whatever was waiting.
    pub(super) fn refine(&mut self, file: usize, base: Option<Vec<u8>>, head: Option<Vec<u8>>) {
        let Some(path) = self.review.files.get(file).map(|f| f.path.clone()) else {
            return;
        };
        self.start_refiner();
        let (slot, waiting) = &*self.refiner.slot;
        if let Ok(mut held) = slot.lock() {
            // Replaced, not queued: the reviewer has moved on from whatever was
            // in there.
            *held = Some(Request {
                file,
                base,
                head,
                path,
            });
            waiting.notify_one();
        }
    }

    fn start_refiner(&mut self) {
        if self.refiner.started {
            return;
        }
        self.refiner.started = true;
        let slot = Arc::clone(&self.refiner.slot);
        let sender = self.refiner.sender.clone();
        std::thread::spawn(move || {
            let (held, waiting) = &*slot;
            loop {
                let request = {
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
                let Some(request) = request else {
                    continue;
                };
                let diff = diff::compute(request.base.as_deref(), request.head.as_deref(), &request.path);
                if sender
                    .send(Refined {
                        file: request.file,
                        diff,
                    })
                    .is_err()
                {
                    return;
                }
            }
        });
    }

    /// Takes any refined diff that has landed and puts it in place of the fast
    /// one, keeping the cursor on the line it was on.
    pub(super) fn collect_refined(&mut self) -> bool {
        let mut arrived = false;
        while let Ok(refined) = self.refiner.results.try_recv() {
            self.apply_refined(refined);
            arrived = true;
        }
        arrived
    }

    fn apply_refined(&mut self, refined: Refined) {
        let line = (refined.file == self.file_index)
            .then(|| self.selected_line().and_then(|line| line.right.or(line.left)))
            .flatten();
        if let Some(slot) = self.diffs.get_mut(refined.file) {
            *slot = Some(refined.diff);
        }
        self.refining.remove(&refined.file);
        if let Some(number) = line {
            self.resettle_on_line(number);
        }
    }

    /// Puts the cursor back on source line `number`, or the nearest line the
    /// refined diff still carries.
    ///
    /// "Nearest" and not "the same" because the structural diff may not contain
    /// that line at all: the fallback emits a context line for every unchanged
    /// line around a change and difftastic emits none, so most of what the fast
    /// diff shows is absent from the one that replaces it. Landing on the nearest
    /// surviving line keeps the reviewer where they were looking; keeping the row
    /// index would move them somewhere unrelated.
    fn resettle_on_line(&mut self, number: u32) {
        let found = self.selected_diff().and_then(|diff| {
            diff.lines
                .iter()
                .enumerate()
                .filter_map(|(index, line)| {
                    let at = line.right.or(line.left)?;
                    Some((at.abs_diff(number), index))
                })
                .min()
                .map(|(_, index)| index)
        });
        if let Some(line) = found {
            let row = self.plan().row_of_line(line).unwrap_or(0);
            self.set_cursor_row(row);
        }
    }

    /// Blocks until the selected file's structural diff has landed.
    ///
    /// The reviewer never waits for this — the event loop swaps as results arrive.
    /// It exists so a caller that wants to *look* at a finished diff can, without
    /// racing the worker that finishes it.
    pub fn finish_loading(&mut self) {
        while self.refining() {
            match self.refiner_result() {
                Some(refined) => self.apply_refined(refined),
                None => return,
            }
        }
    }

    fn refiner_result(&self) -> Option<Refined> {
        self.refiner.results.recv().ok()
    }

    /// Whether the diff on screen is the fast one, still waiting to be refined.
    #[must_use]
    pub fn refining(&self) -> bool {
        self.refining.contains(&self.file_index)
    }
}
