//! One key press in, one [`Action`] out — with no terminal anywhere in reach.

use anyhow::Result;
use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyModifiers;

use super::Action;
use super::App;
use super::HelpStage;
use super::Mode;
use super::SidebarTab;
use super::bindings::AppCommand;
use super::bindings::Command;
use super::bindings::CommentCommand;
use super::bindings::CursorCommand;
use super::bindings::DiffCommand;
use super::bindings::FilesCommand;
use super::bindings::LayoutCommand;
use super::bindings::Leader;
use super::bindings::PaneCommand;
use super::keymap::RuntimeBinding;

/// How many percentage points one press of `<` or `>` moves the divider.
///
/// Two rather than one: a resize the reviewer cannot see happen is a resize
/// they will hold the key down for.
const NUDGE: i16 = 2;

/// How many columns one press of `H` or `L` scrolls the focused pane sideways.
/// Eight: enough that a long line is crossed in a few presses, few enough that
/// the text stays followable while it moves.
const HSCROLL_STEP: isize = 8;

impl App {
    /// Handles one key press, modifiers and all.
    ///
    /// Ctrl+C is answered here rather than below because the state machine is
    /// written against plain [`KeyCode`]s — which is what makes it testable
    /// without a pty — and `Char('c')` with CONTROL held is indistinguishable
    /// from a plain `c` once the modifiers are dropped. In raw mode the
    /// terminal raises no SIGINT, so without this the one key every terminal
    /// user reaches for would open the comment box and type into it.
    ///
    /// It quits from any mode, half-typed comment included: an abort that first
    /// asks you to `Esc` is not an abort.
    pub fn on_key_event(&mut self, event: KeyEvent) -> Result<Action> {
        if event.modifiers.contains(KeyModifiers::CONTROL)
            && matches!(event.code, KeyCode::Char('c'))
        {
            return Ok(Action::Quit);
        }
        // Shift+arrows scroll the focused pane's text sideways. Answered here
        // for the same reason Ctrl+C is: dropping the modifier would turn
        // Shift+Left into a plain Left and drill the tree or move the focus
        // instead. Browse only — behind the keymap or in a mode that takes
        // text, a shifted arrow must stay as inert as any other key there.
        if event.modifiers.contains(KeyModifiers::SHIFT)
            && self.mode == Mode::Browse
            && self.help == HelpStage::Closed
        {
            match event.code {
                KeyCode::Left => {
                    return self.run_command(Command::Cursor(CursorCommand::ScrollLeft));
                }
                KeyCode::Right => {
                    return self.run_command(Command::Cursor(CursorCommand::ScrollRight));
                }
                _ => {}
            }
        }
        self.on_key(event.code)
    }

    /// Handles one key press.
    ///
    /// The keymap is answered ahead of the mode because it is a modal window
    /// rather than a mode: it can only be raised from [`Mode::Browse`] and
    /// nothing behind it can change the mode.
    pub fn on_key(&mut self, key: KeyCode) -> Result<Action> {
        if self.help != HelpStage::Closed {
            return Ok(self.on_key_help(key));
        }

        match self.mode {
            Mode::Browse => self.on_key_browse(key),
            Mode::Comment => self.on_key_comment(key),
            Mode::ConfirmDelete { .. } => self.on_key_confirm_delete(key),
            Mode::Pick => self.on_key_pick(key),
        }
    }

    /// The five keys the `?` popup answers; everything else is inert while it
    /// is up, because a reviewer reading about `d` must not discover what it
    /// does by pressing it.
    ///
    /// `?` walks the stages — the tip grows into the whole keymap, the keymap
    /// closes — and `q` **closes** rather than quits: the reviewer with the
    /// manual open is the one least sure what the keys do, and ending their
    /// review is the most expensive way to find out.
    fn on_key_help(&mut self, key: KeyCode) -> Action {
        match key {
            KeyCode::Char('?') => {
                self.help = match self.help {
                    HelpStage::Closed | HelpStage::Tip => HelpStage::Full,
                    HelpStage::Full => HelpStage::Closed,
                };
                self.help_scroll = 0;
            }
            KeyCode::Char('q') | KeyCode::Esc => self.help = HelpStage::Closed,
            KeyCode::Char('j') | KeyCode::Down => self.scroll_help(1),
            KeyCode::Char('k') | KeyCode::Up => self.scroll_help(-1),
            _ => {}
        }
        Action::Continue
    }

    /// Puts the change tooltip away, or brings it back.
    ///
    /// It shows itself on highlight, so this is the way *out* rather than the way
    /// in: a reviewer who wants the diff pane whole while they walk a stack should
    /// not have to move the cursor off a change to get it.
    fn toggle_info(&mut self) {
        self.info_dismissed = !self.info_dismissed;
        self.info_scroll = 0;
        self.status = if self.info_dismissed {
            "change details hidden — i brings them back".to_owned()
        } else {
            "change details shown".to_owned()
        };
    }

    /// Flips the full-file-context toggle. Full context is the default (spec
    /// §5, walked back), and this is what turns it off — a reviewer who
    /// wants to see less on a big file, or who has hit a "context
    /// unavailable" file where the merge legitimately declined and wants
    /// the changed-only view without the title suffix.
    fn toggle_full_context(&mut self) {
        self.set_full_context(!self.full_context());
        self.status = if self.full_context() {
            "full-file context — f shows only the changes".to_owned()
        } else {
            "changes only — f shows the whole file".to_owned()
        };
    }

    /// `v g`: groups each hunk's removals before its additions instead of
    /// difftastic's interleaving. Session-only.
    fn toggle_grouped(&mut self) {
        self.grouped = !self.grouped;
        self.status = if self.grouped {
            "grouped diff — v g interleaves again".to_owned()
        } else {
            "interleaved diff — v g groups by side".to_owned()
        };
    }

    /// `v b`: cycles the diff pane through both sides, the base alone, and the
    /// head alone.
    fn cycle_view_side(&mut self) {
        self.view_side = self.view_side.next();
        self.status = format!("showing {} — v b cycles", self.view_side.label());
    }

    /// Moves the keymap by `delta` rows, which only ever moves anything on a
    /// terminal too small to show the whole table.
    pub(super) fn scroll_help(&mut self, delta: isize) {
        self.help_scroll = self.help_scroll.saturating_add_signed(delta);
    }

    /// Looks `key` up in [`BINDINGS`] and runs whatever row claims it.
    ///
    /// A lookup rather than a `match`: the table is what the `?` popup is drawn
    /// from, so dispatching through it is what makes an undocumented binding
    /// unrepresentable. A key no row claims is inert.
    fn on_key_browse(&mut self, key: KeyCode) -> Result<Action> {
        let context = self.context();
        if let Some(leader) = self.pending_leader.take() {
            if let Some(command) = self
                .keymap
                .lookup(Some(leader), key, context)
                .map(|binding| binding.command)
            {
                return self.run_command(command);
            }
            return Ok(Action::Continue);
        }

        if let Some(leader) = Leader::ALL
            .iter()
            .copied()
            .find(|leader| key == KeyCode::Char(self.keymap.leader_key(*leader)))
        {
            let live: Vec<&RuntimeBinding> = self
                .keymap
                .bindings()
                .iter()
                .filter(|binding| {
                    binding.leader == Some(leader)
                        && rt_shown_in(binding, context)
                        && binding.command.targets_cursor()
                        && self.rt_binding_enabled(binding)
                })
                .collect();
            if let [only] = live[..] {
                self.status = format!("{} → {}", leader.label(), only.what);
                return self.run_command(only.command);
            }
            self.pending_leader = Some(leader);
            return Ok(Action::Continue);
        }

        let Some(command) = self
            .keymap
            .lookup(None, key, context)
            .map(|binding| binding.command)
        else {
            return Ok(Action::Continue);
        };
        self.run_command(command)
    }

    /// Runs one row of [`BINDINGS`]: one delegation per command group, each
    /// exhaustive over its sub-enum, which is the other half of the table's
    /// anti-drift claim.
    fn run_command(&mut self, command: Command) -> Result<Action> {
        match command {
            Command::Cursor(command) => self.run_cursor(command)?,
            Command::Pane(command) => self.run_pane(command)?,
            Command::Files(command) => self.run_files(command)?,
            Command::Diff(command) => self.run_diff(command)?,
            Command::Comment(command) => self.run_comment(command)?,
            Command::Layout(command) => self.run_layout(command),
            Command::App(command) => return self.run_app(command),
        }
        Ok(Action::Continue)
    }

    fn run_cursor(&mut self, command: CursorCommand) -> Result<()> {
        match command {
            CursorCommand::NextRow => self.move_forward()?,
            CursorCommand::PrevRow => self.move_back()?,
            CursorCommand::PageDown => self.page_forward()?,
            CursorCommand::PageUp => self.page_backward()?,
            CursorCommand::FirstRow => self.jump_first()?,
            CursorCommand::LastRow => self.jump_last()?,
            CursorCommand::ScrollLeft => self.hscroll_focused(-HSCROLL_STEP),
            CursorCommand::ScrollRight => self.hscroll_focused(HSCROLL_STEP),
        }
        Ok(())
    }

    fn run_pane(&mut self, command: PaneCommand) -> Result<()> {
        match command {
            PaneCommand::FocusLeft => self.focus_left()?,
            PaneCommand::FocusRight => self.focus_right()?,
            PaneCommand::Open => self.on_enter()?,
            PaneCommand::BackOut => self.escape(),
            PaneCommand::CycleTab => self.cycle_mode()?,
            PaneCommand::GotoFiles => self.goto_mode(SidebarTab::Files)?,
            PaneCommand::GotoCommits => self.goto_mode(SidebarTab::Commits)?,
            PaneCommand::GotoComments => self.goto_mode(SidebarTab::Comments)?,
            PaneCommand::GotoDiff => self.focus = super::Focus::Diff,
        }
        Ok(())
    }

    fn run_files(&mut self, command: FilesCommand) -> Result<()> {
        match command {
            // `[` and `]` consult no focus at all, so walking a review never
            // costs a trip through the sidebar.
            FilesCommand::Next => self.select_file(self.file_index.saturating_add(1))?,
            FilesCommand::Prev => self.select_file(self.file_index.saturating_sub(1))?,
            FilesCommand::ToggleTree => self.toggle_tree(),
            FilesCommand::CycleSort => self.cycle_sort(),
            FilesCommand::ToggleTint => self.toggle_tint(),
            FilesCommand::ToggleCounts => self.toggle_counts(),
        }
        Ok(())
    }

    fn run_diff(&mut self, command: DiffCommand) -> Result<()> {
        match command {
            DiffCommand::NextHunk => self.next_hunk(),
            DiffCommand::PrevHunk => self.previous_hunk(),
            DiffCommand::NextSymbol => self.next_symbol()?,
            DiffCommand::PrevSymbol => self.previous_symbol()?,
            DiffCommand::FindSymbol => self.begin_pick(),
            DiffCommand::ToggleFullContext => self.toggle_full_context(),
            DiffCommand::GroupBySide => self.toggle_grouped(),
            DiffCommand::CycleSide => self.cycle_view_side(),
        }
        Ok(())
    }

    fn run_comment(&mut self, command: CommentCommand) -> Result<()> {
        match command {
            CommentCommand::Write => self.begin_comment(),
            CommentCommand::Delete => self.begin_delete(),
            CommentCommand::Resolve => self.resolve_comment()?,
            CommentCommand::Abandon => self.abandon_comment()?,
            CommentCommand::ToggleFold => self.toggle_collapse(),
        }
        Ok(())
    }

    fn run_layout(&mut self, command: LayoutCommand) {
        match command {
            LayoutCommand::SidebarNarrower => self.split = self.split.nudged(-NUDGE),
            LayoutCommand::SidebarWider => self.split = self.split.nudged(NUDGE),
            LayoutCommand::ToggleSidebar => self.toggle_sidebar(),
        }
    }

    fn run_app(&mut self, command: AppCommand) -> Result<Action> {
        match command {
            AppCommand::Quit => return Ok(Action::Quit),
            AppCommand::OpenEditor => return Ok(self.begin_edit()),
            AppCommand::Refresh => self.refresh()?,
            AppCommand::ToggleChangeDetails => self.toggle_info(),
            AppCommand::Help => {
                // The tip first: what `?` answers is "what can I do here", and
                // the whole manual is one more press away.
                self.help = HelpStage::Tip;
                // Opened at the top, always: the geometry it was last scrolled
                // against may have changed since.
                self.help_scroll = 0;
            }
        }
        Ok(Action::Continue)
    }
}

fn rt_shown_in(binding: &RuntimeBinding, context: super::Context) -> bool {
    binding.contexts.is_empty() || binding.contexts.contains(&context)
}
