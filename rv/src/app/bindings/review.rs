//! The direct action keys, and the review-progress and code-navigation keys
//! added in 1.8. Split from [`super::table`] for the 400-line rule and
//! concatenated back into `BINDINGS` there.
//!
//! **Uppercase acts, lowercase moves.** Every key that writes to `.review/`
//! or leaves the reviewer is a shifted letter — `C D R A F X E` — so a
//! mutation is always a deliberate chord, and a lowercase letter is always
//! safe to press: it moves, toggles a view, or opens a menu. `N` is the one
//! documented exception, a convention older than the rule, and it mutates
//! nothing.

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

pub(super) const REVIEW: [Binding; 16] = [
    Binding {
        keys: "C",
        group: Group::Comment,
        leader: None,
        contexts: &[],
        what: "comment",
        codes: &[KeyCode::Char('C')],
        command: Command::Comment(CommentCommand::Write),
    },
    Binding {
        keys: "D",
        group: Group::Comment,
        leader: None,
        contexts: &[],
        what: "delete",
        codes: &[KeyCode::Char('D')],
        command: Command::Comment(CommentCommand::Delete),
    },
    Binding {
        keys: "R",
        group: Group::Comment,
        leader: None,
        contexts: &[],
        what: "resolve",
        codes: &[KeyCode::Char('R')],
        command: Command::Comment(CommentCommand::Resolve),
    },
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
        keys: "X",
        group: Group::Review,
        leader: None,
        contexts: &[],
        what: "reviewed",
        codes: &[KeyCode::Char('X')],
        command: Command::Files(FilesCommand::ToggleReviewed),
    },
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
