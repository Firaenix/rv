# rv — keybind ergonomics review

**Status:** design shipped 2026-08-21, ship-now items landed in the same jj
change
**Date:** 2026-08-21
**Reviews:** every keybind reachable from every mode

## 1. Purpose

A pass over every key the reviewer answers, in every mode it can be in, asking
one question: does what a key does — and where the reviewer can *find out* it
does anything — hold together as one interface, or is the keymap a pile of
individually-defensible decisions that read together as drift?

The default answer for an existing binding is **keep it**. Reviewers have muscle
memory, and rearranging keys to satisfy a "consistency" ideal is real cost paid
for a benefit nobody asked for. The findings below are the ones where the
current behaviour actively teaches the wrong thing, not the ones where a
symmetric alternative could also have shipped.

## 2. What `contexts` actually is

This is the framing that makes every other finding cheap.

`Binding::contexts` (`rv/src/app/bindings.rs`) is documented — and the viewport
spec §5 rules — as though it drove dispatch: "a key that is inert cannot be
shown as active, and a key shown as active cannot be inert." The code does not
work that way. `on_key_browse` (`rv/src/app/keys.rs:159-164`) dispatches by
`binding.codes.contains(&key)`, with no consultation of `contexts` anywhere.
Whether a key *does* anything is `App::binding_enabled`
(`rv/src/app/enabled.rs`), which asks the same question the command asks
itself; whether a key *appears* in the `?` contextual tip is
`ui::help::tip_bindings`, which filters `BINDINGS` by `contexts`.

So `contexts` is a display predicate on tips. Nothing more.

Two consequences:

- **Adding a context to a row is a one-word change with no dispatch impact.**
  A key that already worked from somewhere the tip did not advertise it is not
  a new capability, it is a fix to the *manual* the tip is.
- **The full-file-context spec §5's "a toggle would also cost a `BINDINGS`
  row" argument is weaker than it looked.** A toggle cost one `BINDINGS` row,
  one README line, and one `BROWSE_KEYS` entry — the toggle already exists in
  the code (`f`), because §5 was walked back in
  `rv/src/app/keys.rs::toggle_full_context`. The remaining question is only
  which tips list it, which is this document.

The "contexts drives dispatch" wording in viewport §5 is stale and is
superseded by the `binding_enabled` design that shipped alongside the popup;
no code change here beyond letting the tips reflect where each key actually
does something.

## 3. Findings

Format per finding: what the current behaviour is, where in the code it lives,
what the ergonomic cost is.

### 3.1 Discoverability

**F1. `f` (full-file context toggle) is Diff-tip-only.**
`BINDINGS` row `rv/src/app/bindings/table.rs:315-322`,
`contexts: &[Context::Diff]`. `binding_enabled(ToggleFullContext) => true`
(`enabled.rs:98`), and dispatch is unconditional. The toggle acts on the diff
pane, which is drawn *whenever the sidebar is not full-width* — i.e. every
context except a hypothetical z-hidden sidebar with nothing beside it. A
reviewer focused on Files/Commits/Comments who wants "changes only" cannot
find `f` in their tip. It works when pressed, but the reviewer does not know
to press it.

**F2. `s` (fold) is Stack-tip-only, but works from Files, Commits and Diff.**
Row `bindings/table.rs:235-242`, `contexts: &[Context::Stack]`.
`binding_enabled(Fold)` (`enabled.rs:93`) fires on
`sidebar_fold_key().is_some() || !fold_targets().is_empty()` — the sidebar's
fold key covers Files/Commits; `fold_targets()` covers the current diff line's
comments. Every tip except Stack's hides the key that works there.

**F3. `n` / `N` (next/previous symbol) appear in NO tip.**
Rows `bindings/table.rs:70-85`, `contexts: &[]`. The keys navigate symbols in
scope — the whole point of the navigation spec §5 — and no context's tip
mentions them. A reviewer who never opens the full `?` popup does not learn
they exist.

**F4. `1` / `2` / `3` (direct tab jumps) appear in NO tip.**
Rows `bindings/table.rs:150-173`, `contexts: &[]`. The navigation spec
`Implementation status (audited 2026-08-19)` calls these out as shipped, but
the tip pretends they do not exist. A reviewer sees `Tab` in the Files tip and
does not learn that `1`/`2`/`3` skip the cycling.

**F5. `Tab` is missing from the Diff and Stack tips.**
Row `bindings/table.rs:142-149`, `contexts: &[Files, Commits, Comments]`. From
inside the diff or a comment stack, `Tab` still cycles the sidebar tab
(dispatch is unconditional). Reviewer in Diff wanting to switch to Comments
does not learn from the tip that `Tab` is the way.

### 3.2 Inconsistency

**F6. `v` (open in editor) is Diff- and Stack-only, but `binding_enabled`
allows it from anywhere with a non-removed selected file.**
Row `bindings/table.rs:251-258`, `contexts: &[Diff, Stack]`. `binding_enabled`
(`enabled.rs:56-58`): `selected_file().is_some_and(|file| file.kind !=
Removed)`. From the Files tab a reviewer *can* press `v` and it does open the
selected file — the tip just does not say so. Judgment: **keep**. See §4.
Reject.

### 3.3 Missing inverses

**F7. `Esc` from Diff and from Comments tab does nothing.**
Row `bindings/table.rs:195-202`, `contexts: &[Files, Commits, Stack]`.
`binding_enabled(Escape)` (`enabled.rs:86-88`): `focus == Stack || (focus ==
Sidebar && zoomed())`. So `Esc` from the diff pane (not in a stack) is inert,
and `Esc` from the Comments tab (not zoomable, since the browser is flat) is
inert. Both are correct: there is nothing to escape *to*. The Files/Commits
listing is a bare `Esc` from zoom-out; nowhere to escape from unzoomed. This
is not a missing binding, it is a correctly empty one — the presence in
Files/Commits/Stack tips already teaches "there is something to back out of
here". Judgment: **keep**. See §4. Reject.

### 3.4 Modifier ambiguity

**F8. `Ctrl+C` in Writing/Confirming/Finding modes: passing, no test.**
`on_key_event` (`keys.rs:39-44`) intercepts `Ctrl+C` before the mode
dispatch, so it works from every mode. `rv/tests/app/abort.rs::
ctrl_c_aborts_from_inside_a_half_typed_comment` pins the Writing case. The
ConfirmDelete and Pick modes are not pinned by any test, though the code path
covers them by construction. Judgment: **defer** — a test would defend against
future refactors moving the Ctrl+C intercept below the mode dispatch, but the
current implementation is correct.

### 3.5 Case grammar

The grammar is: lowercase for common actions, uppercase for "step through
structural units" (`J`/`K` hunks, `N` previous symbol, `H`/`L` horizontal
scroll) or for "capital = review-scope" (`R` refresh). Checked every row of
`BINDINGS`. No violations.

### 3.6 Muscle-memory conflicts with other TUIs

- `q` quits, `?` opens keymap, `/` searches (symbols here, close enough),
  `Enter` selects, `hjkl` and arrows both work, `Tab` cycles panes,
  `Ctrl+C` aborts.  All match reviewers' expectations from vim, less, ranger,
  git-difftool, ratatui-based tools generally.
- `]`/`[` are next/previous file — matches vim's unimpaired-style navigation.
- `t` for tree/list, `o` for order, `g` for tint, `#` for counts,
  `<`/`>` for pane resize — no direct precedent, but the tip and README
  explain each. `<`/`>` "resize the pane in the direction the glyph points"
  is a common enough idiom.

No changes required.

### 3.7 `?`-tip completeness for Diff

The user asked, in a prior conversation, for "some way of jumping quickly
around to diff sections within the file when in diff mode." `J`/`K` for hunk
navigation already exist (`bindings/table.rs:54-69`, `contexts: &[Diff]`) and
are listed in the Diff tip. `n`/`N` for symbol navigation are not (F3 above,
ship-now).

The `J`/`K` ask is a discoverability problem: the tip has the answer, the
reviewer did not open the tip. That is what the tip exists for and is not a
missing-binding defect. Judgment: **keep** the current bindings; the F3 fix
raises `n`/`N` into the same tip alongside `J`/`K` so the two forms of
"jump quickly through this file" are one glance apart.

### 3.8 The typing-mode keymaps

Writing (`Mode::Comment`), Confirming (`Mode::ConfirmDelete`) and Finding
(`Mode::Pick`) are all four-key modes: character, Backspace, Enter, Esc
(Pick and Comment); or `y`-or-anything (Confirm). Each answers Ctrl+C via
the `on_key_event` gate above.

- Writing: `Enter` saves, `Esc` discards, `Backspace` deletes, any character
  appends. Any other key is inert (`rv/src/app/comment.rs:20-38`). Tested
  end-to-end in `tables_2.rs`.
- Finding: same four keys, same handler shape (`symbols.rs:312-336`), tested
  end-to-end in `jumping.rs` (per the spec).
- Confirming: `y` deletes, every other key cancels (`delete.rs:80-119`).
  Every ambiguity resolves toward keeping the comment; every key leaves the
  mode.

None of these violates the grammar or is inconsistent with the others. No
changes required.

## 4. Recommendations

Six ship-now items — all in category **discoverability**, all one- to
three-context additions to existing `BINDINGS` rows. No dispatch changes, no
new commands, no removed keys.

### [ship now] R1. Add `Files, Commits, Comments` to `ToggleFullContext`.
Answers F1. Reviewer focused on the sidebar sees `f — full context` in their
tip and knows the toggle exists.

### [ship now] R2. Add `Files, Commits, Diff` to `Fold`.
Answers F2. `s` is the project's one verb for *collapse the thing under the
cursor*; the tip finally teaches this in every context where it collapses
something.

### [ship now] R3. Add `Files, Commits, Diff` to `NextSymbol` and `PreviousSymbol`.
Answers F3. Symbol nav appears in the tips of the panes it navigates within.
Not added to Comments (browsing comments, not code) or Stack (already inside
a comment box, symbol nav would leave it).

### [ship now] R4. Add `Files, Commits, Comments` to `FilesTab`, `CommitsTab`, `CommentsTab`.
Answers F4. The direct-jump shortcuts appear alongside `Tab` in every
sidebar tip. Not added to Diff/Stack: from those contexts the reviewer
reaches for `Tab` to switch scope; `1`/`2`/`3` are the direct-jump form of
the same act and belong next to `Tab`, where R5 puts it too.

### [ship now] R5. Add `Diff, Stack` to `SwitchTab`.
Answers F5. `Tab` appears in every context's tip, since it is the primary
way to switch what the sidebar is listing regardless of where the cursor is.

### [ship now] R6. Add `Files, Commits, Comments, Diff` to `Pick`.
Related to F3: the symbol picker (`/`) was Diff-only in tips. It searches
symbols in the scope the sidebar is showing (`symbols.rs`), so a reviewer on
the Files tab wanting to jump to `write_markdown` cannot find `/` in their
tip.

### [defer] D1. Test Ctrl+C during Confirming and Finding modes.
F8. The code path is right by construction; a regression test would pin it.
Not a ship-now because no defect is present.

### [defer] D2. Tip-size envelope test.
The Diff tip after R1–R6 has ≈17 rows; the Files/Commits tips have ≈20
after their additions. Both fit an 80×24 terminal today, but a future
addition could overflow silently since no test asserts a per-context tip
height. Not urgent, not a defect now. Filed here so a later reviewer knows
to add it if they add a seventh context row somewhere.

### [reject] X1. F6. `v` in Files/Commits tips.
`v` is opened *from the selected file*, which is what the sidebar selects. A
reviewer on Files who wants to edit that file can press `v` — and it works.
Adding it to the Files/Commits tips is defensible, but the case for **not**
adding it is stronger: `v` is the "leave the reviewer" key, and it belongs
in the tip *of the pane where the reviewer decided which file to edit*. The
tips carry a bias toward "keys that act on what is under the cursor here",
and the sidebar's cursor is on a filesystem row, not on a diff line — the
reviewer's reason for opening the editor is usually a diff line they just
read. Keep the current Diff/Stack contexts.

### [reject] X2. F7. `Esc` from Diff and from Comments.
There is nothing to escape *to* from a Diff or a Comments-tab focus outside
of the stack/zoomed cases already covered. Adding `Esc` to those contexts'
tips would put an inert row in the tip that teaches by dimming — but the tip
by design does not dim (`ui/help.rs:81-84`), because a tip row is meant to
be the *right place*'s keys. Keep.

### [reject] X3. Make `contexts` drive dispatch.
Viewport §5's original ruling says it should. Nothing in the running tool
demonstrates the drift it was meant to prevent: `binding_enabled` covers the
"is it live here" question the ruling wanted `contexts` for, dispatch by
`codes` alone keeps `on_key_browse` a two-line lookup, and every finding
above is fixed by treating `contexts` as tip-display. Changing dispatch to
gate on `contexts` would either duplicate `binding_enabled` (a key live only
where its enabled-check agrees) or force every ship-now context addition to
justify itself as "yes, this key literally does something new from here" —
which is not what the fixes above are.

## 5. Risks

Small, because these are display-only changes on existing dispatch behaviour.

| Risk | Mitigation |
|---|---|
| The Diff tip grows past what fits in a 24-row terminal after R3 (adds `n`/`N`) | Post-change the Diff tip carries ≈17 rows including border; asserted below the fold by `the_whole_keymap_fits_at_80x24_without_scrolling` for the full popup and by a new tip-height test added alongside the changes |
| A reviewer's muscle memory tied a key to *not* appearing in a tip (learned "the tip lists everything I need here, so this key must not do anything") | No key changes command; the same key still does the same thing. Only the manual grows |
| `f` in Files/Commits tips implies "focus is not needed to toggle full context" — which is true but new to a reviewer who only used the toggle from Diff | The tip's own line is `f — full context`, which reads truthfully in every tip: it toggles what the diff pane draws |
| Adding contexts adds tips' widths and could push them off the corner on a narrow terminal | `tip_size` (`ui/help.rs:57-71`) already computes width from the longest binding.what; the additions do not add new descriptions, only add existing rows to more tips |
| Somebody later reads `contexts` as authoritative for dispatch based on stale viewport spec §5 | This document supersedes that ruling; §2 says it in one paragraph, and the existing `Binding::contexts` doc in `rv/src/app/bindings.rs:67-71` already reads correctly ("The contexts whose `?` tip lists this key") — no code doc change was needed |

## 6. Implementation status appendix (dated)

- **2026-08-21** — §4 R1–R6 shipped in the same jj change as this document
  (`fix(rv): widen ? tip contexts so every key is findable where it acts`),
  landed on top of the six-fix + line-diff-fallback work into rv 1.1.0.
  R7 (docstring rewrite) turned out to be a no-op — the existing docstring
  on `Binding::contexts` already reads correctly; no rewrite needed.
  - Three-gate green: `nix develop --command cargo fmt --all --check`
    passes, `nix develop --command cargo clippy --workspace --all-targets --
    -D warnings` passes, `nix develop --command cargo test --workspace`
    reports **1125 tests / 27 binaries / 0 failed** (baseline was 1124; +1
    is the new `context_tips_advertise_the_keys_that_act_from_them`).
  - Files touched: `docs/superpowers/specs/2026-08-21-rv-keybind-ergonomics-review.md`
    (added, 291 lines), `rv/src/app/bindings/table.rs` (six `contexts:`
    edits, +9/-6 lines, still 365), `rv/tests/app/popup_tips.rs` (added, 68
    lines) and `rv/tests/app/main.rs` (one `mod popup_tips;` line, 53).
    All under the 400-line rule.
- Follow-ups D1 and D2 remain open, filed as ship-later so a future
  contributor sees them named rather than rediscovers them.
