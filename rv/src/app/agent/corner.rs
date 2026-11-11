//! What the corner over the bar says about the shown file: its purpose as the
//! agent put it, how many of the agent's flags in it are still open, and the
//! agent keys as the keymap spells them now.

use rv_core::store::FlaggedBy;

use crate::app::App;
use crate::app::bindings::AgentCommand;
use crate::app::bindings::Command;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CornerTip {
    /// Empty until the file has been oriented at its current blob.
    pub purpose: String,
    pub flagged: usize,
    pub hints: Vec<String>,
}

impl App {
    /// `None` with no agent configured or no file selected. Reads the memo
    /// [`App::collect_agent`] keeps, so a frame hashes nothing.
    #[must_use]
    pub fn corner_tip(&self) -> Option<CornerTip> {
        let live = self.assistant.live.as_ref()?;
        let file = self.selected_file()?;
        let purpose = match &live.shown {
            Some((index, Some(key))) if *index == self.file_index => {
                live.purposes.get(key).cloned().unwrap_or_default()
            }
            _ => String::new(),
        };
        let flagged = self
            .flags
            .iter()
            .filter(|flag| {
                flag.by == FlaggedBy::Agent
                    && !flag.acknowledged
                    && (flag.anchor.file == file.path
                        || file.source_path.as_deref() == Some(flag.anchor.file.as_str()))
            })
            .count();
        let wanted = [
            (Command::Agent(AgentCommand::Ask), "ask"),
            (Command::Agent(AgentCommand::NextFlag), "next flag"),
            (Command::Agent(AgentCommand::ShowAnswer), "answer"),
        ];
        let hints = wanted
            .into_iter()
            .filter_map(|(command, what)| {
                self.keymap
                    .chord_for(command)
                    .map(|chord| format!("{chord} {what}"))
            })
            .collect();
        Some(CornerTip {
            purpose,
            flagged,
            hints,
        })
    }
}
