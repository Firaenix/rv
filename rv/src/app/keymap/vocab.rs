//! The config file's command vocabulary: one canonical `<group>_<action>`
//! name per [`Command`], derived from the enum path so the prefix says what
//! is acted on. No aliases — two spellings would be two dialects of config
//! file for other people's dotfiles to disagree in.

use super::super::bindings::{
    AppCommand, BINDINGS, Command, CommentCommand, CursorCommand, DiffCommand, FilesCommand, Group,
    LayoutCommand, PaneCommand,
};

/// Exhaustive by construction: a new variant without a config name is a
/// compile error, not a test failure.
pub(in crate::app) fn config_name(command: Command) -> &'static str {
    match command {
        Command::Cursor(c) => match c {
            CursorCommand::NextRow => "cursor_next_row",
            CursorCommand::PrevRow => "cursor_prev_row",
            CursorCommand::PageDown => "cursor_page_down",
            CursorCommand::PageUp => "cursor_page_up",
            CursorCommand::FirstRow => "cursor_first_row",
            CursorCommand::LastRow => "cursor_last_row",
            CursorCommand::ScrollLeft => "cursor_scroll_left",
            CursorCommand::ScrollRight => "cursor_scroll_right",
        },
        Command::Pane(c) => match c {
            PaneCommand::FocusLeft => "pane_focus_left",
            PaneCommand::FocusRight => "pane_focus_right",
            PaneCommand::Open => "pane_open",
            PaneCommand::BackOut => "pane_back_out",
            PaneCommand::CycleTab => "pane_cycle_tab",
            PaneCommand::GotoFiles => "pane_goto_files",
            PaneCommand::GotoCommits => "pane_goto_commits",
            PaneCommand::GotoComments => "pane_goto_comments",
            PaneCommand::GotoDiff => "pane_goto_diff",
        },
        Command::Files(c) => match c {
            FilesCommand::Next => "files_next",
            FilesCommand::Prev => "files_prev",
            FilesCommand::ToggleTree => "files_toggle_tree",
            FilesCommand::CycleSort => "files_cycle_sort",
            FilesCommand::ToggleTint => "files_toggle_tint",
            FilesCommand::ToggleCounts => "files_toggle_counts",
        },
        Command::Diff(c) => match c {
            DiffCommand::NextHunk => "diff_next_hunk",
            DiffCommand::PrevHunk => "diff_prev_hunk",
            DiffCommand::NextSymbol => "diff_next_symbol",
            DiffCommand::PrevSymbol => "diff_prev_symbol",
            DiffCommand::FindSymbol => "diff_find_symbol",
            DiffCommand::ToggleFullContext => "diff_toggle_full_context",
            DiffCommand::GroupBySide => "diff_group_by_side",
            DiffCommand::CycleSide => "diff_cycle_side",
        },
        Command::Comment(c) => match c {
            CommentCommand::Write => "comment_write",
            CommentCommand::Delete => "comment_delete",
            CommentCommand::Resolve => "comment_resolve",
            CommentCommand::Abandon => "comment_abandon",
            CommentCommand::ToggleFold => "comment_toggle_fold",
        },
        Command::Layout(c) => match c {
            LayoutCommand::SidebarWider => "layout_sidebar_wider",
            LayoutCommand::SidebarNarrower => "layout_sidebar_narrower",
            LayoutCommand::ToggleSidebar => "layout_toggle_sidebar",
        },
        Command::App(c) => match c {
            AppCommand::Help => "app_help",
            AppCommand::Quit => "app_quit",
            AppCommand::Refresh => "app_refresh",
            AppCommand::OpenEditor => "app_open_editor",
            AppCommand::ToggleChangeDetails => "app_toggle_change_details",
        },
    }
}

/// Every command, in the order the seeded config template lists them.
pub(in crate::app) const ALL_COMMANDS: &[Command] = &[
    Command::Cursor(CursorCommand::NextRow),
    Command::Cursor(CursorCommand::PrevRow),
    Command::Cursor(CursorCommand::PageDown),
    Command::Cursor(CursorCommand::PageUp),
    Command::Cursor(CursorCommand::FirstRow),
    Command::Cursor(CursorCommand::LastRow),
    Command::Cursor(CursorCommand::ScrollLeft),
    Command::Cursor(CursorCommand::ScrollRight),
    Command::Pane(PaneCommand::FocusLeft),
    Command::Pane(PaneCommand::FocusRight),
    Command::Pane(PaneCommand::Open),
    Command::Pane(PaneCommand::BackOut),
    Command::Pane(PaneCommand::CycleTab),
    Command::Pane(PaneCommand::GotoFiles),
    Command::Pane(PaneCommand::GotoCommits),
    Command::Pane(PaneCommand::GotoComments),
    Command::Pane(PaneCommand::GotoDiff),
    Command::Files(FilesCommand::Next),
    Command::Files(FilesCommand::Prev),
    Command::Files(FilesCommand::ToggleTree),
    Command::Files(FilesCommand::CycleSort),
    Command::Files(FilesCommand::ToggleTint),
    Command::Files(FilesCommand::ToggleCounts),
    Command::Diff(DiffCommand::NextHunk),
    Command::Diff(DiffCommand::PrevHunk),
    Command::Diff(DiffCommand::NextSymbol),
    Command::Diff(DiffCommand::PrevSymbol),
    Command::Diff(DiffCommand::FindSymbol),
    Command::Diff(DiffCommand::ToggleFullContext),
    Command::Diff(DiffCommand::GroupBySide),
    Command::Diff(DiffCommand::CycleSide),
    Command::Comment(CommentCommand::Write),
    Command::Comment(CommentCommand::Delete),
    Command::Comment(CommentCommand::Resolve),
    Command::Comment(CommentCommand::Abandon),
    Command::Comment(CommentCommand::ToggleFold),
    Command::Layout(LayoutCommand::SidebarWider),
    Command::Layout(LayoutCommand::SidebarNarrower),
    Command::Layout(LayoutCommand::ToggleSidebar),
    Command::App(AppCommand::Help),
    Command::App(AppCommand::Quit),
    Command::App(AppCommand::Refresh),
    Command::App(AppCommand::OpenEditor),
    Command::App(AppCommand::ToggleChangeDetails),
];

pub(super) fn command_by_name(name: &str) -> Option<Command> {
    ALL_COMMANDS
        .iter()
        .copied()
        .find(|command| config_name(*command) == name)
}

/// The heading and description a user-created row takes: the command's first
/// default row, or the fallback for the two scroll commands the default table
/// never lists (they ship on hardwired Shift+arrows).
pub(super) fn meta_for(command: Command) -> (Group, &'static str) {
    if let Some(row) = BINDINGS.iter().find(|binding| binding.command == command) {
        return (row.group, row.what);
    }
    match command {
        Command::Cursor(CursorCommand::ScrollLeft) => (Group::Scroll, "scroll left"),
        Command::Cursor(CursorCommand::ScrollRight) => (Group::Scroll, "scroll right"),
        _ => (Group::View, config_name(command)),
    }
}
