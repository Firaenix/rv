//! Flags in the reviewer: the read-through copy, the lines they sit on, and
//! the keys that write, acknowledge and walk them (flags spec §5).

use anyhow::Context as _;
use anyhow::Result;
use rv_core::diff::DiffLine;
use rv_core::store::Flag;

use super::App;
use super::Focus;
use super::Mode;
use crate::session;

impl App {
    pub fn flags(&self) -> &[Flag] {
        &self.flags
    }

    /// The flags on diff line `index` of the shown diff, oldest first —
    /// matched exactly as [`App::comments_for_line`] matches a comment.
    pub fn flags_for_line(&self, index: usize) -> Vec<&Flag> {
        let displayed = self.displayed_lines();
        let Some(line) = displayed.get(index) else {
            return Vec::new();
        };
        self.flags_anchored_at(line)
    }

    /// Which flag of the selected line the cursor is on. Only meaningful while
    /// the focus is [`Focus::Flag`].
    pub fn flag_index(&self) -> usize {
        self.flag_index
    }

    /// The flag the cursor is on, or `None` off [`Focus::Flag`] — for the
    /// reason [`App::selected_comment`] is `None` off the stack: `D` asks this,
    /// and a flag the reviewer has not selected is the wrong one to delete.
    pub fn selected_flag(&self) -> Option<&Flag> {
        if self.focus != Focus::Flag {
            return None;
        }
        self.flags_for_line(self.line_index())
            .get(self.flag_index)
            .copied()
    }

    pub(super) fn flag_stack_len(&self) -> usize {
        self.flags_for_line(self.line_index()).len()
    }

    /// Steps the cursor onto the `position`-th flag of the selected line.
    pub(super) fn enter_flag(&mut self, position: usize) {
        self.focus = Focus::Flag;
        self.flag_index = position;
    }

    pub(super) fn flags_anchored_at(&self, line: &DiffLine) -> Vec<&Flag> {
        let Some(target) = self.anchor_target(line) else {
            return Vec::new();
        };
        self.flags
            .iter()
            .filter(|flag| {
                flag.anchor.file == target.path
                    && flag.anchor.side == target.side
                    && flag.anchor.line == target.number
            })
            .collect()
    }

    /// Enters [`Mode::Flag`] on an empty buffer, unless there is no line to
    /// point at.
    pub(super) fn begin_flag(&mut self) {
        if self.selected_line().is_none() {
            self.status = "no diff line selected, nothing to flag".to_owned();
            return;
        }
        self.mode = Mode::Flag;
        self.buffer.clear();
    }

    /// Saves the typed reason as a flag on the selected line.
    pub(super) fn commit_flag(&mut self) -> Result<()> {
        let reason = self.buffer.trim().to_owned();
        if reason.is_empty() {
            self.status = "empty flag, nothing saved".to_owned();
            return Ok(());
        }
        let Some(line) = self.selected_line() else {
            self.status = "no diff line selected, nothing saved".to_owned();
            return Ok(());
        };
        let Some(target) = self.anchor_target(&line) else {
            self.status = "this line has no number on the side it belongs to".to_owned();
            return Ok(());
        };
        let flag = session::flags::save_flag(
            &self.review,
            target.path,
            target.side,
            target.number,
            &target.commit,
            &reason,
        )?;
        let line = self.line_index();
        self.reload_flags()?;
        self.resettle_cursor(line);
        self.status = format!("flagged {}:{}", flag.anchor.file, flag.anchor.line);
        Ok(())
    }

    /// Which flags `A` would acknowledge: the one the cursor is on, in the
    /// flag browser or on a flag; every one on the selected line, from the
    /// diff.
    fn ack_targets(&self) -> Vec<(String, bool)> {
        let one = match self.focus {
            Focus::Sidebar => Some(self.browsed_flag()),
            Focus::Flag => Some(self.selected_flag()),
            Focus::Diff | Focus::Stack => None,
        };
        if let Some(flag) = one {
            return flag
                .map(|flag| (flag.id.clone(), flag.acknowledged))
                .into_iter()
                .collect();
        }
        self.flags_for_line(self.line_index())
            .iter()
            .map(|flag| (flag.id.clone(), flag.acknowledged))
            .collect()
    }

    pub(super) fn can_acknowledge(&self) -> bool {
        !self.ack_targets().is_empty()
    }

    /// Acknowledges the selected line's flags, or — when all of them already
    /// are — un-acknowledges them, so one key is also the undo.
    pub(super) fn acknowledge_flags(&mut self) -> Result<()> {
        let targets = self.ack_targets();
        if targets.is_empty() {
            self.status = "no flags on this line".to_owned();
            return Ok(());
        }
        let all_seen = targets.iter().all(|(_, seen)| *seen);
        for (id, _) in &targets {
            self.review
                .store
                .acknowledge_flag(id, !all_seen)
                .with_context(|| format!("could not update flag {id}"))?;
            if all_seen {
                self.collapsed.remove(id);
            } else {
                self.collapsed.insert(id.clone());
            }
        }
        let line = self.line_index();
        self.reload_flags()?;
        self.resettle_cursor(line);
        self.status = format!(
            "{} {} flag{}",
            if all_seen { "reopened" } else { "acknowledged" },
            targets.len(),
            if targets.len() == 1 { "" } else { "s" }
        );
        Ok(())
    }

    /// Every flag in the review in reading order: by file as the review
    /// lists it, then by line.
    fn flags_in_order(&self) -> Vec<(usize, usize)> {
        let mut order: Vec<(usize, usize)> = self
            .flags
            .iter()
            .enumerate()
            .filter_map(|(index, flag)| {
                let file = self.review.files.iter().position(|file| {
                    file.path == flag.anchor.file
                        || file.source_path.as_deref() == Some(flag.anchor.file.as_str())
                })?;
                Some((file, index))
            })
            .collect();
        order.sort_by_key(|&(file, index)| (file, self.flags[index].anchor.line));
        order
    }

    pub(super) fn has_flags(&self) -> bool {
        !self.flags_in_order().is_empty()
    }

    /// `g f` / `g F`: to the next or previous flag after the cursor, wrapping
    /// round the review.
    pub(super) fn jump_flag(&mut self, forward: bool) -> Result<()> {
        let order = self.flags_in_order();
        if order.is_empty() {
            self.status = "no flags in this review".to_owned();
            return Ok(());
        }
        let here_line = self
            .selected_line()
            .and_then(|line| self.anchor_target(&line).map(|target| target.number))
            .unwrap_or(0);
        let here = (self.file_index, here_line);
        let position = |&(file, index): &(usize, usize)| (file, self.flags[index].anchor.line);
        let next = if forward {
            order
                .iter()
                .find(|entry| position(entry) > here)
                .or_else(|| order.first())
        } else {
            order
                .iter()
                .rev()
                .find(|entry| position(entry) < here)
                .or_else(|| order.last())
        };
        let Some(&(_, index)) = next else {
            return Ok(());
        };
        let anchor = self.flags[index].anchor.clone();
        self.jump_to_anchor(&anchor)?;
        self.focus = Focus::Diff;
        Ok(())
    }

    pub(super) fn reload_flags(&mut self) -> Result<()> {
        self.flags = session::flags::in_range(
            &self.review,
            self.review
                .store
                .flags()
                .context("could not re-read the saved flags")?,
        );
        Ok(())
    }
}
