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

## The tool closes its own loop

The stretch that turned rv from a reviewer into a review *system*, and ended
with rv working through its own 31-comment review.

**States became real.** `outdated` had been specified since milestone 1 and
derived by nothing — `anchor::resolve` was written, tested, and called by
no one, so `rv status` claimed twenty-two open comments where fourteen were
stale. It is now derived on every load, in the TUI, `status` and `render`
alike, because two commands reporting different states for one review is worse
than either being wrong. `resolve` and `abandon` record who settled a comment;
the export's protocol block stopped forbidding what the tool now attributes.

**Queries stopped writing.** Every invocation used to pass through
`session::build`, so `rv status --json` rewrote `session.toml` and its
`started_at` on each run. `session::read` resolves the same range and writes
nothing; only opening the TUI records a session. The refusal-to-re-point remedy
was implemented and withdrawn — three tests that legitimately narrow a
commented range failed, and they were right to.

**The keystroke stopped waiting.** Measured: difftastic is a flat ~26 ms
process spawn per file regardless of size; tree-sitter is the part that scales
(165 ms at 128k lines); the in-process engine answers in 0.2 ms. Both moved off
the paint path — highlights parse per blob on worker threads, and the
structural diff refines a fast diff through a single-slot worker where a
request arriving replaces the one waiting, so scrolling ten files costs one
spawn. The swap keeps the reviewer's place by nearest surviving source line,
because most of what the fast diff shows is context the structural one omits.

**The agent loop.** `rv comment`, `rv resolve`, `rv abandon` — the CLI half the
TUI already had, built on the same functions (`session::save_comment` is the
one construction path, `session::owning_change` the one attribution rule) so
the two cannot drift. Two skills teach the roles: `rv-reviewer` writes anchored
comments; `rv-worker` polls, fixes, replies through the export at column 0, and
ticks off with attribution. `R` refreshes the TUI against the repository as it
now stands — re-asking the original `--from`/`--to`, so `@` means now — which
is how a human sees what the agents did. A nix flake ships the binary with
difftastic wrapped onto its PATH.

**Running the loop on rv's own review found four bugs in the loop's tooling**:
comments attributed to the empty working-copy change, a second
comment-construction path already diverging, `rv status` counting comments the
range cannot show, and `rv resolve` unable to tick off a comment whose file had
left the range. One reviewer finding was *refuted* by building the accused
commit in a detached worktree — the claimed compile failure was a mid-edit
shared working copy, the same trap this document already records. Final state:
28 resolved, 3 abandoned, 0 open, every settlement labelled with who made it.

---

## Closing the appendices — everything the specs still called open

v1.0.0 shipped on 2026-08-19 with five specs each ending in an
implementation-status appendix, and each appendix naming what had not been
built. This stretch closed all of them, six agents in parallel over one working
copy, and the release pipeline that v1.0.0 had quietly failed to complete.

**The release had not actually released.** The tag, the binaries and the GitHub
release were all fine; the Homebrew job had failed and nobody had looked.
`Firaenix/homebrew-tap` was an empty repository — no branches at all — so
`actions/checkout` died on `couldn't find remote ref refs/heads/main`. The
formula had been built correctly and attached to the release as `rv.rb`; it had
simply never been pushed anywhere a `brew install` could find it. Seeding the
tap and backfilling that same formula was the whole fix. The second failure was
the same shape: `RELEASE_PLZ_TOKEN` did not exist, and release-plz reports a
missing secret as `environment variable GITHUB_TOKEN is empty`, which reads like
a bug in the action rather than an unset secret. Both jobs now fail with a
sentence naming the secret. **A green CI badge on the repository's default
branch says nothing about whether the release job that runs only on a tag
succeeded.**

Third, found only by installing the shipped artefact: the nix package wraps
difftastic onto rv's PATH and the Homebrew formula did not, so `brew install`
shipped a binary that silently degraded to the in-process engine. One
`stage = ["run"]` dependency. The lesson is the general one — the flake and the
formula are two declarations of the same runtime, and the second was never
checked against the first.

### What the agents built

**The anchor's confidence reached the screen.** `Weak` means the content is gone
and only the line number is still standing; acting on it as though it were an
exact hit is precisely the mistake the tier exists to name, and until now the
TUI showed no difference. Only the drifting tiers are labelled — `Exact` is the
common case, and a word on every box to report that nothing happened is noise.
The cascade was not duplicated: `stale::survey` runs the existing one once per
load and `mark_outdated` became a wrapper over it, because a second cascade
would be a second answer to one question.

**The outdated before/after block.** Expanding an outdated comment now diffs the
stored `anchor.context` against whatever stands there now, in-process only — the
paint path must never pay difftastic's ~26 ms spawn. The test that proves it
does so by contrast: it first asserts the *pane's* diff is genuinely difftastic
in the same process on the same PATH, so it cannot pass vacuously on a machine
where difft is missing.

**`difft --version`, and a fallback that says why.** The JSON-parse fallback
covered malformed output but not a silently incompatible schema. The probe runs
once per process and the minimum was pinned by reading difftastic's own
`display/json.rs` across tags 0.50–0.70 rather than by picking a tested version:
0.51.0 introduced `--display json`, and the field set rv reads is byte-stable
from there to 0.70. `DiffSource::Similar` gained a reason, so "rv was told not to
run difft" and "difft is too old" stopped rendering as the same word.

**Comments moved into `session.toml`.** One file, one atomic rename, and the
cross-file ordering rule that a comment and its scope could disagree is simply
gone. The migration deviates from the storage spec deliberately: §6 said leave
`comments.json` alone, and it is deleted instead, because a legacy file never
removed is re-read on every open forever and a stale copy of a since-resolved
comment is a live hazard rather than an inert artefact. The deviation is written
into the appendix rather than made silently.

**`parse_replies` and its corpus are gone**, exactly one release after the
CLI-loop amendment said they would be.

### Two rulings that came from measuring instead of arguing

The "N files with no semantic change" note had a real tension behind it:
suppression is known only from a computed `FileDiff`, and the lazy-blob rule
exists on purpose. Rather than pick a side, the agent measured — difftastic is a
flat ~26.7 ms per file (n=40, median 26.7, total 1069 ms), so an eager pass would
cost a 40-file review a full second of dead time before the first frame, every
run, to print one sentence. So the note states what it knows and carries its own
denominator until it knows everything: `2/7 · no semantic change` while partial,
`2 · no semantic change` once settled. The ratio leads, so a narrow border clips
a partial answer into a shorter partial answer and never into the complete one.

Hunk boundaries went the same way. difftastic 0.70 emits **no context lines at
all**, so three edits a dozen lines apart arrive as one unbroken run of changed
lines: a rule reading only `LineKind` would call that a single hunk and leave `J`
with nowhere to go in exactly the file it exists for. Boundaries are derived from
line-number contiguity instead, which is engine-independent and survives a diff
read back out of the store.

### The bug no test would have found

The editor key (`v`, the letter `less`, `ranger`, `lf` and `mutt` all use for it)
first restored the screen with `terminal.clear()`. Every test passed. The real
binary died with *"the cursor position could not be read within a normal
duration"* — ratatui's clear issues a cursor-position query, and a terminal that
has just handed the screen back from a child does not always answer it promptly.
Replacing the terminal outright repaints every cell with no query. It was found
by running the program in a pty, which is the only thing that could have found
it, and it is the argument for smoke-testing the binary rather than the suite.

### What running six agents over one working copy cost

Three collisions, all caught by agents talking to each other rather than by the
suite. A shared `rv/tests/cli.rs` was split while another agent had nine pending
edits to it — resolved by doing the split as a *pure move* first, byte-identical
bodies, so the concurrent edits landed as small conflicts in named modules
instead of a scramble across a rewritten 981-line file. Two agents nearly
declared the same "no semantic change" sentence twice, which is the
second-detection-table mistake this document already records, wearing yet another
hat. And one agent misattributed 30 failing tests to another's migration; the
failures were a single `assert_eq!(browser_index(), 0)` in a shared rewind
helper, and **every one of them named line 99 of the same file**, which is what
one shared cause looks like when it is mistaken for a wide blast radius.

That last one is worth keeping. Adding heading rows to the comment browser broke
every test that had asserted a literal browser index as a proxy for "the Nth
comment". None of those assertions were weakened to make them pass; each was
rewritten to state the invariant its own name had always claimed — walk to the
row that browses this comment id, rather than trusting that row *n* is comment
*n*. A test that asserts a position which happens to be right is the weaker test
even while it is green.

---

## In progress

**Every source file is now under the 400-line rule.** The three that had been
left — `rv-core`'s markdown, diff and vcs modules — were split on 2026-08-20 as
the v1.1 work touched them: `markdown.rs` by deleting its parse half outright,
`diff.rs` into a six-module directory, `vcs.rs` into `vcs.rs` plus `vcs/`. What
remains over the line is test files, which are a separate debt.

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

**The context wave** — the interface learned where the reviewer is. A `Context`
derived from mode, focus and tab names the bar's mode segment (`FILES`,
`COMMITS`, `DIFF`, `STACK`…) in that context's own hue as bold ink on the bar's
ground; the first `?` is now a contextual tip in the corner above the `? help`
hint, listing only this context's keys, with the whole keymap one more press
away. The chrome's colours became ANSI palette indices (`rv::theme`), so the
terminal's own theme decides them, live — only additions and removals keep RGB,
because a proportion is a blend and an index cannot blend. The sidebar's cell
bar died in favour of that blend on the row's own text — the name runs green
through a wide smooth seam to red where the change is split — with `g` and `#`
toggling the tint and the counts. `H`/`L`, the wheel's other axis and (in the
diff) `Shift+←/→` scroll both panes sideways with a leading `…`; in the sidebar
the shifted arrows walk the tree instead — `Shift+→`/`Enter` zooms into a
directory or change, making its contents the whole view under a `▴` row that
names the place, and `Shift+←`/`Esc` backs out. Dogfooding found the readability
holes the same hour they shipped: black-on-magenta mode text became coloured
ink, and the tip stopped dimming momentarily-inapplicable keys.

**The agent loop becomes the CLI** (CLI-loop spec, same day it was written) —
the markdown demoted from writable database to one-way view. `rv comments
--json` is the read channel, `rv reply` the answer channel, `rv diff --json`
issues the side-aware coordinates `rv comment --line` accepts, `-m -` takes
bodies on stdin, and `rv status --check` turns the worker's poll and a CI gate
into one exit code. Saving, settling and replying no longer rewrite
`REVIEW-FEEDBACK.md`; only `rv render` (stdout, `--out` for a file) and the
TUI's `e` produce it, and nothing reads it back. The reply-ingest ordering
rule, `write_markdown_if_current` and the TUI's ingest-on-export all went;
`parse_replies` survives exactly one release as the §5 rescue that folds a
pre-amendment reply into the store on load, and its hostile-input test corpus
stays with it until both are deleted together. Both skills rewritten around
the three-command loop; the reviewer's `jj diff` dependency is gone.

**The spec reconciliation** — two auditors swept all five design specs against
the code; everything they found was either fixed, implemented, or written
down. Fixed: abandoned comments had no section in the rendered markdown and
silently vanished from it — the one outcome the storage spec forbids.
Implemented from the specs' unmet promises: the anchor cascade's third tier
(`Weak` — content gone, the raw line number still standing — so a rewritten
line keeps its comment instead of outdating it) with rename-following on
re-anchor, and `confidence`/`resolved_line` surfaced in `rv comments --json`;
statuses now expire off the bar after eight seconds on the alerts' injected
clock; `1`/`2`/`3` jump straight to a sidebar tab, and switching tabs
preserves position — the cursor lands on the selected file's row, under the
newest change that touched it in the commits tab; the `/` picker matches
every query word against name-then-file (`store write` finds
`write_markdown` in `store.rs`) and shows each match's kind; the bar's
position segment carries `path:line`, plus the enclosing symbol whenever the
index is already warm. Every spec gained a dated implementation-status
appendix naming what was superseded by later rulings and what remains open.
Every item on that list was closed on 2026-08-20 except one: **file-scoped
comments** (§9 `Scope::File`, and with them commenting on binary files) is
still unbuilt and still the only thing the five appendices call open.
