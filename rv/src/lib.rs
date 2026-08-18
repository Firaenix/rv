//! `rv` — the CLI and TUI front end over [`rv_core`].
//!
//! [`session`] assembles a review from a repository and a revision range;
//! [`app`] and [`ui`] hold the terminal front end, over the terminal-free row
//! model in [`rows`]. Everything reviewable lives in `rv-core`, which knows
//! nothing about terminals.

pub mod app;
pub mod rows;
pub mod session;
pub mod ui;
