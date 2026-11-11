//! Language-server navigation: go-to-definition, references and hover from an
//! external server over stdio. Positions in this API are rv's: 1-based lines
//! and 0-based *character* columns; conversion to LSP's 0-based UTF-16
//! positions happens inside. Servers read files from disk, so answers are only
//! meaningful for the working copy.

mod answers;
mod cache;
mod client;
mod discovery;
mod driver;
mod framing;
mod protocol;
mod servers;
mod uri;

use std::path::PathBuf;

pub use cache::{AnswerCache, CacheKey};
pub use client::{LspHub, ServerSummary, Ticket};
pub use discovery::{Discovery, SearchPath};
pub use framing::{FrameError, read_frame, write_frame};
pub use protocol::{char_column_to_utf16, utf16_to_char_column};
pub use servers::{Language, ServerRegistry, find_program, language_for_path, language_names};
pub use uri::{path_to_uri, uri_to_path};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum RequestKind {
    Definition,
    References,
    Hover,
}

/// `path` is relative to the repo root when the target is inside it, and
/// absolute otherwise (a standard-library or dependency source, say).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Location {
    pub path: PathBuf,
    pub line: u32,
    pub column: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HoverFormat {
    Markdown,
    PlainText,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Hover {
    pub format: HoverFormat,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Answer {
    Locations(Vec<Location>),
    Hover(Option<Hover>),
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum LspError {
    #[error("language server unavailable: {0}")]
    Unavailable(String),
    #[error("language server error {code}: {message}")]
    Server { code: i64, message: String },
    #[error("unexpected reply from language server: {0}")]
    Malformed(String),
    #[error("cannot read {path}: {reason}")]
    Unreadable { path: PathBuf, reason: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ServerStatus {
    /// Configured but not spawned yet: servers start on first use.
    Idle,
    Starting,
    Indexing {
        title: String,
        percentage: Option<u32>,
    },
    Ready,
    Unavailable(String),
}
