//! What a diff *is*, independent of which engine produced it.

use std::fmt;

use serde::Deserialize;
use serde::Serialize;

/// How a file differs between the two endpoints of a review.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FileDiff {
    pub path: String,
    pub lines: Vec<DiffLine>,
    pub source: DiffSource,
    /// Set when the file differs but the difference is not one the lines above
    /// can show a reviewer, so they are not worth showing by default:
    /// difftastic's `unchanged` status (e.g. pure reindentation), or — on the
    /// fallback path — a difference that lives entirely in the line
    /// terminators, which no line's `text` carries.
    pub suppressed: bool,
}

/// One line of a [`FileDiff`]. `left` and `right` are 1-based line numbers on
/// the base and head side respectively; either may be absent, but both are
/// populated when difftastic aligns a changed line with its counterpart on
/// the other side.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DiffLine {
    pub kind: LineKind,
    pub left: Option<u32>,
    pub right: Option<u32>,
    pub text: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum LineKind {
    Context,
    Added,
    Removed,
}

/// Where the lines of a [`FileDiff`] came from.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum DiffSource {
    /// Produced from difftastic's JSON output. `language` is whatever
    /// difftastic detected on the *first* invocation (it always reports one,
    /// defaulting to `"Text"`).
    ///
    /// `line_oriented` records that rv had to re-invoke difftastic with
    /// `--byte-limit 0` — its line-oriented engine — before the merge could
    /// build honest full context, per the design spec's §4.6. The
    /// invocation that carried it always reports its own language as
    /// `"Text (N B exceeded DFT_BYTE_LIMIT)"`, describing the engine's
    /// choice rather than the file; that string is deliberately discarded
    /// and the first invocation's language is kept, because the enum
    /// records what happened to the file, not what the fallback engine
    /// reported. `false` is the ordinary case: the syntax-aware answer
    /// stood on its own.
    Difftastic {
        language: String,
        line_oriented: bool,
    },
    /// Produced by the `similar` crate's line diff, for the stated reason.
    Similar { reason: FallbackReason },
    /// Neither side was diffed: a NUL byte was found on at least one side.
    Binary,
}

/// Why a diff is a `similar` line diff rather than difftastic's structural
/// one. A reviewer reading a degraded diff is owed the difference between
/// "difftastic was never asked" and "difftastic is here but cannot be
/// believed" — collapsing the two would present a guess as a fact.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FallbackReason {
    /// difftastic was not consulted at all: `RV_NO_DIFFT`, or an explicit
    /// `use_difft: false`. Nothing was learned about difftastic, and nothing
    /// is claimed about it.
    NotAttempted,
    /// `difft` could not be run.
    NotInstalled,
    /// `difft --version` ran but printed no version this crate recognises, so
    /// whether its JSON matches the parser is unknown.
    UnreadableVersion,
    /// Installed, and older than the release whose JSON shape the parser
    /// reads.
    TooOld(DifftVersion),
    /// The version probe passed, but this file's run did not produce the JSON
    /// the parser expects — a schema that drifted within a version, or a run
    /// that failed outright.
    UnreadableOutput,
}

/// What the one-shot `difft --version` probe concluded. A statement about the
/// installed binary, not about any particular file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DifftVerdict {
    Usable(DifftVersion),
    NotInstalled,
    UnreadableVersion,
    TooOld(DifftVersion),
}

impl DifftVerdict {
    /// Why difftastic must not be run, or `None` when it may be.
    pub fn refusal(self) -> Option<FallbackReason> {
        match self {
            Self::Usable(_) => None,
            Self::NotInstalled => Some(FallbackReason::NotInstalled),
            Self::UnreadableVersion => Some(FallbackReason::UnreadableVersion),
            Self::TooOld(version) => Some(FallbackReason::TooOld(version)),
        }
    }
}

/// A difftastic release. Field order is the comparison order, so `Ord` is
/// version precedence.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct DifftVersion {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl fmt::Display for DifftVersion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}
