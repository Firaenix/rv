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
use super::bindings::BINDINGS;
use super::bindings::Command;

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
        // Shift+arrows are the plain arrows one layer deeper. In the sidebar
        // they walk the *tree* — right into the folder or change under the
        // cursor, left back out — and in the diff they scroll the text
        // sideways, as `H` and `L` do. Answered here for the same reason
        // Ctrl+C is: dropping the modifiers would turn Shift+Left into a
        // plain Left and move the focus instead. Browse only — behind the
        // keymap or in a mode that takes text, a shifted arrow must stay as
        // inert as any other key there.
        if event.modifiers.contains(KeyModifiers::SHIFT)
            && self.mode == Mode::Browse
            && self.help == HelpStage::Closed
        {
            let in_tree = self.focus == super::Focus::Sidebar
                && self.sidebar_tab != super::SidebarTab::Comments;
            match (event.code, in_tree) {
                (KeyCode::Right, true) => {
                    self.zoom_into_under_cursor();
                    return Ok(Action::Continue);
                }
                (KeyCode::Left, true) => {
                    self.zoom_out();
                    return Ok(Action::Continue);
                }
                (KeyCode::Left, false) => return self.run_command(Command::ScrollLeft),
                (KeyCode::Right, false) => return self.run_command(Command::ScrollRight),
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
        let Some(binding) = BINDINGS.iter().find(|binding| binding.codes.contains(&key)) else {
            return Ok(Action::Continue);
        };
        self.run_command(binding.command)
    }

    /// Runs one row of [`BINDINGS`]. Exhaustive over [`Command`] by
    /// construction, which is the other half of the table's anti-drift claim.
    fn run_command(&mut self, command: Command) -> Result<Action> {
        match command {
            Command::Quit => return Ok(Action::Quit),
            Command::FocusLeft => self.focus_left(),
            Command::FocusRight => self.focus_right(),
            Command::Forward => self.move_forward()?,
            Command::Back => self.move_back()?,
            Command::PageForward => self.page_forward()?,
            Command::PageBackward => self.page_backward()?,
            Command::JumpFirst => self.jump_first()?,
            Command::JumpLast => self.jump_last()?,
            // `[` and `]` consult no focus at all, so walking a review never
            // costs a trip through the sidebar.
            Command::NextFile => self.select_file(self.file_index.saturating_add(1))?,
            Command::PreviousFile => self.select_file(self.file_index.saturating_sub(1))?,
            Command::NextHunk => self.next_hunk(),
            Command::PreviousHunk => self.previous_hunk(),
            Command::NextSymbol => self.next_symbol()?,
            Command::PreviousSymbol => self.previous_symbol()?,
            Command::ScrollLeft => self.hscroll_focused(-HSCROLL_STEP),
            Command::ScrollRight => self.hscroll_focused(HSCROLL_STEP),
            Command::Pick => self.begin_pick(),
            Command::Comment => self.begin_comment(),
            Command::Delete => self.begin_delete(),
            Command::Resolve => self.resolve_comment()?,
            Command::Abandon => self.abandon_comment()?,
            Command::Export => self.export()?,
            Command::OpenEditor => return Ok(self.begin_edit()),
            Command::Fold => self.toggle_collapse(),
            // Focus-free, like `[` and `]`.
            Command::SwitchTab => self.switch_tab()?,
            Command::FilesTab => self.goto_tab(SidebarTab::Files)?,
            Command::CommitsTab => self.goto_tab(SidebarTab::Commits)?,
            Command::CommentsTab => self.goto_tab(SidebarTab::Comments)?,
            Command::Enter => self.on_enter()?,
            Command::FoldRow => self.fold_row()?,
            Command::Escape => self.escape(),
            Command::Narrower => self.split = self.split.nudged(-NUDGE),
            Command::Wider => self.split = self.split.nudged(NUDGE),
            Command::ToggleSidebar => self.toggle_sidebar(),
            Command::ToggleTree => self.toggle_tree(),
            Command::CycleSort => self.cycle_sort(),
            Command::ToggleTint => self.toggle_tint(),
            Command::ToggleCounts => self.toggle_counts(),
            Command::ToggleFullContext => self.toggle_full_context(),
            Command::Info => self.toggle_info(),
            Command::Refresh => self.refresh()?,
            Command::Help => {
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
