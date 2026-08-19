//! Tests for the review TUI's state machine and its one frame of output.
//!
//! [`rv::app::App::on_key`] is deliberately terminal-free, so everything under
//! `tests/app/` drives the reviewer the way a user does — one `KeyCode` at a
//! time — and then inspects the `.review/` store through a *fresh* `Store`,
//! never the app's own handle. That is what makes these tests about
//! persistence rather than about in-memory bookkeeping.
//!
//! One binary, many modules: each topic below is its own file so none of them
//! grows past reading size, and `tests/support/` holds the fixtures they share.

#[path = "../support/mod.rs"]
mod support;

mod abort;
mod alerts;
mod bar;
mod border;
mod boxes_1;
mod boxes_2;
mod browser_1;
mod browser_2;
mod clipping;
mod collapse;
mod comment_1;
mod comment_2;
mod commits;
mod cursor;
mod delete;
mod export;
mod fold;
mod hscroll;
mod keymap;
mod mouse_1;
mod mouse_2;
mod panes;
mod popup;
mod resize;
mod settle;
mod stack;
mod symbols;
mod syntax_1;
mod syntax_2;
mod tree_1;
mod tree_2;
mod zoom;
