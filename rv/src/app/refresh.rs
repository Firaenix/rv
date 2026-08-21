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
    /// of fields to clear is a list that rots. What is *kept* is the short
    /// list: view preferences, and the fold state, which describes the
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

        let mut fresh = Self::build(review, self.engine)?;
        fresh.split = self.split;
        fresh.tree = self.tree;
        fresh.sort = self.sort;
        fresh.sidebar_tab = self.sidebar_tab;
        fresh.sidebar_hidden = self.sidebar_hidden;
        fresh.info_dismissed = self.info_dismissed;
        // Cloned, not taken: `select_file` below can fail, and an error path
        // that had already emptied the old app's fold state would leave the
        // reviewer in the un-refreshed review with their folds gone.
        fresh.collapsed = self.collapsed.clone();
        fresh.collapsed_dirs = self.collapsed_dirs.clone();
        // Carry the `f` toggle across: it is a display preference like the
        // split and the sort, and losing it on `R` would silently reset a
        // reviewer's chosen view.
        fresh.set_full_context(self.full_context());

        // The file, not the index: a rebased stack lists files in a new order,
        // and index 3 of the new list is not what the reviewer was reading.
        if let Some(path) = selected
            && let Some(index) = fresh.review.files.iter().position(|file| file.path == path)
        {
            fresh.select_file(index)?;
        }
        fresh.resettle_sidebar();
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
