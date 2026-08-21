//! Diff production: difftastic's structural, syntax-aware diff with a plain
//! line-based fallback from the `similar` crate.
//!
//! This module takes plain byte slices in and produces a [`FileDiff`] out — no
//! `jj_lib` type crosses this boundary, so it can be exercised without a
//! repository at all.
//!
//! [`compute`] and [`compute_with`] never panic and never return an error:
//! anything that goes wrong — no `difft` on `PATH`, a difftastic too old for
//! the JSON shape [`difftastic::parse`] reads, output that is not that shape,
//! non-UTF-8 content — degrades to the `similar` fallback rather than failing
//! the caller. Each of those is a *different* reason, and the [`FallbackReason`]
//! on the resulting [`DiffSource::Similar`] says which: a reviewer looking at a
//! degraded diff is owed the difference between "difftastic was never asked"
//! and "difftastic is installed but this rv cannot read it".

mod context;
mod difftastic;
mod fallback;
mod model;
mod ordering;
mod probe;

pub use context::merge as merge_context;

pub use model::DiffLine;
pub use model::DiffSource;
pub use model::DifftVerdict;
pub use model::DifftVersion;
pub use model::FallbackReason;
pub use model::FileDiff;
pub use model::LineKind;
pub use probe::MINIMUM_DIFFT;

/// Set to force the `similar` fallback, bypassing difftastic even when it is
/// on `PATH`. Read only by `compute`; [`compute_with`] takes the choice as an
/// explicit argument so it never touches the environment or a process.
const RV_NO_DIFFT: &str = "RV_NO_DIFFT";

/// Computes the diff between `old` and `new`, trying difftastic unless
/// `RV_NO_DIFFT` is set in the environment, and falling back to `similar`
/// otherwise. `old`/`new` of `None` mean the file does not exist on that
/// side (a whole-file add or remove).
pub fn compute(old: Option<&[u8]>, new: Option<&[u8]>, path: &str) -> FileDiff {
    let use_difft = std::env::var_os(RV_NO_DIFFT).is_none();
    compute_with(old, new, path, use_difft)
}

/// As [`compute`], but with explicit control over whether difftastic is
/// attempted. Passing `false` never spawns a process — not even the version
/// probe — and never reads the environment, which is what keeps
/// fallback-focused tests hermetic.
pub fn compute_with(
    old: Option<&[u8]>,
    new: Option<&[u8]>,
    path: &str,
    use_difft: bool,
) -> FileDiff {
    if !use_difft {
        return binary_or(old, new, path, FallbackReason::NotAttempted);
    }
    // Before the probe, not after: a binary file is never handed to difftastic
    // whatever version it is, so a review of nothing but binaries should not
    // fork to ask about a tool it will not run.
    if is_binary(old) || is_binary(new) {
        return binary(path);
    }
    compute_with_verdict(old, new, path, difft_verdict())
}

/// As [`compute_with`], but told outright what to make of the difftastic on
/// this machine instead of probing for it.
///
/// The seam exists because "is this difftastic one rv can read?" is a question
/// about the host, not about the two byte slices, and the honest answer for a
/// difftastic rv *cannot* read has to be testable without installing one.
pub fn compute_with_verdict(
    old: Option<&[u8]>,
    new: Option<&[u8]>,
    path: &str,
    verdict: DifftVerdict,
) -> FileDiff {
    if let Some(reason) = verdict.refusal() {
        return binary_or(old, new, path, reason);
    }
    if is_binary(old) || is_binary(new) {
        return binary(path);
    }
    match difftastic::run(old, new, path) {
        Some((lines, source, suppressed)) => FileDiff {
            path: path.to_owned(),
            lines,
            source,
            suppressed,
        },
        None => similar(old, new, path, FallbackReason::UnreadableOutput),
    }
}

/// What `difft --version` says about the difftastic on `PATH`, probed once per
/// process and cached for the rest of it.
pub fn difft_verdict() -> DifftVerdict {
    probe::verdict()
}

/// How many `difft` processes this thread has run, probe and diffs together.
///
/// Public because [`compute_with`]'s "`false` spawns nothing" is a promise the
/// callers that pass `false` — the TUI's first-frame line counts, every
/// hermetic test — are relying on, and a promise about processes cannot be
/// checked by looking at the [`FileDiff`] that comes back.
pub fn difft_spawns() -> usize {
    probe::spawns()
}

/// Runs difftastic once with `--byte-limit 0`, the switch that selects its
/// line-oriented engine. Returns just the [`DiffLine`]s and whether the
/// answer was suppressed, because the caller — [`crate::app::merges`]' worker
/// in `rv` — is retrying against a previous syntax-aware answer whose
/// `DiffSource`'s `language` it already holds, so the retry's own language
/// (always `"Text (N B exceeded DFT_BYTE_LIMIT)"`, describing the engine's
/// choice not the file) is deliberately discarded.
///
/// Returns `None` when the retry produced nothing usable — a spawn that
/// failed, JSON that did not parse, or a difftastic that could not be run at
/// all. See the design spec §4.6 for the calling protocol.
pub fn compute_line_oriented(
    old: Option<&[u8]>,
    new: Option<&[u8]>,
    path: &str,
) -> Option<(Vec<DiffLine>, bool)> {
    if is_binary(old) || is_binary(new) {
        return None;
    }
    if difft_verdict().refusal().is_some() {
        return None;
    }
    let (lines, _source, suppressed) = difftastic::run_line_oriented(old, new, path)?;
    Some((lines, suppressed))
}

fn binary_or(
    old: Option<&[u8]>,
    new: Option<&[u8]>,
    path: &str,
    reason: FallbackReason,
) -> FileDiff {
    if is_binary(old) || is_binary(new) {
        binary(path)
    } else {
        similar(old, new, path, reason)
    }
}

fn binary(path: &str) -> FileDiff {
    FileDiff {
        path: path.to_owned(),
        lines: Vec::new(),
        source: DiffSource::Binary,
        suppressed: false,
    }
}

fn similar(old: Option<&[u8]>, new: Option<&[u8]>, path: &str, reason: FallbackReason) -> FileDiff {
    let (lines, suppressed) = fallback::diff(old, new);
    FileDiff {
        path: path.to_owned(),
        lines,
        source: DiffSource::Similar { reason },
        suppressed,
    }
}

fn is_binary(side: Option<&[u8]>) -> bool {
    side.is_some_and(|bytes| bytes.contains(&0))
}

/// Lossy UTF-8 decode: good enough to diff, per the module's contract that it
/// never fails on non-UTF-8 input.
fn decode(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// A 0-based index to this module's 1-based line number.
fn index_to_line(index: Option<usize>) -> Option<u32> {
    let index = u32::try_from(index?).ok()?;
    index.checked_add(1)
}
