//! Noticing that the repository moved under a running review.
//!
//! The probe is jj's operation heads: **every** mutation — a commit, a
//! checkout, a rebase, a describe, a snapshot — moves the op head, so one
//! readdir per couple of seconds answers "did anything happen" without
//! loading a repo or spawning a process. Plain file edits are caught the
//! moment anything snapshots them, which includes the refresh this watch
//! triggers and every jj command the reviewer or their agent runs.
//!
//! Deliberately terminal-free and clock-free: [`Watch::moved`] takes `now` as
//! an argument, per the module rule that a clock is ambient input.

use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;
use std::time::Instant;

/// How often the op heads are asked about. Two seconds: an auto-refresh that
/// lands within a breath of the `jj` command that caused it reads as
/// immediate, and a readdir every two seconds costs nothing worth counting.
pub(super) const WATCH_INTERVAL: Duration = Duration::from_secs(2);

pub(super) struct Watch {
    enabled: bool,
    fingerprint: Option<String>,
    checked: Option<Instant>,
}

impl Watch {
    pub fn new(enabled: bool, root: &Path) -> Self {
        Self {
            enabled,
            fingerprint: fingerprint(root),
            checked: None,
        }
    }

    /// Whether the repository moved since the last look — rate-limited to
    /// [`WATCH_INTERVAL`], and silently `false` wherever the op heads cannot
    /// be read: a watch that cannot see must not keep reporting movement.
    pub fn moved(&mut self, root: &Path, now: Instant) -> bool {
        if !self.enabled {
            return false;
        }
        if let Some(checked) = self.checked
            && now.duration_since(checked) < WATCH_INTERVAL
        {
            return false;
        }
        self.checked = Some(now);
        let current = fingerprint(root);
        if current.is_none() || current == self.fingerprint {
            return false;
        }
        self.fingerprint = current;
        true
    }

    /// Re-reads the baseline — called after a refresh, whose own snapshot
    /// moves the op head; without this the refresh would schedule the next.
    pub fn settle(&mut self, root: &Path) {
        self.fingerprint = fingerprint(root);
    }

    /// Whether the event loop needs to wake for this watch at all.
    pub fn enabled(&self) -> bool {
        self.enabled
    }
}

/// The operation heads as one string: the sorted file names under
/// `.jj/repo/op_heads/heads/`.
fn fingerprint(root: &Path) -> Option<String> {
    let mut names: Vec<String> = std::fs::read_dir(op_heads_dir(root)?)
        .ok()?
        .filter_map(std::result::Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    Some(names.join(","))
}

/// Where the op heads live, following the pointer a secondary workspace
/// leaves: its `.jj/repo` is a file naming the primary's repo directory.
fn op_heads_dir(root: &Path) -> Option<PathBuf> {
    let repo = root.join(".jj").join("repo");
    let repo_dir = if repo.is_file() {
        let target = PathBuf::from(std::fs::read_to_string(&repo).ok()?.trim());
        if target.is_absolute() {
            target
        } else {
            root.join(".jj").join(target)
        }
    } else {
        repo
    };
    Some(repo_dir.join("op_heads").join("heads"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unreadable_repo_never_reports_movement() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut watch = Watch::new(true, dir.path());
        assert!(!watch.moved(dir.path(), Instant::now()));
    }

    #[test]
    fn a_disabled_watch_is_inert() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut watch = Watch::new(false, dir.path());
        assert!(!watch.enabled());
        assert!(!watch.moved(dir.path(), Instant::now()));
    }

    #[test]
    fn a_new_op_head_reports_movement_once() {
        let dir = tempfile::tempdir().expect("tempdir");
        let heads = dir.path().join(".jj/repo/op_heads/heads");
        std::fs::create_dir_all(&heads).expect("create heads");
        std::fs::write(heads.join("op1"), b"").expect("first op");

        let mut watch = Watch::new(true, dir.path());
        let t0 = Instant::now();
        assert!(!watch.moved(dir.path(), t0), "nothing moved yet");

        std::fs::remove_file(heads.join("op1")).expect("advance");
        std::fs::write(heads.join("op2"), b"").expect("second op");
        let later = t0 + WATCH_INTERVAL + Duration::from_millis(1);
        assert!(watch.moved(dir.path(), later), "the op head moved");
        let much_later = later + WATCH_INTERVAL + Duration::from_millis(1);
        assert!(
            !watch.moved(dir.path(), much_later),
            "one movement is one report"
        );
    }

    #[test]
    fn looks_are_rate_limited() {
        let dir = tempfile::tempdir().expect("tempdir");
        let heads = dir.path().join(".jj/repo/op_heads/heads");
        std::fs::create_dir_all(&heads).expect("create heads");
        std::fs::write(heads.join("op1"), b"").expect("first op");

        let mut watch = Watch::new(true, dir.path());
        let t0 = Instant::now();
        assert!(!watch.moved(dir.path(), t0));
        std::fs::write(heads.join("op2"), b"").expect("second op");
        assert!(
            !watch.moved(dir.path(), t0 + Duration::from_millis(10)),
            "a look inside the interval is skipped"
        );
    }

    #[test]
    fn a_secondary_workspace_pointer_is_followed() {
        let dir = tempfile::tempdir().expect("tempdir");
        let primary = dir.path().join("primary/.jj/repo/op_heads/heads");
        std::fs::create_dir_all(&primary).expect("create primary heads");
        std::fs::write(primary.join("op1"), b"").expect("op");
        let secondary = dir.path().join("secondary/.jj");
        std::fs::create_dir_all(&secondary).expect("create secondary");
        std::fs::write(
            secondary.join("repo"),
            dir.path().join("primary/.jj/repo").display().to_string(),
        )
        .expect("write the pointer");

        let mut watch = Watch::new(true, &dir.path().join("secondary"));
        let later = Instant::now() + WATCH_INTERVAL + Duration::from_millis(1);
        std::fs::write(primary.join("op2"), b"").expect("advance");
        assert!(
            watch.moved(&dir.path().join("secondary"), later),
            "the secondary workspace watches the primary's op heads"
        );
    }
}
