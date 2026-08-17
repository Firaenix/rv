# rv — one file, and what makes a stale comment useful

**Status:** design approved in conversation, awaiting spec review
**Date:** 2026-08-17
**Amends:** `2026-08-17-rv-branch-reviewer-design.md` §9 (lifecycle) and §10 (storage)
**Affects:** `2026-08-17-rv-inline-comments-design.md` (rendering and delete paths)

## 1. The problem

Milestone 1 spreads one review across four files and keeps them in step by
rewriting a derived one after every keystroke.

| Where | Holds | Verdict |
|---|---|---|
| `.review/comments.json` | id, change, commit, anchor (file, side, line, hash, **context**), body, state, reply | the authority |
| `.review/session.toml` | revset, base, head, change list, started_at | the session's scope |
| `.review/snapshots/<id>` | the anchor's **context lines** | **the same bytes as `anchor.context`** |
| `.review/REVIEW-FEEDBACK.md` | every comment rendered, plus replies | **derived from the first two** |

`snapshots/` has no justification. `append_comment` writes it as
`comment.anchor.context.join("\n")` — one value, serialised twice, in one call. The
original design wanted snapshots so that verification would survive history
rewriting, since `jj abandon` or a squash can destroy the commit a comment was
pinned to. But `anchor.context` already is that copy: verbatim, atomic, and
already read back by the renderer. A second copy of the same bytes protects
nothing.

The markdown is a rendering of state that exists elsewhere, rewritten from scratch
after every save.

## 2. The decision

**One file rv maintains: `.review/session.toml`. One file rv exports on request:
`REVIEW-FEEDBACK.md`. Nothing else.**

`session.toml` grows to hold the comments alongside the scope it already records:

```toml
revset = "trunk()..@"
base_commit = "0000000000000000000000000000000000000000"
head_commit = "4e78eb14095aa43e223050d420409839fb7ce920"
started_at = "epoch:1787000844"

[[changes]]
change_id = "ytskpxpwyunutroxvszuoklmnnrrxlkq"
commit_id = "62ba3a58bd68745b6d72d48d17e660037371097d"
description = "test(rv-core): close the jj_lib alias bypass"

[[comments]]
id = "6ce52206"
change_id = "ytskpxpwyunutroxvszuoklmnnrrxlkq"
commit_id = "62ba3a58bd68745b6d72d48d17e660037371097d"
body = "4-hex ids collide at review scale."
reply = "Widened to 8 hex."
state = "open"                      # open | dismissed | (outdated is derived)
dismissed_by = ""                   # "user" | "agent", when state = "dismissed"

[comments.anchor]
file = "rv-core/src/store.rs"
side = "right"
line = 238
content_hash = "9e21ab…"
context_start = 233
context = [
  "    pub fn append_comment(&self, comment: &Comment) -> Result<(), Error> {",
  "        let mut comments = self.comments()?;",
  "        match comments",
  "            .iter_mut()",
  "            .find(|existing| existing.id == comment.id)",
]
```

- `comments.json` is gone; the comments move here.
- `snapshots/` is gone.
- `context_start` is new, and it is what lets the tool say **which** of the stored
  lines is the anchored one — the gap a real reviewing session hit when the excerpt
  gave no pointer.
- The whole file is written through the existing atomic temp-file-plus-rename
  helper, so a crash cannot half-update a review, and there is no longer any
  cross-file ordering rule to get right.

**Ruling — TOML, not JSON, and one file rather than two.** The file is meant to be
readable and hand-fixable; a reviewer who needs to retract a comment before rv has
a delete key should be able to. Arbitrary code and comment text is representable:
basic strings escape everything, and the array-of-strings form for `context` avoids
the multi-line-literal traps entirely.

## 3. The lifecycle

Three states, and only one of them is stored.

| State | Where it comes from | How it renders |
|---|---|---|
| `open` | the default | expanded, blue box |
| `dismissed` | someone decided it is done — stored, with **who** | collapsed, dim |
| `outdated` | **derived**: the anchor no longer resolves in the current code | collapsed, grey, expandable to a before/after |

**Ruling — `outdated` is derived on every load, never stored.** If the code moves
back, or a rebase restores it, the comment un-outdates itself. A stored flag would
have to be invalidated by something, and nothing is watching.

**Ruling — dismissal records who did it: `user` or `agent`.** The original design
forbade an agent from resolving anything, on the grounds that an agent grading its
own homework is how bad fixes land. This relaxes that into something more useful:
an agent may dismiss, but the file and the UI always say it was the agent, and
agent-dismissed comments render distinctly from user-dismissed ones. Hiding the
distinction would be the actual danger; forbidding the action just pushes it into
prose nobody reads.

`awaiting-verification` disappears as a state. A comment with a reply and no
dismissal is simply open with a reply, which is what it always meant.

## 4. What an outdated comment is for

This is the part that justifies keeping the context at all.

When an anchor stops resolving, the comment is not deleted and not hidden — it
collapses to one grey row and stays where it was, in file and line order, so it can
still be found. Expanding it opens a **contained block inside the diff pane**
showing the difference between the code the comment was written against and the
code that is there now:

```
   238 -         .find(|existing| existing.change_id == comment.change_id)
      ╭─ 6ce52206 · outdated ────────────────────────────────────╮
      │ 4-hex ids collide at review scale.                       │
      ├─ when this was written ──── now ─────────────────────────┤
      │ - .find(|existing| existing.change_id == comment...      │
      │ + .find(|existing| existing.id == comment.id)            │
      ╰──────────────────────────────────────────────────────────╯
```

The before/after is produced by the existing `diff::compute` over
(`anchor.context` joined, the current lines at the resolved region) — the same
function the diff pane already uses, so there is no second diff implementation and
no new dependency. When the anchor cannot be located at all, the block says so and
shows the stored lines alone, which is still the most useful thing available.

**Ruling — the before/after is an inline block, not a modal over the screen.** It
lives in the diff pane where the comment lives, so the reviewer keeps their place
and every ordinary key still works. A modal would need its own dismissal, its own
focus rules, and its own scroll.

This is what makes a stale comment worth keeping: *"you wrote this about code that
has since changed — here is what changed"* is exactly the question a reviewer
returning to a review after a model has been editing needs answered.

## 5. The export

`REVIEW-FEEDBACK.md` is written by `rv render` and by nothing else. Saving,
dismissing, and deleting a comment no longer touch it.

Replies come back through an explicit ingest: `rv render` ingests before
re-exporting, and the TUI ingests on launch, so a reviewer returning after a model
has replied sees the replies. The export can therefore be stale, which is stated
rather than hoped away: `rv status` reports it, `rv status --json` carries it as a
field, the TUI status line reads `export stale — rv render to refresh`, and the
exported document names the commit and time it was written.

Every hardening the markdown parser gained in milestone 1 stays. Ingest still reads
a file a model wrote into, so mangled markers, unbalanced fences and quoted
structure remain in scope. Only *when* parsing runs changes.

## 6. Migration

A milestone-1 `.review/` has `comments.json`, `snapshots/`, and possibly a markdown
newer than both.

- On first run, rv reads `comments.json` if `session.toml` has no `comments` array,
  ingests the markdown so replies survive, writes the merged result to
  `session.toml`, and then leaves `comments.json` alone.
- `snapshots/` and `comments.json` are never read again and never deleted. The
  README says they are milestone-1 artefacts and safe to remove. Deleting a user's
  files to tidy a format is not rv's business.

## 7. Consequences for the in-flight work

- `Store` loses `append_comment`'s snapshot write, `snapshots_dir`, and the
  comments-before-snapshot ordering rule. It gains `read_review`/`write_review`
  over the single file, and `remove_comment` becomes a rewrite of that file.
- The inline-comments plan's Task 1 no longer asserts anything about snapshot
  files; its delete test asserts on `session.toml` and on one end-to-end
  `rv render`.
- The comment browser (that plan's Task 13) shows state per row, and outdated rows
  render grey — the browser is where a reviewer will actually notice that four of
  their comments went stale under a rebase.
- `Anchor` gains `context_start`, which also closes the milestone-1 gap where the
  excerpt never marked which line was anchored.

## 8. Testing

- One file: after any sequence of saves, dismissals and deletions, `.review/`
  contains `session.toml` and nothing else rv wrote.
- Round trip: an arbitrary review — bodies with newlines, quotes, TOML
  metacharacters, unicode, empty replies — survives write-then-read byte-identically.
- `outdated` is derived: a comment whose line still resolves is open; edit the file
  so the anchor cannot resolve and it reads outdated; restore the code and it reads
  open again, with no stored flag anywhere in the file.
- Dismissal records its actor, and a user-dismissed and an agent-dismissed comment
  are distinguishable in both the file and the rendered screen.
- The before/after block: for a comment whose code changed, it shows both the stored
  line and the current line; for a comment whose file is gone, it shows the stored
  lines and says the anchor could not be located; it never panics on a zero-height
  pane.
- Saving a comment does not modify `REVIEW-FEEDBACK.md` (compare mtime and bytes).
- `rv render` after several saves exports every comment; a reply appended to the
  export survives the next `rv render` and lands in `session.toml`.
- Migration: a directory containing a milestone-1 `comments.json`, a `snapshots/`
  directory, and a reply-bearing markdown loads without error, keeps the reply, and
  leaves both legacy artefacts on disk.

## 9. Non-goals

- No comment editing (delete and re-add).
- No stored `outdated` flag, no cached confidence.
- No deletion or rewriting of a user's legacy `comments.json` or `snapshots/`.
- No modal windows anywhere in the TUI.
- No history of a comment's state changes. The current state and who set it is the
  whole record.
