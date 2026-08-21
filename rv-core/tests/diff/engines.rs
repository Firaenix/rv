//! What each engine produces for a given pair of blobs.

use crate::support::*;

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
            language: "Rust".to_owned(),
            line_oriented: false,
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
            language: "Rust".to_owned(),
            line_oriented: false,
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

/// difftastic can report the same change in two chunks: for old `"b\n\n"` vs
/// new `"a\n\n"`, difft 0.70 returns
/// `chunks: [[{lhs:0,rhs:0}],[{lhs:0,rhs:0}]]` — the identical entry twice.
/// Concatenating chunks blindly turned a one-line change in a two-line file
/// into four diff lines, and the TUI, which windows `diff.lines` in order,
/// drew the same change twice. The module owns chunk concatenation, so it is
/// the module that must show the change once.
///
/// Needs `difft` on `PATH`, like `changed_line`.
#[test]
fn a_change_difftastic_reports_in_two_chunks_is_shown_once() {
    let old = b"b\n\n";
    let new = b"a\n\n";

    let diff = compute_with(Some(old), Some(new), "notes.txt", true);

    assert!(
        matches!(diff.source, DiffSource::Difftastic { .. }),
        "{diff:#?}"
    );
    assert_eq!(diff.lines.len(), 2, "{diff:#?}");
    assert_eq!(diff.lines[0].kind, LineKind::Removed, "{diff:#?}");
    assert_eq!(diff.lines[0].text, "b", "{diff:#?}");
    assert_eq!(diff.lines[0].left, Some(1), "{diff:#?}");
    assert_eq!(diff.lines[1].kind, LineKind::Added, "{diff:#?}");
    assert_eq!(diff.lines[1].text, "a", "{diff:#?}");
    assert_eq!(diff.lines[1].right, Some(1), "{diff:#?}");
}

/// difftastic's chunks are not ordered by line number: for old
/// `"a\nb\nc\nd\n"` vs new `"a\nB\nc\nd\ne\n"`, difft 0.70 reports the entry
/// for new line 5 *before* the entry for line 2. The TUI renders
/// `diff.lines` in order, so a reviewer was shown the file's last line above
/// its second. The module must put the lines back in file order.
///
/// Needs `difft` on `PATH`, like `changed_line`.
#[test]
fn difftastic_chunks_reported_out_of_order_are_shown_in_file_order() {
    let old = b"a\nb\nc\nd\n";
    let new = b"a\nB\nc\nd\ne\n";

    let diff = compute_with(Some(old), Some(new), "notes.txt", true);

    assert!(
        matches!(diff.source, DiffSource::Difftastic { .. }),
        "{diff:#?}"
    );
    let rendered: Vec<String> = diff
        .lines
        .iter()
        .map(|line| {
            let sigil = match line.kind {
                LineKind::Added => '+',
                LineKind::Removed => '-',
                LineKind::Context => ' ',
            };
            format!(
                "{sigil}{}/{} {}",
                line.left.map_or_else(|| ".".to_owned(), |n| n.to_string()),
                line.right.map_or_else(|| ".".to_owned(), |n| n.to_string()),
                line.text
            )
        })
        .collect();
    assert_eq!(
        rendered,
        vec![
            "-2/2 b".to_owned(),
            "+2/2 B".to_owned(),
            "+./5 e".to_owned()
        ],
        "{diff:#?}"
    );
}

/// The same ordering requirement when difftastic emits *no aligned pair at
/// all*, which is the shape
/// `difftastic_chunks_reported_out_of_order_are_shown_in_file_order` cannot
/// reach: every hunk here is a pure insertion or a pure deletion, so there is
/// no both-sides entry for a one-sided one to be positioned against.
///
/// For these two files difft 0.70 reports four chunks —
/// `rhs 1`, `lhs 3`, `rhs 12`, `lhs 15` (0-based) — i.e. base lines 4 and 16
/// deleted and head lines 2 and 13 inserted. Placing each entry by its own
/// side's number alone sorted every removal above every insertion and rendered
/// `-4, -16, +2, +13`: the reviewer saw the file's deletions as one block and
/// its insertions as another, instead of in file order.
///
/// Needs `difft` on `PATH`, like `changed_line`.
#[test]
fn one_sided_difftastic_hunks_interleave_with_each_other() {
    let old = b"a\nb\nc\nd\ne\nf\ng\nh\ni\nj\nk\nl\nm\nn\no\np\nq\nr\n";
    let new = b"a\nZ\nb\nc\ne\nf\ng\nh\ni\nj\nk\nl\nY\nm\nn\no\nq\nr\n";

    let diff = compute_with(Some(old), Some(new), "notes.txt", true);

    assert!(
        matches!(diff.source, DiffSource::Difftastic { .. }),
        "{diff:#?}"
    );
    assert_eq!(
        render(&diff),
        vec![
            "+./2 Z".to_owned(),
            "-4/. d".to_owned(),
            "+./13 Y".to_owned(),
            "-16/. p".to_owned(),
        ],
        "{diff:#?}"
    );
}

/// The smallest version of the same shape, and the one that shows the order
/// is not merely "insertions first": here the deletion (base line 3) comes
/// before the insertion (head line 5), because two lines the files share
/// (`a`, `b`) precede the deletion and four (`a`, `b`, `d`, `e`) precede the
/// insertion. difft 0.70 reports these as two separate one-entry chunks.
///
/// Needs `difft` on `PATH`, like `changed_line`.
#[test]
fn a_pure_deletion_and_a_pure_insertion_interleave_by_shared_position() {
    let old = b"a\nb\nc\nd\ne\nf\n";
    let new = b"a\nb\nd\ne\nQ\nf\n";

    let diff = compute_with(Some(old), Some(new), "notes.txt", true);

    assert!(
        matches!(diff.source, DiffSource::Difftastic { .. }),
        "{diff:#?}"
    );
    assert_eq!(
        render(&diff),
        vec!["-3/. c".to_owned(), "+./5 Q".to_owned()],
        "{diff:#?}"
    );
}

/// `<sigil><left>/<right> <text>` per line, `.` for an absent number — the
/// same compact rendering `tests/prop_diff.rs` uses.
fn render(diff: &rv_core::diff::FileDiff) -> Vec<String> {
    diff.lines
        .iter()
        .map(|line| {
            let sigil = match line.kind {
                LineKind::Added => '+',
                LineKind::Removed => '-',
                LineKind::Context => ' ',
            };
            format!(
                "{sigil}{}/{} {}",
                line.left.map_or_else(|| ".".to_owned(), |n| n.to_string()),
                line.right.map_or_else(|| ".".to_owned(), |n| n.to_string()),
                line.text
            )
        })
        .collect()
}

/// `compute_with(..., false)` never spawns `difft` or reads the environment,
/// so this test is hermetic regardless of what is installed.
#[test]
fn fallback() {
    let old = b"a\n";
    let new = b"a\nc\n";

    let diff = compute_with(Some(old), Some(new), "notes.txt", false);

    assert_eq!(diff.source, NOT_ATTEMPTED, "{diff:?}");
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

/// A brand-new file (`old: None`) is difftastic's `status: "created"`, which
/// carries no `chunks` but does correctly detect the language — so, like
/// `changed_line`, this needs a real `difft` on `PATH` to see the
/// `Difftastic` label rather than a `similar` fallback. `all_removals` below
/// is the mirror image on the delete side.
#[test]
fn all_additions() {
    let new = b"x\ny\n";

    let diff = compute(None, Some(new), "new.txt");

    assert_eq!(
        diff.source,
        DiffSource::Difftastic {
            language: "Text".to_owned(),
            line_oriented: false,
        },
        "{diff:?}"
    );
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

/// The mirror of `all_additions` on the delete side: a whole-file removal
/// (`new: None`) is difftastic's `status: "deleted"`, also chunk-less but
/// also a successfully parsed, language-detected response. Covers the
/// `all_removed` path in `parse_difft_json`, which `all_additions` cannot.
#[test]
fn all_removals() {
    let old = b"x\ny\n";

    let diff = compute(Some(old), None, "old.txt");

    assert_eq!(
        diff.source,
        DiffSource::Difftastic {
            language: "Text".to_owned(),
            line_oriented: false,
        },
        "{diff:?}"
    );
    assert!(!diff.suppressed, "{diff:?}");
    assert_eq!(diff.lines.len(), 2, "{diff:?}");
    for line in &diff.lines {
        assert_eq!(line.kind, LineKind::Removed, "{diff:?}");
        assert_eq!(line.right, None, "{diff:?}");
    }
    assert_eq!(diff.lines[0].text, "x");
    assert_eq!(diff.lines[0].left, Some(1));
    assert_eq!(diff.lines[1].text, "y");
    assert_eq!(diff.lines[1].left, Some(2));
}

/// `all_additions`'s behavior when difftastic is bypassed: still two Added
/// lines, just labeled `Similar` — confirms the fix only changes the label
/// on a *successful* difft "created"/"deleted" parse, not the fallback
/// itself. Hermetic: `use_difft: false` never spawns a process.
#[test]
fn all_additions_without_difft() {
    let new = b"x\ny\n";

    let diff = compute_with(None, Some(new), "new.txt", false);

    assert_eq!(diff.source, NOT_ATTEMPTED, "{diff:?}");
    assert!(!diff.suppressed, "{diff:?}");
    assert_eq!(diff.lines.len(), 2, "{diff:?}");
    for line in &diff.lines {
        assert_eq!(line.kind, LineKind::Added, "{diff:?}");
        assert_eq!(line.left, None, "{diff:?}");
    }
}
