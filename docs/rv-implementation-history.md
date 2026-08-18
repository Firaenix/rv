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

## Inline comments, and the pane that draws them

Comments render as blue bordered boxes beneath the line they annotate, drawn
inside the diff pane's own `Text` rather than as nested widgets — ratatui cannot
nest a `Block` inside a `Paragraph`, and hand-drawn borders keep the pane a pure
`state → Text` function a `TestBackend` can assert on cell by cell.

Two defects the wave produced and paid for:

1. **The cursor walked diff lines while the pane drew rows.** A box is several
   rows tall, so a cursor moving by line *stepped over* it: with a box taller
   than the pane, its middle rows were in no window at any cursor position, and
   the comment could not be read at all. The cursor is now a row of the plan and
   the line index is derived from it — the reverse would leave two cursors to
   keep in step, which is what caused the defect.
2. **A comment id blind to the side.** `same.rs` rewrites a line without moving
   it, so its two halves are `same.rs:2` on opposite sides; an id seeded without
   the side made one body typed on each half overwrite itself, under a "comment
   saved" status line. The id is eight hex characters, not the spec's four: at
   four, the birthday bound gives a ~7% chance of losing a comment on a
   hundred-comment review.

---

## The viewport wave — a screen you can arrange

Panes you can resize by key or by drag, a `?` keymap popup drawn *from the
binding table itself*, syntax highlighting inside the diff's green and red
washes, a file list that is a tree or a flat list, and a zellij-style segmented
status bar along the bottom.

The rules that came out of it, each paid for:

- **One layout.** No other module computes a `Rect`. Painting and hit-testing
  read the same `Layout`, because a click that resolves to the wrong row looks
  exactly like a click that resolved to the right one — there is no red test,
  only a reviewer whose comment landed on the wrong line.
- **Two colour layers, split by channel rather than by hue.** The diff owns green
  and red as *backgrounds*; the syntax palette owns them as *foregrounds* and
  emits indexed colours only, so code takes the terminal's own theme. A comment
  rendering as terminal-white was the defect that forced the split.
- **The keymap cannot drift.** `BINDINGS` is the only thing the browse handler
  dispatches from, and the popup and README are both held to it. A key not in the
  table reaches no code; a row pointing at nothing does not compile.
- **No background wash on a sidebar row.** Two rounds of looking at the running
  tool settled it: thirty files became thirty slabs of green and the tree stopped
  looking like a tree. The colour lives in the counts — `+204` green, `-12` red —
  and selection is the only full-row background.

---

## Splitting what had grown

Four files had passed the point where they could be held in one reading:
`tests/app.rs` at 5,657 lines, `tests/app_cases.rs` at 3,966, `src/app.rs` at
2,900 and `src/ui.rs` at 1,799 — with doc comments running to 40% of `app.rs`,
most of it rationale that belonged in a spec where it could be revised once
instead of echoed everywhere.

`src/app.rs` is now 354 lines and fifteen modules, one per what-a-keystroke-does;
`src/ui.rs` is 175 and eleven, one per thing it draws; each test file is one
binary with topic modules over shared fixtures. The proof that nothing was lost
is the count, not the author's word: **1,097 tests before, 1,097 after**, and the
same 161 test attributes and 71 rstest cases.

A concurrent split agent and a live session editing the same tree also cost
something worth recording: five finished files were deleted as scratch by the
session that had not written them, and had to be regenerated. **A working copy
shared with another agent is not a source of truth about that agent's progress.**

---

## The lifecycle, and reading a stack

**Resolve and abandon** are separate states, not one "dismissed". They record two
different facts — *this was fixed* and *this was dropped without being fixed* —
and a summary adding them together misreports what the review concluded. Both
keys are their own undo, which is why neither asks first; deleting still asks,
because it is the only one that cannot be taken back. `settled_by` records
**who**: an agent may resolve its own finding, but the file and the box say it
was the agent. Forbidding the action would only push it into prose nobody reads.

**The commits tab** lists the stack's changes, each holding the files it touched,
and a file row shows *that change's* diff of the file — computed between the
change's parent and the change itself, cached per row, so two changes touching
one file are two rows with two diffs. The anchor follows the screen: a comment
written there is filed between the change's own commits, because `commit` on an
anchor exists so the quoted text can be read back from the revision it names.

**Symbol navigation.** `n`, `N` and a `/` picker over a tree-sitter index built on
first use and cached per *scope*: the whole bookmark from the Files tab, one
change's files from the Commits tab. Neither key wraps — a jump from the last
symbol to the first looks exactly like a jump that failed.

**Comments outside the range are no longer listed.** `.review/` outlives any one
revset, so a comment can be anchored to a file the open range does not touch. The
browser used to show it and answer `Enter` with an alert: a row existing only to
refuse, inflating the count of comments a reviewer thought they could reach.
Nothing is deleted — the store keeps every one, and a wider range shows them
again.

### What the tests caught that reading did not

Five defects in this stretch were found by a test disagreeing with a plausible
implementation, and they are the argument for the suite:

1. **`Repository::stack` lists the newest change first**, and the commits view
   walked it as though it were oldest-first. Not an off-by-one you see on screen:
   it attributes every file to its neighbour and gives the oldest change a diff
   made entirely of removals. Only a fixture whose two changes touch *different*
   files makes it visible.
2. **`t` and `o` had shipped dispatched and undocumented.** `BINDINGS` and the
   README were each held to `BROWSE_KEYS` in one direction only, so a key in
   neither shipped documented nowhere with the suite green. The missing direction
   — every dispatched binding must reach the manual — found four such keys.
3. **The `?` popup silently stopped fitting.** Twenty-one bindings in five groups
   cannot be dealt into two columns of fourteen without splitting a group, so it
   fell back to a scrolling column and hid three keys.
4. **Switching tabs highlighted a row without selecting it**, leaving the sidebar
   naming one thing and the diff pane showing another.
5. **A selected commits-view pair outlived its tab**, pairing a stale diff with
   whatever file was selected later — one file's lines under another file's name.
   A state-invariant property found it in a single keystroke.

And one test that was lying by construction: the walk order of the symbol index
is the *caller's scope order*, and every fixture numbered its files ascending, so
a mutant ranking by file number passed all twenty-seven. A fixture numbering them
descending kills it, and nothing else does.

### A guard that fired on the sampler

`distinct_comments_are_never_lost_to_each_other` failed about one run in three,
always on its coverage receipt: the shape it exists for — one body on both halves
of one same-position rewrite — turned up in two thirds of samples at 32 cases. A
guard that fires on the random sampler rather than on the code is worse than no
guard, because it teaches its reader to re-run the suite. The case is driven
outright now, before the random ones, and the receipt is a receipt.

---

## In progress

**Three source files remain over the 400-line rule**: `rv-core`'s markdown, diff
and vcs modules, none of them touched since the rule was written. Everything else
is under it — every file in the `rv` crate, and `rv-core`'s highlight and store —
split as it was touched, which is the ruling this session was given.

One thing that split found rather than fixed: a `language_of` had been written
twice, once as a free function for the symbol index and once as a method for the
pane title, over one grammar table. The second was mine, added an hour earlier
without looking for the first. The module's own doc comment warns that a second
detection table would drift; a second *function* over one table is the same
mistake wearing a smaller hat.

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
