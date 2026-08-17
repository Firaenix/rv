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
//! `.review/comments.json` and `.review/snapshots/<id>` before returning, and
//! there is no in-memory cache in front of either file, so a crash mid-review
//! can lose at most the comment currently being written, never one already
//! appended.
//!
//! On-disk formats are chosen to be readable by a human poking around
//! `.review/`, not just by `rv` itself: `comments.json` is pretty-printed,
//! `session.toml` uses the `toml` crate, and [`CommentState`] serializes in
//! kebab-case (`"awaiting-verification"`, not `"AwaitingVerification"` or
//! `"awaiting_verification"`) to match the vocabulary the markdown export
//! (a later task) uses.

use std::fs;
use std::io::ErrorKind;
use std::path::Path;
use std::path::PathBuf;

use serde::Deserialize;
use serde::Serialize;

use crate::model::Anchor;
use crate::model::ChangeRef;

/// The line [`Store::ensure_excluded`] appends to `.git/info/exclude`.
const EXCLUDE_LINE: &str = "/.review/";

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
}

/// A comment's place in the review lifecycle.
///
/// Serializes in kebab-case: `Open` as `"open"`, `AwaitingVerification` as
/// `"awaiting-verification"`, and so on, matching the markdown vocabulary the
/// export task uses.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CommentState {
    Open,
    AwaitingVerification,
    Resolved,
    Outdated,
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
        fs::write(&path, updated).map_err(|source| Error::Io { path, source })?;
        Ok(true)
    }

    /// Overwrites `session.toml` with `session`.
    pub fn write_session(&self, session: &Session) -> Result<(), Error> {
        let serialized = toml::to_string_pretty(session)
            .map_err(|source| Error::SerializeSession(Box::new(source)))?;
        let path = self.session_path();
        fs::write(&path, serialized).map_err(|source| Error::Io { path, source })
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
    /// verbatim to `.review/snapshots/<id>`. Both writes complete before this
    /// returns — there is no buffering, so a crash right after this call
    /// cannot lose the comment.
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
        let comments_path = self.comments_path();
        fs::write(&comments_path, serialized).map_err(|source| Error::Io {
            path: comments_path,
            source,
        })?;

        let snapshot_path = self.snapshots_dir().join(&comment.id);
        let snapshot = comment.anchor.context.join("\n");
        fs::write(&snapshot_path, snapshot).map_err(|source| Error::Io {
            path: snapshot_path,
            source,
        })
    }

    /// Where the markdown export (a later task) writes review feedback.
    /// This module never writes the file itself.
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
