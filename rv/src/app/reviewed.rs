//! `x`: ticking the file under the cursor off as reviewed, in the scope the
//! cursor is looking at (flags spec §6–§8).
//!
//! The tick is the durable fact; folding is derived from it. Ticking folds the
//! file's comments and flags in that scope, and a fresh session seeds its
//! folds from the ticks on disk — the collapse set itself never reaches
//! `.review/`, which keeps the inline-comments ruling that a view preference
//! stays in the session.

use anyhow::Context as _;
use anyhow::Result;
use rv_core::store::ReviewedFile;

use super::App;
use super::Focus;
use super::SidebarTab;
use crate::session::reviewed;
pub use crate::session::reviewed::Freshness;
use crate::tree::NodeKind;

/// What the cursor is asking to tick: one file in one scope, or every file a
/// change touched.
#[derive(Clone, Debug, PartialEq, Eq)]
enum TickTarget {
    File {
        path: String,
        change_id: Option<String>,
    },
    Change {
        change_id: String,
        paths: Vec<String>,
    },
}

impl App {
    /// Every tick on disk, with whether each still describes its diff.
    pub fn reviewed(&self) -> &[(ReviewedFile, Freshness)] {
        &self.reviewed
    }

    /// The tick on `path` in the given scope, if there is one.
    pub fn reviewed_mark(&self, path: &str, change_id: Option<&str>) -> Option<Freshness> {
        self.reviewed
            .iter()
            .find(|(tick, _)| tick.is(path, change_id))
            .map(|(_, freshness)| *freshness)
    }

    /// The tick a change's row carries: [`Freshness::Current`] when every file
    /// it touched is ticked and current under it, [`Freshness::Changed`] when
    /// every file is ticked but one has moved, `None` otherwise. Derived
    /// every time it is asked, never stored: a stored rollup would be a
    /// second cursor to keep in step.
    pub fn change_reviewed(&self, change: usize) -> Option<Freshness> {
        let change_id = &self.review.session.changes.get(change)?.change_id;
        let files = self.commit_index().files_of(change);
        if files.is_empty() {
            return None;
        }
        let mut worst = Freshness::Current;
        for file in files {
            match self.reviewed_mark(&file.path, Some(change_id)) {
                None => return None,
                Some(Freshness::Changed) => worst = Freshness::Changed,
                Some(Freshness::Current) => {}
            }
        }
        Some(worst)
    }

    /// The change whose diff the pane is showing, or `None` for the range's.
    pub fn shown_change_id(&self) -> Option<String> {
        match self.shown_target() {
            super::diffs::Target::Commit(pair) => self.pair_change_id(pair),
            super::diffs::Target::File(_) => None,
        }
    }

    fn pair_change_id(&self, pair: usize) -> Option<String> {
        let (change, _) = self.commit_index().pair(pair)?;
        self.review
            .session
            .changes
            .get(change)
            .map(|change| change.change_id.clone())
    }

    /// The tick on the diff the pane is showing.
    pub fn shown_reviewed(&self) -> Option<Freshness> {
        let path = self.selected_file()?.path.clone();
        self.reviewed_mark(&path, self.shown_change_id().as_deref())
    }

    fn tick_target(&self) -> Option<TickTarget> {
        if self.focus == Focus::Sidebar {
            match self.sidebar_tab {
                SidebarTab::Files => {
                    if let Some(NodeKind::File { index }) = self.node_kind_under_cursor()
                        && let Some(file) = self.review.files.get(index)
                    {
                        return Some(TickTarget::File {
                            path: file.path.clone(),
                            change_id: None,
                        });
                    }
                    return None;
                }
                SidebarTab::Commits => match self.node_kind_under_cursor()? {
                    NodeKind::File { index } => {
                        return Some(TickTarget::File {
                            path: self.commit_path(index)?.to_owned(),
                            change_id: self.pair_change_id(index),
                        });
                    }
                    NodeKind::Commit { change_id, .. } => {
                        let change = self
                            .review
                            .session
                            .changes
                            .iter()
                            .position(|change| change.change_id == change_id)?;
                        return Some(TickTarget::Change {
                            paths: self.commit_change_paths(change),
                            change_id,
                        });
                    }
                    NodeKind::Dir { .. } | NodeKind::Up => return None,
                },
                SidebarTab::Comments | SidebarTab::Flags => {
                    return self.browsed_file_path().map(|path| TickTarget::File {
                        path,
                        change_id: None,
                    });
                }
            }
        }
        Some(TickTarget::File {
            path: self.selected_file()?.path.clone(),
            change_id: self.shown_change_id(),
        })
    }

    fn node_kind_under_cursor(&self) -> Option<NodeKind> {
        self.nodes()
            .get(self.sidebar_row)
            .map(|node| node.kind.clone())
    }

    /// Whether `x` has something to tick where the cursor is.
    pub(super) fn can_tick(&self) -> bool {
        self.tick_target().is_some()
    }

    /// Ticks the file under the cursor, or clears its tick; on a change row,
    /// ticks every file it touched — or clears them all when all are ticked.
    pub(super) fn toggle_reviewed(&mut self) -> Result<()> {
        let Some(target) = self.tick_target() else {
            self.status = "nothing here to mark reviewed".to_owned();
            return Ok(());
        };
        let line = self.line_index();
        let status = match target {
            TickTarget::File { path, change_id } => {
                let ticked = self.reviewed_mark(&path, change_id.as_deref()).is_some();
                if ticked {
                    reviewed::unmark(&self.review, &path, change_id.as_deref())?;
                } else {
                    reviewed::mark(&self.review, &path, change_id.as_deref())?;
                    self.fold_reviewed(&path, change_id.as_deref());
                }
                format!(
                    "{} {path}{}",
                    if ticked { "unreviewed" } else { "reviewed" },
                    change_id.map_or_else(String::new, |change| format!(" in {change}"))
                )
            }
            TickTarget::Change { change_id, paths } => {
                let all_ticked = paths
                    .iter()
                    .all(|path| self.reviewed_mark(path, Some(&change_id)).is_some());
                for path in &paths {
                    if all_ticked {
                        reviewed::unmark(&self.review, path, Some(&change_id))?;
                    } else {
                        reviewed::mark(&self.review, path, Some(&change_id))?;
                        self.fold_reviewed(path, Some(&change_id));
                    }
                }
                format!(
                    "{} {} files in {change_id}",
                    if all_ticked { "unreviewed" } else { "reviewed" },
                    paths.len()
                )
            }
        };
        self.load_reviewed()?;
        self.resettle_cursor(line);
        self.status = status;
        Ok(())
    }

    /// Folds every comment and flag on `path` in the scope just ticked.
    fn fold_reviewed(&mut self, path: &str, change_id: Option<&str>) {
        let in_scope = |file: &str, change: &str| {
            file == path && change_id.is_none_or(|wanted| wanted == change)
        };
        let ids: Vec<String> = self
            .comments
            .iter()
            .filter(|comment| in_scope(&comment.anchor.file, &comment.change_id))
            .map(|comment| comment.id.clone())
            .chain(
                self.flags
                    .iter()
                    .filter(|flag| in_scope(&flag.anchor.file, &flag.change_id))
                    .map(|flag| flag.id.clone()),
            )
            .collect();
        self.collapsed.extend(ids);
    }

    /// Re-reads the ticks from disk and re-checks each against the code.
    pub(super) fn load_reviewed(&mut self) -> Result<()> {
        let ticks = self
            .review
            .store
            .reviewed()
            .context("could not read the reviewed files")?;
        self.reviewed = ticks
            .into_iter()
            .map(|tick| {
                let freshness = reviewed::freshness(&self.review, &tick);
                (tick, freshness)
            })
            .collect();
        Ok(())
    }

    /// Seeds the fold set from the ticks: a reviewed file opens with its
    /// comments and flags out of the way, as it was left.
    pub(super) fn fold_all_reviewed(&mut self) {
        let ticks: Vec<(String, Option<String>)> = self
            .reviewed
            .iter()
            .map(|(tick, _)| (tick.file.clone(), tick.change_id.clone()))
            .collect();
        for (path, change_id) in ticks {
            self.fold_reviewed(&path, change_id.as_deref());
        }
    }
}
