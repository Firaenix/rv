# rv — full-file diff context

**Status:** shipped — see the implementation-status appendix below
**Date:** 2026-08-21
**Builds on:** `2026-08-17-rv-branch-reviewer-design.md` §7 (diff pipeline), `2026-08-18-rv-viewport-design.md` (row model, diff pane)

## 1. Purpose

Reviewing a file today shows only the lines difftastic (or the `similar`
fallback) reports as different. The user's complaint: "I want the file viewer
to show the entire file + the diff, not just the diff, otherwise I lose
context of everything going on around the code that I'm reviewing." This
closes that gap: the pane shows every line of the file, changed lines marked
as they are today, everything else rendered as plain syntax-highlighted
context.

## 2. What is already true, and why it constrains the design

Confirmed by reading, not assumed:

- **`Anchor.line` is already a real file line number**, not diff-relative
  (`rv-core/src/anchor.rs`, `rv-core/src/model.rs`). `anchor::create`,
  `anchor::resolve` and the whole cascade operate on 1-based line numbers in
  the file's own text. Nothing about showing more rows changes what "line 48"
  means. **This means full-file context must not touch the anchor model at
  all** — the risk the brief warned about does not materialize because the
  numbers were never diff-relative to begin with.
- **`DiffLine.left`/`.right` are already absolute 1-based file line
  numbers**, on both engines. difftastic's chunk entries carry the real line
  number from the blob (`difftastic.rs::line_ref`); the `similar` fallback
  numbers by index into the file it split. A context-filling walk therefore
  needs no coordinate translation — it is filling gaps between numbers that
  are already in the right space.
- **The `similar` fallback already produces full-file context.** `LineKind`
  already has `Context`; nothing new is required there. Every downstream
  consumer of `LineKind::Context` (`rows.rs`, `ui/diff.rs`, `ui/code.rs`,
  `app/hunks.rs`, `app/anchor.rs::anchored_side`) already has a correct,
  tested branch for it. **The row model, the pane, hunk navigation and the
  anchor side rule do not need to change** — they need a fuller `DiffLine`
  stream to run over, which is the only thing this design adds.
- **Hunk boundaries are derived from line-number contiguity, not from
  `LineKind`** (`rv/src/app/hunks.rs`). `hunk_starts` already treats a
  `Context` line as ending a run. This is confirmed correct for full-file
  context by inspection — it was written to be engine-independent and is
  already exercised by the fallback engine's full-context diffs today.

## 3. Why this is not simply "read `FileDiff.lines` and fill gaps" for the whole engine

The brief's suggested shape — walk the full new-file text (and old-file text
for pure-deletion regions), synthesizing `Context` for every line number not
covered by a changed entry — is correct **when a gap zips 1:1**: the same
count of unmentioned lines on both sides between two anchor points. This is
the common case and is what "removed plus context equals the old file; added
plus context equals the new" already proves in aggregate for the fallback
engine.

It is **not always true for difftastic**. Verified directly against difft
0.70:

```
old:  fn a() { let x = foo(1, 2, 3); }
                                          <- one line
new:  fn a() {
          let x = foo(
              1, 2, 3,
          );
      }                                  <- reformatted to five lines
```

`difft --display json` reports **zero chunks** for this pair — the
reformatting is not semantically different enough to report — while the
line counts either side of the untouched code differ (1 vs 5). The gap
between "start of file" and the next real change has no 1:1 line
correspondence at all: there is no honest way to print "line 1 of the old
file corresponds to lines 1–5 of the new file, unchanged" as five rows each
carrying one left number and one right number, because that is not what
happened — the file was reformatted there, and difftastic told rv nothing
about how.

**This is exactly the situation the project's honesty principle exists for.**
Presenting five fabricated `Context` rows with invented left/right pairings
would be a guess dressed as a fact — indistinguishable on screen from a
region difftastic actually vouched for as line-for-line identical.

## 4. The design

### 4.1 Where full-file context is computed

**Not inside `rv_core::diff::compute*`.** `FileDiff.lines` keeps its existing
contract — difftastic's changed-only lines, or the fallback's already-full
line list — unchanged. This is deliberate: `rv-core/tests/diff/engines.rs`
and `rv-core/tests/prop_diff.rs` pin dozens of exact-shape assertions
(`diff.lines.len()`, exact `LineKind` sequences, the conservation and
ordering properties) against the current contract, and `FileDiff` is also
`rv diff --json`'s CLI shape (`rv/src/commands/diffcmd.rs`) and the CLI-loop
skills' coordinate contract — none of which asked for this change.

Instead, full-file context is a **new pure function**,
`rv_core::diff::context::merge`, taking `&FileDiff` (or just `&[DiffLine]`)
plus the old and new full file text, and returning
`Option<Vec<DiffLine>>`:

```rust
/// Interleaves `changed` (already-ordered, already-conservation-checked)
/// with synthesized `Context` lines for every gap between two anchor
/// points where the old and new side of the gap contain the same number
/// of lines — the only shape a gap can be interleaved honestly.
///
/// `None` when any gap does not zip 1:1: difftastic reported nothing about
/// that region, so which old line corresponds to which new line is not
/// knowable, and inventing a pairing would be exactly the guess the
/// project's engines never make elsewhere.
pub fn merge(changed: &[DiffLine], old_text: &str, new_text: &str) -> Option<Vec<DiffLine>>;
```

Implementation shape: walk `changed` in the order it is already in (this
order is already proven correct by `diff_lines_are_in_file_order_and_never_
repeated` / `patch_problems`), keeping a `(base_cursor, head_cursor)` pair
exactly as `prop_diff.rs::patch_problems` already does as an oracle. Between
consecutive anchor points (an `Aligned`-shaped pair, or the start/end of
file), if the count of lines strictly between them is equal on both sides,
zip them positionally into `Context` lines carrying `left = base_cursor + i`,
`right = head_cursor + i`, text taken from the **new** side (arbitrary but
consistent choice; `content_hash`/anchor resolution reads real line text
independent of this pick, and a genuinely-unchanged line has the same text on
both sides by construction of the gap — see §4.2 for why an unequal-text
same-count gap still counts as honest). If the count differs, return `None`
for the whole file — no partial success, no half-merged result, per the
project's rule against guessing.

For the `similar` fallback, `merge` is a no-op returning `Some(changed.to_
vec())` unchanged — the fallback already emits full context, so `changed`
*is* the merged result, detected by the caller's `DiffSource` matching
before calling `merge` at all.

### 4.2 Equal-count-but-different-text: still honest, not a guess

An equal-count gap can still contain trailing-whitespace-only edits difftastic
elided (confirmed: `let x = 1;   ` vs `let x = 1;` with a real change
elsewhere in the file reports zero chunks for the whitespace difference).
This is **not** the same failure as an unequal count: difftastic's own
`status`/chunk model treats whitespace-only difference as "not different," and
rv's chunk-reading contract (§7 of the branch-reviewer design) already defers
to that judgement everywhere else — a reindented *whole file* is `suppressed`
outright by the same reasoning. An equal-count gap zipped positionally is
therefore reporting exactly what difftastic already believes: these lines are
the same, texts included, modulo the exact byte differences difftastic itself
declined to surface. Printing the new side's text is consistent with "the pane
shows the file as it is now" and is not a new claim rv did not already make
for suppressed diffs.

### 4.3 Where the merged lines are consumed

`App` gains one more derived, uncached (rebuilt on demand, same discipline as
`App::plan()`) accessor:

```rust
/// The lines to draw for the selected file: full-file context where it can
/// be built honestly, the engine's own (changed-only or already-full) lines
/// otherwise.
pub fn displayed_lines(&self) -> Cow<'_, [DiffLine]>;
```

Computed from `self.selected_diff()` plus the **same old/new blob bytes
already read for `diff::compute`** in `load_selected`/`load_commit_diff`/
`request_refinement` — no new blob read, per the lazy-blob rule. Those blobs
are not currently retained on `App`; the minimal change is to cache them
alongside the diff (parallel to `diffs: Vec<Option<FileDiff>>`, a
`Vec<Option<(Vec<u8>, Vec<u8>)>>` or a struct combining them) so
`displayed_lines` can call `context::merge` without re-reading.

**`rows::plan`, `App::comments_for_line`, `App::selected_line`,
`App::line_index`'s underlying `Plan`, and `hunks::hunk_starts` all switch
from reading `selected_diff().lines` to reading `displayed_lines()`.**
Nothing about *how* they use it changes — they already handle `Context` lines
correctly (§2) — only which `Vec<DiffLine>` they are given. `FileDiff` itself
(`.source`, `.suppressed`, `.path`) is still read from `selected_diff()`
directly wherever those fields are asked for (the pane title, the suppressed
note).

Comment matching stays correct without touching the anchor model: it already
matches by `(path, side, number)` extracted from a `&DiffLine`
(`AnchorTarget`), never by array position, so pointing it at the fuller
vector changes nothing about which comment a line resolves to.

### 4.4 A file where `merge` returns `None`

Falls back to today's behavior — `selected_diff().lines` shown as-is — under
a note in the pane title, in the same voice as `NO_GRAMMAR`:
`" — full context unavailable (a reformatted region difftastic did not
report)"`. This is the extend-not-replace instruction: `ui/diff.rs::title`
gains one more optional suffix, decided the same way `NO_GRAMMAR` already is
— from the shape of the data, not inferred from timing.

### 4.5 Binary, suppressed, still-loading

- **Binary** (`DiffSource::Binary`): `displayed_lines` returns
  `selected_diff().lines` unchanged (empty) — `merge` is never called, since
  there is no text to walk. `ui/diff.rs::body`'s existing binary
  short-circuit is untouched.
- **Suppressed with no lines** (difftastic `unchanged`): `merge` is not
  called either — there is nothing in `changed` to anchor a walk from, and
  the existing `SUPPRESSED_EMPTY` sentence already tells the reviewer
  correctly that nothing is shown. This is arguably the one case where full
  context would have the most value (the file is *unchanged*, so its context
  *is* the whole file) — deferred rather than solved here: doing it requires
  distinguishing "unchanged, zero chunks" from "no gap to anchor a walk on,"
  which `merge`'s current signature cannot express without a third input
  (whether to treat the whole file as one big gap). Left as a follow-up,
  named explicitly rather than silently dropped.
- **Suppressed with lines** (fallback's terminator-only case): already full
  context by construction (§4.1) — no change.
- **Still loading** (`selected_diff()` is `None`): `displayed_lines` returns
  empty; `ui/diff.rs`'s existing "no diff loaded" branch is unreachable per
  the state-invariant test and stays unreachable.

### 4.6 Line-oriented recovery

When the syntax-aware difftastic answer has changed lines but `merge_context` returns `None`, rv automatically asks difftastic once more with `--byte-limit 0`, before presenting the existing unavailable suffix. This selects difftastic's line-oriented engine, which reports whitespace-only edits as chunks. The retry uses the existing JSON parser unchanged: it reads the same `status`, `language`, `chunks`/entry `lhs`/`rhs`/`line_number` fields; `aligned_lines` and sub-line `changes` are not part of rv's contract. If parsing or merging the retry still returns `None`, rv keeps the current changed-only result and unavailable title.

A successful retry is marked in the title as `— full context (line diff)`, composed after the normal engine label: `— difftastic (Rust) — full context (line diff)`. This is automatic, not opt-in: it only runs after the normal full-context attempt failed and either restores honest context or leaves behavior unchanged. The line diff establishes pairings using a conventional line algorithm; it does not fabricate alignment across unequal gaps. Its output may represent a whole line where syntax-aware output would represent a sub-line span. The title tells the reviewer which representation they are reading.

The resulting source shape is exactly `DiffSource::Difftastic { language: String, line_oriented: bool }`. `language` is always the FIRST (syntax-aware) invocation's answer; the second invocation's `Text (N B exceeded DFT_BYTE_LIMIT)` language is deliberately discarded. The enum records what happened to the file, not what the second invocation reported: both invocations describe the same Rust file, while the fallback engine is a separate axis from the file's language. Set `line_oriented = true` only when the second invocation successfully supplies the merged full-context result; otherwise retain false/current unavailable behavior. `--byte-limit` exists in difftastic 0.51.0, rv's minimum; no version bump.

Highlighting is provably unaffected: `rv/src/ui/diff.rs:99-105` computes spans and calls `highlight::language_of(&diff.path)` independently of `DiffSource`, and `rv-core/src/highlight.rs:113-125` selects grammar via `grammar_for_path(path)` (extension/filename). Rust files retain Rust colors even though the retry JSON says `Text`.

## 5. Default, not a toggle

Full-file context is **always on**, not gated behind a binding. The user's
own words — "I want the file viewer to show the entire file... otherwise I
lose context" — describe a standing preference about what a review pane
*is*, not a mode to flip per file. A toggle would also cost a `BINDINGS`
row, a README line and a `BROWSE_KEYS` conformance entry (`rv/src/app/
bindings.rs`, `rv/tests/app/keymap.rs`) for a feature with no natural "I want
the old broken view back" use case — the `merge`-fails-to-`None` fallback
already covers the one case where less-than-full context is shown, and it is
not the reviewer's choice, it is a fact about what difftastic reported.

**Superseded 2026-08-21:** an `f` toggle was added after the reviewer used
full-file context for a session and reported wanting to turn it off on
specific files. The honesty-fallback argument this section makes is
preserved because it is still the argument for the *default* — full context
*on* — and for why the toggle is not needed to escape a `merge`-returned
`None`. The toggle exists for the orthogonal case: a reviewer who wants to
see less on a big or noisy file, which no engine-honesty fallback can
serve. See the appendix's `§5 no-toggle stance` bullet.

## 6. Hunk navigation

Confirmed unmodified. `hunks::hunk_starts` reads `LineKind`/`left`/`right`
per element of whatever slice it is given; it has no knowledge of how many
context rows sit between two hunks today (the fallback engine already
supplies plenty) and none is added by widening the slice it runs over.

## 7. Risks

| Risk | Mitigation |
|---|---|
| `merge` invents a pairing across a reformatted region | `None` on any unequal-count gap; whole-file fallback, never a partial guess |
| Blob retention on `App` grows memory for a large review | Same lifetime as `diffs: Vec<Option<FileDiff>>` already has — one file's blobs, freed when another file's diff replaces the slot the way `Refiner` already discards stale requests |
| `displayed_lines()` rebuilt per frame duplicates `merge`'s walk cost | `merge` is a single linear walk over already-computed `DiffLine`s plus two `str::lines()` splits — no subprocess, no second diff algorithm, cost is the same order as `rows::plan` itself, which already rebuilds every frame |
| A `None`-merge file silently degrades without the reviewer noticing | Title suffix names it explicitly, following `NO_GRAMMAR`'s precedent |
| Suppressed-with-no-lines never gets full context | Named as an explicit follow-up in §4.5 rather than solved partially |
| Line-oriented recovery looks different from syntax-aware output | Mark successful retry `— full context (line diff)`; whole-line whitespace changes are truthful, and the full-context toggle remains available to turn the merge presentation off. |

## 8. Deliberately not done here

- No new `LineKind` variant — `Context` already means what is needed.
- No change to `rv_core::diff::compute*`'s public contract, `FileDiff`'s
  shape, or any existing diff-engine test's expected output.
- No change to the anchor model, `Anchor.line`'s meaning, or the resolution
  cascade.
- No new key binding.

## Implementation status (shipped 2026-08-21)

§4's design shipped as written, with one naming note: the spec above calls
the merge function `context::merge`/`rv_core::diff::context::merge`
throughout. The internal declaration is exactly that —
`rv_core::diff::context::merge` — but `rv_core::diff`'s facade re-exports it
as `merge_context` (`pub use context::merge as merge_context` in
`rv-core/src/diff.rs`), and every external caller (the `rv` crate, both test
suites) uses `rv_core::diff::merge_context`. The two names are the same
function; this is recorded rather than silently reconciled because the spec
text was written against the internal name before the facade export existed.

- **§4.1 `merge_context`** — `rv_core::diff::context::merge`
  (`rv-core/src/diff/context.rs`, a sixth sibling of
  `difftastic.rs`/`fallback.rs`/`model.rs`/`ordering.rs`/`probe.rs` in the
  existing module directory), re-exported as `merge_context`. Signature and
  gap-walk shape as designed: `None` on any unequal-count gap.
- **§4.2 equal-count-different-text** — implemented as designed;
  `fill_gap` always reads text from the new side.
- **§4.3 `App::displayed_lines`** — shipped as `App::displayed_lines(&self)
  -> Vec<DiffLine>` (owned, not `Cow`, since the merge always allocates a new
  vector) in `rv/src/app/diffview.rs`. `rows::plan`, `hunks::hunk_starts`
  (called from `app/enabled.rs` and `app/navigate.rs`), `comments_for_line`
  (`app/comments.rs`), and the jump-to-comment anchor lookup
  (`app/stack.rs::line_of_anchor`) all read from it. The blob cache is
  `App.blobs: Vec<Option<(Vec<u8>, Vec<u8>)>>`, parallel to `App.diffs`
  exactly as designed, plus `App.commit_blobs` parallel to `commit_diffs`
  for the commits view — populated at the same call sites that already read
  the blobs to compute the diff (`app/navigate.rs::load_selected`,
  `app/commits.rs::load_commit_diff`), so no second blob read was added.
- **§4.4 the title suffix** — `ui/diff.rs::title` gained a `bailed: bool`
  parameter, appended after the existing `NO_GRAMMAR` suffix, reading
  `" — full context unavailable (a reformatted region difftastic did not
  report)"`. `App::context_bailed()` is the one place that decides it, read
  by `draw_diff` and nothing else.
- **§4.5 binary / suppressed-empty / still-loading** — implemented as
  designed for binary and still-loading. **Suppressed-with-no-lines remains
  the open item named in §4.5 and the risk table**: `merge_context` is still
  never called for a chunk-less `unchanged` diff, so such a file still shows
  only the `SUPPRESSED_EMPTY` sentence rather than the file's full,
  genuinely-unchanged text. Not solved in this pass; still needs the
  signature extension §4.5 describes (a way to say "treat the whole file as
  one gap" with no anchor line to walk from).
- **§5 no-toggle stance** — superseded 2026-08-21: shipped as designed
  first, then walked back the same day when the reviewer asked for an `f`
  binding to see the changed-only view on specific files. `Command::
  ToggleFullContext`, `App::full_context: bool` (default true), and an
  `f` row in `BINDINGS`/`BROWSE_KEYS`/the README's Browsing table all
  landed together. §5's honesty-fallback argument stays as the reason the
  toggle *defaults* to on; the toggle exists for the orthogonal
  see-less-on-purpose case §5 did not model.
- **§6 hunk navigation** — confirmed unmodified in code; `hunks::hunk_starts`
  itself was not touched, only re-pointed at `displayed_lines()` by its
  callers. Its correctness under full-file context is pinned by
  `rv/tests/app/hunks.rs` and `rv/tests/app_cases/tables_1.rs`'s
  `next_hunk` case, both updated for the fact that the reviewer now opens on
  the file's first line rather than its first changed line, which moves
  where the *first* `J`/`K` press lands without changing that a later press
  still jumps straight to the next real hunk.

Tests: `rv-core/tests/diff/context.rs` (three cases — multi-hunk
conservation, identical-numbered context lines, the §3 reformatted-region
`None` case reproduced against a real `difft` binary) and one proptest
property added to `rv-core/tests/prop_diff.rs` extending the existing
ordering suite to the merged stream. `rv/tests/app/fullcontext.rs` (six
cases: no wash on context lines, wash retained on changed lines, a comment
box's row survives the fuller stream, binary and suppressed-empty diffs
never attempt a merge, the title suffix on a bailed merge). Full workspace
suite green: `cargo fmt --all --check`, `cargo clippy --workspace
--all-targets -- -D warnings`, and `cargo test --workspace` all clean, 1124
tests passing.
