//! What the top-right tooltip says while an ask runs.

use std::time::Duration;
use std::time::Instant;

use crate::app::App;
use crate::app::bindings::Command;
use crate::app::bindings::PaneCommand;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Thinking {
    pub agent: String,
    pub about: String,
    pub elapsed: Duration,
    pub stop_hint: Option<String>,
}

impl App {
    /// `None` unless an ask is running.
    #[must_use]
    pub fn thinking(&self, now: Instant) -> Option<Thinking> {
        let live = self.assistant.live.as_ref()?;
        let asked = live.flight.current()?;
        let elapsed = live.first_seen.map_or(Duration::ZERO, |started| {
            now.saturating_duration_since(started)
        });
        let stop_hint = self
            .keymap
            .chord_for(Command::Pane(PaneCommand::BackOut))
            .map(|chord| format!("{chord} stop"));
        Some(Thinking {
            agent: live.agent.name(),
            about: asked.about(),
            elapsed,
            stop_hint,
        })
    }
}
