//! Reading and rewriting `comments.json`, which is the authority on which
//! comments exist.
//!
//! Every write here goes through [`write_atomic`](super::write_atomic), so a
//! crash mid-write leaves the previous complete file rather than a truncated
//! mix. The one ordering rule is stated on [`Store::remove_comment`]: the
//! entry leaves `comments.json` before its snapshot leaves disk, so at every
//! instant every comment the file claims still has its snapshot.

use std::fs;
use std::io::ErrorKind;

use super::Comment;
use super::CommentState;
use super::Error;
use super::SettledBy;
use super::Store;
use super::write_atomic;

impl Store {
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

}
