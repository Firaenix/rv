---
name: rv-worker
description: Watch an rv review for open comments, fix the code they point at, reply, and resolve them. Use when working on a jj branch that a reviewer (human or agent) is leaving rv comments on.
---

# Working through rv comments

You are the **worker**. While you build features on this branch, a reviewer
(human in the rv TUI, or an agent with the `rv-reviewer` skill) leaves anchored
comments in `.review/`. Your loop: notice new comments, fix what they point at,
reply saying what you did, and tick them off. A human verifies your resolutions
in the TUI afterwards — everything you settle is labelled as agent-settled, so
never try to hide an unfixed thing as resolved.

## How the review reaches you

| File | Your use of it |
|---|---|
| `.review/comments.json` | Poll its mtime to notice changes. **Read-only, never edit** |
| `.review/REVIEW-FEEDBACK.md` | What you actually read, and the one file you may append to |
| `.review/session.toml` | The range the comments were made against. Read-only |

## The loop

### 1. Notice work

```sh
rv status --json    # .comments.open > 0 means there is work
```

Poll this between your own tasks, or watch `.review/comments.json`'s mtime and
run it when the file changes. Queries never modify the review, so polling is
free.

### 2. Read the open comments

```sh
rv render           # refreshes .review/REVIEW-FEEDBACK.md
```

Then read `REVIEW-FEEDBACK.md`. Its `## Open (n)` section holds one entry per
comment:

```markdown
### 3. `rv-core/src/store.rs:238`
<!-- rv:anchor id=6ce52206 change=… commit=… side=right line=238 hash=… -->

  Lines 233–243; the comment is on line 238 — row 6 of 11 below.

  ```rust
  …excerpt of the code as the reviewer saw it…
  ```

**Comment:** 4-hex ids collide at review scale.
```

- The **id** in the marker is what you resolve by.
- The **caption** says which excerpt row is the commented line — do not assume
  it is the middle one.
- `side=left` means the comment is about a **removed** line: the line number
  refers to the base version, and the ask is usually about what replaced it.
- The excerpt is the code **as the reviewer saw it** — if the file has moved
  on since, trust the live file and use the excerpt to find the place.

### 3. Fix the code

Ordinary work: edit, build, test. Commit with jj as you normally would.

### 4. Reply — in the export, at column 0

Append a reply line directly beneath the entry's `**Comment:**` block:

```markdown
**Reply:** Widened the id to 8 hex; `prop_store.rs` now pins the width.
```

Hard rules, because a parser reads this file:

- `**Reply:**` starts **at column 0** — never indented, never inside a list —
  or it is not read at all.
- **Never** edit `<!-- rv: -->` markers, headings, or section order.
- **Never** write a state into the document — resolving happens via the CLI,
  and the next render would overwrite anything you wrote here anyway.
- One reply per comment; a second reply replaces the first.

Then run `rv render` again: it **ingests replies before re-exporting**, which
is what moves your reply from the document into the store. A reply you never
render in is a reply the next export erases.

### 5. Tick it off

```sh
rv resolve <id>     # it was addressed; records settled_by=agent
rv abandon <id>     # dropped without being addressed — say why in the reply first
```

- Resolved and abandoned are **different conclusions**: fixed, versus decided
  against. Do not abandon something you failed to fix — reply with what
  blocked you and leave it open instead.
- Re-applying the same command **reopens** the comment (it is its own undo).
- Both refresh the export, so the reviewer's next poll sees the new state.
- Deleting is not yours: only the TUI's `d` deletes, behind a human
  confirmation.

### 6. Verify the loop closed

```sh
rv status --json    # open should have gone down; resolved up
```

## States you will see

| State | Meaning | Yours to set? |
|---|---|---|
| `open` | Waiting on you | — |
| `resolved` | Addressed (records who) | yes, after fixing |
| `abandoned` | Dropped unfixed (records who) | yes, with justification |
| `outdated` | The anchored code no longer exists — **derived, never stored** | no — it clears itself if the code returns |

A comment often goes `outdated` *because you fixed its line* — the anchor no
longer resolves against the new code. That is expected: reply and resolve it
anyway; settled states are facts about what happened and do not revert to
outdated.

## Pitfalls

- Run commands from the repo root, or pass `--repo <path>`.
- The review's range is fixed in `session.toml`; comments on files your new
  commits add will not appear until the reviewer re-opens the range (`@` moves
  with the working copy, so usually they just re-poll).
- If `rv status --json` reports `degraded_base: true`, the range is the whole
  history, not a branch — flag it rather than "fixing" 200 files.
- Your resolutions render distinctly (`resolved by agent`) in the TUI. That is
  by design; the human's verification pass depends on it.
