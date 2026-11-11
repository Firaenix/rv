//! Asking off the caller's thread: a handle to poll and cancel, and a
//! single-flight slot, because maki refuses a second process on a busy
//! session.

use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::sync::mpsc;
use std::sync::mpsc::TryRecvError;

use super::Agent;
use super::AgentError;
use super::Reply;

/// A running ask. Dropping it cancels the ask and kills the agent.
pub struct AskHandle {
    cancelled: Arc<AtomicBool>,
    receiver: mpsc::Receiver<Result<Reply, AgentError>>,
    done: bool,
}

impl Agent {
    /// Starts [`Agent::ask_resuming`] on a worker thread.
    #[must_use]
    pub fn spawn_ask(&self, prompt: String, resume: Option<String>) -> AskHandle {
        let cancelled = Arc::new(AtomicBool::new(false));
        let (sender, receiver) = mpsc::channel();
        let agent = self.clone();
        let flag = Arc::clone(&cancelled);
        std::thread::spawn(move || {
            let reply = super::resuming(resume.as_deref(), |resume| {
                agent.ask_cancellable(&prompt, resume, &flag)
            });
            let _ = sender.send(reply);
        });
        AskHandle {
            cancelled,
            receiver,
            done: false,
        }
    }
}

impl AskHandle {
    /// The outcome once the ask has finished, exactly once; `None` while it
    /// runs and after it has been taken.
    pub fn try_take(&mut self) -> Option<Result<Reply, AgentError>> {
        if self.done {
            return None;
        }
        let outcome = match self.receiver.try_recv() {
            Ok(outcome) => outcome,
            Err(TryRecvError::Empty) => return None,
            Err(TryRecvError::Disconnected) => {
                Err(AgentError::Spawn("the agent worker stopped".to_owned()))
            }
        };
        self.done = true;
        Some(outcome)
    }

    /// Blocks until the ask finishes.
    pub fn wait(mut self) -> Result<Reply, AgentError> {
        self.done = true;
        self.receiver
            .recv()
            .unwrap_or_else(|_| Err(AgentError::Spawn("the agent worker stopped".to_owned())))
    }

    /// Kills the agent; the outcome becomes [`AgentError::Cancelled`].
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }
}

impl Drop for AskHandle {
    fn drop(&mut self) {
        self.cancel();
    }
}

/// At most one ask at a time, tagged with what it was about (a path and blob
/// hash, a selection) so the outcome can be filed where it belongs.
pub struct SingleFlight<T> {
    running: Option<(T, AskHandle)>,
}

impl<T> Default for SingleFlight<T> {
    fn default() -> Self {
        Self { running: None }
    }
}

impl<T> SingleFlight<T> {
    #[must_use]
    pub fn is_busy(&self) -> bool {
        self.running.is_some()
    }

    #[must_use]
    pub fn current(&self) -> Option<&T> {
        self.running.as_ref().map(|(tag, _)| tag)
    }

    /// Starts the ask unless one is already running, in which case the tag
    /// comes back unstarted.
    pub fn start(
        &mut self,
        agent: &Agent,
        tag: T,
        prompt: String,
        resume: Option<String>,
    ) -> Result<(), T> {
        if self.running.is_some() {
            return Err(tag);
        }
        self.running = Some((tag, agent.spawn_ask(prompt, resume)));
        Ok(())
    }

    pub fn poll(&mut self) -> Option<(T, Result<Reply, AgentError>)> {
        let (_, handle) = self.running.as_mut()?;
        let outcome = handle.try_take()?;
        let (tag, _) = self.running.take()?;
        Some((tag, outcome))
    }

    /// Cancels the running ask, returning what it was about.
    pub fn cancel(&mut self) -> Option<T> {
        self.running.take().map(|(tag, handle)| {
            handle.cancel();
            tag
        })
    }
}
