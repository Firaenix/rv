//! What the diff pane claims about the engine that produced its lines.
//!
//! Asked of [`rv::ui::title`] — the very function [`rv::ui::draw`] builds the
//! pane's block from — because the claim is the point: a reviewer decides how
//! much to trust a diff by reading it. `the_fallback_diff_is_labelled…` in
//! `tests/app_cases/fallback.rs` is the other half, proving this string reaches
//! the frame; the cases here are the ones no machine can reach by running,
//! since they describe a difftastic this machine does not have.

use rv_core::diff::DiffSource;
use rv_core::diff::DifftVersion;
use rv_core::diff::FallbackReason;
use rv_core::diff::FileDiff;

/// A degraded diff names its cause, and each cause reads differently — the
/// reviewer's next move depends on which one it is: install difftastic,
/// upgrade it, or report the file that broke it.
///
/// `NotAttempted` is deliberately bare. rv was told not to run difftastic, so
/// "fallback" is already the whole truth, and a parenthetical would only
/// restate the flag the reviewer just passed.
#[test]
fn a_degraded_pane_says_why_it_is_degraded() {
    let labelled = |reason| rv::ui::title(&fallback(reason), Some("Rust"));

    assert_eq!(
        labelled(FallbackReason::NotAttempted),
        "ctx.rs — fallback",
        "a fallback rv chose explains itself as though something went wrong"
    );
    assert_eq!(
        labelled(FallbackReason::NotInstalled),
        "ctx.rs — fallback (no difft on PATH)"
    );
    assert_eq!(
        labelled(FallbackReason::UnreadableVersion),
        "ctx.rs — fallback (difft version unreadable)"
    );
    assert_eq!(
        labelled(FallbackReason::TooOld(DifftVersion {
            major: 0,
            minor: 40,
            patch: 0
        })),
        "ctx.rs — fallback (difft 0.40.0 predates 0.51.0)",
        "the title names neither the version found nor the version needed"
    );
    assert_eq!(
        labelled(FallbackReason::UnreadableOutput),
        "ctx.rs — fallback (difft output unreadable)"
    );
}

/// The distinction the reasons exist for: an installed-but-unreadable
/// difftastic must never render as the same title a reviewer sees when they
/// asked for the fallback themselves. Collapsing the two is what presenting a
/// guess as a fact would look like on screen.
#[test]
fn an_unusable_difft_is_not_labelled_like_a_chosen_fallback() {
    let chosen = rv::ui::title(&fallback(FallbackReason::NotAttempted), Some("Rust"));

    for reason in [
        FallbackReason::NotInstalled,
        FallbackReason::UnreadableVersion,
        FallbackReason::TooOld(DifftVersion {
            major: 0,
            minor: 50,
            patch: 9,
        }),
        FallbackReason::UnreadableOutput,
    ] {
        assert_ne!(
            rv::ui::title(&fallback(reason), Some("Rust")),
            chosen,
            "{reason:?} reads exactly like a fallback the reviewer asked for"
        );
    }
}

/// A structural diff still says difftastic and its language, and a binary file
/// still says binary — the reasons are additions to the fallback arm, not a
/// rewrite of the pane's vocabulary. `— no highlighting` still follows a title
/// whose file rv has no grammar for, and still does not follow a binary one.
#[test]
fn the_other_titles_are_unchanged() {
    let structural = FileDiff {
        source: DiffSource::Difftastic {
            language: "Rust".to_owned(),
        },
        ..fallback(FallbackReason::NotAttempted)
    };
    assert_eq!(
        rv::ui::title(&structural, Some("Rust")),
        "ctx.rs — difftastic (Rust)"
    );
    assert_eq!(
        rv::ui::title(&structural, None),
        "ctx.rs — difftastic (Rust) — no highlighting"
    );

    let binary = FileDiff {
        source: DiffSource::Binary,
        ..fallback(FallbackReason::NotAttempted)
    };
    assert_eq!(rv::ui::title(&binary, None), "ctx.rs — binary");

    assert_eq!(
        rv::ui::title(&fallback(FallbackReason::NotInstalled), None),
        "ctx.rs — fallback (no difft on PATH) — no highlighting",
        "the grammar note stopped following a reason-bearing fallback"
    );
}

fn fallback(reason: FallbackReason) -> FileDiff {
    FileDiff {
        path: "ctx.rs".to_owned(),
        lines: Vec::new(),
        source: DiffSource::Similar { reason },
        suppressed: false,
    }
}
