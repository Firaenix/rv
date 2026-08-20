//! The one-shot `difft --version` probe.
//!
//! difftastic's JSON is explicitly unstable — it is gated behind
//! `DFT_UNSTABLE` — so a difftastic whose schema has moved on is not a broken
//! difftastic, it is a *different* one. Parsing failures alone cannot tell the
//! two apart: an incompatible schema and a crashed run both arrive as "that is
//! not the JSON I expected", and both used to be reported as the same
//! anonymous fallback. The version is the only thing difftastic will state
//! about itself before it is asked to diff anything, so it is asked first.

use std::cell::Cell;
use std::process::Command;
use std::sync::LazyLock;

use super::model::DifftVerdict;
use super::model::DifftVersion;

/// The oldest difftastic whose `--display json` output has the field set
/// `difftastic::parse` reads: `status`, `language`, and
/// `chunks[][].lhs`/`.rhs.line_number`.
///
/// 0.51.0 is the release that introduced the JSON display mode, and the shape
/// above is unchanged from it through 0.70 (0.69 added `aligned_lines`, which
/// this crate does not read). Anything older does not accept `--display json`
/// at all, so its diff is not one this crate could show.
pub const MINIMUM_DIFFT: DifftVersion = DifftVersion {
    major: 0,
    minor: 51,
    patch: 0,
};

/// The probe's answer, computed at most once per process.
///
/// Cached rather than repeated because a review spawns one `difft` per file
/// and the answer cannot change under a running process: a second, third and
/// hundredth `difft --version` would each cost a fork to learn what the first
/// already knows.
pub fn verdict() -> DifftVerdict {
    static CACHED: LazyLock<DifftVerdict> = LazyLock::new(run);
    *CACHED
}

fn run() -> DifftVerdict {
    let Ok(output) = command().arg("--version").output() else {
        return DifftVerdict::NotInstalled;
    };
    let text = String::from_utf8_lossy(&output.stdout);
    let Some(version) = parse_version(&text) else {
        return DifftVerdict::UnreadableVersion;
    };
    if version < MINIMUM_DIFFT {
        DifftVerdict::TooOld(version)
    } else {
        DifftVerdict::Usable(version)
    }
}

/// A `difft` invocation, counted. Every process this module's crate runs is
/// created here, so [`spawns`] sees all of them.
pub fn command() -> Command {
    SPAWNS.with(|count| count.set(count.get().saturating_add(1)));
    Command::new("difft")
}

thread_local! {
    static SPAWNS: Cell<usize> = const { Cell::new(0) };
}

/// How many times this thread has run `difft`, probe and diffs together.
///
/// Per-thread rather than global because the number exists to hold
/// [`super::compute_with`]'s promise that `use_difft: false` reaches no
/// process, and a global count would be raced by every other test in the same
/// binary. Every spawn happens on the thread that asked for the diff, so the
/// thread's own count is the exact one that promise is about.
pub fn spawns() -> usize {
    SPAWNS.with(Cell::get)
}

/// The version out of `difft --version`, whose first line is
/// `Difftastic 0.70.0` on current releases and a bare `0.51.0 (abc1234
/// 2023-08-25)` on the oldest ones this crate accepts. Later lines describe
/// the toolchain and host and are not a version of difftastic.
///
/// Every whitespace-separated word of that line is tried, and the first that
/// reads as `major.minor[.patch]` wins — so neither a renamed product word nor
/// an added prefix makes a present, readable version look absent.
fn parse_version(text: &str) -> Option<DifftVersion> {
    text.lines()
        .find(|line| !line.trim().is_empty())?
        .split_whitespace()
        .find_map(parse_word)
}

fn parse_word(word: &str) -> Option<DifftVersion> {
    let mut parts = word.trim_start_matches('v').split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    // An absent patch is 0; a patch carrying a pre-release suffix (`1-rc2`)
    // counts as that patch, which is the conservative reading — a pre-release
    // orders below its own release, and this crate's floor is a release.
    let patch = match parts.next() {
        None => 0,
        Some(part) => part
            .split(|character: char| !character.is_ascii_digit())
            .next()?
            .parse()
            .ok()?,
    };
    Some(DifftVersion {
        major,
        minor,
        patch,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shapes `difft --version` has actually printed, plus the ones a
    /// defensive parser exists for. Not a test of difftastic: a test that the
    /// version is read out of the line it is on, and that a line naming no
    /// version is reported as naming none rather than as some old release.
    #[test]
    fn versions_are_read_out_of_the_first_line() {
        let v = |major, minor, patch| {
            Some(DifftVersion {
                major,
                minor,
                patch,
            })
        };
        assert_eq!(
            parse_version("Difftastic 0.70.0\n\nToolchain: 1.97.1\nSystem:    macos aarch64\n"),
            v(0, 70, 0)
        );
        assert_eq!(parse_version("0.51.0 (abc1234 2023-08-25)\n"), v(0, 51, 0));
        assert_eq!(parse_version("\n\ndifft v1.2.3\n"), v(1, 2, 3));
        assert_eq!(parse_version("Difftastic 0.71\n"), v(0, 71, 0));
        assert_eq!(parse_version("Difftastic 1.0.0-rc2\n"), v(1, 0, 0));
        // A toolchain line is not difftastic's version, so a first line with
        // no version must not borrow one from below it.
        assert_eq!(parse_version("Difftastic\nToolchain: 1.97.1\n"), None);
        assert_eq!(parse_version(""), None);
        assert_eq!(parse_version("difft: command not found\n"), None);
    }

    /// The floor is the release that introduced `--display json`, and the
    /// verdict is a comparison against it rather than an equality with one
    /// tested version: 0.70 must not become unusable when 0.71 ships.
    #[test]
    fn the_floor_is_the_json_display_release() {
        assert_eq!(MINIMUM_DIFFT.to_string(), "0.51.0");
        assert!(
            parse_version("Difftastic 0.50.0\n").expect("a version") < MINIMUM_DIFFT,
            "a difftastic predating --display json is not below the floor"
        );
        for line in ["Difftastic 0.51.0\n", "Difftastic 0.70.0\n", "1.0.0\n"] {
            assert!(
                parse_version(line).expect("a version") >= MINIMUM_DIFFT,
                "{line:?} reads as below the floor"
            );
        }
    }
}
