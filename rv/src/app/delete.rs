//! Deleting a comment, behind the one confirmation in the reviewer.
//!
//! A delete goes through the store and stops there: the entry and its snapshot
//! go, the in-memory copy is re-read, and `REVIEW-FEEDBACK.md` is **not**
//! rewritten. The asymmetry with saving is deliberate — the markdown is an
//! *export* (storage-model spec §5), and a delete that rewrote it would also be
//! rewriting whatever reply an LLM had appended since.

use anyhow::Context as _;
use anyhow::Result;
use crossterm::event::KeyCode;
use rv_core::store::Comment;

use super::Action;
use super::App;
use super::Focus;
use super::Mode;
use super::SidebarTab;
use super::status::DELETE_NEEDS_A_COMMENT;
use super::status::NO_COMMENTS;
use super::status::NO_COMMENTS_IN_REVIEW;

impl App {
    /// Which comment `d` would ask about, or `None` where it would refuse.
    ///
    /// [`App::binding_enabled`] asks the same question to decide whether to dim
    /// the row, so the popup cannot claim `d` is live somewhere it refuses.
    pub(super) fn delete_target(&self) -> Option<&Comment> {
        match self.focus {
            Focus::Stack => self.selected_comment(),
            Focus::Diff => self.comments_for_line(self.line_index()).last().copied(),
            // `browsed_comment` is already `None` on the Files tab, so this
            // covers both of the sidebar's shapes.
            Focus::Sidebar => self.browsed_comment(),
        }
    }

    /// Asks before deleting: picks what `d` would remove and enters
    /// [`Mode::ConfirmDelete`] with the question in the status line.
    ///
    /// The rules differ by cursor because the situations do. Inside the stack
    /// `d` takes the comment the cursor is on. On the diff it takes the
    /// *newest* on the line — the one just written, and the one a reviewer
    /// reaching for `d` means; the oldest is the note they have lived with
    /// longest. In the sidebar's **Comments** tab it takes the comment on
    /// screen. The **Files** tab deletes nothing: `c` writes against the
    /// selected diff line from there and the symmetry is tempting, but `c`
    /// creates and `d` destroys, and a `d` pressed at a list of *files* would
    /// be aimed at a comment the reviewer cannot see.
    ///
    /// With nothing to delete there is no question worth asking.
    pub(super) fn begin_delete(&mut self) {
        let Some(comment) = self.delete_target() else {
            self.status = match (self.focus, self.sidebar_tab) {
                (Focus::Sidebar, SidebarTab::Files) => DELETE_NEEDS_A_COMMENT,
                (Focus::Sidebar, SidebarTab::Comments) => NO_COMMENTS_IN_REVIEW,
                _ => NO_COMMENTS,
            }
            .to_owned();
            return;
        };

        let label = format!("{}:{}", comment.anchor.file, comment.anchor.line);
        let id = comment.id.clone();
        self.status = format!("delete comment at {label}? (y/n)");
        self.mode = Mode::ConfirmDelete { id, label };
    }

    /// Answers the confirmation — `y` deletes, anything else cancels — and
    /// leaves [`Mode::ConfirmDelete`] either way.
    ///
    /// The mode is taken out *first*, so leaving it is not something any branch
    /// below could forget: whatever happens after that line, the `?` on an
    /// unwritable store included, the reviewer's keyboard does what it did
    /// before. A confirmation nobody can dismiss is worse than none at all.
    ///
    /// Only a lowercase `y` confirms. Every ambiguity resolves toward keeping
    /// the comment, because one of the two mistakes is recoverable by pressing
    /// `d` again and the other is not recoverable at all.
    pub(super) fn on_key_confirm_delete(&mut self, key: KeyCode) -> Result<Action> {
        let Mode::ConfirmDelete { id, label } = std::mem::replace(&mut self.mode, Mode::Browse)
        else {
            // Unreachable: dispatch reaches here only from `ConfirmDelete`.
            return Ok(Action::Continue);
        };

        if key != KeyCode::Char('y') {
            self.status = format!("deletion cancelled, {label} kept");
            return Ok(Action::Continue);
        }

        // Counted from the line rather than the review: "1 of 3" is what says
        // how much of what the reviewer was looking at is still there. Read
        // before the removal, like the line — a delete takes a box's rows out
        // of the plan the cursor indexes.
        let before = self.stack_len();
        let line = self.line_index();
        let removed = self
            .review
            .store
            .remove_comment(&id)
            .with_context(|| format!("could not delete the comment at {label}"))?;
        self.reload_comments()?;
        // A folded comment that is gone is not folded, it is gone: leaving the
        // id behind would fold a later comment that hashed to it — the same
        // body on the same line — under a preference about a deleted one.
        self.collapsed.remove(&id);
        self.status = if removed {
            format!("deleted {label} (1 of {before} on this line)")
        } else {
            // Another process deleted it, or this one is re-answering a
            // question about a comment that has already gone. Idempotent, and
            // said out loud rather than reported as a deletion.
            format!("nothing to delete at {label}, it was already gone")
        };
        self.resettle_cursor(line);
        self.sync_stack();
        Ok(Action::Continue)
    }
}
