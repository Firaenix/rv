//! The agent's keys. Asking is a shifted letter like every other key that
//! writes to `.review/`: an answer files its focus lines as flags and keeps
//! the agent's session id, and the prompt leaves the machine. Reading the
//! answer writes nothing, so it is lowercase.

use crossterm::event::KeyCode;

use super::AgentCommand;
use super::Binding;
use super::Command;
use super::Group;
use super::Leader;

pub(super) const AGENT: [Binding; 5] = [
    Binding {
        keys: "K",
        group: Group::Review,
        leader: None,
        contexts: &[],
        what: "ask agent",
        codes: &[KeyCode::Char('K')],
        command: Command::Agent(AgentCommand::Ask),
    },
    Binding {
        keys: "B",
        group: Group::Review,
        leader: None,
        contexts: &[],
        what: "brief agent",
        codes: &[KeyCode::Char('B')],
        command: Command::Agent(AgentCommand::OrientReview),
    },
    Binding {
        keys: "a",
        group: Group::Review,
        leader: Some(Leader::Goto),
        contexts: &[],
        what: "next agent",
        codes: &[KeyCode::Char('a')],
        command: Command::Agent(AgentCommand::NextFlag),
    },
    Binding {
        keys: "A",
        group: Group::Review,
        leader: Some(Leader::Goto),
        contexts: &[],
        what: "prev agent",
        codes: &[KeyCode::Char('A')],
        command: Command::Agent(AgentCommand::PrevFlag),
    },
    Binding {
        keys: "a",
        group: Group::View,
        leader: None,
        contexts: &[],
        what: "answer",
        codes: &[KeyCode::Char('a')],
        command: Command::Agent(AgentCommand::ShowAnswer),
    },
];
