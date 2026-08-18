//! Resolving and abandoning a comment.
//!
//! Both are ordinary state changes and both are reversible — `r` on a resolved
//! comment puts it back to open — so neither asks first. Deleting is the only
//! act that does, because deleting is the only one that cannot be undone
//! (storage spec §3).

use anyhow::Context as _;
use anyhow::Result;
use rv_core::store::CommentState;
use rv_core::store::SettledBy;

use super::App;
use super::status::NO_COMMENTS;
use super::status::NO_COMMENTS_IN_REVIEW;
use super::status::SETTLE_NEEDS_A_COMMENT;
use super::{Focus, SidebarTab};

impl App {
    /// Which comment `r` and `a` would act on, or `None` where they refuse.
    ///
    /// The same target `d` takes, because a reviewer aiming at a comment means
    /// the one they can see whichever key they reach for.
    pub(super) fn settle_target(&self) -> Option<&rv_core::store::Comment> {
        self.delete_target()
    }

    /// Resolves the comment under the cursor, or reopens it if it is already
    /// resolved.
    pub(super) fn resolve_comment(&mut self) -> Result<()> {
        self.settle(CommentState::Resolved, "resolved")
    }

    /// Abandons the comment under the cursor, or reopens it if it is already
    /// abandoned.
    pub(super) fn abandon_comment(&mut self) -> Result<()> {
        self.settle(CommentState::Abandoned, "abandoned")
    }

    /// Moves the comment under the cursor to `wanted`, or back to `Open` when
    /// it is already there — which is what makes both keys their own undo.
    fn settle(&mut self, wanted: CommentState, verb: &str) -> Result<()> {
        let Some(comment) = self.settle_target() else {
            self.status = match (self.focus, self.sidebar_tab) {
                (Focus::Sidebar, SidebarTab::Files) => SETTLE_NEEDS_A_COMMENT,
                (Focus::Sidebar, SidebarTab::Comments) => NO_COMMENTS_IN_REVIEW,
                _ => NO_COMMENTS,
            }
            .to_owned();
            return Ok(());
        };

        let id = comment.id.clone();
        let label = format!("{}:{}", comment.anchor.file, comment.anchor.line);
        let reopening = comment.state == wanted;
        let state = if reopening { CommentState::Open } else { wanted };

        self.review
            .store
            .settle_comment(&id, state, SettledBy::User)
            .with_context(|| format!("could not update the comment at {label}"))?;
        self.reload_comments()?;

        self.status = if reopening {
            format!("reopened {label}")
        } else {
            format!("{verb} {label}")
        };
        Ok(())
    }
}
