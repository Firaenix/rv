use rv_core::anchor::create;
use rv_core::anchor::resolve;
use rv_core::anchor::snapshot_of;
use rv_core::model::Confidence;
use rv_core::model::Side;

/// A line that has not moved and has not changed re-resolves to itself,
/// `Exact`.
#[test]
fn unchanged_resolves_exact() {
    let text = "fn a() {\n    let x = 1;\n}\n";
    let anchor = create("a.rs", Side::Left, 2, text);

    let (line, confidence) = resolve(&anchor, text);

    assert_eq!(line, Some(2));
    assert_eq!(confidence, Confidence::Exact);
}

/// Prepending two lines pushes the anchored line from 2 to 4; its content is
/// untouched, so the hash still matches and it resolves `Moved` to its new
/// position rather than staying pinned to the stale line number.
#[test]
fn shifted_line_resolves_moved() {
    let text = "fn a() {\n    let x = 1;\n}\n";
    let anchor = create("a.rs", Side::Left, 2, text);

    let shifted = format!("// header\n// more header\n{text}");
    let (line, confidence) = resolve(&anchor, &shifted);

    assert_eq!(line, Some(4));
    assert_eq!(confidence, Confidence::Moved);
}

/// Reindenting a line (spaces collapsed, tabs swapped in) does not change its
/// normalized content, so the hash still matches at the same line number:
/// `Exact`, not `Moved`.
#[test]
fn reindent_survives_hash_exact() {
    let text = "fn a() {\n    let   x = 1;\n}\n";
    let anchor = create("a.rs", Side::Left, 2, text);

    let reindented = "fn a() {\n\tlet x = 1;\n}\n";
    let (line, confidence) = resolve(&anchor, reindented);

    assert_eq!(line, Some(2));
    assert_eq!(confidence, Confidence::Exact);
}

/// The anchored line is deleted outright and its content appears nowhere
/// else in the new text: no hash match anywhere, so resolution gives up
/// rather than guessing.
#[test]
fn deleted_line_outdated() {
    let text = "a\nb\nc\n";
    let anchor = create("f.txt", Side::Right, 2, text);

    let edited = "a\nc\n";
    let (line, confidence) = resolve(&anchor, edited);

    assert_eq!(line, None);
    assert_eq!(confidence, Confidence::Outdated);
}

/// The anchored line's content ("x") appears twice in the edited text, at
/// lines 1 and 4; the anchor's original line was 3, so line 4 (distance 1)
/// is the nearer match over line 1 (distance 2) and wins.
#[test]
fn duplicate_content_resolves_nearest() {
    let text = "a\nb\nx\nc\n";
    let anchor = create("f.txt", Side::Left, 3, text);

    let edited = "x\na\nb\nx\nc\n";
    let (line, confidence) = resolve(&anchor, edited);

    assert_eq!(line, Some(4));
    assert_eq!(confidence, Confidence::Moved);
}

/// Centering on line 7 of a 12-line file captures 5 lines before, the line
/// itself, and 5 lines after: 11 lines of context, none clamped since line 7
/// is far enough from either edge.
#[test]
fn snapshot_captures_context() {
    let text = (1..=12)
        .map(|n| format!("line{n}"))
        .collect::<Vec<_>>()
        .join("\n");

    let context = snapshot_of(&text, 7);

    assert_eq!(context.len(), 11);
}
