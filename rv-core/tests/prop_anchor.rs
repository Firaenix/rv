//! Property and parameterized tests for `rv_core::anchor`.
//!
//! The hand-written cases in `tests/anchor.rs` pin one example per documented
//! behaviour. This file goes after the *contract*: the invariants an anchor has
//! to keep for the tool to be trustworthy across a history rewrite, expressed
//! against oracles that are independent of the implementation (a char-by-char
//! reimplementation of normalization, line arithmetic recomputed from the
//! generated edit, and conservation/bound laws) rather than against a copy of
//! the code under test.

use proptest::prelude::*;
use rstest::rstest;
use rv_core::anchor::content_hash;
use rv_core::anchor::create;
use rv_core::anchor::normalize;
use rv_core::anchor::resolve;
use rv_core::anchor::snapshot_of;
use rv_core::model::Confidence;
use rv_core::model::Side;

// ---------------------------------------------------------------------------
// Oracles
// ---------------------------------------------------------------------------

/// Independent reimplementation of the normalization `anchor` documents: every
/// run of Unicode whitespace collapses to a single space, and leading and
/// trailing whitespace disappears. Deliberately written as a char loop instead
/// of `split_whitespace().collect().join(" ")` so that it is a real oracle for
/// [`normalize`] and not a second copy of it.
fn ref_normalize(line: &str) -> String {
    let mut out = String::new();
    let mut gap = false;
    for ch in line.chars() {
        if ch.is_whitespace() {
            gap = !out.is_empty();
        } else {
            if gap {
                out.push(' ');
                gap = false;
            }
            out.push(ch);
        }
    }
    out
}

/// The non-whitespace runs of `line`, in order — the part of a line that
/// normalization is required to preserve verbatim. Used to *build* whitespace
/// reshapings, so the reshaping generator does not depend on the function whose
/// invariance under reshaping is being tested.
fn ref_tokens(line: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    for ch in line.chars() {
        if ch.is_whitespace() {
            if !current.is_empty() {
                tokens.push(std::mem::take(&mut current));
            }
        } else {
            current.push(ch);
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

// ---------------------------------------------------------------------------
// Generators
// ---------------------------------------------------------------------------

/// Appended as the last line of every generated text. `["a", ""].join("\n")`
/// is `"a\n"`, whose `lines()` is `["a"]` — a trailing empty line does not
/// survive the round trip — so texts always end in a non-blank line, which
/// makes `text_of(&lines).lines() == lines` hold exactly and lets a property
/// talk about "line n" without guessing. Contains characters the line
/// generator cannot produce, so it never collides with generated content.
const SENTINEL: &str = "END-OF-TEXT";

/// Characters a generated line is built from: ASCII code-ish punctuation, a
/// couple of letters and digits so duplicate lines are reachable, plus
/// whitespace, a combining mark and an emoji so normalization is exercised on
/// multi-byte and zero-width-ish content. No `\n` or `\r`, so a line is always
/// exactly one line.
///
/// The whitespace here is deliberately not just ASCII. `normalize`'s contract is
/// stated over *Unicode* whitespace, and the cheapest way to lose that half of
/// it is to reimplement the split as `is_ascii_whitespace() || c == '\u{a0}'` —
/// which every property in this file would pass if the only whitespace the
/// generators emitted were space, tab, NBSP and CR. So the pool also carries
/// U+000B (vertical tab: `char::is_whitespace`, but *not*
/// `char::is_ascii_whitespace`), U+2028 (line separator — whitespace, yet not a
/// break for `str::lines`, so it stays inside one line) and U+3000 (ideographic
/// space, three bytes wide).
fn line_char() -> impl Strategy<Value = char> {
    prop::sample::select(vec![
        'a', 'b', 'Z', '0', '1', ' ', ' ', '\t', '{', '}', '(', ')', ';', '=', '_', 'é', '→', '😀',
        '\u{0301}', '\u{00a0}', '\u{000b}', '\u{2028}', '\u{3000}',
    ])
}

fn line() -> impl Strategy<Value = String> {
    prop::collection::vec(line_char(), 0..10).prop_map(|chars| chars.into_iter().collect())
}

/// A non-empty run of whitespace, for reshaping indentation. Deliberately long
/// and strongly biased towards the plain space: a *partial* collapse (one pass of
/// "replace two spaces with one", say) only gives itself away on a run of three
/// or more spaces, and drawing uniformly from a three-character pool of runs of
/// length 1..4 produces one of those about once in a hundred draws — rarely
/// enough that a property about repeated normalization would look non-vacuous
/// while actually depending on luck.
///
/// The two exotic characters carry the Unicode half of the contract into the
/// reshaping properties as well (see [`line_char`]). Their weights are small and
/// the other three were scaled up to match, so the plain space keeps exactly the
/// 3-in-4 share the partial-collapse argument above depends on.
fn ws_run() -> impl Strategy<Value = String> {
    prop::collection::vec(
        prop_oneof![
            24 => Just(' '),
            3 => Just('\t'),
            3 => Just('\u{00a0}'),
            1 => Just('\u{000b}'),
            1 => Just('\u{3000}'),
        ],
        1..7,
    )
    .prop_map(|chars| chars.into_iter().collect())
}

/// Whitespace or nothing: what may legally sit at the start or end of a line
/// without changing its normalized content.
fn opt_ws() -> impl Strategy<Value = String> {
    prop_oneof![Just(String::new()), ws_run()]
}

/// A line with a whitespace run spliced into the middle and optional
/// indentation, for the properties that are about whitespace handling itself.
/// [`line`] draws characters independently, so it reaches long whitespace runs
/// only by coincidence; these properties need them by construction.
fn whitespace_heavy_line() -> impl Strategy<Value = String> {
    (opt_ws(), line(), ws_run(), line(), opt_ws())
        .prop_map(|(lead, left, gap, right, trail)| format!("{lead}{left}{gap}{right}{trail}"))
}

/// Either kind of line: general content, or content built around whitespace runs.
fn any_line() -> impl Strategy<Value = String> {
    prop_oneof![2 => line(), 1 => whitespace_heavy_line()]
}

/// A line together with a whitespace-only reshaping of it: same non-whitespace
/// tokens in the same order, re-indented and with internal whitespace runs
/// rewritten. Both members must therefore normalize and hash identically.
fn reshaped_pair() -> impl Strategy<Value = (String, String)> {
    (line(), opt_ws(), ws_run(), opt_ws()).prop_map(|(original, lead, sep, trail)| {
        let reshaped = format!("{lead}{}{trail}", ref_tokens(&original).join(&sep));
        (original, reshaped)
    })
}

/// Like [`line`] but may contain bare carriage returns, which is where
/// terminator handling gets interesting: `str::lines()` strips one trailing
/// `\r` from every line, so a line whose *content* ends in `\r` is
/// indistinguishable from a CRLF terminator, and converting the file to CRLF
/// leaves such a line with a `\r` the split does not remove. Only `normalize`
/// (for which `\r` is whitespace) makes the two spellings hash alike.
fn cr_bearing_line() -> impl Strategy<Value = String> {
    prop::collection::vec(prop_oneof![3 => line_char(), 1 => Just('\r')], 0..8)
        .prop_map(|chars| chars.into_iter().collect())
}

/// A line that is blank (empty or whitespace-only) about a quarter of the time,
/// so the blank-line branches of `resolve` are actually reached.
fn blankish_line() -> impl Strategy<Value = String> {
    prop_oneof![1 => opt_ws(), 3 => line()]
}

/// Renders `lines` as text whose `str::lines()` yields `lines` again, plus the
/// [`SENTINEL`] terminator. Returns both so a property can index the text's
/// lines without re-deriving them.
/// Text with no structure imposed at all, for the totality property: mostly
/// proptest's own `String` strategy, but one draw in four is an arbitrary run of
/// `char`s, which is the only way control characters — tabs, bare `\r`, embedded
/// `\n`, `\u{0}` — reach the API at all (see [`the_anchor_api_is_total`]).
fn arbitrary_chunk() -> impl Strategy<Value = String> {
    prop_oneof![
        3 => any::<String>(),
        1 => prop::collection::vec(any::<char>(), 0..24)
            .prop_map(|chars| chars.into_iter().collect::<String>()),
    ]
}

fn text_of(lines: &[String]) -> (String, Vec<String>) {
    let mut all = lines.to_vec();
    all.push(SENTINEL.to_owned());
    (all.join("\n"), all)
}

fn side_of(left: bool) -> Side {
    if left { Side::Left } else { Side::Right }
}

/// Line numbers worth trying against any text: small in-range ones, the
/// out-of-range `0`, and the arithmetic edges of `u32`.
fn wild_line_number() -> impl Strategy<Value = u32> {
    prop_oneof![
        3 => 0u32..12,
        1 => Just(0u32),
        1 => Just(u32::MAX),
        1 => Just(u32::MAX - 1),
        1 => Just(i32::MAX as u32),
        2 => any::<u32>(),
    ]
}

/// Two versions of a file drawn from one small pool of lines, plus the line
/// number to anchor on (usually inside the file, sometimes past its end) and a
/// side. Both texts come from the same pool so that hash matches — including
/// duplicated matches and blank-line near-misses — actually occur; two
/// independently random texts would essentially never share a line, and every
/// property about the `Moved` scan would pass vacuously.
///
/// The anchor line is drawn from the `before` text's *own* range four times out
/// of five. Drawn from a fixed `0u32..10` instead — as this generator used to —
/// it lands past the end of a text of at most 7 lines in 60% of cases, and an
/// out-of-range anchor short-circuits to `(None, Outdated)` before any of the
/// cascade runs: measured over 200k draws, that wasted 60.17% of every
/// scenario-driven property's cases and left only 2.87% of them reaching a
/// `Moved` with two or more candidates to choose between. Out of range is still
/// generated — it is a real shape and [`out_of_range_anchor_never_resolves`]
/// depends on it — just as the minority branch.
type Scenario = (Vec<String>, Vec<String>, u32, bool);

fn scenario() -> impl Strategy<Value = Scenario> {
    prop::collection::vec(blankish_line(), 1..5)
        .prop_flat_map(|pool| {
            let count = pool.len();
            (
                Just(pool),
                prop::collection::vec(0..count, 0..7),
                prop::collection::vec(0..count, 0..7),
                any::<bool>(),
            )
        })
        .prop_flat_map(|(pool, before_ix, after_ix, left)| {
            // `text_of` appends the sentinel, so the `before` text has one more
            // line than there are picks.
            let before_len = before_ix.len() as u32 + 1;
            (
                Just(pool),
                Just(before_ix),
                Just(after_ix),
                prop_oneof![4 => 1u32..=before_len, 1 => 0u32..10],
                Just(left),
            )
        })
        .prop_map(|(pool, before_ix, after_ix, line, left)| {
            let pick = |ix: Vec<usize>| ix.into_iter().map(|i| pool[i].clone()).collect();
            (pick(before_ix), pick(after_ix), line, left)
        })
}

/// The state a `resolve` property needs: the anchor, the text it is resolved
/// against with that text's lines, and — computed with [`ref_normalize`] rather
/// than with `content_hash` — the normalized content the anchor is pointing at,
/// or `None` when it was created out of range and points at nothing.
struct Resolution {
    line: u32,
    after_lines: Vec<String>,
    target: Option<String>,
    resolved: Option<u32>,
    confidence: Confidence,
}

fn resolve_scenario((before, after, line, left): Scenario) -> Resolution {
    let (before_text, before_lines) = text_of(&before);
    let (after_text, after_lines) = text_of(&after);
    let anchor = create("f.txt", side_of(left), line, &before_text);
    let (resolved, confidence) = resolve(&anchor, &after_text);
    let target = (line >= 1 && (line as usize) <= before_lines.len())
        .then(|| ref_normalize(&before_lines[line as usize - 1]));
    Resolution {
        line,
        after_lines,
        target,
        resolved,
        confidence,
    }
}

impl Resolution {
    /// Every line of the new text the anchor's content could legitimately have
    /// moved to: same normalized content, and not blank (a blank line carries
    /// no identity, so the design refuses to treat one as a move target).
    fn candidates(&self) -> Vec<u32> {
        self.after_lines
            .iter()
            .enumerate()
            .filter(|(_, candidate)| {
                let normalized = ref_normalize(candidate);
                !normalized.is_empty() && self.target.as_deref() == Some(normalized.as_str())
            })
            .map(|(index, _)| index as u32 + 1)
            .collect()
    }

    /// Whether the line still sitting at the anchor's original number carries
    /// the anchored content (blank or not).
    fn same_line_matches(&self) -> bool {
        self.line >= 1
            && (self.line as usize) <= self.after_lines.len()
            && self.target.as_deref()
                == Some(ref_normalize(&self.after_lines[self.line as usize - 1]).as_str())
    }
}

// ---------------------------------------------------------------------------
// Properties: normalize / content_hash
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// Differential test against [`ref_normalize`]: whatever the implementation
    /// strategy, the observable rule must stay "collapse whitespace runs to one
    /// space, strip the edges".
    #[test]
    fn normalize_matches_reference_implementation(raw in any_line()) {
        prop_assert_eq!(normalize(&raw), ref_normalize(&raw));
    }

    /// Normalization is a projection onto canonical forms: applying it to an
    /// already-normalized line changes nothing. Without this, two spellings of
    /// the same content could hash differently depending on how many times they
    /// had been round-tripped.
    #[test]
    fn normalize_is_idempotent(raw in any_line()) {
        let once = normalize(&raw);
        let twice = normalize(&once);
        prop_assert_eq!(twice, once);
    }

    /// The canonical form is recognizable on sight: no leading or trailing
    /// whitespace, no whitespace run longer than one character, and every
    /// whitespace character is a plain ASCII space. Plus a conservation bound —
    /// normalization only ever removes bytes, never invents them.
    #[test]
    fn normalize_output_is_canonical(raw in any_line()) {
        let normalized = normalize(&raw);
        prop_assert!(!normalized.starts_with(char::is_whitespace));
        prop_assert!(!normalized.ends_with(char::is_whitespace));
        prop_assert!(normalized.chars().filter(|c| c.is_whitespace()).all(|c| c == ' '));
        prop_assert!(!normalized.contains("  "));
        prop_assert!(normalized.len() <= raw.len());
    }

    /// Reindentation is exactly the edit an anchor has to survive: the same
    /// tokens, differently spaced, must hash the same. The reshaping is built
    /// from [`ref_tokens`], so this does not merely re-run the implementation.
    #[test]
    fn hashing_survives_whitespace_reshaping((original, reshaped) in reshaped_pair()) {
        prop_assert_eq!(normalize(&reshaped), normalize(&original));
        prop_assert_eq!(content_hash(&reshaped), content_hash(&original));
    }

    /// The hash is a faithful stand-in for normalized content in both
    /// directions: equal hashes mean equal content (so `resolve` cannot be
    /// fooled into matching an unrelated line) and different content means
    /// different hashes (so a real move is not missed). Half the pairs are
    /// whitespace reshapings of each other, which is where the "equal" side of
    /// the biconditional gets its coverage.
    #[test]
    fn content_hash_equals_iff_normalized_content_equals(
        (left, right) in prop_oneof![(any_line(), any_line()), reshaped_pair()],
    ) {
        let same_content = ref_normalize(&left) == ref_normalize(&right);
        prop_assert_eq!(content_hash(&left) == content_hash(&right), same_content);
    }

    /// A digest, not a copy: 64 lowercase hex characters whatever the input.
    /// A `content_hash` that leaked normalized text would let a crafted line
    /// forge the [`create`] out-of-range sentinel.
    #[test]
    fn content_hash_is_a_hex_digest(raw in any_line()) {
        let hash = content_hash(&raw);
        prop_assert_eq!(hash.len(), 64);
        prop_assert!(hash.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
    }
}

// ---------------------------------------------------------------------------
// Properties: snapshot_of / create
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// The snapshot is a contiguous window of real lines, at most 11 of them,
    /// centred on the anchored line as far as the file's edges allow: the
    /// anchored line sits at offset `min(index, 5)`, and the window is maximal
    /// — it gives up context only where the file runs out.
    #[test]
    fn snapshot_is_a_bounded_centered_window(
        lines in prop::collection::vec(blankish_line(), 0..20),
        line in 0u32..24,
    ) {
        let (text, lines) = text_of(&lines);
        prop_assert_eq!(text.lines().collect::<Vec<_>>(), lines.iter().map(String::as_str).collect::<Vec<_>>());
        let snapshot = snapshot_of(&text, line);

        if line == 0 || line as usize > lines.len() {
            prop_assert!(snapshot.is_empty());
        } else {
            let index = line as usize - 1;
            let lead = index.min(5);
            let trail = (lines.len() - 1 - index).min(5);
            prop_assert!(snapshot.len() <= 11);
            prop_assert_eq!(snapshot.len(), 1 + lead + trail);
            prop_assert_eq!(&snapshot[lead], &lines[index]);
            for (offset, entry) in snapshot.iter().enumerate() {
                prop_assert_eq!(entry, &lines[index - lead + offset]);
            }
        }
    }

    /// `create` records the line number it was handed verbatim, invents no
    /// context (every snapshot entry is a real line of the text), and carries a
    /// non-empty snapshot exactly when the line exists. An out-of-range anchor
    /// is deliberately contextless — there is nothing to show.
    #[test]
    fn create_records_line_and_real_context(
        lines in prop::collection::vec(blankish_line(), 0..12),
        line in wild_line_number(),
        left in any::<bool>(),
    ) {
        let (text, lines) = text_of(&lines);
        let anchor = create("f.txt", side_of(left), line, &text);
        let in_range = line >= 1 && (line as usize) <= lines.len();

        prop_assert_eq!(anchor.line, line);
        prop_assert_eq!(!anchor.context.is_empty(), in_range);
        prop_assert!(anchor.context.iter().all(|entry| lines.contains(entry)));
        prop_assert_eq!(&anchor.context, &snapshot_of(&text, line));
    }

    /// The two fields that say *which* text an anchor is about are carried
    /// through verbatim: they are the anchor's only record of the file and the
    /// diff pane it belongs to, and nothing downstream can recover them if
    /// `create` rewrites them. Resolution, conversely, must ignore both — it is
    /// handed the text by its caller, so a side-dependent cascade would place
    /// the same comment differently depending on which pane asked.
    #[test]
    fn create_preserves_identity_which_resolve_then_ignores(
        lines in prop::collection::vec(blankish_line(), 0..8),
        path in "[a-z][a-z/._]{0,11}",
        line in wild_line_number(),
        left in any::<bool>(),
    ) {
        let (text, _) = text_of(&lines);
        let anchor = create(&path, side_of(left), line, &text);
        prop_assert_eq!(&anchor.file, &path);
        prop_assert_eq!(anchor.side, side_of(left));

        let mirrored = create("other/path.rs", side_of(!left), line, &text);
        prop_assert_eq!(&mirrored.content_hash, &anchor.content_hash);
        prop_assert_eq!(resolve(&mirrored, &text), resolve(&anchor, &text));
    }
}

// ---------------------------------------------------------------------------
// Properties: resolve
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// Identity round trip: an anchor resolved against the very text it was
    /// created from lands on its own line, `Exact`. Holds for every line of
    /// every text, blank and whitespace-only lines included — the same-line
    /// check does not care whether the content is blank.
    #[test]
    fn create_then_resolve_same_text_is_exact(
        lines in prop::collection::vec(blankish_line(), 1..12),
        seed in 0u32..14,
        left in any::<bool>(),
    ) {
        let (text, lines) = text_of(&lines);
        let line = seed % lines.len() as u32 + 1;
        let anchor = create("f.txt", side_of(left), line, &text);
        prop_assert_eq!(resolve(&anchor, &text), (Some(line), Confidence::Exact));
    }

    /// Whole-file reindentation is invisible to resolution: every anchor in the
    /// file still resolves `Exact` to its own line number after every line has
    /// been re-indented and had its internal whitespace runs rewritten.
    #[test]
    fn reindenting_the_whole_file_keeps_every_anchor_exact(
        pairs in prop::collection::vec(reshaped_pair(), 1..10),
        left in any::<bool>(),
    ) {
        let originals: Vec<String> = pairs.iter().map(|(o, _)| o.clone()).collect();
        let reshapings: Vec<String> = pairs.iter().map(|(_, r)| r.clone()).collect();
        let (text, lines) = text_of(&originals);
        let (reindented, _) = text_of(&reshapings);

        for line in 1..=lines.len() as u32 {
            let anchor = create("f.txt", side_of(left), line, &text);
            prop_assert_eq!(
                resolve(&anchor, &reindented),
                (Some(line), Confidence::Exact),
                "line {} of {:?} lost its anchor in {:?}", line, text, reindented
            );
        }
    }

    /// Inserting `k` lines before position `p` shifts exactly the anchors at or
    /// after `p`, by exactly `k`, and leaves earlier ones alone: line arithmetic
    /// recomputed from the edit, not read back from the code. The generated
    /// lines all carry distinct `u<i>:`/`p<j>:` prefixes, so the anchored
    /// content is non-blank and unique and there is only one place it can be.
    #[test]
    fn inserting_lines_shifts_a_unique_anchor(
        bodies in prop::collection::vec(line(), 1..7),
        inserts in prop::collection::vec(line(), 1..5),
        position_seed in 0usize..8,
        line_seed in 0usize..8,
        left in any::<bool>(),
    ) {
        let base: Vec<String> = bodies.iter().enumerate().map(|(i, s)| format!("u{i}:{s}")).collect();
        let extra: Vec<String> = inserts.iter().enumerate().map(|(j, s)| format!("p{j}:{s}")).collect();
        let inserted = extra.len() as u32;
        let position = (position_seed % (base.len() + 1) + 1) as u32;
        let line = (line_seed % base.len() + 1) as u32;

        let before = base.join("\n");
        let mut after_lines = base.clone();
        after_lines.splice((position as usize - 1)..(position as usize - 1), extra);
        let after = after_lines.join("\n");

        // Generator invariant: distinct prefixes must give distinct normalized
        // lines, or "the unique match" below would not be unique.
        let mut normalized: Vec<String> = after_lines.iter().map(|l| ref_normalize(l)).collect();
        let total = normalized.len();
        normalized.sort();
        normalized.dedup();
        prop_assert_eq!(normalized.len(), total);
        prop_assert_eq!(before.lines().count(), base.len());

        let anchor = create("f.txt", side_of(left), line, &before);
        let expected = if line < position {
            (Some(line), Confidence::Exact)
        } else {
            (Some(line + inserted), Confidence::Moved)
        };
        prop_assert_eq!(resolve(&anchor, &after), expected);
    }

    /// Deleting one line of a file of unique lines: deleting the anchored line
    /// itself loses it (`Outdated`, never a guess at a neighbour); deleting an
    /// earlier line shifts it up one (`Moved`); deleting a later line leaves it
    /// where it was (`Exact`).
    #[test]
    fn deleting_a_line_moves_or_outdates_a_unique_anchor(
        bodies in prop::collection::vec(line(), 2..8),
        line_seed in 0usize..8,
        deleted_seed in 0usize..8,
        left in any::<bool>(),
    ) {
        let base: Vec<String> = bodies.iter().enumerate().map(|(i, s)| format!("u{i}:{s}")).collect();
        let line = (line_seed % base.len() + 1) as u32;
        let deleted = (deleted_seed % base.len() + 1) as u32;

        let before = base.join("\n");
        let mut after_lines = base.clone();
        after_lines.remove(deleted as usize - 1);
        let after = after_lines.join("\n");

        let anchor = create("f.txt", side_of(left), line, &before);
        let expected = if deleted == line {
            (None, Confidence::Outdated)
        } else if deleted < line {
            (Some(line - 1), Confidence::Moved)
        } else {
            (Some(line), Confidence::Exact)
        };
        prop_assert_eq!(resolve(&anchor, &after), expected);
    }

    /// Soundness — resolution never invents a location. Whenever `resolve`
    /// returns a line, that line exists in the new text and actually carries
    /// the anchored content (checked with [`ref_normalize`], not by re-asking
    /// the hash), `Exact` means the number did not change, `Moved` means it did
    /// and landed on a non-blank line, and `Weak` is never produced in this
    /// milestone.
    #[test]
    fn resolve_never_invents_a_line(raw in scenario()) {
        let outcome = resolve_scenario(raw);
        prop_assert_ne!(outcome.confidence, Confidence::Weak);
        match outcome.resolved {
            None => prop_assert_eq!(outcome.confidence, Confidence::Outdated),
            Some(landed) => {
                prop_assert!(landed >= 1 && (landed as usize) <= outcome.after_lines.len());
                let landed_content = ref_normalize(&outcome.after_lines[landed as usize - 1]);
                prop_assert_eq!(outcome.target.as_deref(), Some(landed_content.as_str()));
                match outcome.confidence {
                    Confidence::Exact => prop_assert_eq!(landed, outcome.line),
                    Confidence::Moved => {
                        prop_assert_ne!(landed, outcome.line);
                        prop_assert!(!landed_content.is_empty());
                    }
                    other => prop_assert!(false, "unexpected confidence {:?} with a line", other),
                }
            }
        }
    }

    /// Of all the places the content could have moved to, resolution picks one
    /// that is nearest to the original line number, and breaks a tie in favour
    /// of the earlier line. Both are recomputed here from the candidate set.
    #[test]
    fn resolve_returns_the_nearest_matching_line(raw in scenario()) {
        let outcome = resolve_scenario(raw);
        let candidates = outcome.candidates();
        if let Some(landed) = outcome.resolved {
            for candidate in &candidates {
                prop_assert!(
                    landed.abs_diff(outcome.line) <= candidate.abs_diff(outcome.line),
                    "landed on {} but {} is nearer to {}", landed, candidate, outcome.line
                );
            }
            if outcome.confidence == Confidence::Moved {
                prop_assert!(candidates.contains(&landed));
                let nearest = candidates
                    .iter()
                    .copied()
                    .min_by_key(|candidate| (candidate.abs_diff(outcome.line), *candidate));
                prop_assert_eq!(Some(landed), nearest);
            }
        }
    }

    /// The whole reason `resolve` is a nearest-match scan and not a `find`: with
    /// the anchored content duplicated on *both* sides of the anchor's old line
    /// number, resolution lands on whichever copy is nearer, and on the earlier
    /// one when they are equidistant.
    ///
    /// Built by construction, because [`scenario`] cannot be relied on to build
    /// it: measured over 200k draws, only 2.87% of scenario cases reach a `Moved`
    /// with two or more candidates at all, and only 1.73% place the nearest one
    /// anywhere but first — so a `resolve` that took the *first* hash match
    /// instead of the nearest one survived 2 of 12 runs of
    /// [`resolve_returns_the_nearest_matching_line`], and measuring the distance
    /// from the 0-based index rather than the line number survived 10 of 12.
    /// Everything here is still generated — the content, the filler around it,
    /// the anchor's line number and *both* distances — but the two-candidate
    /// shape itself is guaranteed, and the tie is one branch of it rather than a
    /// separate test:
    ///
    /// - `before_distance < after_distance` — the earlier copy wins on distance;
    /// - `before_distance > after_distance` — the later copy wins on distance,
    ///   which is the direction a first-match scan can never produce;
    /// - `before_distance == after_distance` — the documented tie-break, the
    ///   earlier line, which is the difference between a comment landing on the
    ///   same duplicate every time and one that hops between them as the file
    ///   changes shape around it.
    #[test]
    fn resolve_lands_on_the_nearer_duplicate_and_breaks_ties_earlier(
        marked_body in line(),
        filler_bodies in prop::collection::vec(line(), 12),
        anchor_seed in 0usize..6,
        // Ties are the case "nearest wins" cannot decide, so they are drawn as
        // their own branch rather than left to a 1-in-6 coincidence.
        (before_distance, after_distance) in prop_oneof![
            2 => (1usize..6).prop_map(|distance| (distance, distance)),
            3 => (1usize..7, 1usize..7),
        ],
    ) {
        // Strictly greater than `before_distance`, so the earlier copy is still
        // line >= 1, and both copies are off the anchor's own line so the
        // same-line check cannot answer before the scan is reached.
        let anchor_line = (anchor_seed + before_distance + 1) as u32;
        let marked = format!("m:{marked_body}");
        // Distinct `f<i>:` prefixes: no filler can normalize to the marked
        // content, and no two fillers to each other, so the two copies of
        // `marked` are the only candidates in the edited text.
        let filler = |index: usize| format!("f{index}:{}", filler_bodies[index % 12]);

        let mut origin: Vec<String> = (0..anchor_line as usize - 1).map(filler).collect();
        origin.push(marked.clone());
        let (origin_text, origin_lines) = text_of(&origin);
        prop_assert_eq!(&origin_lines[anchor_line as usize - 1], &marked);

        let earlier = anchor_line as usize - before_distance;
        let later = anchor_line as usize + after_distance;
        let mut edited: Vec<String> = (0..later).map(filler).collect();
        edited[earlier - 1] = marked.clone();
        edited[later - 1] = marked.clone();
        let (edited_text, edited_lines) = text_of(&edited);
        // Generator invariants: exactly two candidates, at the two distances
        // asked for, and neither of them on the anchor's own line.
        let candidates: Vec<usize> = edited_lines
            .iter()
            .enumerate()
            .filter(|(_, candidate)| ref_normalize(candidate) == ref_normalize(&marked))
            .map(|(index, _)| index + 1)
            .collect();
        prop_assert_eq!(&candidates, &vec![earlier, later]);
        prop_assert_ne!(&edited_lines[anchor_line as usize - 1], &marked);

        // Nearest wins; on a tie the earlier line does.
        let expected = if before_distance <= after_distance { earlier } else { later };
        let anchor = create("f.txt", Side::Right, anchor_line, &origin_text);
        prop_assert_eq!(
            resolve(&anchor, &edited_text),
            (Some(expected as u32), Confidence::Moved),
            "content at distance {} before and {} after line {}",
            before_distance, after_distance, anchor_line
        );
    }

    /// Completeness — resolution gives up only when it has to. `Outdated` with
    /// no line requires that nothing in the new text carries the anchored
    /// content: neither the original line number nor any non-blank line
    /// anywhere.
    #[test]
    fn resolve_is_outdated_only_when_nothing_matches(raw in scenario()) {
        let outcome = resolve_scenario(raw);
        let nothing_matches = outcome.candidates().is_empty() && !outcome.same_line_matches();
        prop_assert_eq!(outcome.resolved.is_none(), nothing_matches);
    }

    /// A blank line has no identity: every blank or whitespace-only line
    /// normalizes to `""`, so "the blank line moved to line 9" would be a
    /// fabrication. A blank anchor therefore never resolves `Moved` — it stays
    /// put (`Exact`) while the line at its number is still blank, and fails
    /// safe to `(None, Outdated)` the moment it is not. This is a deliberate
    /// design decision, not an accident of the current cascade.
    #[test]
    fn blank_line_anchor_never_resolves_moved(
        lines in prop::collection::vec(blankish_line(), 1..7),
        blank_seed in 0usize..7,
        pad in opt_ws(),
        after in prop::collection::vec(blankish_line(), 0..8),
        left in any::<bool>(),
    ) {
        let mut before = lines;
        let index = blank_seed % before.len();
        before[index] = pad;
        let line = index as u32 + 1;
        let (before_text, _) = text_of(&before);
        let (after_text, after_lines) = text_of(&after);

        let anchor = create("f.txt", side_of(left), line, &before_text);
        prop_assert_eq!(&anchor.content_hash, &content_hash(""));

        let (resolved, confidence) = resolve(&anchor, &after_text);
        prop_assert_ne!(confidence, Confidence::Moved);
        let still_blank = (line as usize) <= after_lines.len()
            && ref_normalize(&after_lines[line as usize - 1]).is_empty();
        if still_blank {
            prop_assert_eq!((resolved, confidence), (Some(line), Confidence::Exact));
        } else {
            prop_assert_eq!((resolved, confidence), (None, Confidence::Outdated));
        }
    }

    /// An anchor created for a line that does not exist points at nothing, and
    /// must keep pointing at nothing forever: no text can make it resolve —
    /// least of all a text full of blank lines, which is what it would collide
    /// with if the sentinel were replaced by the hash of the empty string.
    #[test]
    fn out_of_range_anchor_never_resolves(
        before in prop::collection::vec(blankish_line(), 0..6),
        after in prop::collection::vec(blankish_line(), 0..8),
        overshoot in 0u32..3,
        use_zero in any::<bool>(),
        left in any::<bool>(),
    ) {
        let (before_text, before_lines) = text_of(&before);
        let (after_text, _) = text_of(&after);
        // `0` and "past the end" are the two ways to be out of range.
        let line = if use_zero { 0 } else { before_lines.len() as u32 + 1 + overshoot };

        let anchor = create("f.txt", side_of(left), line, &before_text);
        prop_assert!(anchor.context.is_empty());
        prop_assert_eq!(resolve(&anchor, &after_text), (None, Confidence::Outdated));
        // The text that a hash-of-the-empty-string sentinel would resolve
        // against is one whose *own* line `anchor.line` is blank, so build it for
        // the line number actually generated instead of hoping a fixed-size blank
        // text is long enough to reach it — with fixed texts this caught the
        // regression only in the runs where the generated line landed inside them.
        let blank_file = "\n".repeat(line as usize + 2);
        prop_assert!(blank_file.lines().count() > line as usize);
        prop_assert_eq!(resolve(&anchor, &blank_file), (None, Confidence::Outdated));
        let whitespace_file = " \t \n".repeat(line as usize + 2);
        prop_assert_eq!(resolve(&anchor, &whitespace_file), (None, Confidence::Outdated));
        prop_assert_eq!(resolve(&anchor, ""), (None, Confidence::Outdated));
    }

    /// Line-terminator style is not content: adding a missing trailing newline
    /// or checking the file out with CRLF endings changes neither the anchor's
    /// hash nor where it resolves. The lines here may contain bare carriage
    /// returns (see [`cr_bearing_line`]) precisely so this is not vacuous —
    /// with them, LF and CRLF renderings really do produce different line
    /// *contents* (`"x"` versus `"x\r"`) and it is normalization that has to
    /// make them agree.
    #[test]
    fn resolve_ignores_line_terminator_style(
        lines in prop::collection::vec(cr_bearing_line(), 1..8),
        cr_seed in 0usize..8,
        line in wild_line_number(),
        left in any::<bool>(),
    ) {
        let mut lines = lines;
        // One line is *guaranteed* to end in a carriage return, because that is
        // the only shape the two renderings can be told apart by: `str::lines()`
        // strips exactly one trailing `\r`, so a line whose own content ends in
        // one comes back as `"x"` from the LF text and `"x\r"` from the CRLF text,
        // and only normalization makes those hash alike. Left to chance, the `\r`
        // half of this property fired in some runs and not others.
        let cr_index = cr_seed % lines.len();
        lines[cr_index] = format!("{}\r", lines[cr_index]);
        let cr_line = cr_index as u32 + 1;

        let (text, _) = text_of(&lines);
        let crlf = text.replace('\n', "\r\n");
        let anchor = create("f.txt", side_of(left), line, &text);
        let baseline = resolve(&anchor, &text);

        prop_assert_eq!(text.lines().count(), crlf.lines().count());
        prop_assert_eq!(&create("f.txt", side_of(left), line, &crlf).content_hash, &anchor.content_hash);
        prop_assert_eq!(resolve(&anchor, &format!("{text}\n")), baseline);
        prop_assert_eq!(resolve(&anchor, &crlf), baseline);
        prop_assert_eq!(resolve(&anchor, &format!("{crlf}\r\n")), baseline);

        // Anchored exactly on the carriage-return-bearing line, where `create`
        // really is handed different content by the two renderings.
        let from_lf = create("f.txt", side_of(left), cr_line, &text);
        let from_crlf = create("f.txt", side_of(left), cr_line, &crlf);
        prop_assert_eq!(&from_crlf.content_hash, &from_lf.content_hash);
        prop_assert_eq!(resolve(&from_lf, &crlf), (Some(cr_line), Confidence::Exact));
        prop_assert_eq!(resolve(&from_crlf, &text), (Some(cr_line), Confidence::Exact));
    }

    /// The stored snapshot is documented as purely descriptive: it is what a
    /// reviewer reads when an anchor dangles, and it must never influence where
    /// a comment lands. Corrupting it wholesale changes nothing about
    /// resolution.
    ///
    /// Asserted twice, because the two witnesses catch different failures.
    ///
    /// The first is an arbitrary pair of texts, which catches a resolve whose
    /// *candidate set* depends on the snapshot (one that only considered lines it
    /// recognized from `context`, say).
    ///
    /// The second is built so the snapshot could change the answer without
    /// changing the candidate set — the hole the first witness cannot see, and
    /// the reason it is not enough on its own. `context` stores lines *verbatim*,
    /// while `resolve` matches on normalized content, so two spellings of the
    /// same line are interchangeable to the hash and distinguishable to a
    /// snapshot lookup. The edited text puts a whitespace *reshaping* of the
    /// anchored line nearer to the anchor's old number than the verbatim line
    /// itself, so a resolve that let a snapshot hit outrank a nearer line would
    /// answer with the far copy under the real context and the near one under
    /// junk. Drawing both texts from one pool of verbatim lines — as the first
    /// witness does — makes every candidate for a given content byte-identical,
    /// which is exactly why that shape cannot tell the two apart.
    #[test]
    fn stale_context_does_not_change_resolution(
        raw in scenario(),
        junk in prop::collection::vec(line(), 0..4),
        marked_body in line(),
        filler_bodies in prop::collection::vec(line(), 12),
        lead in ws_run(),
        separator in ws_run(),
        anchor_seed in 0usize..5,
        near_seed in 0usize..3,
        far_seed in 0usize..3,
    ) {
        let (before, after, seed, left) = raw;
        let (before_text, before_lines) = text_of(&before);
        let (after_text, _) = text_of(&after);
        let line = seed % (before_lines.len() as u32 + 2);

        let anchor = create("f.txt", side_of(left), line, &before_text);
        let mut stale = anchor.clone();
        stale.context = junk.clone();
        prop_assert_eq!(resolve(&stale, &after_text), resolve(&anchor, &after_text));

        // Two spellings of one line: `marked` is what `create` stores in
        // `context`, `reshaped` has the same tokens re-spaced and re-indented, so
        // it hashes identically and is byte-equal to nothing in the snapshot.
        let marked = format!("m:{marked_body}");
        let reshaped = format!("{lead}{}", ref_tokens(&marked).join(&separator));
        let near = near_seed + 1;
        let far = near + far_seed + 1;
        let anchor_line = (anchor_seed + near + 1) as u32;
        let filler = |index: usize| format!("f{index}:{}", filler_bodies[index % 12]);

        let mut origin: Vec<String> = (0..anchor_line as usize - 1).map(filler).collect();
        origin.push(marked.clone());
        let (origin_text, _) = text_of(&origin);
        let near_line = anchor_line as usize - near;
        let far_line = anchor_line as usize + far;
        let mut edited: Vec<String> = (0..far_line).map(filler).collect();
        edited[near_line - 1] = reshaped.clone();
        edited[far_line - 1] = marked.clone();
        let (edited_text, _) = text_of(&edited);

        let snapshotted = create("f.txt", side_of(left), anchor_line, &origin_text);
        // The construction, asserted rather than assumed: the far copy is in the
        // snapshot, the near one is not, and the hash cannot tell them apart.
        prop_assert!(snapshotted.context.contains(&marked));
        prop_assert!(!snapshotted.context.contains(&reshaped));
        prop_assert!(!junk.contains(&marked));
        prop_assert_ne!(&reshaped, &marked);
        prop_assert_eq!(content_hash(&reshaped), content_hash(&marked));

        let mut blinded = snapshotted.clone();
        blinded.context = junk;
        prop_assert_eq!(
            resolve(&blinded, &edited_text),
            resolve(&snapshotted, &edited_text),
            "the snapshot decided between a reshaped copy at line {} and the \
             verbatim line at {}", near_line, far_line
        );
    }

    /// A comment has to survive a *chain* of rewrites, not just one, and the way
    /// it does that is by being re-anchored where it last resolved — so
    /// re-anchoring must be a fixed point. If the re-anchored hash could drift
    /// even slightly, a comment would walk down the file one line per rebase
    /// while every individual step still looked like a clean `Exact`/`Moved`.
    /// Checked three ways at every hop: the hash never changes from the one the
    /// comment was born with, the fresh anchor resolves straight back to where it
    /// landed, and the content there is the same content it started on.
    #[test]
    fn re_anchoring_across_a_chain_of_rewrites_never_drifts(
        versions in prop::collection::vec(prop::collection::vec(blankish_line(), 0..6), 2..5),
        seed in 0u32..12,
        left in any::<bool>(),
    ) {
        let (first_text, first_lines) = text_of(&versions[0]);
        let line = seed % first_lines.len() as u32 + 1;
        let original = create("f.txt", side_of(left), line, &first_text);
        let origin_content = ref_normalize(&first_lines[line as usize - 1]);

        let mut anchor = original.clone();
        for version in &versions[1..] {
            let (next_text, next_lines) = text_of(version);
            let (resolved, confidence) = resolve(&anchor, &next_text);
            let Some(landed) = resolved else {
                prop_assert_eq!(confidence, Confidence::Outdated);
                break;
            };
            prop_assert_eq!(&ref_normalize(&next_lines[landed as usize - 1]), &origin_content);

            let re_anchored = create("f.txt", side_of(left), landed, &next_text);
            prop_assert_eq!(&re_anchored.content_hash, &original.content_hash);
            prop_assert_eq!(
                resolve(&re_anchored, &next_text),
                (Some(landed), Confidence::Exact)
            );
            anchor = re_anchored;
        }
    }

    /// Totality: no combination of arbitrary text (any Unicode, any number of
    /// lines), any line number including `0` and `u32::MAX`, and either side
    /// panics, and the cheap postconditions hold everywhere.
    ///
    /// One chunk in four is built from `any::<char>()` rather than
    /// `any::<String>()`, because proptest's `String` strategy is the regex
    /// `\PC*` — it excludes *every* control character, so on its own it would
    /// make "arbitrary Unicode text" mean "arbitrary text with no tab, no `\r`
    /// and no `\n`", which is precisely the alphabet a line-splitting,
    /// whitespace-collapsing module has to be total over.
    #[test]
    fn the_anchor_api_is_total(
        chunks in prop::collection::vec(arbitrary_chunk(), 0..5),
        line in wild_line_number(),
        left in any::<bool>(),
        other in prop::collection::vec(arbitrary_chunk(), 0..5),
    ) {
        let text = chunks.join("\n");
        let other = other.join("\n");

        prop_assert!(normalize(&text).len() <= text.len());
        prop_assert_eq!(content_hash(&text).len(), 64);
        prop_assert!(snapshot_of(&text, line).len() <= 11);

        let anchor = create("f.txt", side_of(left), line, &text);
        for target in [text.as_str(), other.as_str(), ""] {
            let (resolved, confidence) = resolve(&anchor, target);
            prop_assert_eq!(resolved.is_none(), confidence == Confidence::Outdated);
            if let Some(landed) = resolved {
                prop_assert!(landed >= 1 && (landed as usize) <= target.lines().count());
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Parameterized edge shapes a random generator rarely lands on
// ---------------------------------------------------------------------------

/// Every anchor resolved against its own text — the shapes that break naive
/// line handling: no text at all, no trailing newline, CRLF, combining marks,
/// a 50k-character line, nothing but blank lines, nothing but duplicates.
#[rstest]
#[case::empty_text("".into(), 1, None, Confidence::Outdated)]
#[case::empty_text_line_zero("".into(), 0, None, Confidence::Outdated)]
#[case::lone_newline_is_one_blank_line("\n".into(), 1, Some(1), Confidence::Exact)]
#[case::single_line_no_trailing_newline("solo".into(), 1, Some(1), Confidence::Exact)]
#[case::single_line_trailing_newline("solo\n".into(), 1, Some(1), Confidence::Exact)]
#[case::single_line_past_eof("solo\n".into(), 2, None, Confidence::Outdated)]
#[case::crlf_middle_line("a\r\nb\r\nc\r\n".into(), 2, Some(2), Confidence::Exact)]
#[case::crlf_unterminated_last_line("a\r\nb\r\nc".into(), 3, Some(3), Confidence::Exact)]
#[case::unicode_and_emoji("héllo\n😀 🇦🇺 ok\nz".into(), 2, Some(2), Confidence::Exact)]
#[case::combining_marks("e\u{301}\u{301}x\ny".into(), 1, Some(1), Confidence::Exact)]
#[case::very_long_line("x".repeat(50_000), 1, Some(1), Confidence::Exact)]
#[case::only_blank_lines("\n \n\t\n".into(), 2, Some(2), Confidence::Exact)]
#[case::only_blank_lines_past_eof("\n \n\t\n".into(), 4, None, Confidence::Outdated)]
#[case::all_lines_identical("dup\ndup\ndup\ndup\n".into(), 3, Some(3), Confidence::Exact)]
fn edge_shape_resolves_against_its_own_text(
    #[case] text: String,
    #[case] line: u32,
    #[case] expected_line: Option<u32>,
    #[case] expected_confidence: Confidence,
) {
    let anchor = create("f.txt", Side::Right, line, &text);
    assert_eq!(
        resolve(&anchor, &text),
        (expected_line, expected_confidence)
    );
}

/// The same edge shapes, but resolved against an *edited* text — where the
/// cascade actually has to choose.
#[rstest]
#[case::very_long_line_shifted(
    "x".repeat(20_000),
    1,
    format!("pad\n{}", "x".repeat(20_000)),
    Some(2),
    Confidence::Moved
)]
#[case::identical_lines_one_deleted(
    "dup\ndup\ndup\ndup\ndup\n".into(), 3, "dup\ndup\ndup\ndup\n".into(), Some(3), Confidence::Exact
)]
#[case::identical_lines_truncated_past_anchor(
    "dup\ndup\ndup\ndup\ndup\n".into(), 5, "dup\ndup\ndup\n".into(), Some(3), Confidence::Moved
)]
#[case::crlf_converted_to_lf("a\r\nb\r\nc\r\n".into(), 2, "a\nb\nc\n".into(), Some(2), Confidence::Exact)]
#[case::lf_converted_to_crlf("a\nb\nc\n".into(), 2, "a\r\nb\r\nc\r\n".into(), Some(2), Confidence::Exact)]
#[case::trailing_newline_added("a\nb".into(), 2, "a\nb\n".into(), Some(2), Confidence::Exact)]
#[case::reindented_single_line("\tlet   x = 1;".into(), 1, "        let x  =  1;".into(), Some(1), Confidence::Exact)]
#[case::emoji_line_moved("é\n😀\nz\n".into(), 2, "z\nq\n😀\n".into(), Some(3), Confidence::Moved)]
#[case::combining_mark_is_not_precomposed("e\u{301}\n".into(), 1, "é\n".into(), None, Confidence::Outdated)]
#[case::blank_anchor_into_blank_text("a\n\nb\n".into(), 2, "\n\n\n\n".into(), Some(2), Confidence::Exact)]
#[case::blank_anchor_into_nonblank_text("a\n\nb\n".into(), 2, "a\nb\nc\n".into(), None, Confidence::Outdated)]
#[case::everything_deleted("a\nb\n".into(), 1, "".into(), None, Confidence::Outdated)]
#[case::file_became_blank("a\nb\n".into(), 1, "\n\n".into(), None, Confidence::Outdated)]
fn edge_shape_resolves_across_texts(
    #[case] before: String,
    #[case] line: u32,
    #[case] after: String,
    #[case] expected_line: Option<u32>,
    #[case] expected_confidence: Confidence,
) {
    let anchor = create("f.txt", Side::Left, line, &before);
    assert_eq!(
        resolve(&anchor, &after),
        (expected_line, expected_confidence)
    );
}

/// Which lines normalization treats as the same content. The `false` rows are
/// the interesting ones: whitespace is collapsed but never inserted, no Unicode
/// NFC folding happens, and a zero-width space is not whitespace.
#[rstest]
#[case::leading_and_trailing_stripped("  x  ", "x", true)]
#[case::tab_equals_space("a\tb", "a b", true)]
#[case::runs_collapse("a  \t  b", "a b", true)]
#[case::empty_equals_spaces("", "   ", true)]
#[case::empty_equals_exotic_whitespace("", "\t \u{000b}\u{000c}", true)]
#[case::carriage_return_is_whitespace("a\rb", "a b", true)]
#[case::nbsp_is_unicode_whitespace("a\u{00a0}b", "a b", true)]
#[case::ideographic_space_is_whitespace("a\u{3000}b", "a b", true)]
#[case::emoji_keeps_identity("😀", "  😀\t", true)]
#[case::zero_width_space_is_not_whitespace("a\u{200b}b", "a b", false)]
#[case::whitespace_is_never_inserted("a b", "ab", false)]
#[case::no_unicode_normalization("é", "e\u{301}", false)]
#[case::case_is_significant("x", "X", false)]
#[case::token_order_matters("a b", "b a", false)]
fn normalized_equivalence_classes(#[case] left: &str, #[case] right: &str, #[case] same: bool) {
    assert_eq!(normalize(left) == normalize(right), same);
    assert_eq!(content_hash(left) == content_hash(right), same);
}

/// Snapshot windows at the shapes where the clamping arithmetic is easiest to
/// get wrong. `center` is where the anchored line must sit inside the window.
#[rstest]
#[case::empty_text("".into(), 1, 0, None)]
#[case::line_zero("a\nb\n".into(), 0, 0, None)]
#[case::line_u32_max("a\nb\n".into(), u32::MAX, 0, None)]
#[case::single_line("solo".into(), 1, 1, Some(0))]
#[case::first_of_three("a\nb\nc".into(), 1, 3, Some(0))]
#[case::unterminated_last_of_three("a\nb\nc".into(), 3, 3, Some(2))]
#[case::crlf_middle("a\r\nb\r\nc\r\n".into(), 2, 3, Some(1))]
#[case::only_blank_lines("\n\n\n".into(), 2, 3, Some(1))]
#[case::middle_of_twelve((1..=12).map(|n| n.to_string()).collect::<Vec<_>>().join("\n"), 6, 11, Some(5))]
#[case::last_of_twelve((1..=12).map(|n| n.to_string()).collect::<Vec<_>>().join("\n"), 12, 6, Some(5))]
#[case::fifth_of_twelve((1..=12).map(|n| n.to_string()).collect::<Vec<_>>().join("\n"), 5, 10, Some(4))]
#[case::identical_lines("dup\n".repeat(20), 10, 11, Some(5))]
fn snapshot_edge_shapes(
    #[case] text: String,
    #[case] line: u32,
    #[case] expected_len: usize,
    #[case] center: Option<usize>,
) {
    let snapshot = snapshot_of(&text, line);
    assert_eq!(snapshot.len(), expected_len);
    match center {
        None => assert!(snapshot.is_empty()),
        Some(center) => {
            let expected = text.lines().nth(line as usize - 1).unwrap();
            assert_eq!(snapshot[center], expected);
            assert!(
                snapshot
                    .iter()
                    .all(|entry| text.lines().any(|l| l == entry))
            );
        }
    }
}
