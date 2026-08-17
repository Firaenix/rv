//! Assembling a review: everything a command needs about one revision range,
//! gathered once.
//!
//! [`build`] is the single entry point every `rv` command starts from, so that
//! the CLI, and later the TUI, agree on what "the review" is: the same stack,
//! the same endpoints, the same file list, and the same `.review/` directory.
//! It reads the repository and creates `.review/` (which is what
//! [`rv_core::store::Store::ensure_excluded`] then keeps out of the change under
//! review); it deliberately does not write `session.toml`, leaving persistence
//! to the command that owns a session's lifetime.

use std::path::Path;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use anyhow::Context as _;
use anyhow::Result;
use rv_core::model::FileChange;
use rv_core::store::Session;
use rv_core::store::Store;
use rv_core::vcs::Repository;

/// The revision the review starts from when the user names none. Rendered into
/// [`Session::revset`] verbatim, so the string here has to match the default
/// [`Repository::stack`] applies.
const DEFAULT_BASE: &str = "trunk()";

/// The revision the review ends at when the user names none: the working copy.
const DEFAULT_HEAD: &str = "@";

/// One assembled review: the repository handle to read more from, the store to
/// record into, and the resolved session and file list.
///
/// `repo` and `store` are kept alongside the data because every command needs
/// at least one of them afterwards — `render` reads comments out of the store,
/// and the TUI reads blobs out of the repository.
pub struct Review {
    pub repo: Repository,
    pub store: Store,
    pub session: Session,
    pub files: Vec<FileChange>,
}

/// Resolves `base..head` in the workspace at `repo_root` into a [`Review`].
///
/// `base` defaults to `trunk()` and `head` to the working copy, matching
/// [`Repository::stack`]. An empty range is an error, surfaced from
/// `rv-core` with both endpoints named: there is nothing to review, and saying
/// so beats presenting a blank session.
///
/// Opening the [`Store`] creates `.review/` and appends it to
/// `.git/info/exclude`, so that the notes a review produces never show up as a
/// modification of the change being reviewed.
pub fn build(repo_root: &Path, base: Option<&str>, head: Option<&str>) -> Result<Review> {
    // `vcs::Error` already names the path in every open failure, so wrapping
    // this one in more context would only repeat it.
    let repo = Repository::open(repo_root)?;

    let changes = repo.stack(base, head)?;
    let (base_commit, head_commit) = repo.endpoints(base, head)?;
    let files = repo
        .files(&base_commit, &head_commit)
        .context("could not enumerate changed files")?;

    let store = Store::open(repo_root)
        .with_context(|| format!("could not open {}/.review", repo_root.display()))?;
    store
        .ensure_excluded()
        .context("could not add /.review/ to .git/info/exclude")?;

    let session = Session {
        revset: format!(
            "{}..{}",
            base.unwrap_or(DEFAULT_BASE),
            head.unwrap_or(DEFAULT_HEAD)
        ),
        base_commit,
        head_commit,
        changes,
        started_at: started_at(),
    };

    Ok(Review {
        repo,
        store,
        session,
        files,
    })
}

/// Now, as `"epoch:<unix_secs>"`.
///
/// The store treats `started_at` as an opaque string, so `rv` records seconds
/// since the epoch rather than taking on a date-time dependency for one header
/// line. A clock set before 1970 has no representation here and reports
/// `epoch:0`, which is a wrong timestamp rather than a failed review.
fn started_at() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0);
    format!("epoch:{seconds}")
}
