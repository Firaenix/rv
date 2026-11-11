//! A landed reply: its focus filed as flags, its session kept, and the whole
//! of it — cleaned for the terminal — kept for the popup `v a` opens.

use crossterm::event::KeyCode;
use rv_core::agent::Reply;
use rv_core::agent::contract::MAX_SUMMARY_CHARS;
use rv_core::agent::contract::MAX_TEXT_CHARS;
use rv_core::agent::contract::MAX_WHY_CHARS;
use rv_core::agent::sanitize;
use rv_core::agent::session as agent_session;

use super::Asked;
use super::MAX_MESSAGE_CHARS;
use crate::app::Action;
use crate::app::App;
use crate::app::bindings::AgentCommand;
use crate::app::bindings::Command;
use crate::session::flags::FocusSaved;

const PAGE: isize = 10;

/// The last reply, every string already through [`sanitize`].
#[derive(Clone, Debug, PartialEq)]
pub struct Answer {
    pub about: String,
    pub summary: String,
    pub text: Vec<String>,
    pub focus: Vec<String>,
    /// What filing the focus did: `2 flagged · 1 already flagged`.
    pub filed: String,
    /// Who answered and what it cost: `maki · 4.1s · $0.0125`.
    pub footer: String,
}

impl Answer {
    fn new(asked: &Asked, reply: &Reply, filed: &FocusSaved, agent: &str) -> Self {
        let answer = &reply.answer;
        let mut footer = vec![sanitize::line(agent, MAX_MESSAGE_CHARS)];
        if let Some(duration) = reply.duration_ms {
            footer.push(format!("{:.1}s", duration as f64 / 1000.0));
        }
        if let Some(cost) = reply.cost_usd {
            footer.push(format!("${cost:.4}"));
        }
        let mut outcome = vec![format!("{} flagged", filed.saved.len())];
        if filed.already_flagged > 0 {
            outcome.push(format!("{} already flagged", filed.already_flagged));
        }
        if !filed.skipped.is_empty() {
            outcome.push(format!("{} skipped", filed.skipped.len()));
        }
        Self {
            about: sanitize::line(&asked.about(), MAX_MESSAGE_CHARS),
            summary: sanitize::line(&answer.summary, MAX_SUMMARY_CHARS),
            text: sanitize::text(&answer.text, MAX_TEXT_CHARS)
                .lines()
                .map(str::to_owned)
                .collect(),
            focus: answer
                .focus
                .iter()
                .map(|focus| {
                    format!(
                        "{}:{} {} — {}",
                        sanitize::line(&focus.path, MAX_MESSAGE_CHARS),
                        focus.line,
                        focus.importance.name(),
                        sanitize::line(&focus.why, MAX_WHY_CHARS)
                    )
                })
                .collect(),
            filed: outcome.join(" · "),
            footer: footer.join(" · "),
        }
    }

    /// The popup's body: the summary, the prose, then the lines it pointed at.
    #[must_use]
    pub fn lines(&self) -> Vec<String> {
        let mut lines = vec![self.summary.clone(), String::new()];
        lines.extend(self.text.iter().cloned());
        if !self.focus.is_empty() {
            lines.push(String::new());
            lines.push(format!("Focus ({})", self.filed));
            lines.extend(self.focus.iter().map(|focus| format!("  {focus}")));
        }
        lines
    }
}

impl App {
    pub(super) fn take_reply(&mut self, asked: Asked, reply: &Reply) {
        if let Err(error) = agent_session::remember(&self.review.store, reply) {
            self.raise(sanitize::line(&error.to_string(), MAX_MESSAGE_CHARS));
        }
        let filed = match crate::session::flags::save_agent_focus(&self.review, &reply.answer.focus)
        {
            Ok(filed) => filed,
            Err(error) => {
                self.raise(format!("could not file the agent's flags: {error:#}"));
                FocusSaved::default()
            }
        };
        if !filed.saved.is_empty() {
            let line = self.line_index();
            if let Err(error) = self.reload_flags() {
                self.raise(format!("{error:#}"));
            }
            self.resettle_cursor(line);
            self.clamp_browser();
        }
        let Some(name) = self.assistant.live.as_ref().map(|live| live.agent.name()) else {
            return;
        };
        let answer = Answer::new(&asked, reply, &filed, &name);
        let hint = self
            .keymap
            .chord_for(Command::Agent(AgentCommand::ShowAnswer))
            .map(|chord| format!(" — {chord} shows it all"))
            .unwrap_or_default();
        self.status = format!(
            "{}: {} · {}{hint}",
            answer.about, answer.summary, answer.filed
        );
        if let Some(live) = self.assistant.live.as_mut() {
            if let Asked::Orientation(key) = asked {
                live.purposes.insert(key, answer.summary.clone());
            }
            live.last = Some(answer);
        }
    }

    /// The last reply the agent gave, about anything.
    #[must_use]
    pub fn answer(&self) -> Option<&Answer> {
        self.assistant.live.as_ref()?.last.as_ref()
    }

    #[must_use]
    pub fn answer_open(&self) -> bool {
        self.answer_scroll().is_some()
    }

    #[must_use]
    pub fn answer_scroll(&self) -> Option<usize> {
        self.assistant.live.as_ref()?.reading
    }

    pub(super) fn show_answer(&mut self) {
        let Some(live) = self.assistant.live.as_mut() else {
            return;
        };
        if live.last.is_none() {
            self.status = "the agent has not answered anything yet".to_owned();
            return;
        }
        live.reading = Some(0);
    }

    /// While the popup is up it answers these keys and no others, like the
    /// `?` keymap: `q` closes it rather than quitting.
    pub(in crate::app) fn on_key_answer(&mut self, key: KeyCode) -> Action {
        match key {
            KeyCode::Esc | KeyCode::Char('q') => self.close_answer(),
            KeyCode::Down | KeyCode::Char('j') => self.scroll_answer(1),
            KeyCode::Up | KeyCode::Char('k') => self.scroll_answer(-1),
            KeyCode::PageDown => self.scroll_answer(PAGE),
            KeyCode::PageUp => self.scroll_answer(-PAGE),
            KeyCode::Home => self.scroll_answer(isize::MIN),
            _ => {}
        }
        Action::Continue
    }

    pub(in crate::app) fn scroll_answer(&mut self, delta: isize) {
        let last_line = self
            .answer()
            .map_or(0, |answer| answer.lines().len().saturating_sub(1));
        if let Some(live) = self.assistant.live.as_mut()
            && let Some(scroll) = live.reading.as_mut()
        {
            *scroll = scroll.saturating_add_signed(delta).min(last_line);
        }
    }

    fn close_answer(&mut self) {
        if let Some(live) = self.assistant.live.as_mut() {
            live.reading = None;
        }
    }
}
