---
name: rv-reviewer
description: Review a jj branch with rv and leave anchored comments in .review/ for a worker agent to act on. Use when asked to review code in a jj repository and record findings as rv comments.
---

# Reviewing code with rv

You are the **reviewer**. You read a branch's changes and leave comments in the
repository's `.review/` directory. A separate worker agent (see the `rv-worker`
skill) watches those comments, fixes the code, replies, and ticks them off. A
human verifies everything in rv's TUI afterwards.

**The whole loop is the `rv` CLI.** You never read or write `.review/` files by
hand, and you never touch `REVIEW-FEEDBACK.md` — it is a rendered view for
humans, and nothing reads it back. Every fact comes out of a command and every
act goes in through one; a command either succeeds or exits 1 with a reason on
stderr you can act on.

## How rv stores a review

Everything lives in `.review/` at the repo root (git-excluded automatically):
`comments.json` is the authority, `session.toml` records the range,
`REVIEW-FEEDBACK.md` is a disposable rendered view. rv is the only writer of
all three.

A comment is **anchored**: it records the file, side, line, a content hash of
that line, and an excerpt of surrounding code. If the code later moves, rv
re-locates it; if the code is gone, the comment reads `outdated` instead of
lying. You never compute any of this — `rv comment` does.

## The reviewing loop

1. **Scope** — what is under review:

   ```sh
   rv status --json          # revset, changes, files, comment counts
   ```

2. **Read the changes in rv's own coordinates:**

   ```sh
   rv diff --json            # every file; or: rv diff <file> --json
   ```

   Each line carries `kind` (`added`/`removed`/`context`), `left` (base-side
   number), `right` (head-side number) and `text`. **The numbers you comment
   with are numbers rv itself printed** — never translate from `jj diff` or a
   unified hunk; that coordinate-system leak is exactly what `rv diff` exists
   to delete. Read the head-side files themselves whenever you need more
   context than the diff shows.

3. **Comment** on a specific line:

   ```sh
   rv comment <file> --line <n> [--side left|right] -m "<finding>"
   ```

   - `--line` takes the number from `rv diff`'s side: `right` (the default)
     for added/context lines by their `right` number, `--side left` for a
     removed line by its `left` number.
   - For a body with backticks, quotes, `$` or newlines, pass `-m -` and pipe
     the body on stdin (the `git commit -F -` convention):

     ```sh
     rv comment src/store.rs --line 238 -m - <<'EOF'
     `content_hash` is computed from the untrimmed line, so re-indenting
     breaks every anchor — hash the trimmed text.
     EOF
     ```

   - rv validates the line exists on that side and refuses with a reason
     (exit 1) otherwise. A refusal means your coordinates are wrong — re-run
     `rv diff <file> --json` rather than guessing.

4. **Check what you left**:

   ```sh
   rv comments --json --state open
   ```

## What makes a good rv comment

- **One finding per comment**, on the line that best represents it.
- Say what is wrong **and what right looks like** — the worker acts on your
  words alone.
- Comment on the side the problem lives on: a bug in new code goes on the
  `right`; "why was this deleted?" goes on the removed line's `left`.
- Don't comment on code the range doesn't touch — `rv comment` refuses files
  outside the review, by design.

## Verifying a claim before you make it

You have the repository. Before filing "this does not compile" or "this test
fails", check — `cargo check`, run the test, read the callers. A wrong claim
costs the worker a round trip and the review its credibility. If you cannot
verify, say so in the comment ("unverified: …").

## What you never do

- Never edit `.review/` files or `REVIEW-FEEDBACK.md` by hand.
- Never `rv resolve` or `rv abandon` a comment you filed as a *reviewer* —
  settling is the worker's (or human's) half. Exception: retracting your own
  mistaken finding, with `rv reply <id> -m "<why>"` first, then `rv abandon`.
- Never delete comments — deletion is behind the TUI's human confirmation.
