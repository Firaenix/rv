//! The only part of the reviewer that touches a terminal: set up, loop, tear
//! down.
//!
//! # Restoring the terminal
//!
//! A TUI that panics in raw mode leaves the user's shell unusable. The panic
//! hook therefore restores the terminal *before* the default hook prints, so
//! the backtrace lands on a working terminal, and [`ratatui::restore`] runs on
//! every ordinary exit path including the error one.
//!
//! **Mouse reporting is part of that.** It is on for the whole run — no toggle,
//! because every current terminal keeps Shift-drag as a bypass for its own text
//! selection, so `rv` needs neither a selection nor a clipboard of its own —
//! and it is turned off again on every exit path, the panic hook included. A
//! terminal left reporting prints escape noise at every click for the rest of
//! the session, which is the same class of damage as one left in raw mode.

use anyhow::Context as _;
use anyhow::Result;
use crossterm::event;
use crossterm::event::DisableMouseCapture;
use crossterm::event::EnableMouseCapture;
use crossterm::event::Event;
use crossterm::event::KeyEventKind;
use crossterm::execute;
use ratatui::DefaultTerminal;
use std::time::Duration;
use std::time::Instant;

use super::Action;
use super::App;
use crate::session::Review;
use crate::ui;

/// How long the loop waits for a keystroke while a blob is still being parsed.
///
/// Short enough that colour arriving reads as immediate, long enough that a
/// reviewer who has walked away is not being woken sixty times a second. The
/// wait ends the moment a key arrives, so this is a ceiling on the delay before
/// a swap is painted, not a frame rate.
const PAINT_POLL: Duration = Duration::from_millis(30);

impl App {
    /// Runs the reviewer on the terminal until the user quits.
    ///
    /// Everything that can fail without a terminal has already failed by the
    /// time raw mode is entered, so such an error prints as a sentence rather
    /// than into a half-initialized screen. `try_init` rather than `init` for
    /// the same reason: an `rv` that was piped somewhere has no terminal to
    /// take over, and that is a sentence too, not a panic.
    pub fn run(review: Review) -> Result<()> {
        let mut app = Self::new(review)?;

        // Before `try_init`, which chains its own restoring hook in front of
        // whatever it finds.
        install_panic_hook();
        let mut terminal = ratatui::try_init().context("could not start the terminal")?;
        // Inside the guard below, not before it: a failure here must still go
        // through the release and the restore.
        let result = capture_mouse().and_then(|()| app.event_loop(&mut terminal));
        // Unconditional, and before the error is returned: a failed loop must
        // still hand the shell back out of raw mode *and* no longer reporting
        // where the pointer is.
        release_mouse();
        ratatui::restore();
        result
    }

    /// Draw, wait, handle one event, repeat.
    ///
    /// **The wait is bounded whenever anything on screen ages.** Sitting in
    /// `event::read` is right for a reviewer with nothing to be told and wrong
    /// the moment a toast is up: an alert raised in front of someone who then
    /// walks away would still be there at t=∞. [`App::next_deadline`] says how
    /// long the loop may block for, and `None` means block as before, so an
    /// idle `rv` with nothing to show still costs nothing.
    ///
    /// This is also the one place the clock is read; everything below takes the
    /// time as a parameter.
    fn event_loop(&mut self, terminal: &mut DefaultTerminal) -> Result<()> {
        loop {
            let now = Instant::now();
            self.expire_alerts(now);
            // Before the frame, so a parse that landed while the reviewer was
            // reading is painted on this pass rather than the next.
            self.collect_highlights();
            terminal
                .draw(|frame| ui::draw(frame, self, now))
                .context("could not draw the review")?;

            // Nothing arrived before the deadline: go round and paint the next
            // step of the fade — or the colour that has just been parsed.
            let deadline = match (self.next_deadline(Instant::now()), self.painting()) {
                (Some(fade), true) => Some(fade.min(PAINT_POLL)),
                (Some(fade), false) => Some(fade),
                (None, true) => Some(PAINT_POLL),
                (None, false) => None,
            };
            if let Some(timeout) = deadline
                && !event::poll(timeout).context("could not wait for an event")?
            {
                continue;
            }

            let action = match event::read().context("could not read an event")? {
                // Key *releases* and repeats are reported by terminals that
                // speak the kitty protocol; acting on presses only keeps one
                // keystroke from typing two characters there.
                Event::Key(key) if key.kind == KeyEventKind::Press => self.on_key_event(key)?,
                Event::Mouse(mouse) => self.on_mouse(mouse)?,
                // A resize repaints on the next pass, and everything else — a
                // focus change, a paste — is not something this reviewer binds.
                _ => Action::Continue,
            };
            if action == Action::Quit {
                return Ok(());
            }
        }
    }
}

/// Turns mouse reporting on for the run.
fn capture_mouse() -> Result<()> {
    execute!(std::io::stdout(), EnableMouseCapture).context("could not enable mouse reporting")
}

/// Turns it off again, on the way out of any exit path.
///
/// Errors are dropped on purpose: this runs while the terminal is being handed
/// back, including from the panic hook, and there is nowhere left to report to.
fn release_mouse() {
    let _ = execute!(std::io::stdout(), DisableMouseCapture);
}

/// Makes a panic restore the terminal before it prints.
///
/// The previous hook runs afterwards, so the message and backtrace land on a
/// terminal that has left raw mode and the alternate screen. Mouse reporting
/// goes first, while `rv` still owns the terminal.
fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        release_mouse();
        ratatui::restore();
        previous(info);
    }));
}
