//! Assembling a review: everything a command needs about one revision range,
//! gathered once.
//!
//! [`build`] is the single entry point every `rv` command starts from, so that
//! the CLI and the TUI agree on what "the review" is: the same stack, the same
//! endpoints, the same file list, and the same `.review/` directory. It reads
//! the repository, creates `.review/` (which is what
//! [`rv_core::store::Store::ensure_excluded`] then keeps out of the change under
//! review), and records `session.toml`.
//!
//! [`write_markdown`] is the other shared entry point: every rewrite of
//! `.review/REVIEW-FEEDBACK.md` — `rv render`'s and the TUI's alike — goes
//! through it, so that [`fold_replies`] runs first and no rewrite can destroy
//! a reply an LLM appended.

use std::fs;
use std::io::ErrorKind;
use std::path::Path;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use anyhow::Context as _;
use anyhow::Result;
use rv_core::markdown;
use rv_core::model::FileChange;
use rv_core::store::Comment;
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
/// modification of the change being reviewed. The resolved session is then
/// written to `session.toml`: every command records what it reviewed, so the
/// file on disk always describes the range the comments beside it were made
/// against.
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
    store
        .write_session(&session)
        .context("could not write .review/session.toml")?;

    Ok(Review {
        repo,
        store,
        session,
        files,
    })
}

/// Rewrites `.review/REVIEW-FEEDBACK.md` from the store's comments.
///
/// The document is a *projection* of `comments.json`, so rendering it fresh
/// would drop anything written into the document that the store does not know
/// about — which is exactly what [`fold_replies`] rescues first. Both writers
/// of the file (`rv render` and the TUI, after every saved comment) go through
/// here for that reason; the write itself is atomic, so a program reading the
/// document while `rv` runs never sees half of one.
pub fn write_markdown(review: &Review) -> Result<()> {
    let mut comments = review
        .store
        .comments()
        .context("could not read the review's comments")?;
    fold_replies(review, &mut comments)?;

    let document = markdown::render(&review.session, &comments);
    review
        .store
        .write_markdown(&document)
        .with_context(|| format!("could not write {}", review.store.markdown_path().display()))
}

/// Folds `**Reply:**` blocks found in the current `REVIEW-FEEDBACK.md` back
/// into the stored comments, in `comments` and in `comments.json` alike.
///
/// A reply is the one thing an LLM may add to the document, and the document
/// is rebuilt from `comments.json` on every write — so without this step the
/// next rewrite would delete work that was never stored. Reading the file back
/// before rewriting it makes the round trip lossless.
///
/// The rules are deliberately narrow:
///
/// - A reply whose id matches no stored comment is ignored. `comments.json` is
///   the authority on which comments exist, and the id in a marker may be one
///   an editor mangled or a comment a later session removed.
/// - Two replies under one id leave the last one written, which is the reading
///   that treats the document as an append-only conversation.
/// - **No state transitions.** A comment with a reply is still `Open`;
///   `awaiting-verification` and verification itself are Milestone 2 (spec
///   §14), and this function is where that work attaches.
///
/// A missing document is not an error: nothing has been rendered yet, so there
/// is nothing to rescue.
pub fn fold_replies(review: &Review, comments: &mut [Comment]) -> Result<()> {
    let path = review.store.markdown_path();
    let document = match fs::read_to_string(&path) {
        Ok(document) => document,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(error).with_context(|| format!("could not read {}", path.display()));
        }
    };

    for (id, reply) in markdown::parse_replies(&document) {
        let Some(comment) = comments.iter_mut().find(|comment| comment.id == id) else {
            continue;
        };
        if comment.reply.as_deref() == Some(reply.as_str()) {
            continue;
        }
        comment.reply = Some(reply);
        review
            .store
            .append_comment(comment)
            .with_context(|| format!("could not store the reply to comment {id}"))?;
    }
    Ok(())
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
