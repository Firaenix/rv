//! Terminal-free core of `rv`: jj access, diff production, comment anchoring and
//! `.review/` I/O. Nothing here may touch a terminal, and `jj_lib` is confined to
//! [`vcs`] so that a jj-lib upgrade has a one-file blast radius.

pub mod anchor;
pub mod diff;
pub mod highlight;
pub mod markdown;
pub mod model;
pub mod store;
pub mod vcs;
