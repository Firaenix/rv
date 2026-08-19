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
//! [`render_markdown`] is the other shared entry point: the markdown is a
//! **one-way view** of the store — rendered on request by `rv render` and the
//! TUI's `e`, and never read back (CLI-loop spec §2). Agents read the review
//! with `rv comments --json` and answer it with `rv reply`; the one remaining
//! read of the document is [`rescue_replies`], the migration that folds a
//! pre-amendment reply into the store once, on load.

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

mod comments;

pub use comments::add_comment;
pub use comments::owning_change;
pub use comments::reply;
pub use comments::save_comment;
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
    /// The range as it was asked for — `--from` and `--to`, unresolved — so a
    /// refresh re-asks the same question. `@` must resolve to where `@` is *now*,
    /// which is the whole point of refreshing; re-using the resolved commits
    /// would pin the review to the moment it was opened.
    pub asked: (Option<String>, Option<String>),
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
///
/// # A review begins here; a query does not
///
/// This writes `session.toml`, because opening a review is when the file should
/// start describing what is being reviewed. [`read`] is the half a *query* wants:
/// it resolves the same range and writes nothing.
///
/// Both used to be this function, and it made `rv status --json` — which reads
/// like a pure query — rewrite the session record and its `started_at` on every
/// run, moving the timestamp in the header of an existing export. Worse,
/// `rv status --to other-branch` overwrote `session.toml` with a range the
/// comments beside it were never made against, breaking the very invariant the
/// old doc comment claimed: *the file on disk always describes the range the
/// comments beside it were made against*.
///
/// # Why re-pointing is allowed, and only the accident is fixed
///
/// The finding that prompted this offered two remedies: read-only queries, or
/// refusing to re-point a session that already holds comments. The refusal was
/// written first and then taken back out, because it makes a legitimate act
/// impossible — a reviewer opening a narrower range of the same stack is asking
/// for exactly that, and three tests that do it failed, which is evidence the
/// workflow is real rather than hypothetical.
///
/// What made re-pointing dangerous was that it happened *without being asked
/// for*, by a command that reads like a question. That is gone. And a comment is
/// self-describing — it carries its own change, commit and anchor — and the
/// reviewer only ever sees the ones the open range can reach (see
/// `App::in_range`), so a narrower range shows fewer comments rather than
/// mislabelling any.
pub fn build(repo_root: &Path, base: Option<&str>, head: Option<&str>) -> Result<Review> {
    let review = resolve(repo_root, base, head)?;
    // The moment the review began, kept across re-openings of the same range:
    // `started_at` says when the reviewer started, and re-stamping it on every
    // command would make it say when they last ran one.
    let session = Session {
        started_at: existing_start(&review).unwrap_or_else(started_at),
        ..review.session.clone()
    };
    review
        .store
        .write_session(&session)
        .context("could not write .review/session.toml")?;
    Ok(Review { session, ..review })
}

/// The same range, resolved and **not** written down.
///
/// What `rv status` and `rv render` use: a query that rewrote the record it is
/// querying would be reporting on itself.
pub fn read(repo_root: &Path, base: Option<&str>, head: Option<&str>) -> Result<Review> {
    let review = resolve(repo_root, base, head)?;
    // The stored `started_at` where the range matches, so a rendered export is
    // headed with when the review began rather than with now.
    let session = Session {
        started_at: existing_start(&review).unwrap_or_else(|| review.session.started_at.clone()),
        ..review.session.clone()
    };
    Ok(Review { session, ..review })
}

/// `started_at` from the stored session, where it describes this same range.
fn existing_start(review: &Review) -> Option<String> {
    review
        .store
        .read_session()
        .ok()
        .filter(|stored| stored.revset == review.session.revset)
        .map(|stored| stored.started_at)
}

/// Resolves `base..head` without writing `session.toml`.
///
/// Opening the store still creates `.review/` and the exclude entry: those are
/// what keep review notes out of the change under review, and a query that left
/// them undone would have the next write do it at a less predictable moment.
fn resolve(repo_root: &Path, base: Option<&str>, head: Option<&str>) -> Result<Review> {
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
    // The last rescue: a reply an agent wrote into a pre-amendment export is
    // folded into the store here, once, before anything reads the comments.
    rescue_replies(&store)?;

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
        asked: (base.map(str::to_owned), head.map(str::to_owned)),
    })
}

/// The review as markdown: the in-range comments, `outdated` derived, rendered.
///
/// The document is a **view**, never read back: agents read the review with
/// `rv comments --json` and reply with `rv reply`, so rendering carries no
/// ingest step and no ordering rule. One function for `rv render` and the
/// TUI's `e`, so the page a key produces and the page a command produces are
/// the same page.
pub fn render_markdown(review: &Review) -> Result<String> {
    let comments = review
        .store
        .comments()
        .context("could not read the review's comments")?;
    // The same view everywhere: the export lists what `rv status` counts and
    // the TUI shows. Rendering the whole store made the worker's gate
    // (`comments.open`) and its work list disagree — an out-of-range comment
    // sat in `## Open` while status said there was no work. The store keeps
    // every comment; a wider range renders them again.
    let mut comments = in_range(review, comments);
    // Every load derives `outdated` — see [`crate::stale::mark_outdated`]. Doing
    // it in `rv status` and not here had the two commands report different states
    // for one review, which is worse than either being wrong.
    crate::stale::mark_outdated(review, &mut comments);
    Ok(markdown::render(&review.session, &comments))
}

/// Renders and writes `.review/REVIEW-FEEDBACK.md`.
///
/// Only two callers remain — `rv render --out` and the TUI's `e` — because the
/// file stopped being refreshed as a side effect of saving, settling or
/// replying: nothing reads it back, so a file nothing reads cannot be
/// dangerously stale. The write itself is atomic, so a program reading the
/// document while `rv` runs never sees half of one.
pub fn write_markdown(review: &Review) -> Result<()> {
    let document = render_markdown(review)?;
    review
        .store
        .write_markdown(&document)
        .with_context(|| format!("could not write {}", review.store.markdown_path().display()))
}

/// The migration (CLI-loop spec §5): folds a `**Reply:**` block a pre-amendment
/// agent wrote into `REVIEW-FEEDBACK.md` back into the stored comment, once.
///
/// This is the parser's one remaining caller, kept for exactly one release so
/// that a reply sitting in an old export when this version lands is rescued
/// rather than orphaned. The rules are the old ingest's, narrowed:
///
/// - Only a comment that has **no stored reply** takes one from the document —
///   the store is the authority, and the CLI is its only writer now.
/// - A reply whose id matches no stored comment is ignored.
/// - No state transitions, and the export itself is not modified: it goes
///   stale harmlessly until the next explicit `rv render` replaces it.
///
/// A missing document is not an error: there is nothing to rescue.
fn rescue_replies(store: &Store) -> Result<()> {
    let path = store.markdown_path();
    let document = match fs::read_to_string(&path) {
        Ok(document) => document,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(error).with_context(|| format!("could not read {}", path.display()));
        }
    };

    let mut comments = store
        .comments()
        .context("could not read the review's comments")?;
    for (id, reply) in markdown::parse_replies(&document) {
        let Some(comment) = comments.iter_mut().find(|comment| comment.id == id) else {
            continue;
        };
        if comment.reply.is_some() {
            continue;
        }
        comment.reply = Some(reply);
        store
            .append_comment(comment)
            .with_context(|| format!("could not store the rescued reply to comment {id}"))?;
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

/// The comments this review can show: the ones anchored to a file it covers.
///
/// `.review/` outlives any one range. A comment written against `trunk()..@`
/// last week may be anchored to a file the range open now does not touch, and
/// listing it offers the reviewer a jump that cannot land — which is exactly
/// what it used to do: the browser showed the row and `Enter` answered with an
/// alert saying the file had left the range.
///
/// Filtered once, where the store is read, rather than in the browser: the count
/// in the bar, the rows in the sidebar and the boxes in the diff then all
/// describe the same set of comments. **The store keeps every one of them** —
/// nothing is deleted and the export still carries them — so a comment hidden
/// by a narrow range comes back with a wider one.
///
/// Either side's path matches, because a comment on a removed line is filed
/// under the base-side path, which for a rename is not the path the file is
/// listed under.
pub fn in_range(review: &Review, comments: Vec<Comment>) -> Vec<Comment> {
    comments
        .into_iter()
        .filter(|comment| {
            review.files.iter().any(|file| {
                file.path == comment.anchor.file
                    || file.source_path.as_deref() == Some(comment.anchor.file.as_str())
            })
        })
        .collect()
}
