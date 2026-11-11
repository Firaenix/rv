//! End-to-end tests for the `rv` binary: they run the real executable against a
//! throwaway jj workspace and inspect what it wrote to disk and to its streams.
//!
//! The cases are grouped by the command surface they cover; `support/` holds the
//! workspace fixture and the assertion helpers they all share.

mod support;

#[cfg(unix)]
mod agent;
mod attention;
mod brief;
mod browse;
mod comments;
mod completions;
#[cfg(unix)]
mod lsp;
mod output;
mod session;
mod status;
