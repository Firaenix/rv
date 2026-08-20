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
state = "open"                      # open | resolved | abandoned  (outdated is derived)
settled_by = ""                     # "user" | "agent", when resolved or abandoned

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

Four states, and only two of them are stored.

| State | Where it comes from | How it renders |
|---|---|---|
| `open` | the default | expanded, blue box |
| `resolved` | the comment was **addressed** — stored, with **who** | collapsed, dim, with a tick |
| `abandoned` | the comment was **dropped without being addressed** — stored, with **who** | collapsed, dim, struck through |
| `outdated` | **derived**: the anchor no longer resolves in the current code | collapsed, grey, expandable to a before/after |

**Ruling — `resolved` and `abandoned` are separate states, not one "dismissed".**
An earlier draft had a single dismissed state, which conflates two different facts
about a review: *this was fixed* and *this is being dropped without being fixed*.
A reviewer returning to a stack needs to tell those apart — the first is work that
happened, the second is work that was decided against, and a summary that counts
them together is lying about what the review concluded.

**Ruling — deleting, resolving and abandoning are three different acts.**
Deleting says the comment should never have existed and removes the record;
resolving and abandoning say it existed and reached an end. That is why deletion
is the only one that asks first: **confirm what cannot be undone, and do not
interrupt what can.** Resolving and abandoning are ordinary state changes and
reversible — setting one back to `open` is just another state change — so they
take a single key and no prompt.

`r` resolves, `a` abandons, and either applied to a comment already in that state
returns it to `open`.

**Ruling — `outdated` is derived on every load, never stored.** If the code moves
back, or a rebase restores it, the comment un-outdates itself. A stored flag would
have to be invalidated by something, and nothing is watching.

**Ruling — settling a comment records who did it: `user` or `agent`.** The original design
forbade an agent from resolving anything, on the grounds that an agent grading its
own homework is how bad fixes land. This relaxes that into something more useful:
an agent may resolve or abandon, but the file and the UI always say it was the
agent, and agent-settled comments render distinctly from user-settled ones. Hiding the
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

The before/after is produced by the existing `diff::compute_with(old, new, path,
false)` over (`anchor.context` joined, the current lines at the resolved region) —
the same engine the diff pane falls back to, so there is no second diff
implementation and no new dependency.

**Ruling — `compute_with(.., false)`, not `compute`.** The sibling that spawns
difftastic writes two temp files and runs a child process on every call, and this
box is drawn *inside a frame*: the comment browser can hold many outdated rows, so
`compute` would put one process spawn per row on the paint path. It is also the
wrong tool for the job — a slice of stored context lines is not a parseable file,
and the language difftastic infers from the extension would be right about the
path and wrong about the fragment.

**Consequence to settle with it:** over a fragment, a terminator-only difference
sets `suppressed`, and on the fallback path `suppressed` arrives with a full set of
`Context` lines. The box therefore needs a rule for "changed, but nothing a line
can show": it prints the stored lines under the existing
`no semantic change` note rather than an empty frame. When the anchor cannot be located at all, the block says so and
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
has replied sees the replies.

**`e` exports from inside the TUI**, through the same ingest-then-write path
`rv render` uses, and the status line names the file it wrote. Making a reviewer
quit to produce the file the whole LLM loop depends on would be a strange place
to put a door. The export can therefore be stale, which is stated
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

## 10. What a comment id is made of

Moved here from `rv/src/app/anchor.rs` by the 2026-08-18 shape refactor, which
cut the code's doc comments back to constraints the code cannot show. The rules
themselves are still enforced there; the history is here.

**Eight hex characters, not four.** The plan and the branch-reviewer spec §10
both write four. `Store::append_comment` upserts by id, so two *different*
comments that share a prefix mean the second save silently replaces the first
in `comments.json` and overwrites its snapshot — under a "comment saved" status
line. A four-character id is a 65,536-value space, which by the birthday bound
is a ~2% chance of losing a comment at 50 of them and ~7% at 100: reachable on
one real review. The guarantee that nothing loses a comment outranks the
literal width, and eight still reads out of a marker at a glance.

**The side is part of the seed.** A location is a side as well as a path and a
number. difftastic aligns a rewritten line with its counterpart and gives both
halves of the pair both numbers, so a rewrite that stays at the same line number
produces a removed line and an added line at, say, `same.rs:2` on the base and
head sides. Without the side, one sentence typed on each half — "which of these
two is right?" — seeds two identical ids and the second save replaces the first.
Unlike a digest collision, that happens with probability 1. The path alone is
not enough: the two paths differ only for a rename.

**Adding the side changed every id this function produces, and that was safe.**
Nothing recomputes an id in order to find a comment: `comments.json` is keyed by
the id it stored, snapshots are filed under it, and `session::fold_replies`
matches the id a document's marker carries against the stored one. A review in
progress therefore keeps working across the change — its comments, snapshots
and replies all still resolve. The only visible effect is that re-typing a
comment saved *before* the change appends a second entry beside it rather than
upserting the first. A duplicate is recoverable; the loss above is not.

**`change_id` is the range's first change, not the change that touched the
line.** It is the same string for every comment in one review, so within a
review the location and the body carry the whole of the seed's discriminating
power. It stays in the seed because ids outlive the review that made them: a
`.review/` from another range, keyed by these ids, must not collide with this
one's. Attributing a comment to the change that introduced its line is
Milestone 2's work and needs per-change diffs.

---

## 10. The export's parser, and why it is forgiving

Moved here out of `rv-core/src/markdown.rs`, whose module doc had grown to
eighty-nine lines — a review comment on that file read *"This is a LOT of
comments. necessary? Code should document itself"*, and it was right: what
follows is argument, not constraint, and the file now states the constraints and
points here.

### Structure lives at column 0

The document interpolates reviewer- and LLM-written prose, and quoted source from
the repository under review, into a structure-sensitive grammar — so content must
not be able to imitate structure. The separation is positional: **in a rendered
document, every column-0 line is structure**, because `render` indents everything
it did not author itself by two spaces: continuation lines of comment and reply
bodies, and whole context fences.

Two spaces is enough to leave column 0 and few enough that markdown renders the
result identically (lazy paragraph continuation; a fenced block strips its
opener's indent from each line). A reviewer quoting the protocol — `**Reply:**` as
the second line of a comment, which is exactly what happens when `rv` reviews
`markdown.rs` — therefore cannot fabricate a reply, and a body or a quoted line
reading `<!-- rv:anchor id=… -->` cannot rebind the parser to a comment that does
not exist. `parse_replies` removes the indent again, so a rendered body parses
back byte-identical.

### Why the parser has no error path

The document is handed to a language model and to a human with an editor, and both
will mangle it. Nothing either can write may cost a comment. The rules, in the
order they matter:

- **A reply binds only within its own entry.** Every entry boundary — a `### <n>.`
  heading, a `## ` section heading, a `<details>` — clears the binding. A marker
  that was deleted, indented or garbled therefore leaves the following reply bound
  to *nothing*, and it is dropped rather than attributed to the entry above it.
- **Marker fields are read by name, not position.** `<!--rv:anchor id=…`,
  `<!-- rv:anchor  id=…` and a reordered `<!-- rv:anchor change=… id=… -->` all
  read. A line announcing itself as `rv:anchor` with no readable `id=` clears the
  binding, for the same reason.
- **A reply above every marker is dropped**, never bound to a *later* id.
- **An unbalanced fence is text, not a fence.** A fence counts only when its
  closing partner sits in the same region — no entry boundary and no column-0
  `**Comment:**`/`**Reply:**` between. One stray fence in a comment body can then
  neither swallow the entry's own reply by pairing with a fence inside it, nor
  reach across a boundary to pair with the next entry's context fence.

`comments.json` remains the authority on which comments exist, so the worst case
is a reply that fails to attach, never a comment that disappears.

### The reply body rule

A reply body is the text after the marker on its own line plus every following
line, up to the first **structural** line at column 0: an entry or section
heading, an HTML comment, a `<details>`/`</details>`/`<summary>`, or another
`**Comment:**`/`**Reply:**`. Blank lines *inside* the body are kept so a
multi-paragraph reply survives whole; leading and trailing ones are trimmed. A
balanced fenced block inside the body is consumed whole.

Only the heading levels `render` emits terminate a body: `## ` and a numbered
`### <n>.`. An LLM writing `### What I changed` or a `# shell comment` inside its
reply keeps them, because truncating a reply loses work that goes nowhere — the
tail would be attributed to nothing and erased by the next render. Blank lines are
not terminators for the same reason, and the cost is cosmetic: stray prose a human
leaves directly below a reply is absorbed into it.

### Milestone 2

Nothing in milestone 1 consumes `parse_replies`. When it does, two things this
signature cannot express are needed: a diagnostics channel alongside the replies,
so a marker the parser gave up on is reported rather than dropped; and an
"Unattached replies" section that `render` preserves, so an LLM's work is not
erased by the next render when it fails to bind.

---

## 11. Snapshots are gone from the code, not only from the design

§1 ruled `snapshots/` redundant and §2 removed it from the target design, but
milestone 1's store kept writing them, and a later relabelling ("crash-safety
data") dressed the duplicate up instead of asking §1's question. A review
comment asked it — *is it actually used?* — and the answer, from the code, was
no: nothing ever read one back.

The store now writes `comments.json` and nothing else per save. Legacy
`snapshots/` directories still load fine; a removed comment's leftover file is
deleted with it, and an orphan is inert — neither resurrected nor adopted when
its id is reused. `Store::open` no longer creates the directory.

---

## Implementation status (audited 2026-08-19)

- **§2 single-file consolidation (`session.toml` absorbing `comments.json`)**
  — **shipped 2026-08-20**: the comments are a `[[comments]]` array on
  `Session`, `comments.json` and `snapshots/` are no longer written, and
  `.review/` holds `session.toml` plus the on-request export and nothing
  else. §6's migration and §7's `read_review`/`write_review` landed with it;
  the first §8 test is `a_save_writes_only_session_toml`.
- **§6 migration** — **shipped 2026-08-20**, and stricter than the section
  described. `Store::open` folds a v1.0.0 `comments.json` into `session.toml`
  and then **deletes** it, rather than leaving it beside the store: a legacy
  file that is never removed is re-read on every open forever, and a stale
  copy of a comment that has since been replied to or resolved is a live
  hazard rather than a harmless artefact. A stored comment always wins over
  its legacy twin, so a half-finished migration cannot roll a reply back. The
  order — atomic rename of `session.toml`, then unlink — is safe at every
  point: interrupted before the rename the JSON is untouched and the next open
  retries; interrupted between the two both files hold the comments and the
  re-fold is idempotent; the one ordering that could lose a comment (unlink
  first) is the one not written. An unparseable `comments.json` is an error
  naming the file, not a shrug. Covered by
  `a_v1_review_migrates_every_comment_into_session_toml`,
  `an_interrupted_migration_loses_no_comment_from_either_side`,
  `a_stored_comment_is_not_overwritten_by_its_legacy_twin` and
  `an_unreadable_legacy_file_is_reported_rather_than_skipped`.
  `snapshots/` is still never read and never deleted, as the section says.
- **§7 `read_review`/`write_review`** — **shipped 2026-08-20**, replacing
  `read_session`/`write_session` outright: one read and one write over the one
  file, with no alias left behind. `Store::comments`, `append_comment`,
  `settle_comment` and `remove_comment` kept their exact signatures and
  semantics; only the backing file changed. A `.review/` with no
  `session.toml` reads as an empty review rather than an error.
- **§3 `awaiting-verification` disappears** — still pending the milestone-2
  verification-flow decision; the state remains stored, counted and rendered
  until that work replaces it.
- **§3 abandoned in the export** — fixed 2026-08-19: the render was silently
  omitting abandoned comments (no section matched them), which was exactly
  the "dropping a comment is never an acceptable outcome" failure. The
  document now carries an `## Abandoned` section, collapsed like resolved.
- **§4 the outdated before/after block** — **shipped 2026-08-20**, in the shape
  this section sketches: expanding an outdated comment opens a `├─ when this
  was written ──── now ─┤` rule inside its own box, and under it the stored
  `anchor.context` diffed against the lines standing where the excerpt used to
  be. `context_start` is what places that window — the offset from it to
  `anchor.line` is where in the excerpt the anchored line sits, which the
  `snapshot_of` clamp makes not always the middle one.

  `compute_with(.., false)` as ruled, and asserted so: the block's `FileDiff`
  must not be `DiffSource::Difftastic`, checked in a test whose *pane* diff is
  difftastic's from the very same process and `PATH`, so the assertion is a
  contrast rather than a claim about the environment. Both consequences the
  section names are handled — a `suppressed` fragment prints its stored lines
  under the existing `no semantic change` note rather than an empty frame (one
  spelling, shared with the diff pane's own `SUPPRESSED_EMPTY`), and an anchor
  that cannot be located at all says so and prints the stored lines alone.

  The rows flow through `rows::plan` like every other interior row, not around
  it, so `window`, `row_of_comment`, `line_of_row` and pointer hit-testing keep
  agreeing about a plan that is now taller. Covered by
  `an_expanded_outdated_comment_shows_the_stored_context_against_the_code_now`,
  `the_before_after_block_never_spawns_difftastic` and
  `a_comment_whose_anchor_cannot_be_placed_says_so_and_shows_what_it_was_written_against`.
- **§5 (the export)** — superseded by the CLI-loop amendment: the markdown is
  a one-way view, `fold_replies` became the one-release `rescue_replies`
  migration, and nothing writes the export as a side effect. That release has
  now been and gone: `rescue_replies` and `markdown::parse_replies` were
  deleted 2026-08-20 with the hostile-input corpus that defended them.
- **§7/§10 details** — closed 2026-08-20 with §2. `snapshots_dir` and
  `comments_path` are gone; `remove_comment` no longer reaches for a legacy
  snapshot, and a `.review/` from milestone 1 keeps whatever is in its
  `snapshots/` directory untouched.
