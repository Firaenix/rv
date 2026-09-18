//! `R`: re-resolve the range and pick up what the repository now says.
//!
//! jj-lib loads a repo at an operation, so the `Review` opened at launch is a
//! snapshot: move a bookmark, absorb a change, or let an agent push while the
//! reviewer is open, and the pane keeps showing the world as it was. This
//! re-asks the *original question* — the `--from`/`--to` the reviewer typed,
//! with `@` resolving to wherever `@` is now — rather than re-using the resolved
//! commits, which would pin the review to the moment it was opened.
//!
//! # Snapshotting the working copy
//!
//! jj's own CLI snapshots the working copy at the start of every command:
//! that is what makes `jj log` show a change to a tracked file the reviewer
//! made in another window. `Repository::open` (rv-core/src/vcs.rs) does
//! **not** — it loads the operation head and reads from it, on purpose,
//! because rv is otherwise read-only. So without help, `R` re-asks jj about
//! the same operation and sees the same tree it saw at launch: files edited
//! since are invisible until the reviewer runs any `jj` command in another
//! terminal to trigger a snapshot.
//!
//! `R` is the one place that legitimately wants to observe the working copy
//! freshly, so it shells out to `jj status --quiet` before rebuilding. That
//! is the smallest jj command that snapshots the working copy and returns
//! quickly; it is chosen over `jj debug snapshot` because status ships in
//! every jj version rv is compatible with and debug commands do not.
//!
//! Snapshot failure is not fatal: an untracked `jj` binary, a `.jj/` locked
//! by a concurrent jj command, or a snapshot that ran but reported an error
//! all raise an alert and refresh against the un-snapshotted operation
//! anyway. Refreshing against yesterday's tree is closer to what the
//! reviewer wanted than refusing to refresh at all.

use std::path::Path;
use std::process::Command;

use anyhow::Result;

use super::App;
use super::Focus;
use super::SidebarTab;
use crate::tree::NodeKind;

/// The jj subcommand that snapshots the working copy with the least side
/// effect. `status --quiet` is a query; jj snapshots as its first act.
const SNAPSHOT_ARGS: &[&str] = &["status", "--quiet"];

impl App {
    /// Rebuilds the review against the repository as it stands, keeping every
    /// preference and, where it survives, the reviewer's place.
    ///
    /// Built as a fresh `App` and moved into place rather than patched field by
    /// field: everything derived — diffs, stats, highlights, the symbol index,
    /// the commits view — must be re-derived from the new snapshot, and a list
    /// of fields to clear is a list that rots. What is *kept* is the
    /// [`View`](super::view::View), and the fold state, which describes the
    /// reviewer's screen rather than the repository.
    pub(super) fn refresh(&mut self) -> Result<()> {
        let (from, to) = self.review.asked.clone();
        let root = self.review.store.root().to_owned();
        // Snapshot first, then rebuild — see the module doc. Nothing here
        // waits on the outcome: a failed snapshot raises an alert and the
        // refresh runs anyway against whatever operation jj knows about.
        if let Err(message) = snapshot_working_copy(&root) {
            self.raise(message);
        }
        let review = crate::session::build(&root, from.as_deref(), to.as_deref())?;
        let selected = self.selected_file().map(|file| file.path.clone());
        let place = self.sidebar_place();

        // Born with the reviewer's view: every display toggle is in `View`, so
        // there is no list of preferences to carry across and nothing to be
        // left off it — which is how `v #` and `v c` were once lost here.
        let mut fresh = Self::build(
            review,
            self.engine,
            &crate::config::Config::default(),
            self.view,
            self.watch.enabled(),
        )?;
        // The keymap comes across; the watch does not — the fresh one was
        // built over the new snapshot's files, which is what it should be
        // watching from here on.
        std::mem::swap(&mut fresh.keymap, &mut self.keymap);
        fresh.sidebar_tab = self.sidebar_tab;
        // The pane stays the pane: a refresh used to hand the focus to the diff
        // from wherever it was, taking the commits list's tooltip with it. A
        // stack or a flag may not exist in the new snapshot, so those two step
        // back to the diff they hang off.
        fresh.focus = match self.focus {
            Focus::Stack | Focus::Flag => Focus::Diff,
            other => other,
        };
        // Cloned, not taken: `select_file` below can fail, and an error path
        // that had already emptied the old app's fold state would leave the
        // reviewer in the un-refreshed review with their folds gone.
        fresh.collapsed = self.collapsed.clone();
        fresh.collapsed_dirs = self.collapsed_dirs.clone();

        // The file, not the index: a rebased stack lists files in a new order,
        // and index 3 of the new list is not what the reviewer was reading.
        if let Some(path) = selected
            && let Some(index) = fresh.review.files.iter().position(|file| file.path == path)
        {
            fresh.select_file(index)?;
        }
        fresh.resettle_sidebar();
        // The sidebar cursor goes back to the *row it was on* — a change
        // heading, a directory, a file under a change — where that row still
        // exists; `resettle_sidebar` knows only the selected file, and in the
        // commits list can only clamp, which put a fresh app's cursor on row
        // 0, the newest change, whatever the reviewer was reading.
        if let Some(row) = place.and_then(|place| fresh.row_of_place(&place)) {
            fresh.sidebar_row = row;
        }
        // A commits-tab refresh must show a commits-tab diff: the file was
        // re-selected in the bookmark's terms above, and leaving it there would
        // put the branch's diff under a change row — the screen/state
        // disagreement the tab switch already guards against.
        if fresh.sidebar_tab == super::SidebarTab::Commits
            && let Some(crate::tree::NodeKind::File { index }) = fresh
                .nodes()
                .get(fresh.sidebar_row())
                .map(|node| node.kind.clone())
        {
            fresh.select_node_file(index)?;
        }
        fresh.status = format!(
            "refreshed — {} files, {} changes, {} comments",
            fresh.review.files.len(),
            fresh.review.session.changes.len(),
            fresh.comments.len()
        );
        *self = fresh;
        Ok(())
    }

    /// What the sidebar cursor is on, as a name that survives the list being
    /// rebuilt: a change by its id, a directory by its key, a file by its path
    /// under the change heading above it. A row number would not — a refresh
    /// re-sorts, and a rebased stack lists its files in a new order.
    fn sidebar_place(&self) -> Option<String> {
        self.place_of_row(self.sidebar_row)
    }

    fn place_of_row(&self, row: usize) -> Option<String> {
        let nodes = self.nodes();
        let node = nodes.get(row)?;
        Some(match &node.kind {
            NodeKind::Commit { change_id, .. } => format!("change {change_id}"),
            NodeKind::Dir { key, .. } => format!("dir {key}"),
            NodeKind::Up => "up".to_owned(),
            NodeKind::File { index } => {
                let path = match self.sidebar_tab {
                    SidebarTab::Commits => self.commit_path(*index)?.to_owned(),
                    _ => self.review.files.get(*index)?.path.clone(),
                };
                let under = nodes[..row]
                    .iter()
                    .rev()
                    .find_map(|node| match &node.kind {
                        NodeKind::Commit { change_id, .. } => Some(change_id.as_str()),
                        _ => None,
                    })
                    .unwrap_or_default();
                format!("file {under} {path}")
            }
        })
    }

    fn row_of_place(&self, place: &str) -> Option<usize> {
        (0..self.nodes().len()).find(|row| self.place_of_row(*row).as_deref() == Some(place))
    }
}

/// Runs `jj status --quiet` in `root`, which snapshots the working copy as
/// its first step. Returns `Err` with a one-sentence alert message on
/// failure — the caller decides whether to refuse the refresh or carry on.
///
/// `.stdin(Stdio::null())` because a jj that suddenly wanted input would
/// otherwise block the reviewer's keystroke. `stdout`/`stderr` are captured
/// so a `jj` that prints to the terminal cannot paint over the reviewer's
/// screen — rv still owns the alternate screen at this point.
fn snapshot_working_copy(root: &Path) -> std::result::Result<(), String> {
    let output = Command::new("jj")
        .args(SNAPSHOT_ARGS)
        .current_dir(root)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .map_err(|error| format!("could not snapshot the working copy: {error}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let first_line = stderr.lines().next().unwrap_or("").trim();
        return Err(if first_line.is_empty() {
            "could not snapshot the working copy: jj status failed".to_owned()
        } else {
            format!("could not snapshot the working copy: {first_line}")
        });
    }
    Ok(())
}
