//! What the renderer and the keyboard read about a review's comments: which one
//! a cursor is on, which ones a diff line carries, and how far each one's anchor
//! has drifted from the code it was written against.
//!
//! Split out of [`super::query`], whose accessors group by subject. Everything
//! here is derived per call except `drift`, which reads the survey taken when
//! the comments were last loaded — see [`crate::stale::survey`] for why that one
//! is not.

use std::collections::HashSet;

use rv_core::model::Confidence;
use rv_core::store::Comment;

use super::App;
use super::Focus;
use super::SidebarTab;
use crate::stale::Drift;

impl App {
    /// Which row of the comment browser the cursor is on.
    pub fn browser_index(&self) -> usize {
        self.browser_index
    }

    /// The comment the browser's cursor is on, or `None` when the sidebar is
    /// not listing comments — or when the cursor is on a **file heading**,
    /// which names a file rather than selecting a comment.
    ///
    /// Gated on the tab, not the focus: `d` asks this to decide what it
    /// destroys, and answering with a comment that is not on screen is how a
    /// delete hits the wrong one. The browser draws its selection whether or
    /// not the keys are pointed at it, so the selection is real either way.
    pub fn browsed_comment(&self) -> Option<&Comment> {
        if self.sidebar_tab != SidebarTab::Comments {
            return None;
        }
        self.comments.get(self.browsed_index()?)
    }

    /// Every comment in the review, in store order (oldest first).
    pub fn comments(&self) -> &[Comment] {
        &self.comments
    }

    /// The ids of the comments the reviewer has folded away.
    pub fn collapsed(&self) -> &HashSet<String> {
        &self.collapsed
    }

    /// Which comment of the selected line's stack the cursor is on. Only
    /// meaningful while [`App::focus`] is [`Focus::Stack`].
    pub fn comment_index(&self) -> usize {
        self.comment_index
    }

    /// The comment the stack cursor is on, or `None` when the cursor is not in
    /// a stack.
    ///
    /// Deliberately `None` off [`Focus::Stack`]: `d` and `s` ask this to decide
    /// what a keystroke acts on, and answering with a comment the reviewer has
    /// not selected is how a delete hits the wrong one.
    pub fn selected_comment(&self) -> Option<&Comment> {
        if self.focus != Focus::Stack {
            return None;
        }
        self.comments_for_line(self.line_index())
            .get(self.comment_index)
            .copied()
    }

    /// The comments anchored to diff line `index` of the selected file, oldest
    /// first.
    ///
    /// Matched by the key the line would anchor *under*, never by its raw
    /// number: the side and the side's path come from the same
    /// [`App::anchor_target`] the save path goes through, so a comment can
    /// never be stored against one line and displayed against another.
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

    /// How many comments the selected line carries.
    pub(super) fn stack_len(&self) -> usize {
        self.comments_for_line(self.line_index()).len()
    }

    /// What `comment`'s anchor has done since it was written, from the survey
    /// taken when the comments were last read — or `None` for a comment the
    /// survey has not seen, one just written and not yet reloaded.
    pub fn drift(&self, comment: &Comment) -> Option<&Drift> {
        self.drift.get(&comment.id)
    }

    /// How confidently `comment`'s anchor was placed in the code as it now
    /// stands.
    ///
    /// [`Confidence::Exact`] for a comment the survey never saw, which is what
    /// such a comment is: its anchor was created against the very text on screen.
    pub fn confidence(&self, comment: &Comment) -> Confidence {
        self.drift(comment)
            .map_or(Confidence::Exact, |drift| drift.confidence)
    }
}
