//! The `v` (view) leader's bindings — the complete, stable list of display
//! toggles — split from [`super::table`] for the 400-line rule and concatenated
//! back into `BINDINGS` there.

use crossterm::event::KeyCode;

use super::AppCommand;
use super::Binding;
use super::Command;
use super::CommentCommand;
use super::DiffCommand;
use super::FilesCommand;
use super::Group;
use super::LayoutCommand;
use super::Leader;

pub(super) const VIEW: [Binding; 12] = [
    Binding {
        keys: "f",
        group: Group::View,
        leader: Some(Leader::View),
        contexts: &[],
        what: "context",
        codes: &[KeyCode::Char('f')],
        command: Command::Diff(DiffCommand::ToggleFullContext),
    },
    Binding {
        keys: "g",
        group: Group::View,
        leader: Some(Leader::View),
        contexts: &[],
        what: "group",
        codes: &[KeyCode::Char('g')],
        command: Command::Diff(DiffCommand::GroupBySide),
    },
    Binding {
        keys: "b",
        group: Group::View,
        leader: Some(Leader::View),
        contexts: &[],
        what: "side",
        codes: &[KeyCode::Char('b')],
        command: Command::Diff(DiffCommand::CycleSide),
    },
    Binding {
        keys: "t",
        group: Group::View,
        leader: Some(Leader::View),
        contexts: &[],
        what: "list / tree",
        codes: &[KeyCode::Char('t')],
        command: Command::Files(FilesCommand::ToggleTree),
    },
    Binding {
        keys: "o",
        group: Group::View,
        leader: Some(Leader::View),
        contexts: &[],
        what: "order",
        codes: &[KeyCode::Char('o')],
        command: Command::Files(FilesCommand::CycleSort),
    },
    Binding {
        keys: "c",
        group: Group::View,
        leader: Some(Leader::View),
        contexts: &[],
        what: "tint",
        codes: &[KeyCode::Char('c')],
        command: Command::Files(FilesCommand::ToggleTint),
    },
    Binding {
        keys: "#",
        group: Group::View,
        leader: Some(Leader::View),
        contexts: &[],
        what: "counts",
        codes: &[KeyCode::Char('#')],
        command: Command::Files(FilesCommand::ToggleCounts),
    },
    Binding {
        keys: "z",
        group: Group::View,
        leader: Some(Leader::View),
        contexts: &[],
        what: "sidebar",
        codes: &[KeyCode::Char('z')],
        command: Command::Layout(LayoutCommand::ToggleSidebar),
    },
    Binding {
        keys: "<",
        group: Group::View,
        leader: Some(Leader::View),
        contexts: &[],
        what: "narrower",
        codes: &[KeyCode::Char('<')],
        command: Command::Layout(LayoutCommand::SidebarNarrower),
    },
    Binding {
        keys: ">",
        group: Group::View,
        leader: Some(Leader::View),
        contexts: &[],
        what: "wider",
        codes: &[KeyCode::Char('>')],
        command: Command::Layout(LayoutCommand::SidebarWider),
    },
    Binding {
        keys: "i",
        group: Group::View,
        leader: Some(Leader::View),
        contexts: &[],
        what: "details",
        codes: &[KeyCode::Char('i')],
        command: Command::App(AppCommand::ToggleChangeDetails),
    },
    Binding {
        keys: "r",
        group: Group::View,
        leader: Some(Leader::View),
        contexts: &[],
        what: "refresh",
        codes: &[KeyCode::Char('r')],
        command: Command::App(AppCommand::Refresh),
    },
];

/// The `g` (goto) and `c` (comment) leaders' children, split from
/// [`super::table`] for length.
pub(super) const JUMPS: [Binding; 9] = [
    Binding {
        keys: "↓",
        group: Group::Move,
        leader: Some(Leader::Goto),
        contexts: &[],
        what: "next hunk",
        codes: &[KeyCode::Down],
        command: Command::Diff(DiffCommand::NextHunk),
    },
    Binding {
        keys: "↑",
        group: Group::Move,
        leader: Some(Leader::Goto),
        contexts: &[],
        what: "prev hunk",
        codes: &[KeyCode::Up],
        command: Command::Diff(DiffCommand::PrevHunk),
    },
    Binding {
        keys: "n",
        group: Group::Move,
        leader: Some(Leader::Goto),
        contexts: &[],
        what: "next sym",
        codes: &[KeyCode::Char('n')],
        command: Command::Diff(DiffCommand::NextSymbol),
    },
    Binding {
        keys: "N",
        group: Group::Move,
        leader: Some(Leader::Goto),
        contexts: &[],
        what: "prev sym",
        codes: &[KeyCode::Char('N')],
        command: Command::Diff(DiffCommand::PrevSymbol),
    },
    Binding {
        keys: "/",
        group: Group::Move,
        leader: Some(Leader::Goto),
        contexts: &[],
        what: "find",
        codes: &[KeyCode::Char('/')],
        command: Command::Diff(DiffCommand::FindSymbol),
    },
    Binding {
        keys: "c",
        group: Group::Comment,
        leader: Some(Leader::Comment),
        contexts: &[],
        what: "write a comment",
        codes: &[KeyCode::Char('c')],
        command: Command::Comment(CommentCommand::Write),
    },
    Binding {
        keys: "d",
        group: Group::Comment,
        leader: Some(Leader::Comment),
        contexts: &[],
        what: "delete comment",
        codes: &[KeyCode::Char('d')],
        command: Command::Comment(CommentCommand::Delete),
    },
    Binding {
        keys: "r",
        group: Group::Comment,
        leader: Some(Leader::Comment),
        contexts: &[],
        what: "resolve",
        codes: &[KeyCode::Char('r')],
        command: Command::Comment(CommentCommand::Resolve),
    },
    Binding {
        keys: "a",
        group: Group::Comment,
        leader: Some(Leader::Comment),
        contexts: &[],
        what: "abandon",
        codes: &[KeyCode::Char('a')],
        command: Command::Comment(CommentCommand::Abandon),
    },
];
