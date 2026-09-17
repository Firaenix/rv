# rv — review flags and reviewed-file tracking

**Status:** draft, awaiting review
**Date:** 2026-09-15
**Builds on:** `2026-08-17-rv-inline-comments-design.md` (comment rendering, sidebar,
collapse), `2026-08-17-rv-storage-model-design.md` (`.review/session.toml`,
`Anchor` resolution)

## 1. Purpose

Two gaps found by using rv to review rv:

1. **Pointing at something isn't feedback.** "Look here, and here's why" is
   today faked with a `Comment` — which drags in reply/resolve semantics that
   don't apply, and clutters the comment browser with notes that aren't
   requests for change. There is no native, lighter-weight way to say *look
   here* with a reason.
2. **Nothing remembers what you've already read.** A file with no open
   comments looks identical whether you scrutinised every line or never
   opened it. Returning to a review after a break means re-scanning the file
   list from scratch, with no record of where you left off.

This spec adds two independent primitives — **flags** (§2–§5) and
**reviewed files** (§6–§9) — and one deliberate coupling between them (§8):
ticking a file off folds its flags and comments so a re-opened review shows
only what's still outstanding.

## 2. Flags: what they are, and what they are not

| | Comment | Flag |
|---|---|---|
| Says | "this needs to change" or "explain this" | "look here — here's why" |
| Lifecycle | open → resolved / abandoned, carries a reply | open → acknowledged, no reply |
| Author | reviewer or agent, either can act on it | typically an agent doing a first pass, but either can create one |
| Rendered as | bordered box, blue | single-line inline marker, amber |
| Blocks `rv status --check` | yes, while open | no, never |
| Lives in the comment browser | yes | no — its own list |

A flag is an **attention pointer, not a review comment**. It never blocks the
agent-loop's `rv status --check` gate, because nothing about it asks for a
code change — conflating the two would make an agent's "here's what I
touched" markers indistinguishable from "here's what's broken."

**Ruling — naming avoids `app::Focus`.** The TUI already has
`enum Focus { Sidebar, Diff, Stack }` for *where the cursor is*
(`rv/src/app/mode.rs:32`). This feature is called **`Flag`** throughout —
store type, CLI verb, sidebar tab — specifically to keep "cursor focus" and
"reviewer attention" from sharing a name in code, docs, or keybindings.

## 3. Data model

New store type, alongside `Comment` in `rv-core/src/store.rs`'s territory but
split into its own submodule (`rv-core/src/store/flags.rs`) to keep
`store.rs` under the 400-line ceiling it is already brushing against (469
lines today):

```rust
pub struct Flag {
    pub id: String,
    pub change_id: String,
    pub commit_id: String,
    pub anchor: Anchor,
    pub reason: String,
    pub acknowledged: bool,
}
```

- **No `state` enum, no `reply`, no `settled_by`.** A flag has exactly one
  transition: unacknowledged → acknowledged. Modelling it as a two-state
  `bool` rather than borrowing `CommentState` keeps "flag" from silently
  growing comment-shaped features later — if it ever needs more states than
  that, it has become a comment and should be one.
- `id` derives the same way `comment_id` does
  (`rv-core/src/anchor.rs:183`, `change_id` + path + side + line + body
  hash), so the collision properties already proven for comments carry over
  without new code.
- `Session` gains `pub flags: Vec<Flag>`, written through the same
  `write_atomic` session rewrite as comments — one file, one atomic write,
  no new failure mode.
- **Staleness reuses `anchor::resolve`.** A flag whose anchor no longer
  matches its stored `content_hash` renders with the same `Confidence`
  machinery comments already use (`rv-core/src/anchor.rs:113`) — `Moved`,
  `Weak`, `Outdated`. No new resolution logic; `App::confidence` and
  `App::drift` become `Comment`-or-`Flag` generic, or gain a `Flag` twin.

## 4. CLI surface

Mirrors the existing `comment`/`comments`/`reply` triad
(`README.md:109-111`) so the agent loop gains a natural way to point without
opening the TUI:

| Command | Effect |
|---|---|
| `rv flag <file> --line <n> [--side left] -m <reason>` | Creates a flag. `-m -` reads from stdin, same as `rv comment` |
| `rv flags [--json] [--open\|--acknowledged]` | Lists flags — id, reason, anchor, state |
| `rv ack <id>` | Marks a flag acknowledged (the reviewer's or an agent's "seen it") |

`rv status --check` is unchanged: flags never gate it. `rv status` gains a
flag count alongside the existing comment counts, because "3 open comments,
5 unacknowledged flags" is the number a reviewer actually wants before
opening the TUI.

## 5. TUI: rendering, navigation, keymap

### Rendering

Flags render **inline, one line, no border** — the visual language a comment
box uses (border + title + body) is reserved for comments, so a flag must
not look like one at a glance:

```
   42 +     let digest = blake3::hash(seed.as_bytes());
      ⚑ this is where the collision window shrinks from 8 hex to 4
```

Acknowledged flags render dim with the glyph hollowed (`⚐`), collapsed by
default the same way a dismissed comment is — present in file/line order,
never hidden. Long reasons wrap like comment bodies; there is no expand/
collapse *content* distinction the way a comment box has, since a flag has
no body/reply split to hide.

**Ruling — flags get their own rows in the diff plan, not comment rows.**
`rows::Row` (`rv/src/rows.rs:68`) gains `FlagLine { line, flag_index }`
alongside the existing `Box*` variants, built by `rows::plan` from a
`flags_for: &dyn Fn(usize) -> Vec<&Flag>` argument passed the same way
`comments_for` is today. Flags always render **above** a line's comment
boxes (attention before feedback) when a line carries both.

### Sidebar: a fourth tab

`SidebarTab` gains `Flags`, listed and browsed exactly like `Comments`
(`rv/src/app/sidebar/browser.rs:8` — `BrowserRow` gains a `Flag` variant next
to `Dir`/`File`/`Comment`): grouped under a bold file heading, `Enter` jumps
to the code, `d` deletes with the same y/n confirmation comments use.

`Tab` cycles Files → Commits → Comments → Flags → Files. The three existing
`Leader::Goto` bindings (`gf`/`gc`/`gC` or however they're currently mapped —
see `rv/src/app/bindings/table.rs:187` `PaneCommand::Goto*`) gain a fourth,
`GotoFlags`.

### Keymap additions

| Key | Effect |
|---|---|
| `f` (in `Focus::Diff`) | flag the selected line — prompts for a reason the way `c` prompts for a comment body |
| `a` (in `Focus::Diff` or the Flags tab) | acknowledge the targeted flag |
| `]f` / `[f` | jump to next/previous flag across the whole review — the same "browse what you were told, not the tree" case §3 of the inline-comments spec makes for comments applies here, arguably more so, since flags exist specifically to be jumped to |
| `d` | delete the targeted flag (Flags tab or `Focus::Diff`), same confirmation flow as comment deletion |

`]f`/`[f` rather than overloading `]`/`[` (already file navigation) or `n`/`N`
(unbound today, but `vim`'s search-result convention risks the wrong
muscle-memory expectation — this isn't a search).

## 6. Reviewed files: scope

A file can be reviewed **in two different contexts**, and marking it
reviewed only means something relative to the one you were looking at:

- **Overall** — the file's diff across the whole `base_commit..head_commit`
  range, the aggregate view the Files tab shows by default.
- **Per-commit** — a single change's diff for that file, reached by drilling
  into a commit (`rv/src/app/commit_diff.rs`).

Reviewing a file's *overall* diff and reviewing its contribution to *one
commit in the stack* are different claims — the first says "I've seen
everything this file changed," the second says "I've seen what this commit
did to it." Conflating them would either under-claim (re-reviewing overall
after checking one commit) or over-claim (marking overall reviewed from a
partial, per-commit look).

```rust
pub enum ReviewedScope { Overall, Commit(String) } // change_id

pub struct ReviewedFile {
    pub scope: ReviewedScope,
    pub file: String,
    pub content_hash: String,
}
```

`Session` gains `pub reviewed: Vec<ReviewedFile>`, in a new
`rv-core/src/store/reviewed.rs` submodule, same atomic-write treatment.
`content_hash` is the hash of the diff's right-hand content at mark time —
the same staleness contract as flags and comments: if the file changes after
being ticked, the tick does not silently keep claiming a file that no longer
exists in the form it was reviewed in.

**Ruling — a commit-level tick never rolls up into an Overall tick, or vice
versa.** Reviewing every commit that touches a file is not the same act as
reviewing the merged result — rebases and multi-commit edits to the same
lines mean the union of per-commit diffs can read differently from the
overall diff. Two separate ticks avoids one silently vouching for the other.

## 7. Rendering the tick

| Where | Rendering |
|---|---|
| Files tab row (`rv/src/ui/files.rs::row_mark`) | `✓` before the existing change-kind marker; `≈✓` if stale |
| Comments (and Flags) tab file heading | same glyph, so the heading a reviewer jumps through already tells them whether to bother |
| Commits tab row | derived aggregate — `✓` if every file the commit touches is ticked *for that commit's scope*, else nothing (no partial glyph; a commit is reviewed or it isn't) |
| Diff pane title (`rv/src/ui/diff.rs::title`) | appended, e.g. `path — difftastic (rust) — ✓ reviewed`, or `— ✓ reviewed, 2 open comments` if it still carries open comments — ticked and unresolved are not mutually exclusive, and hiding one behind the other would be the guess-as-fact mistake this project has already burned itself on once (anchors, see implementation history) |

**Ruling — the commit-level tick is always derived, never stored.** Storing
a separate per-commit rollup risks it drifting from the per-file ticks it
should summarise — the project's standing rule against two cursors kept in
sync by hand (see the inline-comments spec's browser-row ruling) applies
here too.

## 8. Ticking folds flags and comments — the one coupling

Marking a file reviewed (in whichever scope you're viewing it) also folds
every flag and comment anchored to that file **within that scope**:
comments/flags get added to the in-memory collapsed set
(`rv/src/app/fold.rs`), and unacknowledged flags are **not**
auto-acknowledged — folding is a view change, acknowledging is a claim about
having read it, and ticking the file already makes that claim at the file
level. Acknowledging every flag individually on top would be redundant
bookkeeping for no new information.

**Ruling — this reconciles two rulings that would otherwise conflict.** The
inline-comments spec rules that collapse state is "in-memory and
session-scoped... does not belong in `.review/`." Reviewed-file state must
be durable — the entire point is surviving a restart. The reconciliation:
**`reviewed` is the durable fact, `collapsed` remains derived.** On session
load, `App` seeds the collapsed set from `store.reviewed()` before the first
frame — collapse state itself is never written to disk, only recomputed
from what is.

**Ruling — unticking never force-expands.** Folding on tick is a
convenience; the reverse isn't assumed. A reviewer un-ticking to re-examine
a file almost always wants to expand what they're re-checking manually, and
guessing which ones would just be wrong some of the time.

**Ruling — a stale tick still folds.** Content drift changes the glyph
(`✓` → `≈✓`) but not the fold; the reviewer decides whether to re-open,
rather than the tool re-expanding everything on their behalf the moment a
file changes underneath them.

## 9. CLI and keymap

| Command | Effect |
|---|---|
| `rv review <file> [--commit <change-id>]` | Marks reviewed for the given scope; omit `--commit` for Overall |
| `rv unreview <file> [--commit <change-id>]` | Clears it |
| `rv reviewed [--json]` | Lists reviewed files, their scope, and stale/current |

TUI: `r` toggles reviewed for the file under the cursor (Files tab) or the
file currently open in the diff pane, scoped to Overall or the commit
currently being viewed depending on which view you're in — never a prompt
asking which scope, since the view you're looking at already answers that.

## 10. Non-goals

- No flag editing — delete and re-flag, same posture as comments.
- No flag severity/priority levels. One kind of flag, one reason string. If
  triage by severity turns out to matter, that's a follow-up once there's
  dogfood evidence, not a speculative enum now.
- No partial/aggregate glyph for commit-level review (§7) — a commit is
  reviewed or it isn't.
- No cross-reviewer state. Same single-reviewer model `.review/` already
  assumes everywhere else.
- No automatic re-flagging or re-un-reviewing when unrelated files change —
  staleness is per-anchor and per-file, exactly as scoped today for comments.

## 11. Risks

| Risk | Mitigation |
|---|---|
| Flags become comments-by-another-name over time (state creep) | No `state` enum by design (§3); resist adding one under feature pressure — a flag that needs more than open/acknowledged is a comment |
| `Overall` vs `Commit(id)` scope confuses reviewers about what a tick claims | Diff pane title always names the scope implicitly by matching whatever's on screen; `rv reviewed --json` prints scope explicitly for scripting |
| Seeding collapsed-from-reviewed at load time re-introduces the "two things in sync" bug class §8 warns about | Collapse is recomputed fresh from `store.reviewed()` every load, never itself persisted or diffed against a previous in-memory copy |
| Fourth sidebar tab makes the tab bar cluttered at narrow widths | Out of scope here; `followup-comments-pane-and-config-toml` note already flags the sidebar-at-narrow-width problem as a separate follow-up |

---

## Implementation status (shipped 2026-09-15, v1.8)

Implemented as specified, with these rulings made while building it:

- **§5 keymap** — `F` flags the line and `A` acknowledges, as direct keys.
  They shipped that way because a leader with one live child used to collapse
  onto it (how a plain `c` wrote a comment), and a second child under `c`
  would have cost every comment a keystroke; the day after, that collapse
  was removed altogether and *every* mutating key became a capital — `C D R
  A F X E` — so the flag keys turned out to be the rule rather than the
  exception. The flag walk is `g f` / `g F`, under the goto leader beside
  the hunk and symbol walks, rather than `]f` / `[f`. `X` ticks a file.
- **§5 sidebar** — the Flags tab shipped the same day, after the first
  dogfood found the walk alone gave no way to *find* a flag: `m F` / `Tab`
  reach it, it lists flags under their file headings exactly as the Comments
  tab lists comments (`Enter` jumps, `A` acknowledges, `s` folds, `d`
  deletes), and a file or change row in the Files/Commits lists carries `⚑`
  while a flag under it is open — the column exists only in a review that
  has open flags, so the names pay for it only where there is something to
  find.
- **§3 staleness** — a flag is matched to its line by its stored anchor,
  like a comment; the drift survey (moved / weak / outdated) is not yet run
  for flags.
- **§4 CLI** — `rv unflag <id>` added; `rv flags --open` filters.
- **§7 tick rendering** — the tick takes the file icon's column rather than
  a column of its own: two columns on every row to say something about a few
  cost the names more than the tick is worth. A change row carries its
  derived tick in the fold mark's third column.
- **§9** — `x` on a change row ticks every file it touched (or clears them
  all when all are ticked): the row is a list of files and the key means
  "these".

## Open questions for review

1. Is `Flag`/`flag`/`⚑` the right vocabulary, or does it read as "CLI flag"
   too easily in prose and docs?
2. Should `rv flag` support a `--side`/`--commit` pair the way `rv comment`
   implicitly does via the diff being viewed, for parity with the reviewed-
   file CLI's explicit `--commit`?
3. Is per-commit reviewed scope actually needed on day one, or should it
   ship as Overall-only first and per-commit added once the stacked-review
   workflow it's meant to serve is dogfooded?
