//! Line provenance: the head side's gutter of which change last touched each
//! line, and the card naming the change behind the line under the cursor.
//! Both only read, so both are lowercase.

use crossterm::event::KeyCode;

use super::Binding;
use super::Command;
use super::Context;
use super::Group;
use super::Leader;
use super::ProvenanceCommand;

pub(super) const PROVENANCE: [Binding; 2] = [
    Binding {
        keys: "p",
        group: Group::View,
        leader: Some(Leader::View),
        contexts: &[],
        what: "provenance",
        codes: &[KeyCode::Char('p')],
        command: Command::Provenance(ProvenanceCommand::ToggleGutter),
    },
    // `i` in the commits list is the change details; on a line it is the
    // change that wrote it.
    Binding {
        keys: "i",
        group: Group::Review,
        leader: None,
        contexts: &[Context::Diff],
        what: "who wrote",
        codes: &[KeyCode::Char('i')],
        command: Command::Provenance(ProvenanceCommand::ShowOrigin),
    },
];
