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
/// else — but the file still has a line at the anchor's number, so the raw
/// number is the fallback: the third tier, `Weak`, not a guess at content.
#[test]
fn deleted_line_falls_back_to_its_number_weakly() {
    let text = "a\nb\nc\n";
    let anchor = create("f.txt", Side::Right, 2, text);

    let edited = "a\nc\n";
    let (line, confidence) = resolve(&anchor, edited);

    assert_eq!(line, Some(2));
    assert_eq!(confidence, Confidence::Weak);
}

/// With the file shorter than the anchor's number there is no line to fall
/// back on, and resolution gives up rather than guessing.
#[test]
fn a_line_past_the_new_end_is_outdated() {
    let text = "a\nb\nc\n";
    let anchor = create("f.txt", Side::Right, 3, text);

    let (line, confidence) = resolve(&anchor, "a\n");

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

/// A blank line that moves to a different line number cannot be told apart
/// by hash from any *other* blank line in the file — every all-whitespace
/// line normalizes to `""` — so the `Moved` scan excludes blank candidates
/// entirely. A moved blank-line anchor resolves `Outdated` rather than
/// guessing which blank line it moved to.
#[test]
fn blank_line_anchor_moved_falls_back_weakly() {
    let text = "a\n\nb\n";
    let anchor = create("f.txt", Side::Left, 2, text);

    let edited = format!("x\n{text}");
    let (line, confidence) = resolve(&anchor, &edited);

    // Never `Moved` — a blank line has no identity to follow — but its
    // number still exists, so the raw-number tier catches it.
    assert_eq!(line, Some(2));
    assert_eq!(confidence, Confidence::Weak);
}

/// The blank-line exclusion applies only to the `Moved` scan: a blank line
/// that has not moved still hashes the same at its original line number and
/// resolves `Exact`, same as any other unchanged line.
#[test]
fn blank_line_anchor_unmoved_resolves_exact() {
    let text = "a\n\nb\n";
    let anchor = create("f.txt", Side::Left, 2, text);

    let (line, confidence) = resolve(&anchor, text);

    assert_eq!(line, Some(2));
    assert_eq!(confidence, Confidence::Exact);
}

/// `create` past the end of the file records the `OUT_OF_RANGE_HASH`
/// sentinel, which cannot equal any real line's hash — including a blank
/// line's, which is exactly the value such an anchor would have collided
/// with under the old "hash of an empty string" fallback. Resolving against
/// text full of blank lines still gives up safely rather than fabricating a
/// match.
#[test]
fn create_past_eof_resolves_outdated_against_blank_lines() {
    let text = "a\nb\nc\n";
    let anchor = create("f.txt", Side::Left, 10, text);

    let other = "x\n\n\ny\n";
    let (line, confidence) = resolve(&anchor, other);

    assert_eq!(line, None);
    assert_eq!(confidence, Confidence::Outdated);
}
