//! End-to-end tests for the `rv` binary: they run the real executable against a
//! throwaway jj workspace and inspect what it wrote to disk and to its streams.
//!
//! The cases are grouped by the command surface they cover; `support/` holds the
//! workspace fixture and the assertion helpers they all share.

mod support;

mod comments;
mod output;
mod session;
mod status;
