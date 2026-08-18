//! One key press in, one [`Action`] out — with no terminal anywhere in reach.

use anyhow::Result;
use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyModifiers;

use super::Action;
use super::App;
use super::Mode;
use super::bindings::BINDINGS;
use super::bindings::Command;

/// How many percentage points one press of `<` or `>` moves the divider.
///
/// Two rather than one: a resize the reviewer cannot see happen is a resize
/// they will hold the key down for.
const NUDGE: i16 = 2;

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
        self.on_key(event.code)
    }

    /// Handles one key press.
    ///
    /// The keymap is answered ahead of the mode because it is a modal window
    /// rather than a mode: it can only be raised from [`Mode::Browse`] and
    /// nothing behind it can change the mode.
    pub fn on_key(&mut self, key: KeyCode) -> Result<Action> {
        if self.help_open {
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
    /// `q` **closes** rather than quits: the reviewer with the manual open is
    /// the one least sure what the keys do, and ending their review is the most
    /// expensive way to find out.
    fn on_key_help(&mut self, key: KeyCode) -> Action {
        match key {
            KeyCode::Char('?') | KeyCode::Char('q') | KeyCode::Esc => self.help_open = false,
            KeyCode::Char('j') | KeyCode::Down => self.scroll_help(1),
            KeyCode::Char('k') | KeyCode::Up => self.scroll_help(-1),
            _ => {}
        }
        Action::Continue
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
            // `[` and `]` consult no focus at all, so walking a review never
            // costs a trip through the sidebar.
            Command::NextFile => self.select_file(self.file_index.saturating_add(1))?,
            Command::PreviousFile => self.select_file(self.file_index.saturating_sub(1))?,
            Command::NextSymbol => self.next_symbol()?,
            Command::PreviousSymbol => self.previous_symbol()?,
            Command::Pick => self.begin_pick(),
            Command::Comment => self.begin_comment(),
            Command::Delete => self.begin_delete(),
            Command::Resolve => self.resolve_comment()?,
            Command::Abandon => self.abandon_comment()?,
            Command::Export => self.export()?,
            Command::Fold => self.toggle_collapse(),
            // Focus-free, like `[` and `]`.
            Command::SwitchTab => self.switch_tab()?,
            Command::Enter => self.on_enter()?,
            Command::Escape => self.leave_stack(),
            Command::Narrower => self.split = self.split.nudged(-NUDGE),
            Command::Wider => self.split = self.split.nudged(NUDGE),
            Command::ToggleSidebar => self.toggle_sidebar(),
            Command::ToggleTree => self.toggle_tree(),
            Command::CycleSort => self.cycle_sort(),
            Command::Help => {
                self.help_open = true;
                // Opened at the top, always: the geometry it was last scrolled
                // against may have changed since.
                self.help_scroll = 0;
            }
        }
        Ok(Action::Continue)
    }
}
