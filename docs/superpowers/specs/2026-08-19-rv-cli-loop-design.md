# rv — the agent loop is the CLI, and the markdown is a view

**Status:** implemented 2026-08-19. One deliberate deferral: §2's "mangled-input
test corpus goes" waits for the release after — the parser it defends still runs
on every load as §5's migration, and tests defending live code stay while it
lives. They are deleted together.
**Date:** 2026-08-19
**Amends:** `2026-08-17-rv-branch-reviewer-design.md` §2.5 (the handoff), §10 (round-trip surface);
`2026-08-17-rv-storage-model-design.md` §5 (the export)
**Affects:** `.claude/skills/rv-reviewer/SKILL.md`, `.claude/skills/rv-worker/SKILL.md`

## 1. The problem

The original design said *"the LLM handoff is a file, not an integration"* to
mean no network, no API, no model calls. That constraint aged into the file
being read literally: `REVIEW-FEEDBACK.md` became a **writable database with an
unreliable writer**, and everything hard about it follows from that one
mistake.

The agent loop today runs CLI-first everywhere it can:

| Need | How it is done | CLI-native? |
|---|---|---|
| See scope and status | `rv status --json` | yes |
| Leave a comment | `rv comment <file> --line <n> -m` | yes |
| Settle | `rv resolve <id>` / `rv abandon <id>` | yes |
| **Read comment bodies and excerpts** | read `REVIEW-FEEDBACK.md` | **no** |
| **Reply to a comment** | append `**Reply:**` at column 0, then `rv render` to ingest | **no** |

Two gaps force the markdown round trip: `rv status --json` carries only
*counts*, and the only reply channel is editing the export and having
`session::fold_replies` rescue the edit before the next rewrite destroys it.

What the round trip costs, all of it in service of those two gaps:

- A parser hardened against LLM mangling — unbalanced fences, broken
  `<!-- rv: -->` markers, replies that are "at column 0, never indented, never
  inside a list — or not read at all".
- `fold_replies` must run before *every* rewrite, an ordering rule with data
  loss on the other side of it.
- Stale-export machinery: `rv status` reports it, the TUI status line warns
  about it, the exported document timestamps itself.
- Rules the agent obeys by convention only: never edit markers, never reorder
  sections, one reply per comment, never write a state into the document.
- A test corpus for mangled input (`prop_markdown`, golden tests) defending a
  surface that exists only because the reply channel is a text file.

A CLI call either succeeds or exits 1. No parsing, no ingest ordering, no
column-0 folklore. The skills (`rv-reviewer`, `rv-worker`) already teach agents
the CLI; the file's one remaining advantage — self-description via the
`For LLMs:` header — is done better by the skills themselves.

## 2. The decision

**Agents read the review and reply to it through the CLI. The markdown becomes
a one-way view: rendered on request, never read back.**

Five additions, one demotion:

### `rv comments --json` — the read channel

The full comment list, everything the store and a load can say:

```json
[
  {
    "id": "6ce52206",
    "change_id": "ytskpxpwyunutroxvszuoklmnnrrxlkq",
    "commit_id": "62ba3a58bd6…",
    "state": "open",
    "settled_by": null,
    "outdated": false,
    "body": "4-hex ids collide at review scale.",
    "reply": null,
    "anchor": {
      "file": "rv-core/src/store.rs",
      "side": "right",
      "line": 238,
      "context_start": 233,
      "context": ["    pub fn append_comment(…", "…"]
    }
  }
]
```

- `outdated` is in the output and **still never in the store**: `rv comments`
  is a load, and the derived-on-load rule (storage spec §3) applies to it
  exactly as it applies to `rv status` and the TUI. The three of them read the
  same review or the rule is broken.
- The same `in_range` filter the TUI and `rv status` apply: a comment this
  range cannot display is a comment a script acts wrongly on.
- `--state open` filters, because "what is waiting on me" is the worker's
  first question and it should not need `jq` to ask it.
- Text output without `--json` exists and is for humans; the JSON is the
  contract.

### `rv reply <id> -m "<text>"` — the reply channel

Writes the reply straight into the stored comment, through the same
store-then-reload discipline `resolve` and `abandon` use.

- An unknown id is an exit-1 error naming the id — unlike settling, which is
  idempotent by design. A reply is *content*; silently dropping content
  because of a typoed id is the markdown failure mode this command exists to
  delete.
- A second reply replaces the first, which is the reading the round trip
  already had ("two replies under one id leave the last one written").
- Replying does not change state. A comment with a reply is open with a reply
  — the storage spec already ruled this — and `rv resolve` remains its own,
  separate, deliberate act.

### `rv diff [<file>] --json` — the coordinate source

The reviewer skill today tells the agent to read what changed with `jj diff`
and then comment with `rv comment --line`. That is a coordinate-system leak:
`jj diff` emits unified hunks in its own numbering with no side vocabulary,
while `rv comment` expects rv's side-aware numbers — right = the head file,
left = the base file, difftastic-aligned. The agent translates between the
two by inference, and a mistranslation is either a refused comment (the good
case) or an anchor on a plausible-looking *wrong line* (the bad one).

rv already computes the right structure per file — `FileDiff` carries sides,
both line numbers, change kinds and suppression — so the fix is to say it:

```json
{
  "file": "rv-core/src/store.rs",
  "engine": "difftastic",
  "language": "Rust",
  "suppressed": false,
  "lines": [
    { "kind": "context", "left": 237, "right": 237, "text": "…" },
    { "kind": "removed", "left": 238, "right": null, "text": "…" },
    { "kind": "added",   "left": null, "right": 238, "text": "…" }
  ]
}
```

- **The numbers the reviewer comments with are numbers rv itself printed.**
  The same one-source-of-truth discipline that fixed the pane/anchor
  disagreement and the layout/hit-test split: the tool that validates the
  anchor is the tool that issued the coordinates.
- Without `<file>`, every file in the range, one object per file. Diffs stay
  lazily computed per file, as the pipeline already works — this is a query
  iterating them, not an eager whole-range load.
- `engine` and `suppressed` are stated, per the standing rule that a degraded
  or suppressed diff is never presented as a structural one. A suppressed
  file reports its lines exactly as the pane shows them, note and all.
- Binary files report `"binary": true` and no lines.
- The reviewer's loop drops its `jj` dependency entirely: rv becomes
  self-sufficient for the whole of its own loop.

### Bodies from stdin — `-m -`

`rv comment` and `rv reply` take bodies as shell arguments, which makes an
agent's precise multi-sentence finding — backticks, quotes, `$`, newlines —
one quoting mistake from a mangled comment. Shell-quoting failures are
silent corruptions, not exit-1 errors, which is the failure class this whole
amendment deletes elsewhere.

`-m -` reads the body from stdin, the `git commit -F -` convention:

```sh
rv comment rv-core/src/store.rs --line 238 -m - <<'EOF'
`content_hash` is computed from the untrimmed line, so re-indenting breaks
every anchor — hash the trimmed text.
EOF
```

An empty stdin body is refused exactly as an empty `-m` argument is.

### `rv status --check` — the gate

Exit 0 when no comment is open, 1 when any is, nothing printed. The worker's
poll ("is there work?") and a CI gate ("is the review clean?") are both
exit-code questions, and both currently need `--json | jq`. `--check`
composes with `--json` — print the report *and* set the code — and fits the
existing exit-code discipline: 0 success, 1 rv-level condition, 2 usage.

### `rv render` — demoted to a view

- `rv render` prints the markdown to **stdout**. It is a projection for
  reading — a human in a pager, an agent that prefers prose — and nothing
  reads it back, so where it lands is the caller's business.
- `rv render --out <path>` (and the TUI's `e`) still writes a file, for
  whoever wants an artefact to attach or archive. The file carries no
  round-trip duty: the `For LLMs:` protocol block is replaced by one line
  naming the CLI (`rv comments --json`, `rv reply`, `rv resolve`), and the
  `<!-- rv:anchor -->` markers stay only as provenance for the ids they name.

**Ruling — ingest is deleted, not deprecated.** `session::fold_replies`, the
reply-parsing half of `rv-core::markdown` (`parse_replies`), the
ingest-before-rewrite ordering rule, the stale-export field in
`rv status --json`, the TUI's ingest-on-launch, and the mangled-input test
corpus all go. Keeping a second write path "just in case" keeps every parser
and every ordering rule alive to serve it, which is the cost this amendment
exists to stop paying. A reply sitting in an old export when this version
lands is rescued once by migration (§5) and never again.

**Ruling — the export stops being written as a side effect.** Saving,
settling and replying no longer rewrite `REVIEW-FEEDBACK.md`; only `rv render`
and `e` produce it, on request. The side-effect writes existed to keep a
round-trip surface fresh for a reader that might write into it. No such reader
remains, and a file nothing reads back cannot be dangerously stale — the
stale-export warning goes with the danger.

## 3. What "the handoff is a file" now means

The constraint that produced the export was three prohibitions and one
convenience. The prohibitions stand untouched: **no network, no API, no model
calls** — `rv` still never talks to an LLM, and a local subprocess invoking
`rv` is none of those things. The convenience — a self-describing document —
moves to the skills, which already exist, already teach the CLI, and unlike a
header block are read *before* the agent starts rather than if it happens to
scroll up.

The worker's loop becomes:

```
rv status --check                    # is there work? (exit 1 = open comments)
rv comments --json --state open      # what exactly?
…fix the code…
rv reply 6ce52206 -m "Widened to 8 hex; prop_store pins the width."
rv resolve 6ce52206
rv status --json                     # open down, resolved up
```

The reviewer's loop sheds its `jj diff` step:

```
rv status --json                     # scope: changes, files
rv diff --json                       # what changed, in rv's own coordinates
…read the head files as needed…
rv comment rv-core/src/store.rs --line 238 -m - <<'EOF'
…the finding…
EOF
```

## 4. Consequences

- `rv-core::markdown` keeps its render half and loses its parse half. The
  hardening rationale in its module docs moves to this spec as history.
- `session::write_markdown` loses the fold step; `write_markdown_if_current`
  loses its reason to exist once side-effect writes stop, and goes with it.
- `commands::settle` and `session::save_comment` stop refreshing the export.
- The TUI reads replies from the store like every other comment field; its
  launch path stops touching the markdown.
- Both skills are rewritten around the three-command loop; the worker skill's
  "Reply — in the export, at column 0" section and its hard rules are deleted
  whole.
- The mtime-polling hint in the worker skill switches to polling
  `rv status --json`, which it already documents as free.
- `rv diff` reuses the existing per-file `FileDiff` computation and the same
  lazy loading; no new diff code, only serialization of what the pane already
  draws.
- The reviewer skill's step 2 ("use `jj diff`") is replaced by `rv diff
  --json`, removing the skill's only dependency on the `jj` binary.
- This composes with the one-file consolidation the storage spec already
  approved (`session.toml` absorbing `comments.json`): one TOML file as the
  store, the CLI as the sole interface, markdown as a disposable view. Neither
  change depends on the other's ordering.

## 5. Migration

One version knows both worlds:

- On any command that loads the review, if `REVIEW-FEEDBACK.md` exists and
  contains a parseable `**Reply:**` whose id matches a stored comment that has
  no reply, fold it in — the last rescue, using the parser one final time
  before it is deleted in the release after.
- Nothing deletes the user's existing export. It stops being rewritten as a
  side effect and goes stale harmlessly; the next explicit `rv render`
  replaces it with the view-only form.

## 6. Testing

- `rv comments --json` agrees with the TUI: same in-range filter, same derived
  `outdated`, on the same fixture — one review, three readers, one answer.
- `rv reply` on an unknown id exits 1 and stores nothing; on a known id the
  reply survives a reload; a second reply replaces the first.
- `rv reply` then `rv resolve` leaves state `resolved`, `settled_by: agent`,
  reply intact.
- After a save, a settle and a reply, `REVIEW-FEEDBACK.md`'s bytes and mtime
  are untouched; `rv render` to stdout emits the current review; `--out`
  writes it.
- Migration: an export carrying a reply for a reply-less stored comment folds
  it in once; a reply for an unknown id is ignored; the export itself is not
  modified.
- The worker loop end-to-end as a CLI-only session: comment → comments --json
  → reply → resolve → status, with no read of the markdown anywhere.
- `rv diff --json` for a fixture file lists the same kinds and numbers the
  pane draws for it, fallback and difftastic alike; a line `rv diff` reports
  as `right: 238` is a line `rv comment --line 238` accepts.
- A body passed via `-m -` with backticks, quotes, `$` and newlines
  round-trips byte-identically into the store; an empty stdin is refused
  with exit 1.
- `rv status --check` exits 1 with an open comment, 0 once it is resolved,
  and prints nothing without `--json`.

## 7. Non-goals

- No daemon, no watch mode, no push notifications. Polling `rv status
  --check` is the worker's loop.
- No structured *output* format other than JSON, and no JSON *input* — bodies
  arrive as `-m` arguments or plain text on stdin, never as documents to
  parse.
- No `rv delete`. Deletion is the one irreversible act and stays behind the
  TUI's human confirmation; an agent retracting a bad comment has
  `rv abandon` with a reply saying why, which leaves a record.
- No batch settling. Settling is per-comment and deliberate; a loop over ids
  costs an agent nothing, and a batch flag invites rubber-stamping.
- No comment editing via `rv reply` — a reply is a reply, not an amendment
  channel; editing bodies remains delete-and-re-add.
- No removal of the markdown renderer. Humans read prose; the view stays.
- No network, no API, no model calls — unchanged, and restated because this
  amendment reinterprets the sentence that used to carry them.
