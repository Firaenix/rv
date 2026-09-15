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
`session.toml` is the one file rv maintains — the range, the comments, the
flags and the reviewed ticks — and `REVIEW-FEEDBACK.md` is a disposable
rendered view. rv is the only writer of both.

A comment is **anchored**: it records the file, side, line, a content hash of
that line, and an excerpt of surrounding code. If the code later moves, rv
re-locates it; if the code is gone, the comment reads `outdated` instead of
lying. You never compute any of this — `rv comment` does.

## The reviewing loop

1. **Scope** — what is under review, and what the human has already read:

   ```sh
   rv status --json          # revset, changes, files, comment/flag/reviewed counts
   rv reviewed --json        # files the human has ticked off, and which changed since
   ```

   A file the human has ticked is one they consider read: flag it again only
   if it has `changed: true`, and comment on it only for a real finding.

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

3. **Flag** what deserves a look first. A flag is *attention, not feedback*:
   it carries a reason, never blocks the worker's `rv status --check`, and
   is what the human walks with `g f` in the TUI before reading anything
   else. Use it for "start here", "this is where the behaviour changes",
   "the risky bit" — anything you want looked at that does not ask for a
   code change:

   ```sh
   rv flag <file> --line <n> [--side left|right] -m "<why look here>"
   rv flags --json --open    # what you have pointed at
   ```

   Same coordinates and `-m -` stdin convention as `rv comment` below. Keep
   the reason to one line — it is drawn as a single row under the line.
   Never `rv ack` a flag: acknowledging is the human's half.

4. **Comment** on a specific line when something must change:

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

5. **Check what you left**:

   ```sh
   rv comments --json --state open
   rv flags --json --open
   ```

## What makes a good rv comment

- **A comment asks for a change; a flag asks for a look.** If you would
  accept "no change needed" as the answer, it is a flag. The worker is
  polled on open *comments*, so a comment that is really a note costs a
  round trip and blocks the loop.
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
- Never `rv ack` a flag or `rv review` a file — both record that a *human*
  looked.
