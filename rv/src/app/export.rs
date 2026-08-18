//! Writing `REVIEW-FEEDBACK.md` from inside the reviewer.
//!
//! Through the very function `rv render` calls, so the document produced by the
//! key and the document produced by the command are the same document. Making a
//! reviewer quit to produce the file the whole LLM loop depends on would be a
//! strange place to put a door (storage spec §5).

use anyhow::Result;

use super::App;
use crate::session;

impl App {
    /// Exports the review, and says where it landed.
    ///
    /// The export ingests first: a reply a model appended to the document is
    /// folded back into the store before the document is rebuilt from it, so
    /// pressing `e` cannot delete an answer that was never saved.
    ///
    /// The in-memory comments are re-read afterwards for the same reason — the
    /// ingest may have just given a comment on screen a reply it did not have a
    /// moment ago.
    pub(super) fn export(&mut self) -> Result<()> {
        session::write_markdown(&self.review)?;
        self.reload_comments()?;

        let path = self.review.store.markdown_path();
        // The file name rather than the whole path: the status bar is one row,
        // and a reviewer knows which review they are in.
        let name = path
            .file_name()
            .map_or_else(|| path.display().to_string(), |name| name.to_string_lossy().into_owned());
        self.status = format!("wrote {name}");
        Ok(())
    }
}
