//! Noticing that the repository moved under a running review.
//!
//! Two probes, one readdir-cheap look every couple of seconds:
//!
//! * jj's **operation heads**: every mutation — a commit, a checkout, a
//!   rebase, a describe, a snapshot — moves the op head, so one readdir
//!   answers "did jj do anything" without loading a repo or spawning a
//!   process.
//! * the **working copy** of the files under review: an agent's edit, an
//!   editor's save. jj notices those only when something snapshots, and the
//!   refresh this watch triggers is that something — so without this probe
//!   a review of `@` sat on stale text until the reviewer ran a jj command
//!   in another window. The files' mtimes and sizes, and their directories'
//!   mtimes so a file added beside them counts too.
//!
//! Deliberately terminal-free and clock-free: [`Watch::moved`] takes `now` as
//! an argument, per the module rule that a clock is ambient input.

use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;
use std::time::Instant;

/// How often the probes are asked. Two seconds: an auto-refresh that lands
/// within a breath of the edit that caused it reads as immediate, and a
/// handful of stat calls every two seconds costs nothing worth counting.
pub(super) const WATCH_INTERVAL: Duration = Duration::from_secs(2);

pub(super) struct Watch {
    enabled: bool,
    /// The head-side paths of the files under review, relative to the root.
    paths: Vec<PathBuf>,
    op_heads: Option<String>,
    files: String,
    checked: Option<Instant>,
}

impl Watch {
    pub fn new(enabled: bool, root: &Path, paths: Vec<PathBuf>) -> Self {
        Self {
            enabled,
            op_heads: op_heads(root),
            files: files(root, &paths),
            paths,
            checked: None,
        }
    }

    /// Whether the repository moved since the last look — rate-limited to
    /// [`WATCH_INTERVAL`]. The event loop's idle question.
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
        self.look(root)
    }

    /// The same question, asked now regardless of when it was last asked —
    /// for the moment the terminal regains focus, when the reviewer has
    /// just come back from doing the thing that moved it.
    ///
    /// Silently `false` wherever the op heads cannot be read: a watch that
    /// cannot see must not keep reporting movement.
    pub fn look(&mut self, root: &Path) -> bool {
        if !self.enabled {
            return false;
        }
        let op_heads = op_heads(root);
        let files = files(root, &self.paths);
        let moved = (op_heads.is_some() && op_heads != self.op_heads) || files != self.files;
        if op_heads.is_some() {
            self.op_heads = op_heads;
        }
        self.files = files;
        moved
    }

    /// Re-reads the baseline — called after a refresh, whose own snapshot
    /// moves the op head; without this the refresh would schedule the next.
    pub fn settle(&mut self, root: &Path) {
        self.op_heads = op_heads(root);
        self.files = files(root, &self.paths);
    }

    /// Whether the event loop needs to wake for this watch at all.
    pub fn enabled(&self) -> bool {
        self.enabled
    }
}

/// The operation heads as one string: the sorted file names under
/// `.jj/repo/op_heads/heads/`.
fn op_heads(root: &Path) -> Option<String> {
    let mut names: Vec<String> = std::fs::read_dir(op_heads_dir(root)?)
        .ok()?
        .filter_map(std::result::Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    Some(names.join(","))
}

/// The working copy of `paths` as one string: each file's mtime and size,
/// and each parent directory's mtime, in path order. A file that is not
/// there reads as `missing`, so a delete is a change like any other.
fn files(root: &Path, paths: &[PathBuf]) -> String {
    let mut dirs: Vec<&Path> = paths.iter().filter_map(|path| path.parent()).collect();
    dirs.sort();
    dirs.dedup();
    let stamp = |path: &Path| -> String {
        match std::fs::metadata(root.join(path)) {
            Ok(meta) => {
                let modified = meta
                    .modified()
                    .ok()
                    .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                    .map_or(0, |since| since.as_nanos());
                format!("{modified}:{}", meta.len())
            }
            Err(_) => "missing".to_owned(),
        }
    };
    paths
        .iter()
        .map(|path| stamp(path))
        .chain(dirs.into_iter().map(stamp))
        .collect::<Vec<_>>()
        .join(",")
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

    fn heads_dir(root: &Path) -> PathBuf {
        let heads = root.join(".jj/repo/op_heads/heads");
        std::fs::create_dir_all(&heads).expect("create heads");
        std::fs::write(heads.join("op1"), b"").expect("first op");
        heads
    }

    #[test]
    fn an_unreadable_repo_never_reports_movement() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut watch = Watch::new(true, dir.path(), Vec::new());
        assert!(!watch.moved(dir.path(), Instant::now()));
    }

    #[test]
    fn a_disabled_watch_is_inert() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut watch = Watch::new(false, dir.path(), Vec::new());
        assert!(!watch.enabled());
        assert!(!watch.moved(dir.path(), Instant::now()));
        assert!(!watch.look(dir.path()));
    }

    #[test]
    fn a_new_op_head_reports_movement_once() {
        let dir = tempfile::tempdir().expect("tempdir");
        let heads = heads_dir(dir.path());

        let mut watch = Watch::new(true, dir.path(), Vec::new());
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
    fn looks_are_rate_limited_but_a_forced_look_is_not() {
        let dir = tempfile::tempdir().expect("tempdir");
        let heads = heads_dir(dir.path());

        let mut watch = Watch::new(true, dir.path(), Vec::new());
        let t0 = Instant::now();
        assert!(!watch.moved(dir.path(), t0));
        std::fs::write(heads.join("op2"), b"").expect("second op");
        assert!(
            !watch.moved(dir.path(), t0 + Duration::from_millis(10)),
            "a look inside the interval is skipped"
        );
        assert!(watch.look(dir.path()), "a forced look sees it now");
    }

    /// An edit to a watched file — no jj command, no op head moved — is
    /// movement; so is a file appearing beside one.
    #[test]
    fn an_edit_to_a_watched_file_reports_movement() {
        let dir = tempfile::tempdir().expect("tempdir");
        heads_dir(dir.path());
        std::fs::create_dir_all(dir.path().join("src")).expect("src");
        std::fs::write(dir.path().join("src/a.rs"), b"fn a() {}\n").expect("a.rs");

        let mut watch = Watch::new(true, dir.path(), vec![PathBuf::from("src/a.rs")]);
        assert!(!watch.look(dir.path()), "nothing edited yet");

        std::fs::write(dir.path().join("src/a.rs"), b"fn a() { 1 }\n").expect("edit");
        assert!(watch.look(dir.path()), "the file changed size");
        assert!(!watch.look(dir.path()), "one edit is one report");

        std::fs::write(dir.path().join("src/b.rs"), b"").expect("a new file");
        assert!(
            watch.look(dir.path()),
            "a file appeared in a watched directory"
        );

        std::fs::remove_file(dir.path().join("src/a.rs")).expect("delete");
        assert!(watch.look(dir.path()), "the file went missing");
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

        let mut watch = Watch::new(true, &dir.path().join("secondary"), Vec::new());
        let later = Instant::now() + WATCH_INTERVAL + Duration::from_millis(1);
        std::fs::write(primary.join("op2"), b"").expect("advance");
        assert!(
            watch.moved(&dir.path().join("secondary"), later),
            "the secondary workspace watches the primary's op heads"
        );
    }
}
