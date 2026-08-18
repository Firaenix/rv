---
name: rv-reviewer
description: Review a jj branch with rv and leave anchored comments in .review/ for a worker agent to act on. Use when asked to review code in a jj repository and record findings as rv comments.
---

# Reviewing code with rv

You are the **reviewer**. You read a branch's changes and leave comments in the
repository's `.review/` directory. A separate worker agent (see the `rv-worker`
skill) watches those comments, fixes the code, replies, and ticks them off. A
human verifies everything in rv's TUI afterwards.

## How rv stores a review

`rv` is a jj-native branch reviewer. Everything lives in `.review/` at the repo
root (git-excluded automatically):

| File | What it is | Who writes it |
|---|---|---|
| `comments.json` | **The authority.** Every comment, with its anchor | rv only — never by hand |
| `session.toml` | The range under review (revset, base/head commits) | rv only |
| `REVIEW-FEEDBACK.md` | An **export** of the comments, for reading and for replies | `rv render`, and rv after every save |

A comment is **anchored**: it records the file, side, line, a content hash of
that line, and an excerpt of surrounding code. If the code later moves, rv
re-locates it; if the code is gone, the comment reads `outdated` instead of
lying. You never compute any of this — `rv comment` does.

**Never edit `.review/` files by hand.** The ids are content-hashed, the anchors
carry blake3 digests, and a malformed entry is a comment that silently fails to
resolve. The CLI is the interface.

## The workflow

### 1. See what is under review

```sh
rv status --json
```

Key fields: `revset`, `base`/`head` (commit ids), `changes` (the stack, newest
first), `files` (every changed path with its kind: added/modified/removed/
renamed), `comments` (counts by state), `degraded_base` (see Pitfalls).

The default range is `trunk()..@`. Review a different range with
`rv --from <rev> --to <rev> status` — these are queries and never modify the
session record.

### 2. Read the changes

Read the changed files at head with your normal file tools. For what *changed*,
use jj: `jj diff --from <base> --to <head>` or per-file `jj diff -r <change>`.
The `files` list from step 1 tells you which paths are worth opening.

### 3. Leave comments

```sh
rv comment <file> --line <n> -m "<one finding, stated precisely>"
```

- `<file>` is the path exactly as `rv status` lists it (the head-side path).
- `--line` is the 1-based line number **in the file on that side**, not a diff
  hunk offset.
- `--side right` (default): the code as it exists at head — use this for
  almost everything.
- `--side left`: a line that was **removed** — the line number then refers to
  the base version of the file. For a renamed file, rv maps to the old path
  itself.

On success it prints `saved <id> at <file>:<line> (right)`. The id is the
comment's permanent name. The export is refreshed in the same step, so the
worker sees the comment immediately.

Refusals are exit-code-1 errors that name the problem: a path outside the
range, a line past the end of the file, an empty body. Fix your input and
retry.

### 4. Verify, then stop

```sh
rv status --json   # comments.open should equal what you left
```

Do **not** resolve or abandon your own comments — settling is the worker's and
the human's act. Do not reply to your own comments either; replies are the
worker's channel back.

## Writing comments that a worker can act on

- **One finding per comment.** The worker replies and resolves per comment; a
  comment holding three problems can only be ticked off as one.
- **Anchor on the most specific line** — the defective line itself, not the
  function's opening brace.
- State what is wrong, why it matters, and (if you have one) the fix you'd
  accept: `"content_hash is computed from the untrimmed line, so re-indenting
  breaks every anchor — hash the trimmed text"` beats `"hashing seems fragile"`.
- Re-running `rv comment` with the same file, line, side and body **usually
  updates** the existing comment rather than duplicating it — the id is seeded
  from those four things plus the owning change, so if the stack was rewritten
  in between you may get a second comment instead. A different body on the same
  line is always a second comment.

## Pitfalls

- **`degraded_base: true`** in status means `trunk()` resolved to the
  repository root (no origin/upstream main/master/trunk bookmark), so the
  "review" is the whole history and every file reads as added. Say so rather
  than reviewing it as a branch, or ask for an explicit `--from`.
- A comment on a file **outside the current range** is refused. If you must
  comment on it, the range is wrong — re-run with `--from`/`--to`.
- `rv comment` runs from the repo root by default; from elsewhere pass
  `--repo <path>`.
- The subcommand name collides with a bookmark literally named `comment` only
  if you pass it positionally; you never do — the file is the positional.
