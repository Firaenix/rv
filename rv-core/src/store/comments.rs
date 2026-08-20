//! The comment half of `session.toml`, and the migration that puts a v1.0.0
//! `comments.json` there.
//!
//! Every write goes through [`write_atomic`](super::write_atomic) — read the
//! whole review, edit the `comments` array, write the whole review back — so
//! a crash mid-write leaves the previous complete file rather than a truncated
//! mix, and the scope a comment was made against never disagrees with the
//! comment. One file means there is no cross-file ordering rule left to get
//! right.

use std::fs;
use std::io::ErrorKind;

use super::Comment;
use super::CommentState;
use super::Error;
use super::SettledBy;
use super::Store;

impl Store {
    /// The comments in `session.toml`, or an empty `Vec` if no review has been
    /// recorded yet (a session with no comments has nothing to read, not an
    /// error).
    pub fn comments(&self) -> Result<Vec<Comment>, Error> {
        Ok(self.read_review()?.comments)
    }

    /// Persists `comment`: upserts it by `id` into `session.toml`'s
    /// `[[comments]]` array (an existing entry with the same id is updated in
    /// place, keeping its position; a new id is appended). The write goes
    /// through [`write_atomic`](super::write_atomic) and completes before this
    /// returns — there is no buffering, so a crash right after this call
    /// cannot lose the comment.
    ///
    /// `id` is the only identity here. `change_id` deliberately does *not*
    /// participate: every comment a reviewer leaves during one session
    /// against the same change shares that change's id, so keying the upsert
    /// on `change_id` would cap the store at one comment per change and let
    /// each new note silently overwrite the previous one.
    pub fn append_comment(&self, comment: &Comment) -> Result<(), Error> {
        let mut review = self.read_review()?;
        match review
            .comments
            .iter_mut()
            .find(|existing| existing.id == comment.id)
        {
            Some(existing) => *existing = comment.clone(),
            None => review.comments.push(comment.clone()),
        }
        self.write_review(&review)
    }

    /// Moves the comment with `id` to `state`, recording who did it, and
    /// returns whether one was there.
    ///
    /// An unknown id is not an error, for the same reason it is not one in
    /// [`Store::remove_comment`]: settling twice must be safe.
    pub fn settle_comment(
        &self,
        id: &str,
        state: CommentState,
        by: SettledBy,
    ) -> Result<bool, Error> {
        let mut review = self.read_review()?;
        let Some(comment) = review
            .comments
            .iter_mut()
            .find(|existing| existing.id == id)
        else {
            return Ok(false);
        };
        comment.state = state;
        // `Open` is nobody's doing — it is where a comment starts and where
        // un-settling returns it — so the actor is cleared rather than left
        // pointing at whoever last settled it.
        comment.settled_by = (state != CommentState::Open).then_some(by);

        self.write_review(&review)?;
        Ok(true)
    }

    /// Removes the comment with `id`, returning whether one was there.
    ///
    /// An unknown id is not an error, so deleting is idempotent: the retry after
    /// an interrupted delete finds the entry already gone and succeeds.
    pub fn remove_comment(&self, id: &str) -> Result<bool, Error> {
        let mut review = self.read_review()?;
        let before = review.comments.len();
        review.comments.retain(|existing| existing.id != id);
        if review.comments.len() == before {
            return Ok(false);
        }
        self.write_review(&review)?;
        Ok(true)
    }

    /// Folds a v1.0.0 `comments.json` into `session.toml` and then deletes it
    /// (storage spec §6).
    ///
    /// # Why this order cannot lose a comment
    ///
    /// The two steps are a `write_atomic` of `session.toml` followed by an
    /// `unlink` of `comments.json`, and every point between them is safe:
    ///
    /// - Killed *before* the rename: `session.toml` is untouched and
    ///   `comments.json` is still there, so the next open migrates again.
    /// - Killed *between* rename and unlink: both files hold the comments. The
    ///   next open re-reads the JSON and upserts by id onto a `session.toml`
    ///   that already has them, which is idempotent, then deletes the JSON.
    /// - Killed *after* the unlink: `session.toml` is the review, and there is
    ///   nothing left to migrate.
    ///
    /// The one order that *could* lose comments — unlink first — is the one
    /// not written. The failure the user can never be left with is having
    /// neither file, and no interleaving here produces it.
    ///
    /// A stored comment wins over its legacy twin: `session.toml` is what
    /// every write since the migration went to, so a `comments.json` left
    /// behind by a half-finished migration cannot roll a reply back.
    ///
    /// Unparseable JSON is an error rather than a shrug. It is the user's
    /// review, and quietly stepping over a file this tool wrote is how a
    /// reviewer loses a day's comments without being told.
    pub(super) fn absorb_legacy_comments(&self) -> Result<(), Error> {
        let path = self.legacy_comments_path();
        let contents = match fs::read_to_string(&path) {
            Ok(contents) => contents,
            Err(source) if source.kind() == ErrorKind::NotFound => return Ok(()),
            Err(source) => return Err(Error::Io { path, source }),
        };
        let legacy: Vec<Comment> = serde_json::from_str(&contents)
            .map_err(|source| Error::InvalidComments { path, source })?;

        let mut review = self.read_review()?;
        for comment in legacy {
            if !review.comments.iter().any(|stored| stored.id == comment.id) {
                review.comments.push(comment);
            }
        }
        self.write_review(&review)?;

        let path = self.legacy_comments_path();
        match fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(source) if source.kind() == ErrorKind::NotFound => Ok(()),
            Err(source) => Err(Error::Io { path, source }),
        }
    }
}
