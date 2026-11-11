//! The `g` leader's code-navigation children added with hover cards: what a
//! symbol is, opening any file by name, searching every file, and walking
//! back the way the jumps came.

use crossterm::event::KeyCode;

use super::Binding;
use super::Command;
use super::DiffCommand;
use super::Group;
use super::Leader;

pub(super) const GOTO: [Binding; 4] = [
    Binding {
        keys: "i",
        group: Group::Scroll,
        leader: Some(Leader::Goto),
        contexts: &[],
        what: "symbol info",
        codes: &[KeyCode::Char('i')],
        command: Command::Diff(DiffCommand::Hover),
    },
    Binding {
        keys: "o",
        group: Group::Scroll,
        leader: Some(Leader::Goto),
        contexts: &[],
        what: "open file",
        codes: &[KeyCode::Char('o')],
        command: Command::Diff(DiffCommand::OpenFile),
    },
    Binding {
        keys: "s",
        group: Group::Scroll,
        leader: Some(Leader::Goto),
        contexts: &[],
        what: "search all",
        codes: &[KeyCode::Char('s')],
        command: Command::Diff(DiffCommand::SearchRepo),
    },
    Binding {
        keys: "b",
        group: Group::Scroll,
        leader: Some(Leader::Goto),
        contexts: &[],
        what: "jump back",
        codes: &[KeyCode::Char('b')],
        command: Command::Diff(DiffCommand::JumpBack),
    },
];
