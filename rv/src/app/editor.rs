//! `v`: the selected file, at the cursor's line, in `$EDITOR`.
//!
//! Split in two on purpose. Everything here is terminal-free — it resolves
//! `$EDITOR` and the target and builds the child — and [`super::run`] owns
//! leaving the alternate screen, running the child in the foreground and
//! getting the terminal back. That is the same division the rest of [`super`]
//! keeps, and it is what lets a test press `v`, inspect the [`Edit`] it
//! produced and even run it, without a pty.

use std::ffi::OsStr;
use std::io;
use std::path::PathBuf;
use std::process::Command;
use std::process::ExitStatus;

use rv_core::model::ChangeKind;

use super::Action;
use super::App;

/// The editor to open a file in, and where in it.
///
/// Resolved before the terminal is given up, so a review whose `$EDITOR` is
/// unset or whose file has no head side never leaves the screen at all.
pub(super) struct Edit {
    /// The program to run, and the arguments `$EDITOR` carried with it.
    program: PathBuf,
    arguments: Vec<String>,
    /// The absolute path of the file to open.
    path: PathBuf,
    line: u32,
}

impl Edit {
    /// Splits `editor` into a program and its arguments, or [`None`] when it is
    /// unset or empty.
    ///
    /// Whitespace-split rather than shell-parsed. `$EDITOR` holding a command
    /// line — `code -w`, `emacs -nw` — is common enough that taking the whole
    /// string as a program name would refuse to run for those users, and
    /// handing it to `sh -c` would make a file name with a space in it a
    /// command injection. Neither is a trade worth making for the quoting
    /// almost nobody puts in `$EDITOR`.
    fn new(editor: Option<&OsStr>, path: PathBuf, line: u32) -> Option<Self> {
        let editor = editor?.to_str()?;
        let mut words = editor.split_whitespace();
        let program = PathBuf::from(words.next()?);
        Some(Self {
            program,
            arguments: words.map(str::to_owned).collect(),
            path,
            line,
        })
    }

    /// Runs the editor in the foreground and waits for it.
    ///
    /// `+N` before the path: the line-number convention `vi`, `vim`, `emacs`,
    /// `nano` and `kak` all read, and the one `git` itself passes.
    ///
    /// Inherits this process's streams, which is the whole of what makes the
    /// editor usable — the caller has already handed the terminal back.
    fn run(&self) -> io::Result<ExitStatus> {
        Command::new(&self.program)
            .args(&self.arguments)
            .arg(format!("+{}", self.line))
            .arg(&self.path)
            .status()
    }

    /// How the status line names what was opened.
    fn describe(&self) -> String {
        format!(
            "{}:{} in {}",
            self.path.display(),
            self.line,
            self.program.display()
        )
    }
}

impl App {
    /// Resolves what `v` would open, or says why it will not open anything.
    ///
    /// Returns [`Action::Edit`] only once there is something to run, so the
    /// terminal is never given up for a refusal — a screen that flickers out
    /// and back to report `$EDITOR is not set` is worse than the sentence.
    pub(super) fn begin_edit(&mut self) -> Action {
        let Some(file) = self.selected_file() else {
            self.status = "no file to open".to_owned();
            return Action::Continue;
        };
        // A file the change deleted has no head side to edit. Opening the path
        // anyway would create an empty buffer over a file the reviewer is
        // reading the removal of, which is the one outcome nobody wants.
        if file.kind == ChangeKind::Removed {
            self.status = format!("{} is gone at this revision", file.path);
            return Action::Continue;
        }
        let path = self.review.store.root().join(&file.path);
        // The head-side number, since the head side is the file on disk. A
        // removed line has none, and its left number is the closest thing to
        // where it was; the reviewer lands beside the change rather than at the
        // top of the file.
        let line = self
            .selected_line()
            .and_then(|line| line.right.or(line.left))
            .unwrap_or(1);

        let Some(edit) = Edit::new(std::env::var_os("EDITOR").as_deref(), path, line) else {
            // Named rather than guessed: a default of `vi` would drop a
            // reviewer who has never used it into a modal editor they cannot
            // leave, in place of one sentence telling them what to set.
            self.status = "$EDITOR is not set".to_owned();
            return Action::Continue;
        };
        self.pending_edit = Some(edit);
        Action::Edit
    }

    /// Runs whatever `v` resolved, and reports how it went.
    ///
    /// Public and terminal-free: it spawns a child that inherits this process's
    /// streams, and knows nothing about raw mode or the alternate screen.
    /// [`super::run`] gives the terminal up around this call — that is the
    /// whole of the division, and it is what lets a test drive the key, the
    /// child and the message it leaves behind without a pty.
    ///
    /// A failed spawn and a non-zero exit are alerts rather than statuses: the
    /// reviewer just watched their screen leave and come back, and needs to
    /// know whether anything happened. An ordinary exit is a status, because it
    /// is simply what they asked for. Nothing pending is a no-op, so a stray
    /// call cannot open an editor nobody asked for.
    pub fn run_pending_edit(&mut self) {
        let Some(edit) = self.pending_edit.take() else {
            return;
        };
        match edit.run() {
            Ok(status) if status.success() => self.status = format!("edited {}", edit.describe()),
            Ok(status) => self.raise(format!("{} exited with {status}", edit.describe())),
            Err(error) => self.raise(format!("could not run $EDITOR: {error}")),
        }
    }
}
