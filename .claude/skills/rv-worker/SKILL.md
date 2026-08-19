---
name: rv-worker
description: Watch an rv review for open comments, fix the code they point at, reply, and resolve them. Use when working on a jj branch that a reviewer (human or agent) is leaving rv comments on.
---

# Working through rv comments

You are the **worker**. A reviewer — human, in rv's TUI, or an agent with the
`rv-reviewer` skill — leaves anchored comments on the branch you are building.
You fix what they point at, answer through `rv reply`, and tick them off with
`rv resolve`.

**The whole loop is the `rv` CLI.** You never read or write `.review/` files by
hand, and you never touch `REVIEW-FEEDBACK.md` — it is a rendered view for
humans, and nothing reads it back. A command either succeeds or exits 1 with a
reason on stderr.

## The working loop

```sh
rv status --check                    # is there work? exit 1 = open comments
rv comments --json --state open      # what exactly?
# …fix the code…
rv reply <id> -m "Widened to 8 hex; prop_store pins the width."
rv resolve <id>
rv status --json                     # open down, resolved up
```

Poll with `rv status --check` between your own tasks — it is one exit code,
prints nothing, and costs almost nothing. `--check --json` prints the report
*and* sets the code, for when you want both.

## Reading a comment

`rv comments --json` gives you everything per comment:

- `id` — what `reply`, `resolve` and `abandon` take.
- `state` and `settled_by` — only `open` ones are yours to work.
- `outdated: true` — the anchored code is gone or changed; the comment may
  already be answered by the code as it now stands. Say so in a reply and
  resolve it, or fix what clearly remains.
- `body` — the finding. `reply` — any existing answer (a second reply
  replaces it).
- `anchor` — `file`, `side`, `line`, plus `context` (an excerpt) and
  `context_start` (which file line the excerpt begins at), so you can see the
  code the reviewer saw even after the file moved on.

## Fixing and answering

1. Fix the code the comment points at — the anchor names the file and line;
   `rv diff <file> --json` shows the change in the same coordinates.
2. **Reply before you resolve**, saying what you did and where:

   ```sh
   rv reply <id> -m "Hashed the trimmed line; anchor_survives_reindent pins it."
   ```

   Multi-line or shell-hostile replies go via stdin: `rv reply <id> -m -`.
3. `rv resolve <id>` — records `settled_by: agent`, visibly. Resolving your
   own work is allowed *because* it is recorded; the human verifies in the TUI.
4. A comment you decide not to act on: reply with why, then `rv abandon <id>`.
   Abandoned is a distinct state from resolved — *dropped unfixed* and *fixed*
   must not be confused — and the reply is the record of the decision.

An unknown id is an error, not a no-op: if `rv reply` exits 1, you have the
wrong id — re-run `rv comments --json`, never retry blind.

## After a rebase, push or bookmark move

The review range is a revset (`trunk()..@` by default), so `rv status`,
`rv comments` and `rv diff` always resolve against the repository as it now
stands. Re-run `rv comments --json` after history changes — anchors re-resolve
and some comments may have become `outdated`.

## What you never do

- Never edit `.review/` files or `REVIEW-FEEDBACK.md` by hand.
- Never resolve without replying — an unexplained tick-off is work the human
  cannot verify.
- Never batch-settle ids you have not individually addressed.
- Never delete comments — deletion is behind the TUI's human confirmation.
