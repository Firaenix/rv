//! The reviewer's state machine and its event loop.
//!
//! The split in this module is the point of it: [`App::on_key`] and everything
//! it calls are **terminal-free**. They take a [`KeyCode`], change state, read
//! and write `.review/`, and return — no `Terminal`, no raw mode, no PTY. That
//! is what lets `rv/tests/app.rs` drive a whole review, comment and all, as an
//! ordinary unit test. Only [`App::run`] touches the terminal, and it does
//! nothing else: set up, loop, tear down.
//!
//! [`App::on_key_event`] sits in front of [`App::on_key`] for the one decision
//! that cannot be made from a [`KeyCode`] alone — Ctrl+C, which raw mode leaves
//! to the program — and is terminal-free in exactly the same way.
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
//! after Enter. The in-memory copy the pane draws from is then re-read from
//! the store, so what is on screen is what is on disk rather than what this
//! process believes it wrote.
//!
//! Blobs are read lazily, for the selected file only (spec §7), and the
//! computed [`FileDiff`] is cached per file so that stepping back to a file
//! does not re-run difftastic.

use anyhow::Context as _;
use anyhow::Result;
use crossterm::event;
use crossterm::event::Event;
use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyEventKind;
use crossterm::event::KeyModifiers;
use ratatui::DefaultTerminal;
use rv_core::anchor;
use rv_core::diff;
use rv_core::diff::DiffLine;
use rv_core::diff::FileDiff;
use rv_core::diff::LineKind;
use rv_core::model::FileChange;
use rv_core::model::Side;
use rv_core::store::Comment;
use rv_core::store::CommentState;
use rv_core::store::Session;

use crate::session;
use crate::session::Review;
use crate::ui;

/// How many hex characters of the digest make up a comment id.
///
/// Eight, not the four the plan and spec §10 write, because a collision here
/// is not a cosmetic clash: [`rv_core::store::Store::append_comment`] upserts
/// by id, so two *different* comments sharing a prefix mean the second save
/// silently replaces the first in `comments.json` and overwrites its snapshot
/// — under a "comment saved" status line. Four hex characters is a 65,536-value
/// space, which by the birthday bound is a ~2% chance of losing a comment at 50
/// of them and ~7% at 100: reachable on one real review. Spec §10's guarantee
/// that nothing loses a comment, and Task 5's write-through durability, outrank
/// the literal width. Eight still reads out of a marker at a glance, and
/// `markdown::parse_replies` binds whatever id the marker carries, so nothing
/// else changes.
const ID_CHARS: usize = 8;

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

/// Which pane the keys act on.
///
/// A focus rather than a [`Mode`] because modes are for *typing*: a mode
/// changes what a keystroke means, while this only changes what it moves. That
/// is why `j`, `k` and the arrows keep their meaning across all three, and why
/// `[` and `]` are answered before the focus is consulted at all — a reviewer
/// walking the files never has to think about where the cursor is first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Focus {
    /// The left column, which lists the review's files.
    Sidebar,
    /// The diff of the selected file.
    Diff,
    /// Inside the comment stack of the selected diff line. Introduced here so
    /// that the two movement helpers below are total over the enum; nothing
    /// reaches it until the stack itself lands.
    Stack,
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
///
/// `comments` is a *read-through copy* of the store, not a cache in front of
/// it: [`rv_core::store::Store`] stays the authority, every save still goes
/// straight to disk, and this vector is refreshed from disk immediately
/// afterwards (see [`App::reload_comments`]). It exists because the diff pane
/// has to draw the comments on the line it is drawing, and re-reading
/// `comments.json` once per line per frame is not a thing to do sixty times a
/// second.
pub struct App {
    review: Review,
    diffs: Vec<Option<FileDiff>>,
    comments: Vec<Comment>,
    file_index: usize,
    /// Where the highlight sits in each file's diff, parallel to
    /// `review.files`.
    ///
    /// One position per file rather than one shared between them, because
    /// `[`/`]` is how a reviewer compares two files and a shared cursor makes
    /// every round trip cost them their place: the first real review of `rv`
    /// spent a fifth of its keystrokes on `j` walking back down to where it had
    /// just been.
    line_indices: Vec<usize>,
    focus: Focus,
    mode: Mode,
    buffer: String,
    status: String,
    /// Set to skip difftastic for every file in this review and take
    /// [`diff::compute_with`]'s `similar` fallback instead. See
    /// [`App::with_fallback_diffs`].
    force_fallback: bool,
}

impl App {
    /// Opens `review` in the reviewer, loading the first file's diff.
    ///
    /// Which diff engine each file goes through is left to
    /// [`diff::compute`], which honours `RV_NO_DIFFT`.
    pub fn new(review: Review) -> Result<Self> {
        Self::open(review, false)
    }

    /// Opens `review` with difftastic bypassed: every file's diff comes from
    /// the `similar` fallback.
    ///
    /// That is the diff a user with no `difft` on `PATH` gets, and the only one
    /// that carries [`LineKind::Context`] lines and a
    /// [`rv_core::diff::DiffSource::Similar`] label — so it is a distinct set
    /// of branches through this module and through [`crate::ui`], not a
    /// degraded copy of the difftastic path.
    ///
    /// Per-`App` rather than through `RV_NO_DIFFT`, for the same reason
    /// [`diff::compute_with`] takes the choice as an argument: the environment
    /// variable is process-wide, and a caller that wants the fallback for
    /// *this* review should not have to change what every other review in the
    /// process sees.
    pub fn with_fallback_diffs(review: Review) -> Result<Self> {
        Self::open(review, true)
    }

    fn open(review: Review, force_fallback: bool) -> Result<Self> {
        let diffs = vec![None; review.files.len()];
        // Read before the review is moved into `Self`, and before the first
        // diff is computed: a reviewer who quit halfway through yesterday
        // opens on the notes they already made, not on an empty pane that
        // fills in only once they save something new.
        let comments = review
            .store
            .comments()
            .context("could not read the saved comments")?;
        let line_indices = vec![0; review.files.len()];
        let mut app = Self {
            review,
            diffs,
            comments,
            file_index: 0,
            line_indices,
            focus: Focus::Diff,
            mode: Mode::Browse,
            buffer: String::new(),
            status: HELP.to_owned(),
            force_fallback,
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

    /// The range under review: its two endpoint commits and the changes
    /// between them.
    pub fn session(&self) -> &Session {
        &self.review.session
    }

    /// Which file the sidebar has selected.
    pub fn file_index(&self) -> usize {
        self.file_index
    }

    /// Which line of the selected diff is highlighted.
    ///
    /// Zero when the review has no files, which is the only way this can be
    /// asked about a file that does not exist.
    pub fn line_index(&self) -> usize {
        self.line_indices.get(self.file_index).copied().unwrap_or(0)
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// Which pane the movement keys act on. The diff, on launch: that is what
    /// a reviewer came to read.
    pub fn focus(&self) -> Focus {
        self.focus
    }

    /// Every comment in the review, in store order (oldest first).
    pub fn comments(&self) -> &[Comment] {
        &self.comments
    }

    /// The comments anchored to diff line `index` of the selected file, oldest
    /// first.
    ///
    /// A line is matched by the key it would anchor *under*, never by its raw
    /// number: [`App::anchor_target`] derives the side and the side's path from
    /// the same [`anchored_side`] rule [`App::prepare_comment`] saves through,
    /// so a comment can never be stored against one line and displayed against
    /// another. Milestone 1 shipped that bug once; there is deliberately only
    /// one side rule in this file.
    ///
    /// Filtered rather than pre-indexed, and returning borrows rather than a
    /// slice, because the matches are not contiguous in the store's order.
    pub fn comments_for_line(&self, index: usize) -> Vec<&Comment> {
        let Some(line) = self.selected_diff().and_then(|diff| diff.lines.get(index)) else {
            return Vec::new();
        };
        let Some(target) = self.anchor_target(line) else {
            return Vec::new();
        };
        self.comments
            .iter()
            .filter(|comment| {
                comment.anchor.file == target.path
                    && comment.anchor.side == target.side
                    && comment.anchor.line == target.number
            })
            .collect()
    }

    /// The comment being typed, empty outside [`Mode::Comment`].
    pub fn buffer(&self) -> &str {
        &self.buffer
    }

    /// The one-line message under the reviewer's last action.
    pub fn status(&self) -> &str {
        &self.status
    }

    /// Handles one key press, modifiers and all.
    ///
    /// Ctrl+C is answered here rather than in [`App::on_key`] because the state
    /// machine below is written against plain [`KeyCode`]s — which is what
    /// makes it testable without a pty — and `Char('c')` with CONTROL held is
    /// indistinguishable from a plain `c` once the modifiers are dropped. In
    /// raw mode the terminal raises no SIGINT on the reviewer's behalf and `rv`
    /// offers no other abort, so without this the one key every terminal user
    /// reaches for would open the comment box and type into it.
    ///
    /// It quits from any mode, including a half-typed comment: an abort that
    /// first asks you to `Esc` is not an abort. The buffer is dropped
    /// unsaved, which is the same thing `Esc` does with it.
    pub fn on_key_event(&mut self, event: KeyEvent) -> Result<Action> {
        if event.modifiers.contains(KeyModifiers::CONTROL)
            && matches!(event.code, KeyCode::Char('c'))
        {
            return Ok(Action::Quit);
        }
        self.on_key(event.code)
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
                && self.on_key_event(key)? == Action::Quit
            {
                return Ok(());
            }
        }
    }

    fn on_key_browse(&mut self, key: KeyCode) -> Result<Action> {
        match key {
            KeyCode::Char('q') => return Ok(Action::Quit),
            KeyCode::Left => self.focus_left(),
            KeyCode::Right => self.focus_right(),
            KeyCode::Char('j') | KeyCode::Down => self.move_forward()?,
            KeyCode::Char('k') | KeyCode::Up => self.move_back()?,
            // Deliberately answered before the focus is consulted: `[` and `]`
            // mean "the next file" wherever the cursor happens to be, so
            // walking a review never costs a trip through the sidebar.
            KeyCode::Char(']') => self.select_file(self.file_index.saturating_add(1))?,
            KeyCode::Char('[') => self.select_file(self.file_index.saturating_sub(1))?,
            KeyCode::Char('c') => self.begin_comment(),
            _ => {}
        }
        Ok(Action::Continue)
    }

    fn focus_left(&mut self) {
        self.focus = match self.focus {
            Focus::Stack => Focus::Diff,
            Focus::Diff | Focus::Sidebar => Focus::Sidebar,
        };
    }

    /// `Right` from the comment stack does nothing: the stack is drawn inside
    /// the diff pane, so there is no pane to its right. `Left` leads out of
    /// every focus, which is what keeps none of them a trap.
    fn focus_right(&mut self) {
        self.focus = match self.focus {
            Focus::Sidebar => Focus::Diff,
            Focus::Diff | Focus::Stack => self.focus,
        };
    }

    /// `j` / `Down` in the focused pane.
    fn move_forward(&mut self) -> Result<()> {
        match self.focus {
            Focus::Sidebar => self.select_file(self.file_index.saturating_add(1))?,
            Focus::Diff => self.set_line_index(self.line_index().saturating_add(1)),
            Focus::Stack => {}
        }
        Ok(())
    }

    /// `k` / `Up` in the focused pane.
    fn move_back(&mut self) -> Result<()> {
        match self.focus {
            // `select_file(0)` from file 0 is a no-op by its own guard, so `k`
            // at the top of the list stays put rather than wrapping.
            Focus::Sidebar => self.select_file(self.file_index.saturating_sub(1))?,
            Focus::Diff => self.set_line_index(self.line_index().saturating_sub(1)),
            Focus::Stack => {}
        }
        Ok(())
    }

    /// Moves the highlight to `index` of the selected file, clamped to that
    /// file's last diff line.
    ///
    /// The one place a line position is written, so the clamp cannot be
    /// forgotten on some path: a diff with no lines pins it at 0, and a review
    /// with no files has nowhere to put it at all.
    fn set_line_index(&mut self, index: usize) {
        let clamped = index.min(self.line_count().saturating_sub(1));
        if let Some(position) = self.line_indices.get_mut(self.file_index) {
            *position = clamped;
        }
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
    ///
    /// The file is reopened where it was left, not at its top — see
    /// `line_indices`. The position is re-clamped on the way in because it was
    /// clamped against whatever the diff was when it was written, and a file
    /// visited before its diff was computed has none to have been clamped to.
    fn select_file(&mut self, index: usize) -> Result<()> {
        if index >= self.review.files.len() || index == self.file_index {
            return Ok(());
        }
        self.file_index = index;
        self.load_selected()?;
        self.set_line_index(self.line_index());
        Ok(())
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

        let diff = if self.force_fallback {
            diff::compute_with(old.as_deref(), new.as_deref(), &file.path, false)
        } else {
            diff::compute(old.as_deref(), new.as_deref(), &file.path)
        };
        self.diffs[self.file_index] = Some(diff);
        // The clamp is [`App::select_file`]'s, applied once the diff it clamps
        // against is in place.
        Ok(())
    }

    /// Where a comment on `line` of the selected file belongs.
    ///
    /// `None` when the line carries no number on the side it belongs to, which
    /// is the same condition [`App::prepare_comment`] refuses to save under —
    /// so a line that cannot be commented on shows no comments either, rather
    /// than borrowing some other line's.
    fn anchor_target(&self, line: &DiffLine) -> Option<AnchorTarget<'_>> {
        let file = self.selected_file()?;
        let session = &self.review.session;
        let side = anchored_side(line.kind);
        let (path, number, commit) = match side {
            Side::Left => (
                file.source_path.as_deref().unwrap_or(&file.path),
                line.left,
                session.base_commit.as_str(),
            ),
            Side::Right => (file.path.as_str(), line.right, session.head_commit.as_str()),
        };
        Some(AnchorTarget {
            side,
            path,
            number: number?,
            commit,
        })
    }

    /// Re-reads the comments from disk.
    ///
    /// Called after every write, so the pane shows what is stored rather than
    /// what this process believes it stored: the store is the authority, and
    /// its upsert may have replaced an entry rather than added one.
    fn reload_comments(&mut self) -> Result<()> {
        self.comments = self
            .review
            .store
            .comments()
            .context("could not re-read the saved comments")?;
        Ok(())
    }

    fn line_count(&self) -> usize {
        self.selected_diff().map_or(0, |diff| diff.lines.len())
    }

    fn selected_line(&self) -> Option<&DiffLine> {
        self.selected_diff()
            .and_then(|diff| diff.lines.get(self.line_index()))
    }

    /// Saves the typed comment against the selected line, then rewrites the
    /// markdown export.
    ///
    /// Anything that makes the comment unanchorable — an empty body, a diff
    /// with no lines to select at all (a binary file, or difftastic reporting
    /// no semantic change), a diff line with no number on the side it belongs
    /// to — leaves the store untouched and the reason in the status line. A
    /// comment that cannot be placed is never worth storing somewhere
    /// approximate.
    ///
    /// A *suppressed* diff is not on that list, and used to be described as if
    /// it were. Suppression says the difference between the two sides is not
    /// visible in the lines — difftastic's `unchanged`, or the `similar`
    /// fallback's terminator-only change — not that the lines are unreal. The
    /// difftastic case carries no lines, so it is refused by the clause above
    /// and needs no clause of its own; the fallback case carries every line of
    /// the file as `Context`, [`crate::ui`] draws them under a note saying the
    /// difference is elsewhere, and a comment on one of them anchors to a real
    /// line, at a real number, whose text the anchor hashes out of the file
    /// itself. Refusing it would mean refusing a line the reviewer is looking
    /// at.
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
        self.reload_comments()?;
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
    /// Two of the refusals below cannot be provoked from the keyboard alone.
    ///
    /// "the review covers no change to comment on" needs an empty
    /// `session.changes`. [`session::build`] never produces one —
    /// [`rv_core::vcs::Repository::stack`] returns `EmptyRange` for an empty
    /// range — but [`Review`] is `pub` with `pub` fields, so a caller that
    /// assembles one by hand can, and `rv/tests/app_cases.rs` does exactly that
    /// (`a_review_with_no_changes_refuses_to_attribute_a_comment`). It is a
    /// tested refusal rather than an unreachable branch.
    ///
    /// "this line has no number on the side it belongs to" is the one that
    /// really is unreachable, and is kept as defence in depth: it needs a
    /// [`rv_core::diff::DiffLine`] whose anchored side
    /// carries no number, and every producer in [`rv_core::diff`] numbers the
    /// side it dispatches to: difftastic's paired entries set both sides, an
    /// unpaired lhs is `Removed` with `left`, an unpaired rhs is `Added` with
    /// `right`, `all_added`/`all_removed` number their own side, and the
    /// `similar` fallback's Equal/Delete/Insert each set the side
    /// [`anchored_side`] sends them to.
    ///
    /// The body is stored trimmed: surrounding whitespace is a slip of the
    /// keyboard, and it would otherwise end up in the comment id.
    fn prepare_comment(&self) -> Result<Result<Comment, String>> {
        let body = self.buffer.trim();
        if body.is_empty() {
            return Ok(Err("empty comment, nothing saved".to_owned()));
        }
        let Some(line) = self.selected_line() else {
            return Ok(Err("no diff line selected, nothing saved".to_owned()));
        };
        // What `change_id` on the stored comment actually is, stated plainly
        // because the name invites a stronger reading: the *first change of the
        // reviewed range*, the same one for every comment in the review, and
        // not the change that introduced the line being commented on.
        // `Repository::stack` streams newest first, so for the default
        // `trunk()..@` this is `@` — the working copy, which is usually an
        // empty change.
        //
        // Two things follow, both of them current behaviour rather than
        // problems this function should solve: `markdown::render` orders each
        // section by the comment's index in `session.changes` and prints the id
        // in every anchor marker, so today that ordering key is constant and
        // every marker names the same change; and `comment_id`'s digest gets
        // the same `change_id` from every comment, so the seed's whole
        // discriminating power is the location and the body. Attributing a
        // comment to the change that touched its line is Milestone 2's work
        // (spec §14) and needs per-change diffs, which this milestone does not
        // compute.
        //
        // `commit_id` is *not* taken from that change: it comes from the
        // anchored side, along with the path and the number — see
        // [`AnchorTarget`].
        let Some(change) = self.review.session.changes.first() else {
            return Ok(Err("the review covers no change to comment on".to_owned()));
        };

        let Some(target) = self.anchor_target(line) else {
            return Ok(Err(
                "this line has no number on the side it belongs to".to_owned()
            ));
        };

        // The anchor hashes the line as it stands in the file, not as the diff
        // rendered it, so it resolves against the file's own future text.
        let blob = self
            .review
            .repo
            .read_blob(target.commit, target.path)
            .with_context(|| format!("could not read {} to anchor the comment", target.path))?;
        let text = blob.map(|bytes| String::from_utf8_lossy(&bytes).into_owned());
        let anchor = anchor::create(
            target.path,
            target.side,
            target.number,
            text.as_deref().unwrap_or_default(),
        );

        Ok(Ok(Comment {
            id: comment_id(
                &change.change_id,
                target.path,
                target.side,
                target.number,
                body,
            ),
            change_id: change.change_id.clone(),
            commit_id: target.commit.to_owned(),
            anchor,
            body: body.to_owned(),
            state: CommentState::Open,
            reply: None,
        }))
    }
}

/// Where a comment on one diff line belongs: which side it is anchored to, and
/// the path, line number and commit **on that side**.
///
/// Four values, one function ([`App::anchor_target`]), because they have to
/// agree: the pane labels a line with `number`, the store anchors it at
/// `path`:`number` on `side`, and `commit` is the revision whose blob that text
/// is read and hashed from. Milestone 1 shipped a version where the pane and
/// the anchor each decided the side for themselves and disagreed; the first
/// real review of `rv` then found `commit` deciding separately too, and
/// recording the head for text that only exists on the base. A comment on a
/// removed line whose `commit` names the head points at a revision the quoted
/// text cannot be read back from, which is `commit`'s only job.
struct AnchorTarget<'a> {
    side: Side,
    path: &'a str,
    number: u32,
    commit: &'a str,
}

/// Which side of the diff a comment on a line of this kind belongs to: a
/// removed line only exists on the base side, and everything else — added and
/// context alike — is commented against the head.
///
/// Public because [`crate::ui`] labels each line with the number on the side
/// this returns. A pane that showed one number while the anchor stored another
/// would be lying to the reviewer about what they just commented on.
pub fn anchored_side(kind: LineKind) -> Side {
    match kind {
        LineKind::Removed => Side::Left,
        LineKind::Added | LineKind::Context => Side::Right,
    }
}

/// A comment's id: the first [`ID_CHARS`] hex characters of the blake3 digest
/// of the change, location and body it covers.
///
/// Derived rather than random so that re-typing the same comment on the same
/// line of the same change upserts the entry it already made instead of
/// stacking a duplicate beside it.
///
/// `change_id` is the same string for every comment in a review — it is the
/// range's first change, never the change that touched the line, as
/// [`App::prepare_comment`] spells out — so within one review the location and
/// the body carry the whole of the seed's discriminating power. It stays in
/// because ids outlive the review that made them: `.review/` from another
/// range, keyed by these ids, must not collide with this one's.
///
/// # Why `side` is part of the seed
///
/// The *whole* location has to be in here, and a location is a side as well as
/// a path and a number. difftastic aligns a rewritten line with its counterpart
/// and gives both halves of the pair both numbers, so a rewrite that stays at
/// the same line number (nothing inserted above it) produces a removed line and
/// an added line at, say, `same.rs:2` on the base and head sides respectively.
/// Without the side, one sentence typed on each half — "which of these two is
/// right?" — seeds two identical ids, and
/// [`rv_core::store::Store::append_comment`] upserts by id: the second save
/// silently replaces the first, snapshot and all, under a "comment saved"
/// status line. That is the loss [`ID_CHARS`] argues must never happen, and
/// unlike a digest collision it happens with probability 1.
///
/// The path alone is not enough, even though [`App::prepare_comment`] resolves
/// it per side: the two paths differ only for a rename.
///
/// Adding the side changed every id this function produces. Nothing recomputes
/// an id to find a comment — `comments.json` is keyed by the id it stored,
/// snapshots are filed under it, and `session::fold_replies` matches the id a
/// document's marker carries against the stored one — so a review in progress
/// keeps working across the change: its comments, snapshots and replies all
/// still resolve. The only visible effect is that re-typing a comment saved
/// *before* the change no longer upserts that entry; it appends a second one
/// beside it. A duplicate is recoverable; the loss above is not.
fn comment_id(change_id: &str, path: &str, side: Side, line: u32, body: &str) -> String {
    let side = match side {
        Side::Left => "left",
        Side::Right => "right",
    };
    let seed = format!("{change_id}:{path}:{side}:{line}:{body}");
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
