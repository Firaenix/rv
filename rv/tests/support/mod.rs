//! Shared fixtures and frame helpers for the `rv` TUI integration tests.
//!
//! Compiled once per test binary; `dead_code` is allowed because no single
//! binary uses all of it.

#![allow(dead_code)]

pub mod diffpane;
pub mod fixture;
pub mod frame;
pub mod keymaps;
pub mod keys;
pub mod markdown;
pub mod mouse;
pub mod pane;
pub mod rows;

pub use diffpane::*;
pub use fixture::*;
pub use frame::*;
pub use keymaps::*;
pub use keys::*;
pub use markdown::*;
pub use mouse::*;
pub use pane::*;
pub use rows::*;
