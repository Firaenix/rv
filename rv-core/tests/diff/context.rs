//! [`rv_core::diff::merge_context`]: filling difftastic's changed-only lines
//! with the untouched majority of the file.
//!
//! See `docs/superpowers/specs/2026-08-21-rv-full-file-context-design.md`
//! for the design this pins, in particular §3's reformatted-region case,
//! reproduced exactly below.

use crate::support::*;
use rv_core::diff::LineKind;
use rv_core::diff::merge_context;

/// `count` distinct lines, each naming `prefix`, one per line, LF-terminated.
fn numbered(prefix: &str, count: u32) -> String {
    (1..=count)
        .map(|n| format!("{prefix}{n}\n"))
        .collect::<String>()
}

/// Three edits a dozen lines apart in a thirty-line file — the same shape
/// `rv/tests/app/hunks.rs::three_hunks` uses to prove `J`/`K` see three
/// hunks rather than one, reused here to prove the merged stream is a
/// faithful reconstruction of both files around three separated changes.
fn three_hunks() -> (String, String) {
    let old = numbered("keep", 30);
    let mut lines: Vec<String> = old.lines().map(str::to_owned).collect();
    for edited in [3usize, 15, 27] {
        lines[edited - 1] = format!("edited{edited}");
    }
    let new = lines
        .into_iter()
        .map(|line| format!("{line}\n"))
        .collect::<String>();
    (old, new)
}

/// Texts of the merged stream's old side (`Removed` + `Context`), in order.
fn old_side_texts(merged: &[rv_core::diff::DiffLine]) -> Vec<String> {
    merged
        .iter()
        .filter(|line| line.kind != LineKind::Added)
        .map(|line| line.text.clone())
        .collect()
}

/// Texts of the merged stream's new side (`Added` + `Context`), in order.
fn new_side_texts(merged: &[rv_core::diff::DiffLine]) -> Vec<String> {
    merged
        .iter()
        .filter(|line| line.kind != LineKind::Removed)
        .map(|line| line.text.clone())
        .collect()
}

fn old_side_numbers(merged: &[rv_core::diff::DiffLine]) -> Vec<Option<u32>> {
    merged
        .iter()
        .filter(|line| line.kind != LineKind::Added)
        .map(|line| line.left)
        .collect()
}

fn new_side_numbers(merged: &[rv_core::diff::DiffLine]) -> Vec<Option<u32>> {
    merged
        .iter()
        .filter(|line| line.kind != LineKind::Removed)
        .map(|line| line.right)
        .collect()
}

fn one_based(count: usize) -> Vec<Option<u32>> {
    (1..=count)
        .map(|n| Some(u32::try_from(n).expect("fixtures are small")))
        .collect()
}

/// CONSERVATION over the full merged stream, for a real multi-hunk fixture:
/// the old side reconstructs the old file exactly, the new side reconstructs
/// the new file exactly, both numbered `1..=n` with no gaps or repeats.
///
/// Needs `difft` on `PATH`: this is what `merge_context` is meant to run
/// against — difftastic's changed-only output — and the three edits are
/// separated by unchanged lines so the merge has real gaps to fill, not just
/// the two at the ends of the file.
#[test]
fn full_context_conserves_both_files_for_a_real_multi_hunk_fixture() {
    let (old, new) = three_hunks();
    let diff = compute_with(Some(old.as_bytes()), Some(new.as_bytes()), "wide.rs", true);
    assert!(
        matches!(diff.source, DiffSource::Difftastic { .. }),
        "is difft on PATH? {diff:?}"
    );
    // Sanity: difftastic really did report only the changed lines, so the
    // merge below has gaps worth filling rather than nothing to do.
    assert_eq!(diff.lines.len(), 6, "{:?}", diff.lines);
    assert!(
        diff.lines.iter().all(|line| line.kind != LineKind::Context),
        "difftastic already produced context, so this fixture proves nothing: {:?}",
        diff.lines
    );

    let merged = merge_context(&diff.lines, &old, &new)
        .unwrap_or_else(|| panic!("a same-length gap merge must succeed: {:?}", diff.lines));

    let want_old: Vec<String> = old.lines().map(str::to_owned).collect();
    let want_new: Vec<String> = new.lines().map(str::to_owned).collect();

    assert_eq!(old_side_texts(&merged), want_old, "{merged:#?}");
    assert_eq!(new_side_texts(&merged), want_new, "{merged:#?}");
    assert_eq!(
        old_side_numbers(&merged),
        one_based(want_old.len()),
        "{merged:#?}"
    );
    assert_eq!(
        new_side_numbers(&merged),
        one_based(want_new.len()),
        "{merged:#?}"
    );

    // And the changed lines are still marked as such, not swallowed into
    // context: exactly three Removed/Added pairs.
    let changed = merged
        .iter()
        .filter(|line| line.kind != LineKind::Context)
        .count();
    assert_eq!(changed, 6, "{merged:#?}");
}

/// Every synthesized `Context` line carries the **same** old and new line
/// number — it is, by construction, a line neither side moved — and that
/// number is exactly where the line sits in each file.
///
/// Needs `difft` on `PATH`, like the fixture above.
#[test]
fn context_lines_carry_correct_identical_old_new_line_numbers() {
    let (old, new) = three_hunks();
    let diff = compute_with(Some(old.as_bytes()), Some(new.as_bytes()), "wide.rs", true);
    let merged = merge_context(&diff.lines, &old, &new).expect("a same-length gap merge");

    let old_lines: Vec<&str> = old.lines().collect();
    let mut checked = 0;
    for line in &merged {
        if line.kind != LineKind::Context {
            continue;
        }
        checked += 1;
        assert_eq!(
            line.left, line.right,
            "an untouched line changed number across the merge: {line:?}"
        );
        let number = line.left.expect("a context line always carries a number");
        let index = usize::try_from(number).expect("a small line number") - 1;
        assert_eq!(
            old_lines.get(index).copied(),
            Some(line.text.as_str()),
            "context line {number} does not read the text actually at that line: {line:?}"
        );
    }
    assert!(
        checked >= 24,
        "too few context lines to prove anything: {checked}"
    );
}

/// §3's reformatted-region case, reproduced exactly: difftastic reports
/// **zero chunks** for a one-line function reformatted into five, because it
/// is not a semantic change — so old and new both contribute lines to the
/// same gap in different counts, and there is no honest line-for-line
/// pairing to print.
///
/// Needs `difft` on `PATH`.
#[test]
fn a_reformatted_region_difftastic_elides_returns_none() {
    let old = "fn a() { let x = foo(1, 2, 3); }\n";
    let new = "fn a() {\n    let x = foo(\n        1,\n        2,\n        3,\n    );\n}\n";

    let diff = compute_with(Some(old.as_bytes()), Some(new.as_bytes()), "a.rs", true);
    assert!(
        matches!(diff.source, DiffSource::Difftastic { .. }),
        "is difft on PATH? {diff:?}"
    );
    assert!(diff.suppressed, "{diff:?}");
    assert!(
        diff.lines.is_empty(),
        "difftastic reported chunks for this reformat, so the fixture no longer proves \
         the elided-gap case: {:?}",
        diff.lines
    );

    // No chunks at all means no anchor to walk from — this is the
    // suppressed-with-no-lines case §4.5 defers, not the gap-mismatch this
    // test names. Flank the reformatted region with a real change on *both*
    // sides — a header line unchanged, then the reformat, then a tail
    // function whose body genuinely changes — so the mismatched gap sits
    // strictly between two anchors the walk both agree on, rather than
    // abutting the end of the file where a coincidental equal remainder
    // could mask the bug.
    let old =
        "fn header() {}\n\nfn a() { let x = foo(1, 2, 3); }\n\nfn tail() {\n    let z = 1;\n}\n";
    let new = "fn header() {}\n\nfn a() {\n    let x = foo(\n        1,\n        2,\n        3,\n    );\n}\n\nfn tail() {\n    let z = 99;\n}\n";
    let diff = compute_with(Some(old.as_bytes()), Some(new.as_bytes()), "a.rs", true);
    assert!(
        matches!(diff.source, DiffSource::Difftastic { .. }),
        "is difft on PATH? {diff:?}"
    );
    assert!(!diff.suppressed, "{diff:?}");
    assert!(!diff.lines.is_empty(), "{:?}", diff.lines);

    assert_eq!(
        merge_context(&diff.lines, old, new),
        None,
        "the merge fabricated a pairing across a region difftastic elided: {:?}",
        diff.lines
    );
}

/// §4.6: the `--byte-limit 0` retry recovers full context on §3's
/// reformatted-region fixture. Asserts the four acceptance criteria the
/// spec §4.6's implementation-guidance calls out: (a) `merge_context` on
/// the syntax-aware output returns `None`, (b) the retry with
/// `--byte-limit 0` produces non-empty chunks, (c) `merge_context` on the
/// retry output returns `Some`, (d) the composed answer is
/// `DiffSource::Difftastic { language: "Rust", line_oriented: true }`
/// once rv's app-side worker copies the flag onto the source — this test
/// exercises the core primitives, so it stops at (c) plus a language check
/// on the first invocation, and leaves the source-mutation half to
/// `rv/tests/app/fullcontext.rs::a_reformatted_region_recovers_through_the_line_oriented_retry`.
///
/// Needs `difft` on `PATH`.
#[test]
fn a_reformatted_region_the_line_oriented_retry_can_merge() {
    use rv_core::diff::DiffSource;
    use rv_core::diff::compute_line_oriented;
    use rv_core::diff::compute_with;

    // Same fixture as `a_reformatted_region_difftastic_elides_returns_none`'s
    // second half — flanked by a header and a real change so the mismatched
    // gap sits between two anchors.
    let old =
        "fn header() {}\n\nfn a() { let x = foo(1, 2, 3); }\n\nfn tail() {\n    let z = 1;\n}\n";
    let new = "fn header() {}\n\nfn a() {\n    let x = foo(\n        1,\n        2,\n        3,\n    );\n}\n\nfn tail() {\n    let z = 99;\n}\n";

    // (a) The syntax-aware answer's merge still declines.
    let syntax_aware = compute_with(Some(old.as_bytes()), Some(new.as_bytes()), "a.rs", true);
    assert!(
        matches!(
            &syntax_aware.source,
            DiffSource::Difftastic { language, line_oriented: false, .. }
            if language == "Rust"
        ),
        "not the syntax-aware Rust answer: {:?}",
        syntax_aware.source
    );
    assert_eq!(
        merge_context(&syntax_aware.lines, old, new),
        None,
        "the syntax-aware merge unexpectedly succeeded — fixture no longer proves the retry: {:?}",
        syntax_aware.lines
    );

    // (b) The line-oriented retry produces non-empty chunks.
    let (retry_lines, _suppressed) =
        compute_line_oriented(Some(old.as_bytes()), Some(new.as_bytes()), "a.rs")
            .expect("the --byte-limit 0 retry ran and parsed");
    assert!(
        !retry_lines.is_empty(),
        "the retry produced no chunks — the line-oriented engine did not see the whitespace change"
    );

    // (c) `merge_context` on the retry output returns `Some`.
    let merged = merge_context(&retry_lines, old, new)
        .expect("the retry's chunks paired 1:1 through the reformatted region");
    // The merged stream is longer than either the syntax-aware stream or the
    // retry stream alone: it interleaves both files' unchanged lines.
    assert!(
        merged.len() >= retry_lines.len(),
        "the merge dropped lines from the retry: merged={:?} retry={:?}",
        merged,
        retry_lines,
    );
}
