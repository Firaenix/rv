//! Parameterized cases and properties for [`rv::app`] and [`rv::ui`].
//!
//! Additive to `rv/tests/app.rs`, which pins a handful of exact end-to-end
//! behaviours. This file covers the *contract*: the documented keybinding
//! tables (cross-checked against README's two tables), the state invariants
//! that must survive any key sequence at all, the byte-identity of a comment
//! from keystrokes to `comments.json` and back out through the markdown
//! export, and the total-ness of [`rv::ui::draw`] at every terminal size.
//!
//! # Why the fixtures are shared, and how cases stay independent
//!
//! Building a fixture costs ~200 ms of `jj` process time and `App::new` costs
//! ~30 ms of `difft`, so a fresh `App` per proptest case would put this file
//! well over its wall-clock budget. Instead each property owns **one** `App`
//! and drives the proptest runner by hand (see [`run_cases`]), calling
//! [`rewind`] before every case. `rewind` uses only the public keyboard API —
//! `Esc`, then `[` off the left edge, then `k` off the top — so it lands on the
//! same state a fresh `App` starts in (file 0, line 0, `Browse`, empty
//! buffer), and clears `.review/comments.json` where the store matters. That
//! keeps shrinking sound: a shrunk key sequence replays from the same state
//! the failing one did.
//!
//! Fixtures that no test in this file writes comments into are shared through
//! a `OnceLock` across the whole binary; anything that saves a comment gets a
//! fixture of its own, because integration tests run in parallel threads and
//! `.review/` is process-wide state.
//!
//! # Which diff engine each fixture is reviewed through
//!
//! Most fixtures go through difftastic, and the properties whose oracles depend
//! on difftastic's *pairing* of a rewritten line with its counterpart say so out
//! loud with [`assert_difftastic`] — `rv_core::diff::compute` degrades to
//! `similar` when `difft` is missing or `RV_NO_DIFFT` is exported, and every
//! fixture guard in this file survives that swap, so without the assertion the
//! suite would report green while covering different branches than every doc
//! comment here describes.
//!
//! [`Fixture::fallback`] is the deliberate other side of that: it is reviewed
//! through [`App::with_fallback_diffs`], which is the only way this file
//! produces a [`LineKind::Context`] line or a `DiffSource::Similar` label —
//! the diff every user with no `difft` on `PATH` actually sees. Per-`App`
//! rather than by setting `RV_NO_DIFFT`, which is process-wide and would swap
//! the engine under the other tests running in parallel threads.

mod support;

mod alerts;
mod collapsing;
mod deleting;
mod empty;
mod fallback;
mod invariants;
mod jumping;
mod mouse;
mod navigation;
mod rendering;
mod saving_1;
mod saving_2;
mod suppressed;
mod tables_1;
mod tables_2;
mod typing;
