//! Shared by the modules of this binary.

pub use rstest::rstest;
pub use rv_core::diff::DiffSource;
pub use rv_core::diff::DifftVerdict;
pub use rv_core::diff::DifftVersion;
pub use rv_core::diff::FallbackReason;
pub use rv_core::diff::LineKind;
pub use rv_core::diff::MINIMUM_DIFFT;
pub use rv_core::diff::compute;
pub use rv_core::diff::compute_with;
pub use rv_core::diff::compute_with_verdict;
pub use rv_core::diff::difft_spawns;
pub use rv_core::diff::difft_verdict;

/// What `compute_with(.., false)` labels its diffs: difftastic was not asked,
/// so nothing is claimed about it.
pub const NOT_ATTEMPTED: DiffSource = DiffSource::Similar {
    reason: FallbackReason::NotAttempted,
};
