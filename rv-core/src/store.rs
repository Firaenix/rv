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
//! **`session.toml` is the one file `rv` maintains** (storage spec §2): the
//! range under review, the changes in it, and the comments, as a
//! `[[comments]]` array on [`Session`]. [`Store::append_comment`] is
//! write-through — it persists before returning, with no in-memory cache in
//! front of it — and every write this module makes goes through
//! [`write_atomic`]: new content to a fresh temp file in the destination's
//! own directory, fsynced, then renamed into place. `rename` on POSIX either
//! completes wholly or not at all, so a reader can never observe a
//! half-written file and a crash mid-write leaves the *previous* complete
//! contents exactly as they were. One file means no cross-file ordering rule
//! to get right: a comment and the scope it was made against are updated by
//! the same rename or by neither.
//!
//! A `.review/` written by v1.0.0 has its comments in a sibling
//! `comments.json`. [`Store::open`] folds those into `session.toml` and then
//! removes the JSON file; see [`Store::absorb_legacy_comments`] for why that
//! order cannot lose a comment.
//!
//! On-disk format is chosen to be readable and hand-fixable by a human poking
//! around `.review/`, not just by `rv` itself: `session.toml` goes through the
//! `toml` crate, and [`CommentState`] serializes in kebab-case
//! (`"awaiting-verification"`, not `"AwaitingVerification"`) to match the
//! vocabulary the markdown view uses.

use std::fs;
use std::io::ErrorKind;
use std::io::Write as _;
use std::path::Path;
use std::path::PathBuf;

mod comments;

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
    #[error("{path} is not valid v1.0.0 comments.json: {source}")]
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

/// The whole of `session.toml`: the revset and stack a review covers, and the
/// comments made against it (storage spec §2).
///
/// `started_at` is opaque to this module — later tasks fill it with
/// `"epoch:<unix_secs>"` — so it is stored and round-tripped as a plain
/// `String` rather than a parsed timestamp type.
///
/// `comments` is the whole comment list, and [`Store::write_review`] replaces
/// it wholesale. A caller updating only the scope must therefore carry the
/// comments it read across, or write them away.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Session {
    pub revset: String,
    pub base_commit: String,
    pub head_commit: String,
    pub changes: Vec<ChangeRef>,
    pub started_at: String,
    /// Defaulted on read so that a `session.toml` written before the
    /// consolidation — or hand-trimmed to its scope — still loads.
    #[serde(default)]
    pub comments: Vec<Comment>,
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
    /// `.git/`), creating `.review/` if it does not already exist and folding
    /// a v1.0.0 `comments.json` into `session.toml` if one is there.
    pub fn open(root: &Path) -> Result<Self, Error> {
        let store = Self {
            root: root.to_owned(),
        };
        let review_dir = store.review_dir();
        fs::create_dir_all(&review_dir).map_err(|source| Error::Io {
            path: review_dir,
            source,
        })?;
        store.absorb_legacy_comments()?;
        Ok(store)
    }

    /// The repo root this store was opened at.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
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

    /// Overwrites `session.toml` with `review`: scope and comments together,
    /// in one atomic rename.
    pub fn write_review(&self, review: &Session) -> Result<(), Error> {
        let serialized = toml::to_string_pretty(review)
            .map_err(|source| Error::SerializeSession(Box::new(source)))?;
        write_atomic(&self.session_path(), serialized.as_bytes())
    }

    /// Reads and parses `session.toml`.
    ///
    /// A `.review/` with no `session.toml` reads as [`Session::default`]:
    /// [`Store::open`] creates the directory and writes nothing, so a review
    /// nobody has recorded yet is empty rather than an error. Every read of
    /// the store goes through here, so there is one parse and one absence
    /// rule.
    pub fn read_review(&self) -> Result<Session, Error> {
        let path = self.session_path();
        let contents = match fs::read_to_string(&path) {
            Ok(contents) => contents,
            Err(source) if source.kind() == ErrorKind::NotFound => {
                return Ok(Session::default());
            }
            Err(source) => return Err(Error::Io { path, source }),
        };
        toml::from_str(&contents).map_err(|source| Error::InvalidSession {
            path,
            source: Box::new(source),
        })
    }

    /// Overwrites `REVIEW-FEEDBACK.md` with `document`.
    ///
    /// Atomic like every other file this module writes. The document is a
    /// **view**, written only on request (`rv render --out`, the TUI's `e`)
    /// and read back by nothing — but a human may still be reading it in a
    /// pager while `rv` rewrites it, and a half-written file is a bad page
    /// whoever the reader is.
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

    /// Where a v1.0.0 `.review/` kept its comments, read once by
    /// [`Store::absorb_legacy_comments`] and then deleted.
    pub(super) fn legacy_comments_path(&self) -> PathBuf {
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
pub(crate) fn write_atomic(path: &Path, contents: &[u8]) -> Result<(), Error> {
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
