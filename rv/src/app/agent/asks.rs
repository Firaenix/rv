//! Starting an ask: what it is about, the prompt it carries, and the
//! single-flight slot it has to get into.

use anyhow::Result;
use rv_core::agent::Orientation;
use rv_core::agent::prompt;
use rv_core::agent::session as agent_session;
use rv_core::markdown::brief;
use rv_core::model::ChangeKind;

use super::Asked;
use super::FileKey;
use crate::app::App;
use crate::app::Focus;
use crate::app::diffs::Target;

const BRIEF_CONTEXT_LINES: u32 = 5;

impl App {
    /// `K`: on the diff, about the line under the cursor; from a list, what
    /// the selected file is for.
    pub(in crate::app) fn ask_here(&mut self) {
        if !self.agent_ready() {
            return;
        }
        if self.focus == Focus::Sidebar {
            self.orient_shown_file(true);
            return;
        }
        let Some(file) = self.selected_file().map(|file| file.path.clone()) else {
            self.status = "no file selected, nothing to ask about".to_owned();
            return;
        };
        if !matches!(self.shown_target(), Target::File(_)) {
            self.status = "ask from the file list's diff — this is one change's".to_owned();
            return;
        }
        let Some(line) = self.selected_line().and_then(|line| line.right) else {
            self.status = "this line is not in the head of the review, nothing to ask".to_owned();
            return;
        };
        let Some(text) = self.head_text() else {
            self.status = format!("{file} has no text at the head to ask about");
            return;
        };
        let prompt = prompt::selection(&file, &text, line, line, None);
        self.start_ask(Asked::Line { path: file, line }, prompt);
    }

    /// `B`: the whole review, through the same brief `rv brief --ask` sends.
    pub(in crate::app) fn orient_review(&mut self) -> Result<()> {
        if !self.agent_ready() {
            return Ok(());
        }
        let document = brief::render(&crate::session::brief::assemble(
            &self.review,
            BRIEF_CONTEXT_LINES,
        )?);
        self.start_ask(Asked::Review, prompt::brief(&document));
        Ok(())
    }

    /// With `orientation = "files"`, the shown file is asked about the first
    /// time it is seen at a given blob — once, whatever the answer was.
    pub(in crate::app) fn orient_when_opened(&mut self) {
        if self.orientation_mode() == Some(Orientation::Files) && !self.agent_busy() {
            self.orient_shown_file(false);
        }
    }

    /// `forced` re-shows a cached answer, and asks again after a failure.
    fn orient_shown_file(&mut self, forced: bool) {
        let Some(key) = self.shown_file_key() else {
            if forced {
                self.status = "this file has no text at the head to ask about".to_owned();
            }
            return;
        };
        let Some(live) = self.assistant.live.as_mut() else {
            return;
        };
        if !live.available {
            return;
        }
        if let Some(purpose) = live.purposes.get(&key) {
            if forced {
                self.status = format!("{}: {purpose}", key.path);
            }
            return;
        }
        let first_time = live.attempted.insert(key.clone());
        if !forced && !first_time {
            return;
        }
        let Some(text) = self.head_text() else {
            return;
        };
        let prompt = prompt::orientation(&key.path, &text);
        self.start_ask(Asked::Orientation(key), prompt);
    }

    fn agent_ready(&mut self) -> bool {
        let Some(live) = self.assistant.live.as_ref() else {
            self.status = "no agent is configured — see [agent] in Config.toml".to_owned();
            return false;
        };
        if live.agent.config().orientation == Orientation::Off {
            self.status = "the agent is off in the reviewer (orientation = \"off\")".to_owned();
            return false;
        }
        if !live.available {
            self.status = format!(
                "agent `{}` is unavailable — it is not on PATH",
                live.agent.name()
            );
            return false;
        }
        true
    }

    fn start_ask(&mut self, asked: Asked, prompt: String) {
        let resume = agent_session::resume_id(&self.review.store);
        let Some(live) = self.assistant.live.as_mut() else {
            return;
        };
        let about = asked.about();
        match live.flight.start(&live.agent, asked, prompt, resume) {
            Ok(()) => {
                live.first_seen = None;
                self.status = format!("asking {} about {about}…", live.agent.name());
            }
            Err(refused) => {
                if let Asked::Orientation(key) = &refused {
                    live.attempted.remove(key);
                }
                self.status = format!("{} is still thinking — Esc stops it", live.agent.name());
            }
        }
    }

    /// The shown file's key, hashed once per selection.
    pub(in crate::app) fn shown_file_key(&mut self) -> Option<FileKey> {
        let index = self.file_index;
        if let Some(live) = self.assistant.live.as_ref()
            && let Some((shown, key)) = &live.shown
            && *shown == index
        {
            return key.clone();
        }
        let key = self.file_key(index);
        if let Some(live) = self.assistant.live.as_mut() {
            live.shown = Some((index, key.clone()));
        }
        key
    }

    fn file_key(&self, index: usize) -> Option<FileKey> {
        let file = self.review.files.get(index)?;
        if file.kind == ChangeKind::Removed {
            return None;
        }
        let (_, head) = self.blobs.get(index)?.as_ref()?;
        if head.is_empty() || head.contains(&0) {
            return None;
        }
        Some(FileKey {
            path: file.path.clone(),
            blob: blake3::hash(head).to_hex()[..16].to_owned(),
        })
    }

    fn head_text(&self) -> Option<String> {
        let (_, head) = self.blobs.get(self.file_index)?.as_ref()?;
        (!head.is_empty()).then(|| String::from_utf8_lossy(head).into_owned())
    }
}
