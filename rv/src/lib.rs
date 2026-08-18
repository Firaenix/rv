//! `rv` — the CLI and TUI front end over [`rv_core`].
//!
//! [`session`] assembles a review from a repository and a revision range;
//! [`app`] and [`ui`] hold the terminal front end, over the terminal-free row
//! model in [`rows`]. [`layout`] computes every rectangle both painting and
//! hit-testing use, [`tree`] shapes the sidebar's rows and [`gradient`] holds
//! the palette. Everything reviewable lives in `rv-core`, which knows nothing
//! about terminals.

pub mod app;
pub mod gradient;
pub mod layout;
pub mod rows;
pub mod session;
pub mod tree;
pub mod ui;
