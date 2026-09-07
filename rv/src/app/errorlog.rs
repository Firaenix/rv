//! `tracing` sink at `.review/rv.log` — every `tracing::error!` / `warn!` /
//! `info!` etc. the crate emits ends up here, filtered by `RUST_LOG` (default
//! `error`). The reviewer whose toast has already faded still has the log.
//!
//! Installed once by [`crate::app::App::run`]. Tests build the subscriber
//! through [`subscriber`] and scope it with [`tracing::subscriber::with_default`]
//! so a test's alerts land in the test's own workspace and not in a shared
//! process-global sink.

use std::fs::File;
use std::fs::OpenOptions;
use std::path::Path;

use tracing::Subscriber;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::fmt;

/// Where every alert, warning and panic ends up.
const LOG_FILE: &str = "rv.log";

/// Builds a subscriber that writes to `<root>/.review/rv.log`, filtered by
/// `RUST_LOG` and defaulting to `error`. Returns `None` when the sink cannot
/// be created — a full disk, a read-only workspace — because a log the
/// reviewer cannot write to is not an error rv should refuse to run over.
pub fn subscriber(root: &Path) -> Option<impl Subscriber + Send + Sync + 'static> {
    let dir = root.join(".review");
    std::fs::create_dir_all(&dir).ok()?;
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join(LOG_FILE))
        .ok()?;
    Some(build(file))
}

fn build(file: File) -> impl Subscriber + Send + Sync + 'static {
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("error"));
    fmt::Subscriber::builder()
        .with_writer(file)
        .with_ansi(false)
        .with_target(false)
        .with_env_filter(filter)
        .finish()
}

/// Sets the process-global subscriber for the running TUI, once. A second
/// call is silently a no-op — `tracing`'s slot is set-once — and a workspace
/// with no writable `.review/` is silently a no-op too.
pub fn install(root: &Path) {
    let Some(subscriber) = subscriber(root) else {
        return;
    };
    let _ = tracing::subscriber::set_global_default(subscriber);
}
