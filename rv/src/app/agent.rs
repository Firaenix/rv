//! The configured agent CLI in the reviewer: one ask at a time on a worker
//! thread, polled from the event loop, so no key ever waits on it. Its
//! answers are cached per file and blob, shown in the corner, the bar and a
//! popup, and its focus lines are filed as flags by `agent`.

mod answer;
mod asks;
mod corner;
mod thinking;

use std::collections::HashMap;
use std::collections::HashSet;
use std::path::Path;
use std::time::Duration;
use std::time::Instant;

use rv_core::agent::Agent;
use rv_core::agent::AgentConfig;
use rv_core::agent::AgentError;
use rv_core::agent::Orientation;
use rv_core::agent::Reply;
use rv_core::agent::SingleFlight;
use rv_core::agent::sanitize;
use rv_core::store::FlaggedBy;

pub use answer::Answer;
pub use corner::CornerTip;
pub use thinking::Thinking;

use super::App;
use super::bindings::AgentCommand;

/// How long past its own timeout an ask is waited for before the loop gives
/// up on the worker: the agent is killed at `timeout_secs`, so this only
/// fires if the worker itself is stuck.
const SLACK: Duration = Duration::from_secs(10);
const MAX_MESSAGE_CHARS: usize = 300;

/// A file as the agent was asked about it: its path and a hash of the head
/// bytes, so an edit is a new question and an unchanged file never is.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(in crate::app) struct FileKey {
    path: String,
    blob: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::app) enum Asked {
    Orientation(FileKey),
    Line { path: String, line: u32 },
    Review,
}

impl Asked {
    fn about(&self) -> String {
        match self {
            Asked::Orientation(key) => format!("what {} is for", key.path),
            Asked::Line { path, line } => format!("{path}:{line}"),
            Asked::Review => "the review".to_owned(),
        }
    }
}

/// `None` inside is the reviewer with no agent configured, which behaves
/// exactly as a reviewer built before this module existed.
#[derive(Default)]
pub(in crate::app) struct Assistant {
    live: Option<Box<Live>>,
}

struct Live {
    agent: Agent,
    available: bool,
    flight: SingleFlight<Asked>,
    first_seen: Option<Instant>,
    attempted: HashSet<FileKey>,
    purposes: HashMap<FileKey, String>,
    /// The shown file's key, memoized by file index so the blob is hashed
    /// once per selection rather than once per frame.
    shown: Option<(usize, Option<FileKey>)>,
    last: Option<Answer>,
    /// The answer popup's scroll, while it is open.
    reading: Option<usize>,
}

impl Assistant {
    pub(in crate::app) fn open(config: Option<AgentConfig>, workdir: &Path) -> Self {
        let Some(config) = config else {
            return Self::default();
        };
        let agent = Agent::new(config, workdir);
        Self {
            live: Some(Box::new(Live {
                available: agent.is_available(),
                agent,
                flight: SingleFlight::default(),
                first_seen: None,
                attempted: HashSet::new(),
                purposes: HashMap::new(),
                shown: None,
                last: None,
                reading: None,
            })),
        }
    }

    /// What a refreshed review carries: everything but the memo of the shown
    /// file, whose index and bytes the refresh may have changed.
    pub(in crate::app) fn reopened(mut self) -> Self {
        if let Some(live) = self.live.as_mut() {
            live.shown = None;
        }
        self
    }
}

impl App {
    /// Takes a finished ask, if any, then orients the shown file when the
    /// config asks for that and nothing is running. Never blocks.
    pub fn collect_agent(&mut self, now: Instant) -> bool {
        let Some(live) = self.assistant.live.as_mut() else {
            return false;
        };
        let landed = match live.flight.poll() {
            Some(landed) => Some(landed),
            None if live.flight.is_busy() => {
                let first_seen = *live.first_seen.get_or_insert(now);
                let patience = Duration::from_secs(live.agent.config().timeout_secs) + SLACK;
                if now.saturating_duration_since(first_seen) > patience {
                    live.flight
                        .cancel()
                        .map(|asked| (asked, Err(AgentError::TimedOut(patience.as_secs()))))
                } else {
                    None
                }
            }
            None => None,
        };
        let changed = landed.is_some();
        if let Some((asked, outcome)) = landed {
            if let Some(live) = self.assistant.live.as_mut() {
                live.first_seen = None;
            }
            self.land(asked, outcome);
        }
        // Keeps the memo the corner reads current, whatever the orientation.
        self.shown_file_key();
        self.orient_when_opened();
        changed
    }

    #[must_use]
    pub fn agent_busy(&self) -> bool {
        self.assistant
            .live
            .as_ref()
            .is_some_and(|live| live.flight.is_busy())
    }

    /// The bar's `agent:` segment; `None` with no agent configured. A running
    /// ask is the top-right tooltip's to say, not the bar's.
    #[must_use]
    pub fn agent_label(&self) -> Option<String> {
        let live = self.assistant.live.as_ref()?;
        Some(if !live.available {
            "agent: unavailable".to_owned()
        } else {
            format!("agent: {}", live.agent.name())
        })
    }

    fn orientation_mode(&self) -> Option<Orientation> {
        self.assistant
            .live
            .as_ref()
            .map(|live| live.agent.config().orientation)
    }

    /// `Esc` in browse, after the leader menu and the hover card have had
    /// their turn: a step back out where there is one to take, and only
    /// otherwise a stop to the running ask.
    pub(in crate::app) fn back_out(&mut self) {
        if self.escape_steps_back() {
            self.escape();
            return;
        }
        let stopped = self
            .assistant
            .live
            .as_mut()
            .and_then(|live| live.flight.cancel());
        if let Some(asked) = stopped {
            self.status = format!("stopped asking about {}", asked.about());
        }
    }

    pub(in crate::app) fn run_agent(&mut self, command: AgentCommand) -> anyhow::Result<()> {
        match command {
            AgentCommand::Ask => self.ask_here(),
            AgentCommand::OrientReview => self.orient_review()?,
            AgentCommand::NextFlag => self.jump_flag(true, Some(FlaggedBy::Agent))?,
            AgentCommand::PrevFlag => self.jump_flag(false, Some(FlaggedBy::Agent))?,
            AgentCommand::ShowAnswer => self.show_answer(),
        }
        Ok(())
    }

    pub(in crate::app) fn agent_enabled(&self, command: AgentCommand) -> bool {
        let ready = self.assistant.live.as_ref().is_some_and(|live| {
            live.available
                && !live.flight.is_busy()
                && live.agent.config().orientation != Orientation::Off
        });
        match command {
            AgentCommand::Ask => {
                ready
                    && self.selected_file().is_some()
                    && (self.focus == super::Focus::Sidebar || self.on_a_line())
            }
            AgentCommand::OrientReview => ready,
            AgentCommand::NextFlag | AgentCommand::PrevFlag => {
                self.has_flags(Some(FlaggedBy::Agent))
            }
            AgentCommand::ShowAnswer => self.answer().is_some(),
        }
    }

    fn land(&mut self, asked: Asked, outcome: Result<Reply, AgentError>) {
        let reply = match outcome {
            Ok(reply) => reply,
            Err(AgentError::Cancelled) => {
                self.status = format!("stopped asking about {}", asked.about());
                return;
            }
            Err(error) => {
                if let (AgentError::Unavailable(_), Some(live)) =
                    (&error, self.assistant.live.as_mut())
                {
                    live.available = false;
                }
                let message = sanitize::line(
                    &format!("asking about {} failed: {error}", asked.about()),
                    MAX_MESSAGE_CHARS,
                );
                if matches!(asked, Asked::Orientation(_)) {
                    self.status = message;
                } else {
                    self.raise(message);
                }
                return;
            }
        };
        self.take_reply(asked, &reply);
    }
}
