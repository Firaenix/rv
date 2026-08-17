//! Property-based and parameterized coverage for [`rv_core::diff`], additive
//! to the exact-behavior cases in `tests/diff.rs`.
//!
//! The oracles here are deliberately independent of the module:
//! - [`fallback_lines`] recomputes "the lines of a file, as the fallback
//!   renders them" with a hand-written character scan, so the conservation
//!   properties compare the diff against the *input*, not against another
//!   call into `diff`. The module splits lines itself too (it diffs the lines
//!   it renders, not `similar`'s terminator-carrying tokens); these are two
//!   independent implementations of the same rule, and each checks the other.
//! - [`lcs_len`] is a textbook dynamic program, so the context-line count is
//!   checked against the mathematical definition of a minimal line diff.
//!
//! Two facts about the module drive how these tests are split:
//! - The two paths tokenize differently. The `similar` fallback treats `\r`,
//!   `\n` and `\r\n` all as line terminators and strips them; the difftastic
//!   path indexes `str::lines`, which only splits on `\n`. Properties that
//!   span both paths therefore use LF-only fixtures (see [`joined`]), where
//!   the two agree; the CR cases live in the fallback-only properties and in
//!   the case tables, which pin each path's actual behavior.
//! - difftastic's `chunks` are neither ordered by line number (a trailing
//!   insertion can be reported before an earlier edit) nor free of repeats
//!   (the same entry can arrive in two chunks). That is difftastic's business,
//!   but what a reviewer reads is `diff.lines` in `Vec` order, so putting the
//!   entries back into file order and dropping repeats is the module's
//!   business — asserted by `diff_lines_are_in_file_order_and_never_repeated`
//!   for whichever engine answered, and by the two exact-input cases in
//!   `tests/diff.rs` that name the inputs difft 0.70 mis-orders and repeats.
//!   No *line-count* bound is asserted on that path: how much difftastic
//!   chooses to report is still its own affair.
//!
//! Properties that pass `use_difft: true` spawn `difft`, exactly as the
//! existing `tests/diff.rs` cases do, and so share their requirement that
//! difftastic be on `PATH`. They run at low case counts for that reason.

use proptest::prelude::*;
use rstest::rstest;
use rv_core::diff::DiffSource;
use rv_core::diff::FileDiff;
use rv_core::diff::LineKind;
use rv_core::diff::compute;
use rv_core::diff::compute_with;

const EMPTY: &[u8] = &[];

/// Line *contents* — none of these contain a line terminator, so a fixture's
/// line structure is decided entirely by the terminators glued on afterwards.
/// Repeats and near-repeats are intentional: they create the alignment ties
/// that make [`context_line_count_is_the_lcs_length`] a real test.
const LINE_CONTENTS: [&str; 12] = [
    "",
    "a",
    "b",
    "a",
    "  indented",
    "let x = 1;",
    "@@ -1,2 +1,2 @@",
    "-minus",
    "+plus",
    "\ttab",
    "ünïcøde ☃",
    "}",
];

/// Paths with the shapes that matter to the module: an extension, none, a
/// nested directory, a dot-file, a trailing dot, non-ASCII.
const PATHS: [&str; 8] = [
    "notes.txt",
    "src/lib.rs",
    "no_extension",
    "dir/deep/file.md",
    "sp ace.py",
    ".hidden",
    "trailing.",
    "ünï.txt",
];

/// Big enough that any plausible "sniff the first N bytes" binary check would
/// have a window smaller than this. 40 KB was not: 64 KiB is a thoroughly
/// plausible window (git's own buffer is 8 KB, libmagic's is larger), and a
/// `bytes.iter().take(65536)` check would have survived every case.
const BIG_FILE_LEN: usize = 200_000;

// ---------------------------------------------------------------------------
// Oracles and helpers
// ---------------------------------------------------------------------------

/// The lines of `bytes` as the `similar` fallback renders them, recomputed
/// from scratch: lossily decode, then split on the terminators `similar`'s
/// line tokenizer recognizes (`\r\n`, `\n`, `\r`), dropping the empty segment
/// a final terminator leaves behind. Line text excludes the terminator.
///
/// That `\r` alone terminates a line is not obvious — it was established
/// empirically against `similar` 3.2 and is pinned by the
/// `bare_cr_is_a_line_terminator` case below, so this oracle drifting from
/// the crate's behavior cannot pass silently.
fn fallback_lines(bytes: &[u8]) -> Vec<String> {
    let text = String::from_utf8_lossy(bytes);
    let mut lines = Vec::new();
    let mut current = String::new();
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                lines.push(std::mem::take(&mut current));
            }
            '\n' => lines.push(std::mem::take(&mut current)),
            other => current.push(other),
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines
}

/// Whether `old` and `new` differ *only* in their line terminators: they
/// render to the same sequence of lines, yet their decoded text is not the
/// same. Computed from [`fallback_lines`] and a lossy decode, independently of
/// the module.
///
/// This is exactly the situation the fallback has no visible way to show — the
/// terminator is not part of any line's `text` — and so exactly the situation
/// it must report through `suppressed` instead of through `Added`/`Removed`
/// lines.
fn only_terminators_differ(old: &[u8], new: &[u8]) -> bool {
    fallback_lines(old) == fallback_lines(new)
        && String::from_utf8_lossy(old) != String::from_utf8_lossy(new)
}

/// Length of the longest common subsequence of `old` and `new` — the number
/// of lines a minimal line diff leaves unchanged.
fn lcs_len(old: &[&str], new: &[&str]) -> usize {
    let mut table = vec![vec![0usize; new.len() + 1]; old.len() + 1];
    for (i, o) in old.iter().enumerate() {
        for (j, n) in new.iter().enumerate() {
            table[i + 1][j + 1] = if o == n {
                table[i][j] + 1
            } else {
                table[i][j + 1].max(table[i + 1][j])
            };
        }
    }
    table[old.len()][new.len()]
}

/// `lines` as a file with every line LF-terminated: the one line-ending shape
/// where `str::lines` (which the difftastic path indexes) and `similar`'s
/// tokenizer agree exactly.
fn joined(lines: &[&str]) -> String {
    let mut text = String::new();
    for line in lines {
        text.push_str(line);
        text.push('\n');
    }
    text
}

fn owned(lines: &[&str]) -> Vec<String> {
    lines.iter().map(|line| (*line).to_owned()).collect()
}

/// Texts of the lines that exist on the base side (`Removed` + `Context`).
fn old_side_texts(diff: &FileDiff) -> Vec<String> {
    diff.lines
        .iter()
        .filter(|line| line.kind != LineKind::Added)
        .map(|line| line.text.clone())
        .collect()
}

/// Texts of the lines that exist on the head side (`Added` + `Context`).
fn new_side_texts(diff: &FileDiff) -> Vec<String> {
    diff.lines
        .iter()
        .filter(|line| line.kind != LineKind::Removed)
        .map(|line| line.text.clone())
        .collect()
}

fn old_side_numbers(diff: &FileDiff) -> Vec<Option<u32>> {
    diff.lines
        .iter()
        .filter(|line| line.kind != LineKind::Added)
        .map(|line| line.left)
        .collect()
}

fn new_side_numbers(diff: &FileDiff) -> Vec<Option<u32>> {
    diff.lines
        .iter()
        .filter(|line| line.kind != LineKind::Removed)
        .map(|line| line.right)
        .collect()
}

/// `[Some(1), Some(2), …, Some(count)]`.
fn one_based(count: usize) -> Vec<Option<u32>> {
    (1..=count)
        .map(|n| Some(u32::try_from(n).expect("fixtures are small")))
        .collect()
}

fn sigil(kind: LineKind) -> char {
    match kind {
        LineKind::Context => ' ',
        LineKind::Added => '+',
        LineKind::Removed => '-',
    }
}

fn number(value: Option<u32>) -> String {
    value.map_or_else(|| ".".to_owned(), |n| n.to_string())
}

/// A whole diff as one string per line, `<sigil><left>/<right> <text>` —
/// e.g. `-2/. old`, `+./2 new`, ` 1/1 same`. Compact enough that a case table
/// can spell out an entire expected diff inline.
fn render(diff: &FileDiff) -> Vec<String> {
    diff.lines
        .iter()
        .map(|line| {
            format!(
                "{}{}/{} {}",
                sigil(line.kind),
                number(line.left),
                number(line.right),
                line.text
            )
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Strategies
// ---------------------------------------------------------------------------

fn line_content() -> impl Strategy<Value = &'static str> {
    prop::sample::select(LINE_CONTENTS.to_vec())
}

fn path() -> impl Strategy<Value = &'static str> {
    prop::sample::select(PATHS.to_vec())
}

/// LF-terminated fixtures, returned as the line vector too so properties can
/// use it directly as the oracle rather than re-splitting the text.
fn lf_lines() -> impl Strategy<Value = Vec<&'static str>> {
    prop::collection::vec(line_content(), 0..12)
}

fn nonempty_lf_lines() -> impl Strategy<Value = Vec<&'static str>> {
    prop::collection::vec(line_content(), 1..12)
}

/// Text with a mix of `\n`, `\r\n` and bare `\r` terminators, and a 50/50
/// chance of no terminator at the end of the file.
fn mixed_text() -> impl Strategy<Value = String> {
    (
        prop::collection::vec(
            (
                line_content(),
                prop::sample::select(vec!["\n", "\r\n", "\r"]),
            ),
            0..10,
        ),
        any::<bool>(),
    )
        .prop_map(|(parts, trailing_terminator)| {
            let mut text = String::new();
            for (content, terminator) in &parts {
                text.push_str(content);
                text.push_str(terminator);
            }
            if !trailing_terminator && let Some((_, terminator)) = parts.last() {
                text.truncate(text.len() - terminator.len());
            }
            text
        })
}

/// One side of a diff, NUL-free (so it reaches a real diff rather than the
/// binary short-circuit) but not necessarily valid UTF-8 or line-structured.
fn nul_free_side() -> impl Strategy<Value = Vec<u8>> {
    prop_oneof![
        3 => mixed_text().prop_map(String::into_bytes),
        1 => prop::collection::vec(1u8..=255, 0..48),
    ]
}

/// One side of a diff, with the presence and position of a NUL as an explicit
/// dimension so both directions of the binary rule get equal airtime — half
/// of all sides contain a NUL, which can land anywhere including the very
/// first and very last byte.
///
/// The byte pool oversamples the bytes a sloppier "looks binary to me" check
/// might trip on — `0xff` (never valid UTF-8), `0x1b` (ESC), `0x7f` (DEL) —
/// so that the "and nothing else is binary" half of the rule is tested
/// against something, not just against lowercase letters.
fn side_with_optional_nul() -> impl Strategy<Value = Vec<u8>> {
    (
        prop::collection::vec(
            prop_oneof![
                4 => 1u8..=255,
                1 => Just(0xff),
                1 => Just(0x1b),
                1 => Just(0x7f),
                1 => Just(b'\n'),
            ],
            0..40,
        ),
        prop::option::of(0usize..=40),
    )
        .prop_map(|(mut bytes, nul_at)| {
            if let Some(at) = nul_at {
                let at = at.min(bytes.len());
                bytes.insert(at, 0);
            }
            bytes
        })
}

/// What happens to one base-side line on the head side. Weighted so most
/// lines survive: that is what makes a generated diff *mixed* — `Context`
/// lines interleaved with changed ones — which is the shape a numbering
/// off-by-one or a mis-sliced side actually shows up in.
#[derive(Clone, Debug)]
enum Edit {
    /// The line survives verbatim.
    Keep,
    /// The line survives, but with a different terminator — a difference no
    /// rendered line can show.
    Reterminate(&'static str),
    /// The line's content changes.
    Replace(&'static str),
    /// The line is not on the head side at all.
    Delete,
    /// The line survives and a brand-new line follows it.
    InsertAfter(&'static str),
}

fn edit() -> impl Strategy<Value = Edit> {
    prop_oneof![
        6 => Just(Edit::Keep),
        1 => terminator_style().prop_map(Edit::Reterminate),
        2 => line_content().prop_map(Edit::Replace),
        2 => Just(Edit::Delete),
        2 => line_content().prop_map(Edit::InsertAfter),
    ]
}

/// `parts` (content plus the terminator that follows it) as file bytes,
/// optionally dropping the very last terminator.
fn glue(parts: &[(&str, &str)], final_terminator: bool) -> Vec<u8> {
    let mut text = String::new();
    for (content, terminator) in parts {
        text.push_str(content);
        text.push_str(terminator);
    }
    if !final_terminator && let Some((_, terminator)) = parts.last() {
        text.truncate(text.len() - terminator.len());
    }
    text.into_bytes()
}

/// A base side, and a head side that is an *edit* of it: most lines kept, some
/// replaced, some deleted, some inserted, some only reterminated, and either
/// side's final terminator possibly missing.
///
/// Why not two independent draws (which is what this file did first): measured
/// against real `similar` 3.2 over 2000 independently drawn pairs, only 17.6%
/// produced any `Context` line at all and `Context` was 3.2% of every line the
/// properties inspected — two independent draws from a 12-string pool almost
/// never share an alignment worth keeping, so the diff is "whole file removed,
/// whole file added" and conservation is checked on the one shape where a
/// mis-sliced side or a wrong numbering counter cannot show. Deriving the head
/// side from the base side makes the interleaved shape the common case.
fn edited_pair() -> impl Strategy<Value = (Vec<u8>, Vec<u8>)> {
    (
        prop::collection::vec((line_content(), terminator_style(), edit()), 0..10),
        any::<bool>(),
        any::<bool>(),
    )
        .prop_map(|(parts, old_final, new_final)| {
            let mut old: Vec<(&str, &str)> = Vec::new();
            let mut new: Vec<(&str, &str)> = Vec::new();
            for (content, terminator, edit) in parts {
                old.push((content, terminator));
                match edit {
                    Edit::Keep => new.push((content, terminator)),
                    Edit::Reterminate(other) => new.push((content, other)),
                    Edit::Replace(other) => new.push((other, terminator)),
                    Edit::Delete => {}
                    Edit::InsertAfter(other) => {
                        new.push((content, terminator));
                        new.push((other, terminator));
                    }
                }
            }
            (glue(&old, old_final), glue(&new, new_final))
        })
}

/// The pair the fallback properties run on: usually an edit of the base side
/// (so a mixed diff is the common case), with a minority arm of two fully
/// independent draws so the shapes the edit arm cannot reach — an absent side,
/// a side that is raw non-UTF-8 bytes with no line structure at all — keep
/// their airtime.
fn mixed_pair() -> impl Strategy<Value = (Option<Vec<u8>>, Option<Vec<u8>>)> {
    prop_oneof![
        4 => edited_pair().prop_map(|(old, new)| (Some(old), Some(new))),
        1 => (
            prop::option::of(nul_free_side()),
            prop::option::of(nul_free_side()),
        ),
    ]
}

// ---------------------------------------------------------------------------
// Fallback path: conservation, numbering, shape
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]
    /// CONSERVATION. Reading the diff's old side (`Removed` + `Context`) top
    /// to bottom must reproduce the base file exactly — every line, in order,
    /// nothing invented, nothing lost — and those lines must be numbered
    /// `1..=n` with no gaps or repeats. Same for the head side (`Added` +
    /// `Context`). Together this says the diff is a faithful patch and that
    /// every line number points at the text it is printed next to.
    ///
    /// Driven by [`mixed_pair`], so the head side is usually an edit of the
    /// base side: the diffs this sees interleave `Context` with `Added` and
    /// `Removed`, rather than being the whole-file replacements two
    /// independent draws produce.
    #[test]
    fn fallback_reconstructs_both_sides((old, new) in mixed_pair()) {
        let diff = compute_with(old.as_deref(), new.as_deref(), "notes.txt", false);

        let want_old = fallback_lines(old.as_deref().unwrap_or(EMPTY));
        let want_new = fallback_lines(new.as_deref().unwrap_or(EMPTY));

        prop_assert_eq!(old_side_texts(&diff), want_old.clone(), "old side of {:#?}", diff);
        prop_assert_eq!(new_side_texts(&diff), want_new.clone(), "new side of {:#?}", diff);
        prop_assert_eq!(
            old_side_numbers(&diff),
            one_based(want_old.len()),
            "left numbering of {:#?}",
            diff
        );
        prop_assert_eq!(
            new_side_numbers(&diff),
            one_based(want_new.len()),
            "right numbering of {:#?}",
            diff
        );
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]
    /// On the fallback path a line's kind determines exactly which side's
    /// number it carries: an `Added` line exists only on the head side, a
    /// `Removed` line only on the base side, and `Context` on both. (The
    /// difftastic path deliberately differs — see
    /// `difft_lines_are_shaped_by_whichever_engine_answered`.)
    #[test]
    fn fallback_kind_determines_which_numbers_are_present((old, new) in mixed_pair()) {
        let diff = compute_with(old.as_deref(), new.as_deref(), "notes.txt", false);

        let mut problems = Vec::new();
        for line in &diff.lines {
            let present = (line.left.is_some(), line.right.is_some());
            let want = match line.kind {
                LineKind::Added => (false, true),
                LineKind::Removed => (true, false),
                LineKind::Context => (true, true),
            };
            if present != want {
                problems.push(format!("{line:?} has {present:?}, want {want:?}"));
            }
        }
        prop_assert!(problems.is_empty(), "{:?} in {:#?}", problems, diff);
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]
    /// No rendered line may carry a line terminator: the module strips them,
    /// and the TUI prints each `text` as one row. `\n` is impossible on
    /// either path; on the fallback path `\r` is a terminator too, so it is
    /// impossible as well.
    ///
    /// And the suppression rule, in both directions and over every input the
    /// generator can produce: the fallback sets `suppressed` exactly when the
    /// two sides differ *only* in their terminators (see
    /// [`only_terminators_differ`]) — the one difference it has no line to
    /// show. Never for a content change, never for two identical sides.
    ///
    /// Plus the totality checks that hold for any input: the path is echoed
    /// verbatim and no line is numberless.
    #[test]
    fn fallback_strips_terminators_and_flags_a_terminator_only_change(
        (old, new) in mixed_pair(),
        path in path(),
    ) {
        let diff = compute_with(old.as_deref(), new.as_deref(), path, false);

        prop_assert_eq!(&diff.source, &DiffSource::Similar, "{:#?}", diff);
        prop_assert_eq!(
            diff.suppressed,
            only_terminators_differ(old.as_deref().unwrap_or(EMPTY), new.as_deref().unwrap_or(EMPTY)),
            "suppressed is {} for {:?} vs {:?}",
            diff.suppressed,
            old.as_deref().map(String::from_utf8_lossy),
            new.as_deref().map(String::from_utf8_lossy)
        );
        prop_assert_eq!(diff.path.as_str(), path);

        let mut problems = Vec::new();
        for line in &diff.lines {
            if line.text.contains('\n') || line.text.contains('\r') {
                problems.push(format!("terminator in {line:?}"));
            }
            if line.left.is_none() && line.right.is_none() {
                problems.push(format!("no line number on {line:?}"));
            }
        }
        prop_assert!(problems.is_empty(), "{:?} in {:#?}", problems, diff);
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(200))]
    /// Identical sides are not a change: every line comes back as `Context`
    /// with the same number on both sides, and there is exactly one line per
    /// line of the file. (The fallback never suppresses; difftastic reports
    /// the same situation as `suppressed` instead — see
    /// `difft_reports_identical_sides_as_suppressed`.)
    #[test]
    fn identical_sides_produce_only_context(side in nul_free_side()) {
        let diff = compute_with(Some(&side), Some(&side), "same.txt", false);

        prop_assert!(!diff.suppressed, "{:#?}", diff);
        prop_assert_eq!(diff.lines.len(), fallback_lines(&side).len(), "{:#?}", diff);

        let mut problems = Vec::new();
        for line in &diff.lines {
            if line.kind != LineKind::Context {
                problems.push(format!("{line:?} is not Context"));
            }
            if line.left != line.right {
                problems.push(format!("{line:?} has mismatched numbers"));
            }
        }
        prop_assert!(problems.is_empty(), "{:?} in {:#?}", problems, diff);
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]
    /// A side that does not exist (`None`, a whole-file add or remove) and a
    /// side that exists but is empty are indistinguishable to the fallback:
    /// it has no file metadata to tell them apart, so it must not pretend to.
    #[test]
    fn an_absent_side_diffs_like_an_empty_one(
        side in nul_free_side(),
        on_the_old_side in any::<bool>(),
    ) {
        let (absent, empty) = if on_the_old_side {
            (
                compute_with(None, Some(&side), "p.txt", false),
                compute_with(Some(EMPTY), Some(&side), "p.txt", false),
            )
        } else {
            (
                compute_with(Some(&side), None, "p.txt", false),
                compute_with(Some(&side), Some(EMPTY), "p.txt", false),
            )
        };

        prop_assert_eq!(absent, empty);
    }
}

/// `lines` as a file whose terminators are all `style`, optionally dropping
/// the very last one (a file with no trailing newline).
fn styled(lines: &[&str], style: &str, final_terminator: bool) -> String {
    let mut text = String::new();
    for line in lines {
        text.push_str(line);
        text.push_str(style);
    }
    if !final_terminator {
        text.truncate(text.len() - style.len());
    }
    text
}

fn terminator_style() -> impl Strategy<Value = &'static str> {
    prop::sample::select(vec!["\n", "\r\n", "\r"])
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]
    /// What the reviewer sees must be what the diff claims. `DiffLine::text`
    /// is the whole of what gets displayed, so if both sides render to the
    /// same sequence of lines there is nothing for a reviewer to look at and
    /// the diff must not mark anything as `Added` or `Removed`: the whole file
    /// comes back as context, numbered `1..=n` on both sides.
    ///
    /// The bytes did change, though, and staying silent about that would trade
    /// a noisy diff for a lying one. So the second half of the property: the
    /// fallback must report the difference through `suppressed` — "there is a
    /// change here, it just has nothing in it a line can show" — exactly when
    /// the two sides' decoded text differs while their lines do not. That is
    /// the same answer difftastic gives for the same inputs (`status:
    /// "unchanged"`, see `difft_suppresses_semantically_unchanged_files`), so
    /// the two engines now agree about whether the file changed.
    ///
    /// This used to fail: the fallback diffed `similar`'s tokens, which
    /// *include* the line terminator, but rendered text with the terminator
    /// stripped, so a terminator-only change produced `Removed`/`Added` pairs
    /// whose displayed text was character-for-character identical.
    #[test]
    fn fallback_never_reports_a_change_that_renders_identically(
        lines in nonempty_lf_lines(),
        old_style in terminator_style(),
        new_style in terminator_style(),
        old_final in any::<bool>(),
        new_final in any::<bool>(),
    ) {
        // Dropping the final terminator after an empty last line would drop a
        // line rather than only a terminator, which is a real change.
        let last_is_empty = lines.last() == Some(&"");
        let old_text = styled(&lines, old_style, old_final || last_is_empty);
        let new_text = styled(&lines, new_style, new_final || last_is_empty);
        prop_assume!(fallback_lines(old_text.as_bytes()) == fallback_lines(new_text.as_bytes()));

        let diff = compute_with(
            Some(old_text.as_bytes()),
            Some(new_text.as_bytes()),
            "notes.txt",
            false,
        );

        // Every line of the file, as context, numbered on both sides — not a
        // single Added or Removed line, and not an empty diff either.
        let want: Vec<String> = fallback_lines(old_text.as_bytes())
            .iter()
            .enumerate()
            .map(|(index, text)| format!(" {n}/{n} {text}", n = index + 1))
            .collect();
        prop_assert_eq!(
            render(&diff),
            want,
            "{:?} and {:?} both render as {:?}, yet the diff reads {:#?}",
            old_text,
            new_text,
            fallback_lines(old_text.as_bytes()),
            diff
        );

        // ... and the invisible difference is still reported, as suppression.
        prop_assert_eq!(
            diff.suppressed,
            old_text != new_text,
            "{:?} vs {:?}: suppressed is {}",
            old_text,
            new_text,
            diff.suppressed
        );
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]
    /// MINIMALITY, against an independent dynamic program: the number of
    /// `Context` lines is the length of the longest common subsequence of the
    /// two files' lines. A diff that rewrote more than it had to (say, one
    /// that reported every line as removed-then-added) would keep fewer lines
    /// as context and fail here, even though it would still conserve every
    /// line.
    #[test]
    fn context_line_count_is_the_lcs_length(old in lf_lines(), new in lf_lines()) {
        let old_text = joined(&old);
        let new_text = joined(&new);
        let diff =
            compute_with(Some(old_text.as_bytes()), Some(new_text.as_bytes()), "x.txt", false);

        let context = diff
            .lines
            .iter()
            .filter(|line| line.kind == LineKind::Context)
            .count();
        prop_assert_eq!(context, lcs_len(&old, &new), "{:?} vs {:?} -> {:#?}", old, new, diff);
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]
    /// `FileDiff` is `Serialize + Deserialize`; the pair must round-trip, so
    /// a diff can be written out and read back without losing a kind, a line
    /// number or a line of text.
    #[test]
    fn a_file_diff_survives_a_json_round_trip((old, new) in mixed_pair()) {
        let diff = compute_with(old.as_deref(), new.as_deref(), "round.txt", false);

        let json = serde_json::to_string(&diff).expect("FileDiff serializes");
        let back: FileDiff = serde_json::from_str(&json).expect("FileDiff deserializes");

        prop_assert_eq!(back, diff);
    }
}

// ---------------------------------------------------------------------------
// The binary rule
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]
    /// The binary rule in both directions: a NUL on either side means
    /// `Binary` with no lines and no suppression, and the absence of a NUL on
    /// both sides means the file *is* diffed. Runs with difftastic both
    /// enabled and disabled, since the check precedes it — and for binary
    /// input the environment-reading [`compute`] must agree with
    /// `compute_with`, because neither engine is reached.
    ///
    /// Note what that last clause does *not* cover: `RV_NO_DIFFT` is read
    /// under the ambient environment and is never set, cleared or varied here,
    /// so this pins that `compute` agrees with `compute_with` for binary input
    /// under whatever environment the suite happens to run in — not that the
    /// variable's documented effect works. That needs a test which mutates
    /// process-global state, and there is none in this crate.
    #[test]
    fn a_nul_on_either_side_means_binary_and_nothing_else_does(
        old in prop::option::of(side_with_optional_nul()),
        new in prop::option::of(side_with_optional_nul()),
        path in path(),
        use_difft in any::<bool>(),
    ) {
        let diff = compute_with(old.as_deref(), new.as_deref(), path, use_difft);
        prop_assert_eq!(diff.path.as_str(), path);

        let has_nul = |side: &Option<Vec<u8>>| {
            side.as_ref().is_some_and(|bytes| bytes.contains(&0))
        };
        if has_nul(&old) || has_nul(&new) {
            prop_assert_eq!(&diff.source, &DiffSource::Binary, "{:#?}", diff);
            prop_assert!(diff.lines.is_empty(), "{:#?}", diff);
            prop_assert!(!diff.suppressed, "{:#?}", diff);
            prop_assert_eq!(compute(old.as_deref(), new.as_deref(), path), diff);
        } else {
            prop_assert_ne!(&diff.source, &DiffSource::Binary, "{:#?}", diff);
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]
    /// The binary check reads the whole side, not a sniff window: a single
    /// NUL anywhere in a 40 KB file — first byte, last byte, anywhere between
    /// — still makes the file binary. A check that only inspected a prefix
    /// would try to diff a binary blob as text.
    #[test]
    fn a_nul_anywhere_in_a_large_file_is_still_binary(
        index in 0usize..BIG_FILE_LEN,
        on_the_old_side in any::<bool>(),
        use_difft in any::<bool>(),
    ) {
        let mut side = vec![b'a'; BIG_FILE_LEN];
        side[index] = 0;
        let text: &[u8] = b"a\n";

        let diff = if on_the_old_side {
            compute_with(Some(&side), Some(text), "big.bin", use_difft)
        } else {
            compute_with(Some(text), Some(&side), "big.bin", use_difft)
        };

        prop_assert_eq!(
            &diff.source,
            &DiffSource::Binary,
            "NUL at byte {} of {} escaped the binary check",
            index,
            BIG_FILE_LEN
        );
        prop_assert!(diff.lines.is_empty(), "{:#?}", diff);
    }
}

// ---------------------------------------------------------------------------
// difftastic path (spawns `difft`; low case counts)
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]
    /// A whole-file creation goes through difftastic's `created` status,
    /// which carries no chunks. The module builds the lines itself and must
    /// still label the result `Difftastic` (difftastic did answer), add every
    /// line of the new file in order, number them `1..=n` on the right, and
    /// leave the left side empty.
    #[test]
    fn difft_labels_a_whole_file_creation_and_adds_every_line(lines in nonempty_lf_lines()) {
        let text = joined(&lines);
        let diff = compute_with(None, Some(text.as_bytes()), "created.txt", true);

        prop_assert!(
            matches!(diff.source, DiffSource::Difftastic { .. }),
            "{:?} for {:?}",
            diff.source,
            text
        );
        prop_assert!(!diff.suppressed, "{:#?}", diff);
        prop_assert_eq!(new_side_texts(&diff), owned(&lines), "{:#?}", diff);
        prop_assert_eq!(new_side_numbers(&diff), one_based(lines.len()), "{:#?}", diff);

        let mut problems = Vec::new();
        for line in &diff.lines {
            if line.kind != LineKind::Added || line.left.is_some() {
                problems.push(format!("{line:?}"));
            }
        }
        prop_assert!(problems.is_empty(), "{:?} in {:#?}", problems, diff);
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]
    /// The mirror image: a whole-file deletion is difftastic's `deleted`
    /// status, also chunk-less, also `Difftastic`-labelled, with every line
    /// of the old file removed in order and numbered `1..=n` on the left.
    #[test]
    fn difft_labels_a_whole_file_deletion_and_removes_every_line(lines in nonempty_lf_lines()) {
        let text = joined(&lines);
        let diff = compute_with(Some(text.as_bytes()), None, "deleted.txt", true);

        prop_assert!(
            matches!(diff.source, DiffSource::Difftastic { .. }),
            "{:?} for {:?}",
            diff.source,
            text
        );
        prop_assert!(!diff.suppressed, "{:#?}", diff);
        prop_assert_eq!(old_side_texts(&diff), owned(&lines), "{:#?}", diff);
        prop_assert_eq!(old_side_numbers(&diff), one_based(lines.len()), "{:#?}", diff);

        let mut problems = Vec::new();
        for line in &diff.lines {
            if line.kind != LineKind::Removed || line.right.is_some() {
                problems.push(format!("{line:?}"));
            }
        }
        prop_assert!(problems.is_empty(), "{:?} in {:#?}", problems, diff);
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(96))]
    /// A line's own side's number must index the text that line carries: a
    /// `Removed` line's `left` points at that text in the base file, an
    /// `Added` line's `right` at that text in the head file, a `Context`
    /// line's numbers at that same text in *both*, and no number may point
    /// past the end of its file.
    ///
    /// This runs with difftastic enabled and holds whichever engine answers,
    /// which is why it does not care that a `Context` line can only have come
    /// from the fallback (see `difft_lines_are_shaped_by_whichever_engine_
    /// answered` for the per-engine shapes).
    ///
    /// Note what this deliberately does *not* claim. For an aligned pair the
    /// module sets *both* numbers on *both* lines, so a `Removed` line's
    /// `right` is its counterpart's number, not a pointer to its own text —
    /// asserting otherwise would be asserting a bug into the suite.
    #[test]
    fn line_numbers_index_the_text_they_carry(old in lf_lines(), new in lf_lines()) {
        let old_text = joined(&old);
        let new_text = joined(&new);
        let diff =
            compute_with(Some(old_text.as_bytes()), Some(new_text.as_bytes()), "x.txt", true);

        // Which numbers must index this line's own text: a Context line is
        // claiming the same text exists at both numbers.
        let sides = |line: &rv_core::diff::DiffLine| match line.kind {
            LineKind::Removed => vec![(line.left, &old)],
            LineKind::Added => vec![(line.right, &new)],
            LineKind::Context => vec![(line.left, &old), (line.right, &new)],
        };

        let mut problems = Vec::new();
        for line in &diff.lines {
            for (number, want) in sides(line) {
                match number {
                    None => problems.push(format!("{line:?} carries no number for its own side")),
                    Some(number) => {
                        let index = usize::try_from(number).expect("fixtures are small") - 1;
                        match want.get(index) {
                            None => {
                                problems.push(format!("{line:?} points past the end of its file"));
                            }
                            Some(text) if *text != line.text => {
                                problems.push(format!("{line:?} should read {text:?}"));
                            }
                            Some(_) => {}
                        }
                    }
                }
            }
        }
        prop_assert!(problems.is_empty(), "{:?} in {:#?}", problems, diff);
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(96))]
    /// FILE ORDER and NO REPEATS, whichever engine answered. The TUI windows
    /// `diff.lines` in `Vec` order (`rv/src/ui.rs`), so the order of this
    /// vector *is* the order a reviewer reads the file in:
    ///
    /// - Reading top to bottom must walk each file forward: the base-side
    ///   numbers that are present never go backwards, and neither do the
    ///   head-side ones.
    /// - No line of either file may be shown twice: no two `Removed` lines
    ///   share a `left`, no two `Added` lines share a `right`.
    ///
    /// Neither held on the difftastic path before: difftastic's `chunks` are
    /// not ordered by line number (a trailing insertion can be reported before
    /// an earlier edit) and difftastic can report the same entry in two
    /// chunks, both of which the module used to forward verbatim — see
    /// `difftastic_chunks_reported_out_of_order_are_shown_in_file_order` and
    /// `a_change_difftastic_reports_in_two_chunks_is_shown_once` in
    /// `tests/diff.rs` for the exact inputs. The module now merges the chunk
    /// entries into file order and drops entries it has already emitted.
    ///
    /// The fallback path satisfies both by construction, so this holds for
    /// whichever engine answers and no input is skipped.
    #[test]
    fn diff_lines_are_in_file_order_and_never_repeated(old in lf_lines(), new in lf_lines()) {
        let old_text = joined(&old);
        let new_text = joined(&new);
        let diff =
            compute_with(Some(old_text.as_bytes()), Some(new_text.as_bytes()), "x.txt", true);

        let mut problems = Vec::new();
        let mut last_left = 0;
        let mut last_right = 0;
        let mut removed_lefts: Vec<u32> = Vec::new();
        let mut added_rights: Vec<u32> = Vec::new();
        for line in &diff.lines {
            if let Some(left) = line.left {
                if left < last_left {
                    problems.push(format!("{line:?} goes back to base line {left} after {last_left}"));
                }
                last_left = left;
            }
            if let Some(right) = line.right {
                if right < last_right {
                    problems.push(format!("{line:?} goes back to head line {right} after {last_right}"));
                }
                last_right = right;
            }
            match line.kind {
                LineKind::Removed => {
                    if let Some(left) = line.left {
                        if removed_lefts.contains(&left) {
                            problems.push(format!("{line:?} shows base line {left} twice"));
                        }
                        removed_lefts.push(left);
                    }
                }
                LineKind::Added => {
                    if let Some(right) = line.right {
                        if added_rights.contains(&right) {
                            problems.push(format!("{line:?} shows head line {right} twice"));
                        }
                        added_rights.push(right);
                    }
                }
                LineKind::Context => {}
            }
        }
        prop_assert!(problems.is_empty(), "{:?} in {:#?}", problems, diff);
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(96))]
    /// Each engine's line shape, and the fact that the answer always follows
    /// the shape of whichever engine produced it. Both branches assert, so no
    /// input is skipped:
    ///
    /// - difftastic reports only what changed, so it never emits `Context`,
    ///   and a line either carries exactly one number (a one-sided insertion
    ///   or deletion) or is half of an aligned pair — in which case the pair
    ///   is *exactly* two lines, `Removed` immediately followed by `Added`
    ///   with the identical pair of numbers, so a reviewer never sees one half
    ///   of an inline change on its own, and never sees the pair twice.
    ///   Requiring the run to be exactly two is what makes this more than a
    ///   restatement of the code's control flow: mere adjacency is also
    ///   satisfied by `[Removed, Removed, Added, Added]` carrying one number
    ///   pair, which is what the duplicated-chunk defect used to produce.
    /// - `Similar` here means difftastic was tried and did not answer: it is
    ///   absent, it crashed (difftastic 0.70 panics outright on some inputs —
    ///   `["  indented", "", "let x = 1;", "", "a"]` against
    ///   `["  indented", "-minus", "-minus", "-minus", "a"]` is one), or it
    ///   emitted a shape this module would not parse. The module's contract
    ///   is that this degrades to a well-formed `similar` diff rather than
    ///   failing, so the fallback's kind/number rule must hold instead.
    #[test]
    fn difft_lines_are_shaped_by_whichever_engine_answered(
        old in lf_lines(),
        new in lf_lines(),
    ) {
        let old_text = joined(&old);
        let new_text = joined(&new);
        let diff =
            compute_with(Some(old_text.as_bytes()), Some(new_text.as_bytes()), "x.txt", true);

        let mut problems = Vec::new();
        match diff.source {
            DiffSource::Binary => problems.push("NUL-free text reported as binary".to_owned()),
            DiffSource::Similar => {
                for line in &diff.lines {
                    let present = (line.left.is_some(), line.right.is_some());
                    let want = match line.kind {
                        LineKind::Added => (false, true),
                        LineKind::Removed => (true, false),
                        LineKind::Context => (true, true),
                    };
                    if present != want {
                        problems.push(format!("{line:?} has {present:?}, want {want:?}"));
                    }
                }
            }
            DiffSource::Difftastic { .. } => {
                for (index, line) in diff.lines.iter().enumerate() {
                    if line.kind == LineKind::Context {
                        problems.push(format!("Context line from difftastic: {line:?}"));
                        continue;
                    }
                    match (line.left, line.right) {
                        (None, None) => {
                            problems.push(format!("{line:?} carries no numbers at all"));
                        }
                        (Some(_), None) | (None, Some(_)) => {}
                        (Some(_), Some(_)) => {
                            // The neighbour that must be this line's partner,
                            // and the one that must *not* also carry these
                            // numbers — that would make the run longer than
                            // the pair it claims to be.
                            let (partner, outer, want_kind) = if line.kind == LineKind::Removed {
                                (
                                    diff.lines.get(index + 1),
                                    index.checked_sub(1).and_then(|i| diff.lines.get(i)),
                                    LineKind::Added,
                                )
                            } else {
                                (
                                    index.checked_sub(1).and_then(|i| diff.lines.get(i)),
                                    diff.lines.get(index + 1),
                                    LineKind::Removed,
                                )
                            };
                            let matched = partner.is_some_and(|partner| {
                                partner.kind == want_kind
                                    && partner.left == line.left
                                    && partner.right == line.right
                            });
                            if !matched {
                                problems
                                    .push(format!("{line:?} has no aligned partner: {partner:?}"));
                            }
                            let run_is_longer = outer.is_some_and(|outer| {
                                outer.left == line.left && outer.right == line.right
                            });
                            if run_is_longer {
                                problems.push(format!(
                                    "{line:?} is in a run of more than two: {outer:?}"
                                ));
                            }
                        }
                    }
                }
            }
        }
        prop_assert!(problems.is_empty(), "{:?} in {:#?}", problems, diff);
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]
    /// difftastic can tell that nothing changed, and the module must forward
    /// that as `suppressed` with no lines — not as an empty-by-accident diff.
    /// Identical sides are the simplest instance of it.
    #[test]
    fn difft_reports_identical_sides_as_suppressed(lines in nonempty_lf_lines()) {
        let text = joined(&lines);
        let diff = compute_with(Some(text.as_bytes()), Some(text.as_bytes()), "same.txt", true);

        prop_assert!(
            matches!(diff.source, DiffSource::Difftastic { .. }),
            "{:?} for {:?}",
            diff.source,
            text
        );
        prop_assert!(diff.suppressed, "{:#?}", diff);
        prop_assert!(diff.lines.is_empty(), "{:#?}", diff);
    }
}

// ---------------------------------------------------------------------------
// Case tables
// ---------------------------------------------------------------------------

/// Exact fallback renderings for the edge shapes a reviewer actually hits.
/// Hermetic: `use_difft: false` never spawns a process.
///
/// Read the expectations as `<sigil><left>/<right> <text>`, with `.` for an
/// absent line number.
#[rstest]
// Nothing on either side: no lines, not an error.
#[case::absent_vs_absent(None, None, &[])]
#[case::empty_vs_empty(Some(""), Some(""), &[])]
// A whole-file add and a whole-file remove, as the fallback sees them.
#[case::empty_vs_content(Some(""), Some("x\ny\n"), &["+./1 x", "+./2 y"])]
#[case::content_vs_empty(Some("x\ny\n"), Some(""), &["-1/. x", "-2/. y"])]
// One-line files.
#[case::one_line_changed(Some("a\n"), Some("b\n"), &["-1/. a", "+./1 b"])]
#[case::one_line_unchanged(Some("a\n"), Some("a\n"), &[" 1/1 a"])]
// The fallback diffs the lines as it renders them, terminators stripped, so a
// terminator change alongside a real edit shows only the real edit — including
// when the two sides do not even agree on the terminator style. (A
// terminator-only change, with no edit to show, is
// `a_terminator_only_change_renders_as_context_and_is_suppressed` below.)
#[case::missing_trailing_newline_with_edit(
    Some("a\nb"),
    Some("a\nc"),
    &[" 1/1 a", "-2/. b", "+./2 c"]
)]
#[case::crlf_with_edit(
    Some("a\r\nb\r\n"),
    Some("a\r\nc\r\n"),
    &[" 1/1 a", "-2/. b", "+./2 c"]
)]
#[case::an_edit_and_a_terminator_change_at_once(
    Some("a\r\nb\r\n"),
    Some("a\nc\n"),
    &[" 1/1 a", "-2/. b", "+./2 c"]
)]
#[case::an_edit_and_a_final_newline_appearing(
    Some("a\nb"),
    Some("a\nc\n"),
    &[" 1/1 a", "-2/. b", "+./2 c"]
)]
// `similar` treats a bare CR as a line terminator too, so an old Mac-style
// file is split into lines rather than read as one long line. This is the
// fact `fallback_lines` (the conservation oracle) depends on.
#[case::bare_cr_is_a_line_terminator(
    Some("a\rb\r"),
    Some("a\rb\rc\r"),
    &[" 1/1 a", " 2/2 b", "+./3 c"]
)]
// A file that is nothing but blank lines still has lines, and they are
// numbered.
#[case::all_blank_lines(Some("\n\n\n"), Some("\n\n"), &[" 1/1 ", " 2/2 ", "-3/. "])]
// Content that looks like diff syntax is data, not syntax.
#[case::text_that_looks_like_a_diff(
    Some("@@ -1 +1 @@\n-x\n"),
    Some("@@ -1 +1 @@\n+x\n"),
    &[" 1/1 @@ -1 +1 @@", "-2/. -x", "+./2 +x"]
)]
fn fallback_renderings(
    #[case] old: Option<&str>,
    #[case] new: Option<&str>,
    #[case] expected: &[&str],
) {
    let diff = compute_with(
        old.map(str::as_bytes),
        new.map(str::as_bytes),
        "notes.txt",
        false,
    );

    assert_eq!(diff.source, DiffSource::Similar, "{diff:?}");
    assert!(!diff.suppressed, "{diff:?}");
    assert_eq!(render(&diff), owned(expected), "{diff:#?}");
}

/// A difference that lives entirely in the line terminators — a final newline
/// appearing or vanishing, CRLF becoming LF, one line reterminated in the
/// middle of a file — is a real difference in the bytes but not one any line
/// of the diff can show, since `text` is rendered without the terminator. The
/// fallback therefore renders the file as context and reports the difference
/// through `suppressed`, which is also what difftastic answers for the same
/// inputs (`difft_suppresses_semantically_unchanged_files`).
///
/// Hermetic: `use_difft: false` never spawns a process.
#[rstest]
#[case::a_final_newline_appears(Some("a"), Some("a\n"), &[" 1/1 a"])]
#[case::a_final_newline_vanishes(Some("a\n"), Some("a"), &[" 1/1 a"])]
#[case::crlf_to_lf(Some("a\r\nb\r\n"), Some("a\nb\n"), &[" 1/1 a", " 2/2 b"])]
#[case::lf_to_crlf(Some("a\nb\n"), Some("a\r\nb\r\n"), &[" 1/1 a", " 2/2 b"])]
#[case::lf_to_bare_cr(Some("a\nb\n"), Some("a\rb\r"), &[" 1/1 a", " 2/2 b"])]
#[case::one_line_of_three_reterminated(
    Some("a\nb\nc\n"),
    Some("a\r\nb\nc\n"),
    &[" 1/1 a", " 2/2 b", " 3/3 c"]
)]
#[case::a_blank_line_reterminated(Some("\n\n"), Some("\r\n\r\n"), &[" 1/1 ", " 2/2 "])]
fn a_terminator_only_change_renders_as_context_and_is_suppressed(
    #[case] old: Option<&str>,
    #[case] new: Option<&str>,
    #[case] expected: &[&str],
) {
    let diff = compute_with(
        old.map(str::as_bytes),
        new.map(str::as_bytes),
        "notes.txt",
        false,
    );

    assert_eq!(diff.source, DiffSource::Similar, "{diff:?}");
    assert_eq!(render(&diff), owned(expected), "{diff:#?}");
    assert!(
        diff.suppressed,
        "a terminator-only change must still be reported: {diff:#?}"
    );
}

/// The counterweight to the table above: identical sides, and sides that
/// differ in content, must *not* be suppressed — suppression means "something
/// changed that no line can show", not "nothing changed" and not "look away".
#[rstest]
#[case::identical(Some("a\nb\n"), Some("a\nb\n"))]
#[case::identical_without_a_final_newline(Some("a\nb"), Some("a\nb"))]
#[case::both_absent(None, None)]
#[case::absent_versus_empty(None, Some(""))]
#[case::a_content_change(Some("a\nb\n"), Some("a\nc\n"))]
#[case::a_content_change_with_a_terminator_change(Some("a\r\nb\r\n"), Some("a\nc\n"))]
#[case::a_line_added(Some("a\n"), Some("a\nb\n"))]
#[case::a_line_added_and_the_rest_reterminated(Some("a\n"), Some("a\r\nb\r\n"))]
fn only_a_terminator_only_change_is_suppressed(
    #[case] old: Option<&str>,
    #[case] new: Option<&str>,
) {
    let diff = compute_with(
        old.map(str::as_bytes),
        new.map(str::as_bytes),
        "notes.txt",
        false,
    );

    assert!(!diff.suppressed, "{diff:#?}");
}

/// The binary rule, one case per direction, with a NUL at each interesting
/// position — and a control that invalid UTF-8 alone is *not* binary.
/// Hermetic for the binary cases (they never reach difftastic) and run with
/// difftastic disabled for the control.
#[rstest]
#[case::nul_first_byte(Some(b"\0abc".as_slice()), Some(b"abc".as_slice()), true)]
#[case::nul_last_byte(Some(b"abc\0".as_slice()), Some(b"abc".as_slice()), true)]
#[case::nul_in_the_middle(Some(b"ab\0c".as_slice()), Some(b"abc".as_slice()), true)]
#[case::nul_on_the_new_side_only(Some(b"abc\n".as_slice()), Some(b"a\0c\n".as_slice()), true)]
#[case::nul_on_both_sides(Some(b"\0".as_slice()), Some(b"\0".as_slice()), true)]
#[case::nul_with_an_absent_other_side(None, Some(b"\0".as_slice()), true)]
#[case::lone_nul_is_the_whole_file(Some(b"".as_slice()), Some(b"\0".as_slice()), true)]
// Invalid UTF-8 without a NUL is text as far as this module is concerned: it
// decodes lossily and diffs.
#[case::invalid_utf8_without_a_nul(
    Some(b"\xff\xfe\n".as_slice()),
    Some(b"\xff\n".as_slice()),
    false
)]
#[case::high_bytes_only(Some(b"\x80\x81".as_slice()), Some(b"\x80\x82".as_slice()), false)]
fn binary_rule(#[case] old: Option<&[u8]>, #[case] new: Option<&[u8]>, #[case] binary: bool) {
    let diff = compute_with(old, new, "blob.bin", false);

    if binary {
        assert_eq!(diff.source, DiffSource::Binary, "{diff:#?}");
        assert!(diff.lines.is_empty(), "{diff:#?}");
        assert!(!diff.suppressed, "{diff:#?}");
    } else {
        assert_eq!(diff.source, DiffSource::Similar, "{diff:#?}");
        assert!(!diff.lines.is_empty(), "{diff:#?}");
    }
}

/// A very long line is one line, not a truncation or a panic.
#[test]
fn a_very_long_line_is_one_line() {
    let old = format!("{}\n", "x".repeat(200_000));
    let new = format!("{}\n", "y".repeat(200_000));

    let diff = compute_with(
        Some(old.as_bytes()),
        Some(new.as_bytes()),
        "long.txt",
        false,
    );

    assert_eq!(render(&diff).len(), 2, "{:?}", diff.lines.len());
    assert_eq!(diff.lines[0].text.len(), 200_000);
    assert_eq!(diff.lines[0].left, Some(1));
    assert_eq!(diff.lines[1].text.len(), 200_000);
    assert_eq!(diff.lines[1].right, Some(1));
}

/// difftastic detects a language from the file *extension* (the module
/// mirrors the real path's extension onto its temp files), so the same
/// content reports a different language depending on the path — including the
/// no-extension case, where the module passes an empty suffix and difftastic
/// falls back to `Text`.
///
/// Needs `difft` on `PATH`, like the existing `tests/diff.rs` cases.
#[rstest]
#[case::rust_extension("a.rs", "Rust")]
#[case::rust_extension_in_a_nested_path("dir/deep/a.rs", "Rust")]
#[case::text_extension("a.txt", "Text")]
#[case::no_extension("a", "Text")]
fn difft_language_follows_the_path_extension(#[case] path: &str, #[case] language: &str) {
    let old = b"fn a() {\n    let x = 1;\n}\n";
    let new = b"fn a() {\n    let x = 2;\n}\n";

    let diff = compute_with(Some(old), Some(new), path, true);

    assert_eq!(
        diff.source,
        DiffSource::Difftastic {
            language: language.to_owned()
        },
        "{diff:#?}"
    );
    assert_eq!(
        render(&diff),
        owned(&["-2/2     let x = 1;", "+2/2     let x = 2;"]),
        "{diff:#?}"
    );
}

/// Changes difftastic considers no change at all: it reports `unchanged`, and
/// the module forwards that as `suppressed` rather than showing a reviewer a
/// diff. Needs `difft` on `PATH`.
#[rstest]
#[case::both_sides_empty(Some(""), Some(""))]
#[case::trailing_newline_added(Some("a"), Some("a\n"))]
#[case::lf_to_crlf(Some("a\nb\n"), Some("a\r\nb\r\n"))]
#[case::reindented(
    Some("fn a() {\nlet x = 1;\n}\n"),
    Some("fn a() {\n    let x = 1;\n}\n")
)]
fn difft_suppresses_semantically_unchanged_files(
    #[case] old: Option<&str>,
    #[case] new: Option<&str>,
) {
    let diff = compute_with(old.map(str::as_bytes), new.map(str::as_bytes), "a.rs", true);

    assert!(
        matches!(diff.source, DiffSource::Difftastic { .. }),
        "{diff:#?}"
    );
    assert!(diff.suppressed, "{diff:#?}");
    assert!(diff.lines.is_empty(), "{diff:#?}");
}
