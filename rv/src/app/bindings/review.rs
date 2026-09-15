//! The review-progress and code-navigation keys added in 1.8: ticking files
//! off, flags, plain-text search, and the column cursor's definition and
//! reference jumps. Split from [`super::table`] for the 400-line rule and
//! concatenated back into `BINDINGS` there.

use crossterm::event::KeyCode;

use super::Binding;
use super::Command;
use super::CommentCommand;
use super::Context;
use super::CursorCommand;
use super::DiffCommand;
use super::FilesCommand;
use super::Group;
use super::Leader;
use super::PaneCommand;

pub(super) const REVIEW: [Binding; 13] = [
    Binding {
        keys: "F",
        group: Group::Focus,
        leader: Some(Leader::Mode),
        contexts: &[],
        what: "flags",
        codes: &[KeyCode::Char('F')],
        command: Command::Pane(PaneCommand::GotoFlags),
    },
    Binding {
        keys: "x",
        group: Group::Review,
        leader: None,
        contexts: &[],
        what: "reviewed",
        codes: &[KeyCode::Char('x')],
        command: Command::Files(FilesCommand::ToggleReviewed),
    },
    // Direct rather than under `c`: a lone live child collapses its leader,
    // and `c` with only "write" live is how a plain `c` writes a comment —
    // a second child there would cost every comment a keystroke.
    Binding {
        keys: "F",
        group: Group::Review,
        leader: None,
        contexts: &[Context::Diff, Context::Stack],
        what: "flag line",
        codes: &[KeyCode::Char('F')],
        command: Command::Comment(CommentCommand::Flag),
    },
    Binding {
        keys: "A",
        group: Group::Review,
        leader: None,
        contexts: &[Context::Diff, Context::Stack, Context::Flags],
        what: "ack flag",
        codes: &[KeyCode::Char('A')],
        command: Command::Comment(CommentCommand::Acknowledge),
    },
    Binding {
        keys: "f",
        group: Group::Review,
        leader: Some(Leader::Goto),
        contexts: &[],
        what: "next flag",
        codes: &[KeyCode::Char('f')],
        command: Command::Diff(DiffCommand::NextFlag),
    },
    Binding {
        keys: "F",
        group: Group::Review,
        leader: Some(Leader::Goto),
        contexts: &[],
        what: "prev flag",
        codes: &[KeyCode::Char('F')],
        command: Command::Diff(DiffCommand::PrevFlag),
    },
    Binding {
        keys: "/",
        group: Group::Scroll,
        leader: None,
        contexts: &[],
        what: "find text",
        codes: &[KeyCode::Char('/')],
        command: Command::Diff(DiffCommand::Search),
    },
    Binding {
        keys: "n",
        group: Group::Scroll,
        leader: None,
        contexts: &[],
        what: "next hit",
        codes: &[KeyCode::Char('n')],
        command: Command::Diff(DiffCommand::NextMatch),
    },
    Binding {
        keys: "N",
        group: Group::Scroll,
        leader: None,
        contexts: &[],
        what: "prev hit",
        codes: &[KeyCode::Char('N')],
        command: Command::Diff(DiffCommand::PrevMatch),
    },
    // Direct on the diff only: in a list `h`/`l` would be a cursor with no
    // line to sit in.
    Binding {
        keys: "h",
        group: Group::Scroll,
        leader: None,
        contexts: &[Context::Diff],
        what: "prev word",
        codes: &[KeyCode::Char('h')],
        command: Command::Cursor(CursorCommand::WordLeft),
    },
    Binding {
        keys: "l",
        group: Group::Scroll,
        leader: None,
        contexts: &[Context::Diff],
        what: "next word",
        codes: &[KeyCode::Char('l')],
        command: Command::Cursor(CursorCommand::WordRight),
    },
    Binding {
        keys: "d",
        group: Group::Scroll,
        leader: Some(Leader::Goto),
        contexts: &[],
        what: "go to def",
        codes: &[KeyCode::Char('d')],
        command: Command::Diff(DiffCommand::Definition),
    },
    Binding {
        keys: "r",
        group: Group::Scroll,
        leader: Some(Leader::Goto),
        contexts: &[],
        what: "go to ref",
        codes: &[KeyCode::Char('r')],
        command: Command::Diff(DiffCommand::References),
    },
];
