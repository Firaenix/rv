//! Writing a comment: the buffer, the store, and the export.
//!
//! Saving writes `comments.json` and its snapshot atomically through the store
//! and then rewrites `REVIEW-FEEDBACK.md`, folding in any reply an LLM appended
//! first — so the file an agent reads is never stale by more than one
//! keystroke. The in-memory copy is then re-read from the store, so what is on
//! screen is what is on disk rather than what this process believes it wrote.

use anyhow::Context as _;
use anyhow::Result;
use crossterm::event::KeyCode;
use rv_core::anchor;
use rv_core::store::Comment;
use rv_core::store::CommentState;

use super::Action;
use super::App;
use super::Mode;
use super::anchor::comment_id;
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

        // A new box adds rows to the plan the cursor indexes, so the cursor
        // comes back to the line it commented on rather than to a row number
        // that now means something else.
        let line = self.line_index();
        self.review
            .store
            .append_comment(&comment)
            .context("could not save the comment")?;
        self.reload_comments()?;
        self.resettle_cursor(line);
        session::write_markdown(&self.review)?;

        self.status = format!(
            "comment saved at {}:{}",
            comment.anchor.file, comment.anchor.line
        );
        Ok(())
    }

    /// Builds the [`Comment`] the current selection and buffer describe, or —
    /// as the inner `Err` — the sentence to show instead of saving anything.
    /// The outer [`Result`] is reserved for a repository that could not be
    /// read, which is a real failure rather than a refusal.
    ///
    /// Two of the refusals cannot be provoked from the keyboard alone. "the
    /// review covers no change to comment on" needs an empty `session.changes`,
    /// which only a hand-assembled [`crate::session::Review`] has — and
    /// `rv/tests/app_cases` assembles one. "this line has no number on the side
    /// it belongs to" really is unreachable, and is kept as defence in depth:
    /// every producer in [`rv_core::diff`] numbers the side it dispatches to.
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
        // `change_id` is the *first change of the reviewed range* — the same one
        // for every comment in the review, not the change that introduced the
        // line. Attributing a comment to the change that touched its line is
        // Milestone 2's work (spec §14) and needs per-change diffs. `commit_id`
        // is not taken from it: that comes from the anchored side.
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
            settled_by: None,
        }))
    }

    /// Re-reads the comments from disk.
    ///
    /// Called after every write, so the pane shows what is stored rather than
    /// what this process believes it stored: the store is the authority, and
    /// its upsert may have replaced an entry rather than added one.
    pub(super) fn reload_comments(&mut self) -> Result<()> {
        self.comments = self
            .review
            .store
            .comments()
            .context("could not re-read the saved comments")?;
        // The browser indexes this vector, so it is clamped where the vector is
        // written.
        self.clamp_browser();
        Ok(())
    }
}
