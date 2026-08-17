//! The reviewer's state machine and its event loop.
//!
//! The split in this module is the point of it: [`App::on_key`] and everything
//! it calls are **terminal-free**. They take a [`KeyCode`], change state, read
//! and write `.review/`, and return — no `Terminal`, no raw mode, no PTY. That
//! is what lets `rv/tests/app.rs` drive a whole review, comment and all, as an
//! ordinary unit test. Only [`App::run`] touches the terminal, and it does
//! nothing else: set up, loop, tear down.
//!
//! # Restoring the terminal
//!
//! A TUI that panics in raw mode leaves the user's shell unusable — no echo,
//! no line editing, a cursor that never came back. [`App::run`] therefore
//! installs a panic hook that restores the terminal *before* the default hook
//! prints its message, so the backtrace lands on a working terminal, and calls
//! [`ratatui::restore`] on every ordinary exit path too, including the error
//! one.
//!
//! # What a comment costs
//!
//! Saving a comment writes `.review/comments.json` and its snapshot (both
//! atomically, through the store) and then rewrites `REVIEW-FEEDBACK.md` via
//! [`session::write_markdown`], which folds in any reply an LLM appended
//! first. So the file an agent reads is never stale by more than one
//! keystroke, and a comment survives the process being killed the instant
//! after Enter.
//!
//! Blobs are read lazily, for the selected file only (spec §7), and the
//! computed [`FileDiff`] is cached per file so that stepping back to a file
//! does not re-run difftastic.

use anyhow::Context as _;
use anyhow::Result;
use crossterm::event;
use crossterm::event::Event;
use crossterm::event::KeyCode;
use crossterm::event::KeyEventKind;
use ratatui::DefaultTerminal;
use rv_core::anchor;
use rv_core::diff;
use rv_core::diff::FileDiff;
use rv_core::diff::LineKind;
use rv_core::model::FileChange;
use rv_core::model::Side;
use rv_core::store::Comment;
use rv_core::store::CommentState;

use crate::session;
use crate::session::Review;
use crate::ui;

/// How many hex characters of the digest make up a comment id. Four is short
/// enough to read out of a marker in the markdown and long enough that a
/// review-sized set of comments does not collide.
const ID_CHARS: usize = 4;

/// The status line shown before the reviewer has done anything.
const HELP: &str = "j/k line  [/] file  c comment  q quit";

/// What the reviewer is doing with the keyboard.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Keys navigate the diff.
    Browse,
    /// Keys go into the comment buffer.
    Comment,
}

/// What [`App::on_key`] wants the event loop to do next.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Continue,
    Quit,
}

/// One interactive review.
///
/// `diffs` is parallel to `review.files`: `None` means "not computed yet",
/// which is the whole of the lazy-loading scheme (spec §7).
pub struct App {
    review: Review,
    diffs: Vec<Option<FileDiff>>,
    file_index: usize,
    line_index: usize,
    mode: Mode,
    buffer: String,
    status: String,
}

impl App {
    /// Opens `review` in the reviewer, loading the first file's diff.
    pub fn new(review: Review) -> Result<Self> {
        let diffs = vec![None; review.files.len()];
        let mut app = Self {
            review,
            diffs,
            file_index: 0,
            line_index: 0,
            mode: Mode::Browse,
            buffer: String::new(),
            status: HELP.to_owned(),
        };
        app.load_selected()?;
        Ok(app)
    }

    /// Runs the reviewer on the terminal until the user quits.
    ///
    /// Everything that can fail without a terminal — opening the repository,
    /// reading the first diff — has already failed by the time raw mode is
    /// entered, so such an error prints as a sentence rather than into a
    /// half-initialized screen. `try_init` rather than `init` for the same
    /// reason: a `rv` that was piped somewhere has no terminal to take over,
    /// and that is a sentence too, not a panic.
    pub fn run(review: Review) -> Result<()> {
        let mut app = Self::new(review)?;

        // Before `try_init`, which chains its own restoring hook in front of
        // whatever it finds — so the terminal is restored twice on a panic
        // (harmless) rather than depending on ratatui to keep doing it.
        install_panic_hook();
        let mut terminal = ratatui::try_init().context("could not start the terminal")?;
        let result = app.event_loop(&mut terminal);
        // Unconditional, and before the error is returned: a failed loop must
        // still hand the shell back in a usable state.
        ratatui::restore();
        result
    }

    /// The file the sidebar has selected, or `None` when the range changed no
    /// files at all.
    pub fn selected_file(&self) -> Option<&FileChange> {
        self.review.files.get(self.file_index)
    }

    /// The selected file's diff, once it has been loaded.
    pub fn selected_diff(&self) -> Option<&FileDiff> {
        self.diffs.get(self.file_index).and_then(Option::as_ref)
    }

    /// Every file in the review, in sidebar order.
    pub fn files(&self) -> &[FileChange] {
        &self.review.files
    }

    /// Which file the sidebar has selected.
    pub fn file_index(&self) -> usize {
        self.file_index
    }

    /// Which line of the selected diff is highlighted.
    pub fn line_index(&self) -> usize {
        self.line_index
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// The comment being typed, empty outside [`Mode::Comment`].
    pub fn buffer(&self) -> &str {
        &self.buffer
    }

    /// The one-line message under the reviewer's last action.
    pub fn status(&self) -> &str {
        &self.status
    }

    /// Handles one key press. Terminal-free by construction — see the module
    /// docs.
    pub fn on_key(&mut self, key: KeyCode) -> Result<Action> {
        match self.mode {
            Mode::Browse => self.on_key_browse(key),
            Mode::Comment => self.on_key_comment(key),
        }
    }

    fn event_loop(&mut self, terminal: &mut DefaultTerminal) -> Result<()> {
        loop {
            terminal
                .draw(|frame| ui::draw(frame, self))
                .context("could not draw the review")?;

            // Key *releases* and repeats are reported by terminals that speak
            // the kitty protocol; acting on presses only keeps one keystroke
            // from typing two characters there.
            if let Event::Key(key) = event::read().context("could not read a key")?
                && key.kind == KeyEventKind::Press
                && self.on_key(key.code)? == Action::Quit
            {
                return Ok(());
            }
        }
    }

    fn on_key_browse(&mut self, key: KeyCode) -> Result<Action> {
        match key {
            KeyCode::Char('q') => return Ok(Action::Quit),
            KeyCode::Char('j') | KeyCode::Down => {
                let last = self.line_count().saturating_sub(1);
                self.line_index = self.line_index.saturating_add(1).min(last);
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.line_index = self.line_index.saturating_sub(1);
            }
            KeyCode::Char(']') => self.select_file(self.file_index.saturating_add(1))?,
            KeyCode::Char('[') => self.select_file(self.file_index.saturating_sub(1))?,
            KeyCode::Char('c') => self.begin_comment(),
            _ => {}
        }
        Ok(Action::Continue)
    }

    fn on_key_comment(&mut self, key: KeyCode) -> Result<Action> {
        match key {
            KeyCode::Esc => {
                self.mode = Mode::Browse;
                self.buffer.clear();
                self.status = "comment discarded".to_owned();
            }
            KeyCode::Backspace => {
                self.buffer.pop();
            }
            KeyCode::Enter => {
                self.commit_comment()?;
                self.mode = Mode::Browse;
                self.buffer.clear();
            }
            KeyCode::Char(character) => self.buffer.push(character),
            _ => {}
        }
        Ok(Action::Continue)
    }

    /// Enters [`Mode::Comment`] on an empty buffer, unless there is nothing to
    /// anchor a comment to — better to say so now than to take a typed comment
    /// and drop it at Enter.
    fn begin_comment(&mut self) {
        if self.selected_line().is_none() {
            self.status = "no diff line selected, nothing to comment on".to_owned();
            return;
        }
        self.mode = Mode::Comment;
        self.buffer.clear();
    }

    /// Moves the sidebar selection to `index` and loads that file's diff.
    /// Out-of-range indices are ignored, which is what makes `[` at the top
    /// and `]` at the bottom no-ops rather than errors.
    fn select_file(&mut self, index: usize) -> Result<()> {
        if index >= self.review.files.len() || index == self.file_index {
            return Ok(());
        }
        self.file_index = index;
        self.line_index = 0;
        self.load_selected()
    }

    /// Computes the selected file's diff if it has not been computed yet.
    ///
    /// Both sides are read at their own path and their own commit, so a
    /// rename diffs its base-side source against its head-side target rather
    /// than against a file that does not exist. A side the commit has no plain
    /// file at — an add, a delete, a symlink — reads as absent, which is
    /// exactly what [`diff::compute`] wants for a whole-file change.
    fn load_selected(&mut self) -> Result<()> {
        let Some(file) = self.review.files.get(self.file_index) else {
            return Ok(());
        };
        if self.diffs[self.file_index].is_some() {
            return Ok(());
        }

        let session = &self.review.session;
        let base_path = file.source_path.as_deref().unwrap_or(&file.path);
        let old = self
            .review
            .repo
            .read_blob(&session.base_commit, base_path)
            .with_context(|| format!("could not read {base_path} at the base of the review"))?;
        let new = self
            .review
            .repo
            .read_blob(&session.head_commit, &file.path)
            .with_context(|| format!("could not read {} at the head of the review", file.path))?;

        let diff = diff::compute(old.as_deref(), new.as_deref(), &file.path);
        self.line_index = self.line_index.min(diff.lines.len().saturating_sub(1));
        self.diffs[self.file_index] = Some(diff);
        Ok(())
    }

    fn line_count(&self) -> usize {
        self.selected_diff().map_or(0, |diff| diff.lines.len())
    }

    fn selected_line(&self) -> Option<&rv_core::diff::DiffLine> {
        self.selected_diff()
            .and_then(|diff| diff.lines.get(self.line_index))
    }

    /// Saves the typed comment against the selected line, then rewrites the
    /// markdown export.
    ///
    /// Anything that makes the comment unanchorable — an empty body, a binary
    /// or suppressed diff, a diff line with no number on the side it belongs
    /// to — leaves the store untouched and the reason in the status line. A
    /// comment that cannot be placed is never worth storing somewhere
    /// approximate.
    fn commit_comment(&mut self) -> Result<()> {
        let comment = match self.prepare_comment()? {
            Ok(comment) => comment,
            Err(reason) => {
                self.status = reason;
                return Ok(());
            }
        };

        self.review
            .store
            .append_comment(&comment)
            .context("could not save the comment")?;
        session::write_markdown(&self.review)?;

        self.status = format!(
            "comment saved at {}:{}",
            comment.anchor.file, comment.anchor.line
        );
        Ok(())
    }

    /// Builds the [`Comment`] the current selection and buffer describe, or —
    /// as the inner `Err` — the sentence to show instead of saving anything.
    ///
    /// The outer [`Result`] is reserved for a repository that could not be
    /// read, which is a real failure rather than a refusal.
    ///
    /// The body is stored trimmed: surrounding whitespace is a slip of the
    /// keyboard, and it would otherwise end up in the comment id.
    fn prepare_comment(&self) -> Result<Result<Comment, String>> {
        let body = self.buffer.trim();
        if body.is_empty() {
            return Ok(Err("empty comment, nothing saved".to_owned()));
        }
        let (Some(file), Some(line)) = (self.selected_file(), self.selected_line()) else {
            return Ok(Err("no diff line selected, nothing saved".to_owned()));
        };
        let Some(change) = self.review.session.changes.first() else {
            return Ok(Err("the review covers no change to comment on".to_owned()));
        };

        // A removed line only exists on the base side; everything else —
        // added and context alike — is commented against the head.
        let side = match line.kind {
            LineKind::Removed => Side::Left,
            LineKind::Added | LineKind::Context => Side::Right,
        };
        let session = &self.review.session;
        let (commit, path, number) = match side {
            Side::Left => (
                &session.base_commit,
                file.source_path.as_deref().unwrap_or(&file.path),
                line.left,
            ),
            Side::Right => (&session.head_commit, file.path.as_str(), line.right),
        };
        let Some(number) = number else {
            return Ok(Err(
                "this line has no number on the side it belongs to".to_owned()
            ));
        };

        // The anchor hashes the line as it stands in the file, not as the diff
        // rendered it, so it resolves against the file's own future text.
        let blob = self
            .review
            .repo
            .read_blob(commit, path)
            .with_context(|| format!("could not read {path} to anchor the comment"))?;
        let text = blob.map(|bytes| String::from_utf8_lossy(&bytes).into_owned());
        let anchor = anchor::create(path, side, number, text.as_deref().unwrap_or_default());

        Ok(Ok(Comment {
            id: comment_id(&change.change_id, path, number, body),
            change_id: change.change_id.clone(),
            commit_id: change.commit_id.clone(),
            anchor,
            body: body.to_owned(),
            state: CommentState::Open,
            reply: None,
        }))
    }
}

/// A comment's id: the first [`ID_CHARS`] hex characters of the blake3 digest
/// of the change, location and body it covers.
///
/// Derived rather than random so that re-typing the same comment on the same
/// line of the same change upserts the entry it already made instead of
/// stacking a duplicate beside it.
fn comment_id(change_id: &str, path: &str, line: u32, body: &str) -> String {
    let seed = format!("{change_id}:{path}:{line}:{body}");
    let digest = blake3::hash(seed.as_bytes()).to_hex();
    digest[..ID_CHARS].to_owned()
}

/// Makes a panic restore the terminal before it prints.
///
/// The previous hook runs afterwards, so the message and backtrace land on a
/// terminal that has left raw mode and the alternate screen — visible, and on
/// a shell the user can keep using.
fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        ratatui::restore();
        previous(info);
    }));
}
