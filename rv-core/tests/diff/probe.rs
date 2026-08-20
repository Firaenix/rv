//! The `difft --version` probe: what rv concludes about the difftastic it
//! finds, and what it does with that conclusion.

use crate::support::*;

/// Every way an installed difftastic can be unusable produces a diff that says
/// *which* way, rather than the anonymous fallback a genuine parse failure
/// produces. The distinction is the whole point: "difftastic is not here" and
/// "difftastic is here and this rv cannot read it" are different facts, and a
/// reviewer acts on them differently.
///
/// Injected through `compute_with_verdict` rather than by breaking a real
/// difftastic, so it holds on a machine with a perfectly good `difft` on
/// `PATH` — which is exactly the machine that would otherwise never exercise
/// these branches.
#[rstest]
#[case::absent(DifftVerdict::NotInstalled, FallbackReason::NotInstalled)]
#[case::unreadable(DifftVerdict::UnreadableVersion, FallbackReason::UnreadableVersion)]
#[case::ancient(DifftVerdict::TooOld(ANCIENT), FallbackReason::TooOld(ANCIENT))]
fn an_unusable_difft_falls_back_and_says_why(
    #[case] verdict: DifftVerdict,
    #[case] reason: FallbackReason,
) {
    let old = b"a\n";
    let new = b"a\nc\n";

    let diff = compute_with_verdict(Some(old), Some(new), "notes.txt", verdict);

    assert_eq!(diff.source, DiffSource::Similar { reason }, "{diff:?}");
    assert_ne!(diff.source, NOT_ATTEMPTED, "{diff:?}");
    // The diff itself is a real one: refusing difftastic degrades the label,
    // not the content.
    let added: Vec<&str> = diff
        .lines
        .iter()
        .filter(|line| line.kind == LineKind::Added)
        .map(|line| line.text.as_str())
        .collect();
    assert_eq!(added, vec!["c"], "{diff:?}");
}

/// A difftastic older than the JSON display mode itself.
const ANCIENT: DifftVersion = DifftVersion {
    major: 0,
    minor: 40,
    patch: 0,
};

/// The probe is what decides, and a verdict of "usable" is not the same object
/// as a successful parse: given a usable verdict the diff is attempted, and on
/// this machine — where `difft` is on `PATH` — it succeeds and is labelled
/// difftastic's.
///
/// Paired with the refusal cases above, this is what proves the verdict is
/// *consulted* rather than ignored: the same two files produce a structural
/// diff under one verdict and a labelled fallback under the others.
#[test]
fn a_usable_verdict_lets_difftastic_answer() {
    let old = b"fn a() {\n    let x = 1;\n}\n";
    let new = b"fn a() {\n    let x = 2;\n}\n";

    let diff = compute_with_verdict(
        Some(old),
        Some(new),
        "a.rs",
        DifftVerdict::Usable(MINIMUM_DIFFT),
    );

    assert_eq!(
        diff.source,
        DiffSource::Difftastic {
            language: "Rust".to_owned()
        },
        "is difft on PATH? {diff:?}"
    );
}

/// The hermetic seam: `compute_with(.., false)` reaches no process at all, the
/// probe included. Adding a version probe is exactly the change that could
/// have broken this — a probe run before the `use_difft` check would spawn on
/// the path that promises never to.
///
/// Counted rather than inferred from the label, because a probe that ran and
/// was then discarded would leave the label untouched and the promise broken.
#[test]
fn the_fallback_path_spawns_nothing() {
    let before = difft_spawns();

    for _ in 0..3 {
        compute_with(Some(b"a\n"), Some(b"a\nc\n"), "notes.txt", false);
    }
    // Binary and empty inputs take their own early returns; none may spawn.
    compute_with(Some(b"a\0b"), Some(b"c\n"), "blob.bin", false);
    compute_with(None, None, "gone.txt", false);

    assert_eq!(
        difft_spawns(),
        before,
        "the fallback path ran difft, so it is not hermetic"
    );
}

/// And a binary file spawns nothing even with difftastic *enabled*: neither
/// side is text, so there is nothing difftastic could be asked, whatever
/// version it is. Probing anyway would be a fork spent on a question that is
/// not going to be put.
#[test]
fn a_binary_file_spawns_nothing_either() {
    // Warm the probe first, so what is counted below is the diff's own cost
    // rather than this process's one-off.
    difft_verdict();
    let before = difft_spawns();

    let diff = compute_with(Some(b"a\0b"), Some(b"c\n"), "logo.bin", true);

    assert_eq!(diff.source, DiffSource::Binary, "{diff:?}");
    assert_eq!(
        difft_spawns(),
        before,
        "a binary file asked difft about itself"
    );
}

/// And the probe itself costs one spawn per process, not one per file: the
/// answer cannot change under a running process, so asking again is a fork
/// spent to learn what is already known.
///
/// Needs `difft` on `PATH` — with none, the first call already fails to spawn
/// and there is no repetition to catch.
#[test]
fn the_probe_runs_once_however_many_files_are_diffed() {
    // Warm the cache, and count only what the diffs below cost.
    let verdict = difft_verdict();
    assert!(
        matches!(verdict, DifftVerdict::Usable(_)),
        "difft is unusable here, so this proves nothing: {verdict:?}"
    );

    let before = difft_spawns();
    for index in 0..4 {
        let new = format!("a\n{index}\n");
        compute_with(Some(b"a\n"), Some(new.as_bytes()), "notes.txt", true);
    }

    assert_eq!(
        difft_spawns() - before,
        4,
        "four files cost more than four difft runs — the probe is re-running"
    );
}
