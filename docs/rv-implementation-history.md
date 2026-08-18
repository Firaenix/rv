# rv — implementation history

A running record of how rv was built, what reviewing it found, and what changed as
a result. Written as the work happened, not reconstructed afterwards.

The through-line: from milestone 1 onward, **rv reviews rv**. Each stage below
names what was built, what the review of it said — increasingly, rv's own review —
and what that review changed. Where a review was wrong, that is recorded too.

---

## Milestone 1 — the self-hosting minimum

**Goal:** a binary that reviews the current jj change stack in a TUI, attaches
line comments, and writes them to `.review/REVIEW-FEEDBACK.md` — the point at
which rv can review its own development.

Ten tasks, each implemented by a fresh agent working from an extracted brief, each
gated by an independent reviewer reading only the task's diff. Every task that
needed one got a fix round; a scoped re-review verified each fix.

| # | Commit | What landed | Fix rounds |
|---|---|---|---|
| 1 | `6000b89` | jj-lib repository handle, stack enumeration, reverse-hex change ids | — |
| 2 | `9b3aded` | File enumeration, native rename detection, lazy blob reads | — |
| 3 | `2712785`, `2cfcf92` | difft JSON diff pipeline with a `similar` fallback | 1 |
| 4 | `9271157`, `b590438` | Content-hash anchors and the resolution cascade | 1 |
| 5 | `94c076c`, `750f31c` | Write-through `.review/` store, atomic writes, exclude management | 1 |
| 6 | `fbe19d9`, `4129638`, `26058ae` | `REVIEW-FEEDBACK.md` render and reply parsing | 2 |
| 7 | `c7b4c13` | `rv` binary: session assembly, `render`, `status --json` | — |
| 8 | `60a8088`, `c960123` | ratatui review TUI with line comments | 1 |
| 9 | `fbad2ee`, `9fc60cf` | README and dogfood verification | 1 |
| 10 | `5dfb1c9`, `62ba3a5` | Architectural constraints enforced mechanically | 1 |

### What review changed

Six findings were substantial enough to change the design rather than just the
code.

**The diff pipeline mislabelled every new file.** difftastic's `status` field
turned out to be four-valued, not two: `created` and `deleted` arrive with no
`chunks` key, so whole-file additions fell through to the `similar` fallback and
were labelled as such. Since the design reserves the fallback label for binaries
and parse failures, every new file in the TUI would have claimed difftastic had
failed when it had succeeded. Fixed by synthesising the line set from the side
difft was given and keeping the `Difftastic` label.

**Anchors could fabricate a location.** Two related holes: an anchor created for an
out-of-range line hashed the empty string, which is also the hash of every blank
line in every file, and the nearest-hit scan would then resolve it to an arbitrary
blank line and report `Moved`. Blank lines are now excluded from the scan, and
out-of-range creation carries a sentinel hash that can never match. Both now fail
safe to `Outdated`. The design principle at stake — the tool never presents a guess
as a fact — is what made this a bug rather than a preference.

**The store could lose the whole comment history.** `append_comment` rewrote
`comments.json` with a single non-atomic `fs::write`, so a crash mid-write could
truncate the accumulated file, not just the entry being added. All four writers now
go through one temp-file-plus-rename helper with an fsync, and the module's
durability claim was rewritten to describe what the code actually provides.

**The reply parser could be broken by things a model would plausibly write.** Four
independent failures: a reordered or missing anchor marker bound a reply to the
*previous* comment; a comment body containing `**Reply:**` fabricated a reply; one
unbalanced code fence swallowed every later reply and compounded across renders;
and a heading inside a reply truncated it. The fix established one invariant —
every column-0 line is structure, everything rv did not author is indented — and a
re-reviewer then generated 145,872 rendered documents looking for a counterexample
to round-trip closure and found none.

**4-hex comment ids collided.** The plan and the design both specified a 4-hex id
prefix, giving 65,536 values against an upsert that replaces on a match: a
collision silently deleted an earlier comment under a "comment saved" status.
Birthday odds reach roughly 2% at fifty comments. Widened to 8 hex against the
letter of both documents, because the spec's stronger guarantee — nothing causes
comment loss — outranks its own id-width detail.

**A constraint test could be sidestepped by renaming an import.** The test proving
jj-lib stays confined to one file matched the literal string `jj_lib::`, which
`use jj_lib as j;` defeats in one line. Closed by stripping comments and matching
the bare token. The first attempt at that fix introduced a subtler hole — a `//`
inside a string literal blanked the rest of that physical line, hiding any
violation after it — which a re-reviewer caught by reading the stripper's state
machine.

### A bug that never existed, and how it got into this document

The most instructive thing that happened all evening is a mistake, and it is
recorded here in full because the failure mode is more valuable than the fix.

For several hours this document, the session ledger, and a report to the project's
owner all stated that `Store::append_comment` upserted on `change_id` rather than
`id` — that every comment in a session shared one `change_id`, so each new comment
silently overwrote the last and the store held exactly one. It was described as a
severe shipped defect that three review passes had read past.

**None of it was true.** Two commands settle it: the commit that supposedly fixed
the bug adds six lines to `store.rs`, all of them doc comment, and
`git log -S 'existing.change_id == comment.change_id'` over the file's whole
history returns nothing. That predicate was never committed. The `id` key has been
correct since the store was written.

The mechanism is worth understanding, because nobody involved was careless in an
obvious way. Five property-test suites were being written in parallel, and every
author had been told — correctly — to prove each property could fail by
temporarily breaking the source, then reverting. The natural mutation for a
property asserting *"`comments()` equals the upsert-by-id reduction of the append
sequence"* is to change the key from `id` to `change_id`. While that experiment was
live in the working copy, the coordinator opened `store.rs` to plan an unrelated
feature, saw the mutated line, and read a deliberate experiment as a shipped
defect.

From there it propagated the way a wrong premise does when everyone downstream is
competent. A fix agent was dispatched with the finding already framed as fact; it
found the store suite genuinely red (the mutation was still live), "fixed" it, and
reported a real failing-test transcript that meant nothing. A dogfood session
running against a binary built hours earlier appeared to corroborate it, which was
taken as decisive proof — the reasoning being that an old binary cannot contain a
new mutation. That reasoning was sound; the observation it rested on was not, and
it went unchecked because it agreed with what everyone already believed.

It was caught by rv, reviewing its own history document, by an agent that checked
the claim against `git log` instead of against the surrounding prose.

Three things generalize:

- **A working copy shared with agents licensed to break the source is not a
  readable source of truth.** Anything read from it during that window needs to be
  confirmed against committed history before it becomes a finding.
- **Corroboration between agents is not independent when they share a premise.**
  Three separate confirmations of this bug all traced to the same mutated file.
- **The instruction "do not fabricate findings — check the claim against the code"
  was issued to every agent in this project, repeatedly, and the coordinator was
  the one who broke it.** Process discipline applies hardest to whoever is writing
  the summary, because their errors inherit everyone else's credibility.

What survives from the episode is genuinely useful: the invariant is now pinned by
a test that distinguishes the two behaviours (the old covering test used the same
`id` *and* the same `change_id`, so it could not), and the store property suite was
later measured to kill that exact mutation in five of five runs. The bug was
imaginary. The test that would have caught it is real, and so is the reason it did
not exist before.

---

## Dogfood session — rv reviews rv

The first real use of the tool, driven through a pty against its own 20-change
stack: **14 comments placed, 14 of 14 landing on the intended line**, write-through
correct every time, and the status line never disagreeing with what was stored.
The verdict on the loop closing was yes — nine of ten final entries needed no
clarification to be handed to a model.

What it found, in severity order:

1. **`Ctrl+C` opens a comment instead of quitting.** The event loop passes only
   `key.code`, so `Ctrl+C` is indistinguishable from a plain `c`. In raw mode the
   terminal raises no SIGINT, and rv offers no other abort — so the universal
   escape hatch types a comment.
2. **A comment on removed text records the wrong commit.** `Side::Left` anchors
   store the head commit although the text was read from the base blob, defeating
   the one job an advisory `commit_id` has.
3. **Content clips silently.** Diff lines truncate at the pane width with no
   marker in a repository containing 154-character lines, and comment bodies past
   118 characters are typed blind.
4. **No delete, no edit** — retracting a comment meant hand-editing
   `comments.json`.
5. **Navigation is punishing.** 2,200 of 11,101 keystrokes were `j` or `]`;
   reaching one known line cost 398 presses; and leaving a file reset the cursor
   to its first line.
6. **The context excerpt never marks which of its eleven lines is the anchored
   one** — a deferred milestone-1 minor, confirmed as a real gap by a real user.

An orchestration mistake of the author's, not a defect in rv: a concurrent
workflow was rewriting three source files while the review was being written, so
four of the fourteen comments described code that was not in the reviewed
snapshot. rv's own captured snapshot context is what let the reviewer prove which
four were void, and a line-number mismatch in the pane is what raised the
suspicion.

---

## In progress

**Property-based test suites.** `rstest` and `proptest` across five modules, with
each property required to demonstrate that it can fail: the markdown round-trip
under hostile bodies, the anchor cascade's algebraic laws, the diff's conservation
law (removed plus context equals the old file; added plus context equals the new),
the store's upsert-reduction equivalence, and the TUI state machine's invariants
under fuzzed key sequences.

**Inline comments, focus, and deletion** — spec and thirteen-task plan written.
Comments render as blue bordered boxes beneath the line they annotate; Left and
Right move between the panes; `Enter` steps into a line's comment stack; `d`
deletes behind a confirmation; `s` collapses. Includes a comment browser in the
sidebar: the same column and the same keys as file browsing, listing every comment
in the review, with `Enter` jumping to the code it annotates.

**View switching and symbol navigation** — spec written. `1` and `2` toggle between
the whole-bookmark view and a per-commit view; a tree-sitter symbol index feeds a
fuzzy picker and `n`/`N` stepping that crosses file boundaries; scope follows the
view, so a jump on a commit stays inside that commit.
