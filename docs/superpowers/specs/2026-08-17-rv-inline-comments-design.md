# rv — inline comments, pane focus, and comment deletion

**Status:** design approved in conversation, awaiting spec review
**Date:** 2026-08-17
**Builds on:** `2026-08-17-rv-branch-reviewer-design.md` (milestone 1, shipped)

## 1. Purpose

Milestone 1 can write a comment but never shows it again. The reviewer types a
comment, sees a status line, and the comment vanishes into `.review/` — so there
is no way to tell which lines already carry one, no way to read what you wrote,
and no way to remove a comment you regret. This design closes that: comments
render inline in the diff, the two panes are navigable with the arrow keys, and
a comment can be deleted.

Everything here is TUI and store surface. No change to the markdown format.

**Storage:** this document was written before
`2026-08-17-rv-storage-model-design.md`, which supersedes it on every storage
question. That spec is the authority: `.review/session.toml` becomes the one file
rv maintains, holding the comments alongside the session scope; `comments.json`
and `snapshots/` retire; `REVIEW-FEEDBACK.md` becomes an export written only by
`rv render`; and `Anchor` gains `context_start`. Where this document and that one
disagree, that one wins. The sections below have been reconciled with it.

## 2. Requirements

From the user, verbatim in intent:

1. Comments render inline on the diffs.
2. Left and Right arrows move between the files column and the diff.
3. A comment renders as blue, in a box with a border.
4. Comments can be deleted.
5. `Enter` jumps onto a comment stack; arrow keys scroll between the comments in
   it; a comment can be selected for deletion; `c` creates a new one from there.
6. Comment boxes are collapsible with `s`.
7. In the file list, `j`/`k` **and** Up/Down both move the selection.

## 3. Interaction model

Two orthogonal pieces of state. Conflating them is the mistake this section
exists to prevent: *focus* is where the cursor is, *mode* is what typing does.

```rust
enum Focus { Sidebar, Diff, Stack }
enum SidebarTab { Files, Comments }
enum Mode  { Browse, Comment, ConfirmDelete { id: String, label: String } }
```

`Focus::Stack` means the cursor is inside the comment stack belonging to the
currently selected diff line. It is reachable only from `Focus::Diff`, and only
when that line has at least one comment.

`SidebarTab` is what the left column is listing. **The sidebar browses comments
the same way it browses files** — same column, same keys, different content — so
there is no second navigation idiom to learn. `Tab` switches between them.

### The comment browser

The `Comments` tab lists every comment in the review, grouped by file and ordered
by file then line, each row showing `path:line`, the comment's state, and the
first line of its body. `Enter` on a row **jumps to the code**: it selects that
comment's file, loads its diff, puts the cursor on the anchored line, and moves
focus to the diff — so reading a comment and looking at what it is about are one
keystroke apart.

This exists because traversing code to find your own comments does not scale. The
first real reviewing session on this tool spent 2,200 of 11,101 keystrokes on `j`
and `]`, and reaching a known line cost up to 398 presses. A reviewer returning to
a review — which is the normal case once an LLM has replied — wants to move
through what they said, not through the file tree.

**Ruling — jumping sets the diff's cursor rather than opening a separate viewer.**
A jump lands you in the ordinary diff pane with the ordinary keys, so from there
every existing action (comment, delete, collapse, step into the stack) works with
no special cases. A dedicated comment-viewing screen would need its own copy of
all of them.

**Ruling — the comment browser is flat, not a tree.** File grouping is a heading
row, not a collapsible node. Comments are few enough at review scale that hiding
them behind expansion costs more keystrokes than it saves.

### Keymap

| Key | Focus::Sidebar | Focus::Diff | Focus::Stack |
|---|---|---|---|
| `Tab` | switch Files ⇄ Comments tab | switch the sidebar's tab | switch the sidebar's tab |
| `Left` | — | focus Sidebar | leave stack → Diff |
| `Right` | focus Diff | — | — |
| `j` / `Down` | next row in the tab | next diff line | next comment in stack |
| `k` / `Up` | previous row | previous diff line | previous comment |
| `]` / `[` | next / previous file | same | same |
| `Enter` | Files: focus Diff · Comments: **jump to that comment's code** | enter stack if the line has comments | — |
| `Esc` | — | — | leave stack → Diff |
| `c` | — | comment on selected line | new comment on the same line |
| `d` | Comments: delete the selected comment | delete the line's newest comment | delete the selected comment |
| `s` | — | collapse/expand all boxes on the line | collapse/expand selected box |
| `q` | quit | quit | quit |

`d` works from the comment browser too: it is the natural place to prune a review,
and it goes through the same confirmation as everywhere else.

`]`/`[` stay bound to file navigation from every focus. They predate this design,
reviewers have them in muscle memory, and milestone 1's tests pin them.

**Ruling — `Right` from `Focus::Comments` does nothing.** The stack is drawn
*inside* the diff pane, so there is no pane to its right. Left and Esc both leave
it, which gives the reviewer two ways out and no way to get stuck.

**Ruling — `Enter` on a line with no comments is a no-op with a status line**
("no comments on this line"), not an error and not a silent nothing.

### Deletion

`d` never deletes immediately. It enters `Mode::ConfirmDelete`, the bar shows
`delete comment at <path>:<line>? (y/n)`, `y` deletes and any other key cancels.
Deletion is unrecoverable — the comment leaves `comments.json` and its snapshot
file is removed — and this project's stated posture is that a reviewer never
loses written work to a mishap. One extra keystroke is the cheapest possible
insurance against a mistyped `d` while browsing.

From `Focus::Diff` on a line carrying several comments, `d` targets the
**newest** and the status line says `deleted 1 of 3 on this line`, so the
reviewer can see what remains and repeat. From `Focus::Comments` it targets
exactly the selected box, which is the unambiguous path and the one to prefer.

## 4. Rendering

### Comment box

Drawn beneath the diff line it is anchored to, indented to the diff's gutter
width so it visually hangs off that line.

```
   42 +     let digest = blake3::hash(seed.as_bytes());
      ╭─ 7f3a2b1c · open ────────────────────────────╮
      │ 4-hex ids collide at review scale — a        │
      │ collision silently replaces the earlier      │
      │ comment.                                     │
      ╰──────────────────────────────────────────────╯
```

Collapsed, one row:

```
      ▸ 7f3a2b1c · open — 4-hex ids collide at review scale…
```

- **Border and title are `Color::Blue`; body text keeps the default foreground.**
  Blue borders satisfy the requirement while leaving the comment text at full
  contrast, which matters because the body is the part being read. The selected
  box in `Focus::Comments` uses `Color::LightBlue` plus `Modifier::BOLD` on its
  border.
- The title carries the comment id and state. The state model is
  `2026-08-17-rv-storage-model-design.md`'s: `open`, `dismissed` (with the actor,
  `user` or `agent`), and `outdated`, which is derived rather than stored.
- **A dismissed comment renders collapsed and dim; an outdated one renders
  collapsed and grey.** Neither is hidden and neither leaves its place in file and
  line order, so a reviewer can always find what they said. Expanding an outdated
  comment opens the before/after block described in §4 of the storage spec — the
  code the comment was written against, diffed against the code there now, drawn
  inside the same bordered box and produced by the existing `diff::compute`. That
  block is why the anchor's stored context exists at all.
- A `reply` renders inside the same box beneath the comment body, prefixed
  `reply:` and dimmed. A reply is part of the comment; rendering the comment
  inline and hiding the LLM's answer to it would be perverse. This is also the
  surface milestone 2's verification flow will build on.
- Bodies wrap at the pane width. Long bodies are not truncated when expanded —
  collapsing is the mechanism for reclaiming space, and `s` exists for that.

**Ruling — boxes are drawn with box-drawing characters inside the existing
`Paragraph` text, not as nested ratatui `Block` widgets.** A `Block` cannot be
nested inside a `Paragraph`, and switching the diff pane to per-line widgets
would mean computing a layout rect per line. Hand-drawn borders keep the pane's
body a pure `lines → Text` function, which is what makes it testable under
`TestBackend`.

**Ruling — collapse state is in-memory and session-scoped**, held as a set of
collapsed comment ids. It is a viewing preference, not review state, so it does
not belong in `.review/` and does not change the on-disk format.

### Focus indication

The focused pane's border is `Modifier::BOLD` and its title is prefixed `▸`.
Focus is deliberately *not* signalled with colour: blue now means "comment", and
a second colour meaning "focus" would collide with it. The unfocused pane's
selection highlight drops from `REVERSED` to a dim underline, so at a glance
there is exactly one place the next keystroke will land.

### Windowing

This is the one piece of real algorithmic work. Today `window(line_index, total,
height)` assumes one terminal row per diff line. With comment boxes a diff line
occupies 1 row plus, for each of its comments, either 1 row (collapsed) or
`3 + wrapped_body_rows` (expanded).

The diff pane therefore builds a **row list** first:

```rust
enum Row<'a> {
    Diff  { index: usize, line: &'a DiffLine },
    Comment { line_index: usize, comment_index: usize, part: BoxPart<'a> },
}
```

then windows over *rows*, choosing the window so that the selected diff line's
row is visible, and — when focus is `Comments` — so that the selected comment's
box is visible in full where it fits. Row construction is a pure function of
(diff, comments, collapsed set, pane width) and is unit-tested directly, apart
from any terminal.

## 5. Data flow

`App` currently only ever *writes* comments; it has no idea what is stored. Two
additions:

1. `App` loads `store.comments()` at construction and re-loads after every
   mutation (save, delete). Review-sized comment sets are tiny and the store is
   the authority — caching a diff of it would invite exactly the desync this
   project has avoided so far by keeping `Store` stateless.
2. Comments are indexed for lookup by `(anchor.file, anchor.side, anchor.line)`.
   A diff line's key uses the **same side rule the app already uses when
   saving** — `anchored_side(kind)`, `left` for a removed line and `right`
   otherwise, with the path being `source_path` for a `Left` anchor and `path`
   for a `Right` one. Sharing that one function is what keeps display and
   storage from drifting; milestone 1 already had a bug where they disagreed.

Ordering within a line's stack is `comments.json` order, which is insertion
order — so "newest" is last, and the stack reads oldest-first down the screen.

## 6. Store surface

One new method:

```rust
pub fn remove_comment(&self, id: &str) -> Result<bool, Error>
```

Returns whether a comment was removed, and rewrites the review file through the
existing `write_atomic` helper.

Under the storage spec that file is `.review/session.toml`, which holds the
comments, so removal is a single atomic rewrite of one file — there is no
snapshot to delete and no cross-file ordering rule to honour. (Until that
migration lands, the same method rewrites `comments.json` and removes
`snapshots/<id>`, writing the comment list first so a crash leaves an inert
orphaned snapshot rather than a comment whose snapshot is missing. That ordering
rule disappears with the migration, which is one of the reasons for it.)

**A delete does not rewrite `REVIEW-FEEDBACK.md`.** The markdown is an export
produced by `rv render`, which ingests replies before re-exporting. A delete
therefore leaves the export stale until the next render, and staleness is
reported rather than hidden — see the storage spec's §5.

## 7. Testing

Terminal-free where it matters, per the existing boundary.

- **Row construction** (pure): a line with no comments yields one row; with one
  expanded comment yields 1 + 3 + wrapped rows; collapsed yields 2; several
  comments stack in insertion order; a body wider than the pane wraps rather
  than truncating; a zero-width or one-column pane produces no panic.
- **Windowing:** the selected diff line is always inside the window, including
  when preceded by tall expanded boxes; the selected comment's box is visible
  when focus is `Comments`; a box taller than the pane degrades gracefully.
- **Focus state machine** through `on_key`: every cell of the keymap table,
  including `Enter` on a comment-less line, `Right` from `Comments`, and that
  `]`/`[` work from all three focuses.
- **Deletion:** `d` then `y` removes it from a freshly opened `Store` and from
  the rewritten markdown; `d` then any other key leaves the store untouched;
  deleting one of three leaves two and reports the count; deleting the last
  comment on a line returns focus to `Diff` rather than leaving the cursor in an
  empty stack.
- **Collapse:** `s` toggles, survives moving away and back within a session, and
  is absent from `.review/` on disk.
- **Rendering** via `TestBackend`: the blue border characters appear beneath the
  right line; the collapsed form is one row; the reply appears inside the box;
  the focused pane's title carries `▸`. Assert on styles, not only on text —
  "blue and bordered" is the requirement, so a test that only checks the text
  would pass on an unstyled box.
- **Property tests** (the suite added alongside this work): row construction
  never panics for arbitrary comment bodies and pane widths; total rendered rows
  equal the sum of per-line row counts; and — the conservation property — every
  comment in the store appears exactly once in the row list for its file.

## 8. Non-goals

- No comment editing. Delete and re-add is the milestone-1-era answer, and
  editing invites the question of what happens to an existing reply.
- No accept/reopen, no state transitions. That is milestone 2's verification
  flow; this design only *renders* state it does not change.
- No persisted collapse state, no per-user view config.
- No mouse support.
- No sidebar comment counts. A natural companion, but not asked for, and the
  dogfood report may argue for something better.

## 9. Risks

| Risk | Mitigation |
|---|---|
| Variable-height rows break the windowing invariant that the selected line is always visible | Row construction and windowing are pure functions with direct unit tests, including the tall-box and taller-than-pane cases |
| Display and storage disagree about which line a comment belongs to | Both go through `anchored_side`; milestone 1 already shipped that bug once and the shared function is the fix |
| A third focus target makes the keymap hard to hold in the head | One table, in this spec and in the README, plus `?` help remains a milestone-2 item; `Left`/`Esc` both escape the stack so no state is a trap |
| Deleting the wrong comment | `y`/`n` confirmation naming path and line; deletion from the stack targets exactly the highlighted box |
| Blue borders unreadable on some terminal themes | Border colour only, body text left at default contrast; no background fills |
