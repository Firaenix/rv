//! Writing a comment: the buffer, the store, and the export.
//!
//! Saving writes `comments.json` atomically through the store
//! and then rewrites `REVIEW-FEEDBACK.md`, folding in any reply an LLM appended
//! first — so the file an agent reads is never stale by more than one
//! keystroke. The in-memory copy is then re-read from the store, so what is on
//! screen is what is on disk rather than what this process believes it wrote.

use anyhow::Context as _;
use anyhow::Result;
use crossterm::event::KeyCode;
use rv_core::store::Comment;

use super::Action;
use super::App;
use super::Mode;
use crate::session;

impl App {
    pub(super) fn on_key_comment(&mut self, key: KeyCode) -> Result<Action> {
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
    pub(super) fn begin_comment(&mut self) {
        if self.selected_line().is_none() {
            self.status = "no diff line selected, nothing to comment on".to_owned();
            return;
        }
        self.mode = Mode::Comment;
        self.buffer.clear();
    }

    /// Saves the typed comment against the selected line, then rewrites the
    /// markdown export.
    ///
    /// A *suppressed* diff is not a refusal. Suppression says the difference
    /// between the two sides is not visible in the lines, not that the lines
    /// are unreal: the difftastic case carries no lines and is refused for
    /// that, and the fallback case carries every line as `Context` under a note
    /// saying the difference is elsewhere. Refusing it would mean refusing a
    /// line the reviewer is looking at.
    fn commit_comment(&mut self) -> Result<()> {
        let comment = match self.prepare_comment()? {
            Ok(comment) => comment,
            Err(reason) => {
                self.status = reason;
                return Ok(());
            }
        };

        // `prepare_comment` saved it through `session::save_comment`, export
        // refresh included; what is left is the screen. A new box adds rows to
        // the plan the cursor indexes, so the cursor comes back to the line it
        // commented on rather than to a row number that now means something
        // else.
        let line = self.line_index();
        self.reload_comments()?;
        self.resettle_cursor(line);

        self.status = format!(
            "comment saved at {}:{}",
            comment.anchor.file, comment.anchor.line
        );
        Ok(())
    }

    /// Resolves where the comment goes — the selected line's side, path, number
    /// and commit — or, as the inner `Err`, the sentence to show instead.
    ///
    /// Construction and saving happen in [`session::save_comment`], the one
    /// path the CLI also uses: everything after "which line is this about" is
    /// policy the two must not answer differently.
    fn prepare_comment(&self) -> Result<Result<Comment, String>> {
        let body = self.buffer.trim();
        if body.is_empty() {
            return Ok(Err("empty comment, nothing saved".to_owned()));
        }
        let Some(line) = self.selected_line() else {
            return Ok(Err("no diff line selected, nothing saved".to_owned()));
        };
        let Some(target) = self.anchor_target(line) else {
            return Ok(Err(
                "this line has no number on the side it belongs to".to_owned()
            ));
        };
        if self.review.session.changes.is_empty() {
            return Ok(Err("the review covers no change to comment on".to_owned()));
        }
        Ok(Ok(session::save_comment(
            &self.review,
            target.path,
            target.side,
            target.number,
            &target.commit,
            body,
        )?))
    }

    /// Re-reads the comments from disk.
    ///
    /// Called after every write, so the pane shows what is stored rather than
    /// what this process believes it stored: the store is the authority, and
    /// its upsert may have replaced an entry rather than added one.
    pub(super) fn reload_comments(&mut self) -> Result<()> {
        let mut comments = crate::session::in_range(
            &self.review,
            self.review
                .store
                .comments()
                .context("could not re-read the saved comments")?,
        );
        crate::stale::mark_outdated(&self.review, &mut comments);
        self.comments = comments;
        // The browser indexes this vector, so it is clamped where the vector is
        // written.
        self.clamp_browser();
        Ok(())
    }
}
