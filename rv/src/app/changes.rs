//! What the reviewer is told about the change under the cursor.
//!
//! Shown on highlight rather than on a key: moving the cursor onto a change is
//! the act of asking about it. `i` is the way out.

use super::App;
use super::Focus;
use super::SidebarTab;
use crate::gradient::Stat;
use crate::tree;

/// Everything the tooltip shows about one change.
pub struct ChangeInfo {
    pub change_id: String,
    pub commit_id: String,
    pub description: String,
    pub files: Vec<(String, Stat)>,
    pub stat: Stat,
}

impl ChangeInfo {
    /// How tall the tooltip wants to be: its two id rows, its description, a
    /// blank, the totals line, one row per file, and two borders.
    #[must_use]
    pub fn rows(&self) -> u16 {
        let described = self.description.lines().count().max(1);
        let content = 2 + 1 + described + 1 + 1 + self.files.len();
        u16::try_from(content.saturating_add(2)).unwrap_or(u16::MAX)
    }
}

impl App {
    /// Which row the change tooltip hangs off, and how tall it wants to be.
    ///
    /// Shown on **highlight**, with no key: moving the cursor onto a change is the
    /// act of asking about it, and a reviewer who has to press something to see
    /// what they have selected is being made to ask twice. `i` puts it away for as
    /// long as it is unwanted.
    ///
    /// `None` off the Commits tab, off the sidebar, and while it is dismissed.
    #[must_use]
    pub fn tooltip(&self) -> Option<(u16, u16)> {
        let info = self.change_info()?;
        let row = u16::try_from(self.sidebar_row()).ok()?;
        Some((row, info.rows()))
    }

    /// Everything the tooltip shows about the change the cursor is in.
    #[must_use]
    pub fn change_info(&self) -> Option<ChangeInfo> {
        if self.info_dismissed
            || self.focus() != Focus::Sidebar
            || self.sidebar_tab() != SidebarTab::Commits
        {
            return None;
        }
        let index = self.commit_index();
        let change = self.change_under_cursor_index()?;
        let entry = self.review.session.changes.get(change)?;
        let files: Vec<(String, Stat)> = index
            .files_of(change)
            .iter()
            .enumerate()
            .map(|(position, file)| {
                let pair = index.pair_of(change, position).unwrap_or(usize::MAX);
                (file.path.clone(), index.stat_of(pair))
            })
            .collect();
        let stat = files
            .iter()
            .fold(Stat::default(), |total, (_, stat)| total + *stat);
        Some(ChangeInfo {
            change_id: entry.change_id.clone(),
            commit_id: entry.commit_id.clone(),
            description: entry.description.clone(),
            files,
            stat,
        })
    }

    /// Which change the sidebar cursor is on or inside.
    fn change_under_cursor_index(&self) -> Option<usize> {
        let nodes = self.nodes();
        let row = self.sidebar_row().min(nodes.len().saturating_sub(1));
        nodes
            .get(..=row)?
            .iter()
            .rev()
            .find_map(|node| match &node.kind {
                tree::NodeKind::Commit { change_id, .. } => self
                    .review
                    .session
                    .changes
                    .iter()
                    .position(|change| &change.change_id == change_id),
                _ => None,
            })
    }

    /// The change the sidebar cursor is on or inside, as
    /// `(short change id, short commit id, subject)`.
    ///
    /// `None` anywhere but the Commits tab. A file row answers with the change
    /// that holds it, which is what "or inside" means: a reviewer reading a file
    /// under a change is reading that change, and the row above them has scrolled
    /// off as often as not.
    #[must_use]
    pub fn change_under_cursor(&self) -> Option<(String, String, String)> {
        if self.sidebar_tab() != SidebarTab::Commits {
            return None;
        }
        let nodes = self.nodes();
        let row = self.sidebar_row().min(nodes.len().saturating_sub(1));
        nodes
            .get(..=row)?
            .iter()
            .rev()
            .find_map(|node| match &node.kind {
                tree::NodeKind::Commit {
                    short_change,
                    short_commit,
                    subject,
                    ..
                } => Some((short_change.clone(), short_commit.clone(), subject.clone())),
                _ => None,
            })
    }
}
