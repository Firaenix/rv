//! The annotator: single-slot and latest-wins like [`crate::app::diffs`]'
//! refiner, so paging past ten files with the gutter on annotates one.
//!
//! Each job opens the repository afresh: a handle kept from before a refresh
//! would not know the commits made since.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Condvar;
use std::sync::Mutex;
use std::sync::mpsc::Receiver;
use std::sync::mpsc::Sender;
use std::sync::mpsc::channel;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use rv_core::vcs::Annotation;
use rv_core::vcs::Repository;

use super::BlobKey;

pub(super) struct Landed {
    pub(super) key: BlobKey,
    pub(super) outcome: Result<Option<Annotation>, String>,
    /// The wall clock when the walk finished, which a card's "3 days ago" is
    /// measured from — the reviewer's state machine reads no clock itself.
    pub(super) as_of_secs: i64,
}

enum Job {
    Annotate { key: BlobKey, root: PathBuf },
    Shutdown,
}

pub(in crate::app) struct Annotator {
    slot: Arc<(Mutex<Option<Job>>, Condvar)>,
    pub(super) results: Receiver<Landed>,
    sender: Sender<Landed>,
    started: bool,
}

impl Default for Annotator {
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

impl Drop for Annotator {
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

impl Annotator {
    /// Queues `key`, returning the key of a queued job it replaced.
    pub(super) fn submit(&mut self, key: BlobKey, root: PathBuf) -> Option<BlobKey> {
        self.start();
        let (slot, waiting) = &*self.slot;
        let mut held = slot.lock().ok()?;
        let previous = held.replace(Job::Annotate { key, root });
        waiting.notify_one();
        match previous {
            Some(Job::Annotate { key, .. }) => Some(key),
            _ => None,
        }
    }

    fn start(&mut self) {
        if self.started {
            return;
        }
        self.started = true;
        let slot = Arc::clone(&self.slot);
        let sender = self.sender.clone();
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
                let Some(Job::Annotate { key, root }) = job else {
                    return;
                };
                let outcome = annotate(&root, &key);
                let landed = Landed {
                    key,
                    outcome,
                    as_of_secs: now_secs(),
                };
                if sender.send(landed).is_err() {
                    return;
                }
            }
        });
    }
}

fn annotate(
    root: &std::path::Path,
    (commit, path): &BlobKey,
) -> Result<Option<Annotation>, String> {
    let repo = Repository::open(root).map_err(|error| error.to_string())?;
    repo.annotate(commit, path)
        .map_err(|error| error.to_string())
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| {
            i64::try_from(elapsed.as_secs()).unwrap_or(i64::MAX)
        })
}
