//! Writing `REVIEW-FEEDBACK.md` from inside the reviewer.
//!
//! Through the very rendering `rv render` uses, so the document produced by the
//! key and the document produced by the command are the same document. The file
//! is a **view**: nothing reads it back, so `e` is the artefact-on-request key
//! and the store is never touched by it (CLI-loop spec §2).

use anyhow::Result;

use super::App;
use crate::session;

impl App {
    /// Exports the review, and says where it landed.
    pub(super) fn export(&mut self) -> Result<()> {
        session::write_markdown(&self.review)?;

        let path = self.review.store.markdown_path();
        // The file name rather than the whole path: the status bar is one row,
        // and a reviewer knows which review they are in.
        let name = path.file_name().map_or_else(
            || path.display().to_string(),
            |name| name.to_string_lossy().into_owned(),
        );
        self.status = format!("wrote {name}");
        Ok(())
    }
}
