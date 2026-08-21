//! Diff production, engine by engine.
//!
//! One binary, three modules: `engines` covers what difftastic and `similar`
//! each produce for a pair of blobs, `probe` covers whether difftastic is
//! consulted at all, and `support` holds what both name.

mod support;

mod context;
mod engines;
mod probe;
