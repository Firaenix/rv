//! jj-lib's errors, turned into [`Error`] and rendered with their cause chains.

use std::path::Path;

use jj_lib::repo::StoreLoadError;
use jj_lib::workspace::WorkspaceLoadError;

/// The jj-lib version `rv` is built against. Reported when a repository turns out
/// to be written in a format this build cannot read.
pub const LINKED_JJ_LIB: &str = "0.44";

/// Errors from talking to jj.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("no jj workspace at {0} (run `jj git init --colocate` there?)")]
    NotAWorkspace(String),
    #[error(
        "this repository is not readable by jj-lib {linked}, which rv is built against: \
         {source_message}"
    )]
    Incompatible {
        linked: String,
        source_message: String,
    },
    #[error("unresolved revset: {0}")]
    Unresolved(String),
    #[error("revset {base}..{head} is empty: there is nothing to review")]
    EmptyRange { base: String, head: String },
    #[error("jj: {0}")]
    Jj(String),
}

pub(super) fn load_error(path: &Path, error: WorkspaceLoadError) -> Error {
    match &error {
        WorkspaceLoadError::NoWorkspaceHere(at) | WorkspaceLoadError::RepoDoesNotExist(at) => {
            Error::NotAWorkspace(at.display().to_string())
        }
        // An unknown backend or op-store type means the repo was written by a jj
        // that knows a format this build does not.
        WorkspaceLoadError::StoreLoadError(StoreLoadError::UnsupportedType { .. }) => {
            Error::Incompatible {
                linked: LINKED_JJ_LIB.to_owned(),
                source_message: chain(&error),
            }
        }
        _ => Error::Jj(format!("{}: {}", path.display(), chain(&error))),
    }
}

/// Renders an error together with its `source` chain: jj-lib's outer messages are
/// often generic ("Cannot read the repo") and the cause carries the detail.
pub(super) fn chain(error: &dyn std::error::Error) -> String {
    let mut message = error.to_string();
    let mut previous = message.clone();
    let mut source = error.source();
    while let Some(cause) = source {
        let text = cause.to_string();
        // Skip `#[error(transparent)]` links, which repeat their cause verbatim.
        if text != previous {
            message.push_str(": ");
            message.push_str(&text);
        }
        previous = text;
        source = cause.source();
    }
    message
}
