//! Highlighting off the critical path: draw the code plain, colour it when the
//! parse comes back.
//!
//! # Why this is the one thing that moves off the main thread
//!
//! Measured on this repository, release build. Per file, first open:
//!
//! | file | highlight | difftastic |
//! |---|---|---|
//! | 2,000 lines | 13 ms | 26 ms |
//! | 32,000 lines | 40 ms | 27 ms |
//! | 128,000 lines | **165 ms** | 30 ms |
//!
//! difftastic is a **flat** ~26 ms — that is a process spawn, not diff work, and
//! it is the floor under every file switch. Highlighting is the part that
//! *scales*: it overtakes the spawn somewhere around thirty thousand lines and
//! keeps going. So the diff itself stays synchronous, because a pane cannot draw
//! without lines, and the colour is what waits.
//!
//! # Plain is already the fallback
//!
//! [`App::highlights`] answering `None` means "draw it plain", which the renderer
//! has always done for a file whose language ships no grammar. Nothing new had to
//! be invented for the interim state: a blob whose parse has not landed yet is,
//! for one frame, a blob with no grammar.
//!
//! The swap is therefore invisible except as colour arriving. Nothing about the
//! diff, the row plan, the cursor or a comment's anchor depends on it — a comment
//! is anchored to the *blob*, not to how it was painted.

use std::sync::mpsc::Receiver;
use std::sync::mpsc::Sender;
use std::sync::mpsc::TryRecvError;
use std::sync::mpsc::channel;

use rv_core::highlight::Highlights;

use super::App;

/// One parsed blob on its way back to the main thread.
pub(super) struct Parsed {
    pub(super) commit: String,
    pub(super) path: String,
    pub(super) highlights: Highlights,
}

/// The two ends of the channel parses come back on.
pub(super) struct Painter {
    pub(super) sender: Sender<Parsed>,
    pub(super) receiver: Receiver<Parsed>,
}

impl Default for Painter {
    fn default() -> Self {
        let (sender, receiver) = channel();
        Self { sender, receiver }
    }
}

impl App {
    /// Parses `blob`'s highlight spans under `(commit, path)` on another thread,
    /// unless they are already cached or already being computed.
    ///
    /// One thread per blob, and no pool: a review switches files as fast as a
    /// reviewer can press `]`, each parse is tens of milliseconds, and a pool
    /// would exist only to bound something that bounds itself. A side the commit
    /// has no plain file at spawns nothing.
    pub(super) fn parse_highlights(&mut self, commit: String, path: String, blob: Option<&[u8]>) {
        let Some(bytes) = blob else {
            return;
        };
        let key = (commit, path);
        if self.highlights.contains_key(&key) || !self.parsing.insert(key.clone()) {
            return;
        }

        let sender = self.painter.sender.clone();
        let owned = bytes.to_vec();
        let (commit, path) = key;
        // Detached: nothing joins it and nothing needs to. A parse whose result
        // arrives after the reviewer has moved on is inserted into the cache and
        // used the next time that blob is on screen; a parse whose receiver is
        // gone fails to send, which is the process exiting.
        std::thread::spawn(move || {
            let highlights = Highlights::of(&owned, &path);
            let _ = sender.send(Parsed {
                commit,
                path,
                highlights,
            });
        });
    }

    /// Takes every parse that has landed and puts it in the cache.
    ///
    /// Returns whether anything arrived, which is what tells the event loop that
    /// the frame it just painted is out of date.
    pub(super) fn collect_highlights(&mut self) -> bool {
        let mut arrived = false;
        loop {
            match self.painter.receiver.try_recv() {
                Ok(parsed) => {
                    let key = (parsed.commit, parsed.path);
                    self.parsing.remove(&key);
                    self.highlights.insert(key, parsed.highlights);
                    arrived = true;
                }
                // Disconnected cannot happen while `self` holds a sender, and
                // either way there is nothing to wait for.
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => return arrived,
            }
        }
    }

    /// Blocks until every outstanding parse has landed.
    ///
    /// The reviewer never waits for this — the event loop collects parses as they
    /// arrive and repaints. It exists so a caller that wants to *look* at a
    /// finished frame can, without racing the threads that finish it: "the code
    /// is coloured" is a claim about the end state, and a frame grabbed mid-parse
    /// is a claim about neither state.
    pub fn finish_painting(&mut self) {
        while self.painting() {
            match self.painter.receiver.recv() {
                Ok(parsed) => {
                    let key = (parsed.commit, parsed.path);
                    self.parsing.remove(&key);
                    self.highlights.insert(key, parsed.highlights);
                }
                // Every sender is gone, so nothing further can arrive. Cannot
                // happen while `self` holds one, and giving up beats hanging.
                Err(_) => return,
            }
        }
    }

    /// Whether any blob on screen is still waiting to be painted.
    ///
    /// The event loop polls on a short timeout while this is true, so a parse
    /// that lands between keystrokes is drawn without one.
    pub(super) fn painting(&self) -> bool {
        !self.parsing.is_empty()
    }
}
