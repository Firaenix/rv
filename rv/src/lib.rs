//! `rv` — the CLI and TUI front end over [`rv_core`].
//!
//! [`layout`] computes every rectangle both painting and hit-testing use, which
//! is what keeps a click from landing somewhere other than what was drawn.
//! Everything reviewable lives in `rv-core`, which knows nothing about
//! terminals.

pub mod app;
pub mod gradient;
pub mod index;
pub mod layout;
pub mod rows;
pub mod session;
pub mod stale;
pub mod statusbar;
pub mod tree;
pub mod ui;
