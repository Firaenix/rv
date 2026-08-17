use rv_core::diff::DiffSource;
use rv_core::diff::LineKind;
use rv_core::diff::compute;
use rv_core::diff::compute_with;

/// difftastic must be on `PATH` for this test: it exercises the real JSON
/// shape difftastic returns for a one-line semantic change, per the sample
/// captured in the task-3 report.
#[test]
fn changed_line() {
    let old = b"fn a() {\n    let x = 1;\n}\n";
    let new = b"fn a() {\n    let x = 2;\n}\n";

    let diff = compute(Some(old), Some(new), "a.rs");

    assert!(!diff.suppressed, "{diff:?}");
    assert_eq!(
        diff.source,
        DiffSource::Difftastic {
            language: "Rust".to_owned()
        },
        "{diff:?}"
    );
    assert_eq!(diff.lines.len(), 2, "{diff:?}");
    assert_eq!(diff.lines[0].kind, LineKind::Removed);
    assert_eq!(diff.lines[0].left, Some(2));
    assert_eq!(diff.lines[0].right, Some(2));
    assert_eq!(diff.lines[0].text, "    let x = 1;");
    assert_eq!(diff.lines[1].kind, LineKind::Added);
    assert_eq!(diff.lines[1].left, Some(2));
    assert_eq!(diff.lines[1].right, Some(2));
    assert_eq!(diff.lines[1].text, "    let x = 2;");
}

/// Also depends on the real `difft` binary: it is the only way to learn that
/// a change is purely cosmetic (difftastic's `status: "unchanged"`).
#[test]
fn reindentation_only_suppressed() {
    let old = b"fn a() {\n    let x = 1;\n}\n";
    let new = b"fn a() {\n        let x = 1;\n}\n";

    let diff = compute(Some(old), Some(new), "a.rs");

    assert!(diff.suppressed, "{diff:?}");
    assert!(diff.lines.is_empty(), "{diff:?}");
    assert_eq!(
        diff.source,
        DiffSource::Difftastic {
            language: "Rust".to_owned()
        },
        "{diff:?}"
    );
}

/// The binary check runs before difftastic is even considered, so this test
/// needs no external tool and passes on any machine.
#[test]
fn binary() {
    let old = b"plain text\n";
    let new: &[u8] = &[0, 159, 146, 150];

    let diff = compute(Some(old), Some(new), "logo.bin");

    assert_eq!(diff.source, DiffSource::Binary, "{diff:?}");
    assert!(diff.lines.is_empty(), "{diff:?}");
    assert!(!diff.suppressed, "{diff:?}");
}

/// `compute_with(..., false)` never spawns `difft` or reads the environment,
/// so this test is hermetic regardless of what is installed.
#[test]
fn fallback() {
    let old = b"a\n";
    let new = b"a\nc\n";

    let diff = compute_with(Some(old), Some(new), "notes.txt", false);

    assert_eq!(diff.source, DiffSource::Similar, "{diff:?}");
    assert!(!diff.suppressed, "{diff:?}");
    let added: Vec<&rv_core::diff::DiffLine> = diff
        .lines
        .iter()
        .filter(|line| line.kind == LineKind::Added)
        .collect();
    assert_eq!(added.len(), 1, "{diff:?}");
    assert_eq!(added[0].text, "c");
    assert_eq!(added[0].right, Some(2));
    assert_eq!(added[0].left, None);
}

/// A brand-new file (`old: None`) has nothing for difftastic to structurally
/// align against — it reports `status: "created"` with no chunks — so this
/// exercises the fallback path regardless of whether `difft` is installed.
#[test]
fn all_additions() {
    let new = b"x\ny\n";

    let diff = compute(None, Some(new), "new.txt");

    assert_eq!(diff.source, DiffSource::Similar, "{diff:?}");
    assert!(!diff.suppressed, "{diff:?}");
    assert_eq!(diff.lines.len(), 2, "{diff:?}");
    for line in &diff.lines {
        assert_eq!(line.kind, LineKind::Added, "{diff:?}");
        assert_eq!(line.left, None, "{diff:?}");
    }
    assert_eq!(diff.lines[0].text, "x");
    assert_eq!(diff.lines[0].right, Some(1));
    assert_eq!(diff.lines[1].text, "y");
    assert_eq!(diff.lines[1].right, Some(2));
}
