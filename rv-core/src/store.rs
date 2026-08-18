//! The `.review/` on-disk store (spec §10): plain filesystem I/O, no jj-lib,
//! no terminal.
//!
//! `.review/` sits at the repo root, alongside `.jj/` and `.git/`. jj
//! snapshots the whole working copy on every command, so leaving `.review/`
//! untracked is correctness, not hygiene: [`Store::ensure_excluded`] appends
//! it to `.git/info/exclude` (never `.gitignore`, which is shared and would
//! affect every clone) so that writing review notes never mutates the change
//! under review.
//!
//! [`Store::append_comment`] is write-through: it persists to
//! `.review/snapshots/<id>` and then `.review/comments.json` before
//! returning, with no in-memory cache in front of either file. Every write
//! this module makes — those two, plus `session.toml`,
//! `REVIEW-FEEDBACK.md` and the `.git/info/exclude` update — goes through
//! [`write_atomic`]: new content
//! is written to a fresh temp file in the destination's own directory,
//! fsynced, then renamed into place. `rename` on POSIX either completes
//! wholly or not at all, so a reader can never observe a half-written file;
//! a crash mid-write leaves the *previous* complete contents exactly as they
//! were, never a truncated or corrupted mix of old and new. `comments.json`
//! is the authority on which comments exist. Because its snapshot is
//! written first, a crash between the two writes can strand an orphaned
//! snapshot file with no matching entry in `comments.json` (harmless —
//! nothing looks a snapshot up except by an id already found in
//! `comments.json`), but never the reverse: a comment recorded in
//! `comments.json` whose snapshot was never written.
//! [`Store::remove_comment`] runs the same ordering backwards — entry out of
//! `comments.json` first, snapshot file deleted second — to preserve that
//! same one-sided invariant while tearing a comment down.
//!
//! On-disk formats are chosen to be readable by a human poking around
//! `.review/`, not just by `rv` itself: `comments.json` is pretty-printed,
//! `session.toml` uses the `toml` crate, and [`CommentState`] serializes in
//! kebab-case (`"awaiting-verification"`, not `"AwaitingVerification"` or
//! `"awaiting_verification"`) to match the vocabulary the markdown export
//! (a later task) uses.

use std::fs;
use std::io::ErrorKind;
use std::io::Write as _;
use std::path::Path;
use std::path::PathBuf;

use serde::Deserialize;
use serde::Serialize;
use tempfile::Builder;

use crate::model::Anchor;
use crate::model::ChangeRef;

/// The line [`Store::ensure_excluded`] appends to `.git/info/exclude`.
const EXCLUDE_LINE: &str = "/.review/";

/// Prefix on the temp file [`write_atomic`] creates before renaming it into
/// place, so a leftover (only possible if the process is killed between the
/// `fsync` and the `rename` — every other early return drops and so deletes
/// the [`tempfile::NamedTempFile`]) is easy to recognize as `rv`'s.
const ATOMIC_TEMP_PREFIX: &str = ".rv-store-";

/// Errors from reading or writing `.review/`.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("io error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("{path} is not valid comments.json: {source}")]
    InvalidComments {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("{path} is not valid session.toml: {source}")]
    InvalidSession {
        path: PathBuf,
        #[source]
        source: Box<toml::de::Error>,
    },
    #[error("could not serialize comments.json: {0}")]
    SerializeComments(#[source] serde_json::Error),
    #[error("could not serialize session.toml: {0}")]
    SerializeSession(#[source] Box<toml::ser::Error>),
}

/// A reviewer's note on one [`Anchor`] location.
///
/// `change_id` and `commit_id` echo the change the comment was made against
/// (see the identity/advisory distinction on [`ChangeRef`]); `reply` is the
/// author's response to review feedback, filled in later than `body`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Comment {
    pub id: String,
    pub change_id: String,
    pub commit_id: String,
    pub anchor: Anchor,
    pub body: String,
    pub state: CommentState,
    pub reply: Option<String>,
    /// Who moved the comment out of `Open`, where anybody has.
    ///
    /// Defaulted on read so that a `.review/` written before settling existed
    /// still loads.
    #[serde(default)]
    pub settled_by: Option<SettledBy>,
}

/// A comment's place in the review lifecycle.
///
/// Serializes in kebab-case: `Open` as `"open"`, `AwaitingVerification` as
/// `"awaiting-verification"`, and so on, matching the markdown vocabulary the
/// export task uses.
///
/// `Resolved` and `Abandoned` are separate states rather than one "dismissed"
/// because they record two different facts about a review — *this was fixed*
/// and *this was dropped without being fixed* — and a count that adds them
/// together misreports what the review concluded (storage spec §3).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CommentState {
    Open,
    AwaitingVerification,
    Resolved,
    Abandoned,
    Outdated,
}

/// Who settled a comment.
///
/// Stored, and shown, rather than forbidden: an agent may resolve or abandon,
/// but the file and the screen always say it was the agent. Hiding the
/// distinction is the actual danger; forbidding the action only pushes it into
/// prose nobody reads (storage spec §3).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SettledBy {
    User,
    Agent,
}

/// The revset and stack a review session covers.
///
/// `started_at` is opaque to this module — later tasks fill it with
/// `"epoch:<unix_secs>"` — so it is stored and round-tripped as a plain
/// `String` rather than a parsed timestamp type.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Session {
    pub revset: String,
    pub base_commit: String,
    pub head_commit: String,
    pub changes: Vec<ChangeRef>,
    pub started_at: String,
}

/// A handle on the `.review/` directory under a repo root.
///
/// Holds no cached state: every method reads or writes the filesystem
/// directly, which is what makes [`Store::append_comment`] write-through by
/// construction rather than by extra bookkeeping.
pub struct Store {
    root: PathBuf,
}

impl Store {
    /// Opens the store rooted at `root` (the repo root, holding `.jj/` and
    /// `.git/`), creating `.review/snapshots` (and so also `.review/` itself)
    /// if it does not already exist.
    pub fn open(root: &Path) -> Result<Self, Error> {
        let store = Self {
            root: root.to_owned(),
        };
        let snapshots_dir = store.snapshots_dir();
        fs::create_dir_all(&snapshots_dir).map_err(|source| Error::Io {
            path: snapshots_dir,
            source,
        })?;
        Ok(store)
    }

    /// Appends [`EXCLUDE_LINE`] to `.git/info/exclude` unless it is already
    /// there, creating `.git/info/` and the `exclude` file itself if either
    /// is missing. Returns `true` if it added the line, `false` if the line
    /// was already present (existing lines, including other tools' entries,
    /// are left untouched either way).
    pub fn ensure_excluded(&self) -> Result<bool, Error> {
        let path = self.exclude_path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|source| Error::Io {
                path: parent.to_owned(),
                source,
            })?;
        }

        let existing = match fs::read_to_string(&path) {
            Ok(contents) => contents,
            Err(source) if source.kind() == ErrorKind::NotFound => String::new(),
            Err(source) => {
                return Err(Error::Io {
                    path: path.clone(),
                    source,
                });
            }
        };
        if existing.lines().any(|line| line == EXCLUDE_LINE) {
            return Ok(false);
        }

        let mut updated = existing;
        if !updated.is_empty() && !updated.ends_with('\n') {
            updated.push('\n');
        }
        updated.push_str(EXCLUDE_LINE);
        updated.push('\n');
        write_atomic(&path, updated.as_bytes())?;
        Ok(true)
    }

    /// Overwrites `session.toml` with `session`.
    pub fn write_session(&self, session: &Session) -> Result<(), Error> {
        let serialized = toml::to_string_pretty(session)
            .map_err(|source| Error::SerializeSession(Box::new(source)))?;
        write_atomic(&self.session_path(), serialized.as_bytes())
    }

    /// Reads and parses `session.toml`.
    pub fn read_session(&self) -> Result<Session, Error> {
        let path = self.session_path();
        let contents = fs::read_to_string(&path).map_err(|source| Error::Io {
            path: path.clone(),
            source,
        })?;
        toml::from_str(&contents).map_err(|source| Error::InvalidSession {
            path,
            source: Box::new(source),
        })
    }

    /// The comments currently in `comments.json`, or an empty `Vec` if the
    /// file does not exist yet (a session with no comments has nothing to
    /// read, not an error).
    pub fn comments(&self) -> Result<Vec<Comment>, Error> {
        let path = self.comments_path();
        let contents = match fs::read_to_string(&path) {
            Ok(contents) => contents,
            Err(source) if source.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
            Err(source) => return Err(Error::Io { path, source }),
        };
        serde_json::from_str(&contents).map_err(|source| Error::InvalidComments { path, source })
    }

    /// Persists `comment`: upserts it by `id` into `comments.json` (an
    /// existing entry with the same id is updated in place, keeping its
    /// position; a new id is appended) and writes its anchor's context lines
    /// verbatim to `.review/snapshots/<id>`. Both writes go through
    /// [`write_atomic`] and complete before this returns — there is no
    /// buffering, so a crash right after this call cannot lose the comment.
    ///
    /// `id` is the only identity here. `change_id` deliberately does *not*
    /// participate: every comment a reviewer leaves during one session
    /// against the same change shares that change's id, so keying the upsert
    /// on `change_id` would cap the store at one comment per change and let
    /// each new note silently overwrite the previous one.
    ///
    /// The snapshot is written first and `comments.json` — the authority on
    /// which comments exist — last, so a crash between the two can only
    /// leave a harmless orphaned snapshot file, never a comment that
    /// `comments.json` claims exists but whose snapshot was never written.
    pub fn append_comment(&self, comment: &Comment) -> Result<(), Error> {
        let mut comments = self.comments()?;
        match comments
            .iter_mut()
            .find(|existing| existing.id == comment.id)
        {
            Some(existing) => *existing = comment.clone(),
            None => comments.push(comment.clone()),
        }
        let serialized =
            serde_json::to_string_pretty(&comments).map_err(Error::SerializeComments)?;

        let snapshot_path = self.snapshots_dir().join(&comment.id);
        let snapshot = comment.anchor.context.join("\n");
        write_atomic(&snapshot_path, snapshot.as_bytes())?;

        write_atomic(&self.comments_path(), serialized.as_bytes())
    }

    /// Moves the comment with `id` to `state`, recording who did it, and
    /// returns whether one was there.
    ///
    /// Settling touches no snapshot: the anchor is unchanged, so the stored
    /// context still describes the code the comment was written against. Only
    /// `comments.json` is rewritten, through the same atomic path every other
    /// write uses.
    ///
    /// An unknown id is not an error, for the same reason it is not one in
    /// [`Store::remove_comment`]: settling twice must be safe.
    pub fn settle_comment(
        &self,
        id: &str,
        state: CommentState,
        by: SettledBy,
    ) -> Result<bool, Error> {
        let mut comments = self.comments()?;
        let Some(comment) = comments.iter_mut().find(|existing| existing.id == id) else {
            return Ok(false);
        };
        comment.state = state;
        // `Open` is nobody's doing — it is where a comment starts and where
        // un-settling returns it — so the actor is cleared rather than left
        // pointing at whoever last settled it.
        comment.settled_by = (state != CommentState::Open).then_some(by);

        let serialized =
            serde_json::to_string_pretty(&comments).map_err(Error::SerializeComments)?;
        write_atomic(&self.comments_path(), serialized.as_bytes())?;
        Ok(true)
    }

    /// Removes the comment with `id`, returning whether one was there.
    ///
    /// `comments.json` is rewritten *before* the snapshot is deleted — the
    /// reverse of [`Store::append_comment`]'s order, holding the same
    /// invariant: at every instant, every comment `comments.json` claims
    /// exists still has its snapshot on disk. A crash between the two strands
    /// an inert snapshot rather than orphaning a live comment.
    ///
    /// An unknown id is not an error, so deleting is idempotent.
    pub fn remove_comment(&self, id: &str) -> Result<bool, Error> {
        let mut comments = self.comments()?;
        let before = comments.len();
        comments.retain(|existing| existing.id != id);
        if comments.len() == before {
            return Ok(false);
        }

        let serialized =
            serde_json::to_string_pretty(&comments).map_err(Error::SerializeComments)?;
        write_atomic(&self.comments_path(), serialized.as_bytes())?;

        let snapshot_path = self.snapshots_dir().join(id);
        match fs::remove_file(&snapshot_path) {
            Ok(()) => Ok(true),
            Err(source) if source.kind() == ErrorKind::NotFound => Ok(true),
            Err(source) => Err(Error::Io {
                path: snapshot_path,
                source,
            }),
        }
    }

    /// Overwrites `REVIEW-FEEDBACK.md` with `document`.
    ///
    /// Atomic like every other file this module writes, which matters more
    /// here than anywhere else: the markdown is the one file another program
    /// reads *while* `rv` is running, and it is rewritten from
    /// `comments.json` after every saved comment. A reader that caught a
    /// half-written document would see an entry without its anchor marker, or
    /// a truncated reply.
    ///
    /// Rendering the document is [`crate::markdown::render`]'s job; this
    /// method only puts the bytes on disk.
    pub fn write_markdown(&self, document: &str) -> Result<(), Error> {
        write_atomic(&self.markdown_path(), document.as_bytes())
    }

    /// Where [`Store::write_markdown`] puts the review feedback document.
    pub fn markdown_path(&self) -> PathBuf {
        self.review_dir().join("REVIEW-FEEDBACK.md")
    }

    fn review_dir(&self) -> PathBuf {
        self.root.join(".review")
    }

    fn snapshots_dir(&self) -> PathBuf {
        self.review_dir().join("snapshots")
    }

    fn comments_path(&self) -> PathBuf {
        self.review_dir().join("comments.json")
    }

    fn session_path(&self) -> PathBuf {
        self.review_dir().join("session.toml")
    }

    fn exclude_path(&self) -> PathBuf {
        self.root.join(".git").join("info").join("exclude")
    }
}

/// Writes `contents` to `path` without ever leaving `path` itself partially
/// written.
///
/// The bytes go to a fresh, uniquely-named temp file created in `path`'s own
/// directory — never a shared temp directory, since `rename` is only atomic
/// when source and destination share a filesystem — and are fsynced there
/// (so the write survives a power loss, not just a killed process) before
/// the temp file is renamed onto `path`. `rename` on POSIX is atomic: any
/// reader of `path` sees either the old complete contents or the new
/// complete contents, never a mix. This function does not additionally
/// fsync `path`'s parent directory after the rename, so it does not
/// guarantee the *directory entry* update itself survives a power loss the
/// instant after `persist` returns — closing that gap needs an extra
/// directory fsync this module skips as unwarranted complexity for a local
/// review scratch directory.
fn write_atomic(path: &Path, contents: &[u8]) -> Result<(), Error> {
    let dir = path.parent().ok_or_else(|| Error::Io {
        path: path.to_owned(),
        source: std::io::Error::new(
            ErrorKind::InvalidInput,
            "path has no parent directory to hold its temp file",
        ),
    })?;

    let mut temp = Builder::new()
        .prefix(ATOMIC_TEMP_PREFIX)
        .suffix(".tmp")
        .tempfile_in(dir)
        .map_err(|source| Error::Io {
            path: dir.to_owned(),
            source,
        })?;
    temp.write_all(contents).map_err(|source| Error::Io {
        path: path.to_owned(),
        source,
    })?;
    temp.as_file().sync_all().map_err(|source| Error::Io {
        path: path.to_owned(),
        source,
    })?;
    temp.persist(path).map_err(|error| Error::Io {
        path: path.to_owned(),
        source: error.error,
    })?;
    Ok(())
}
