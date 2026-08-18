# rv — a viewport you can move around in

**Status:** design approved in conversation, awaiting spec review
**Date:** 2026-08-18
**Builds on:** `2026-08-17-rv-inline-comments-design.md` (focus model, comment boxes),
`2026-08-17-rv-navigation-design.md` (tree-sitter, views), `2026-08-17-rv-storage-model-design.md`
**Supersedes:** the "No mouse support" non-goal in both of the above.

## 1. Purpose

rv currently renders a fixed 30/70 split, in one colour per diff line, with a
sidebar that is one flat list and a keymap you can only learn by reading the
README. This makes the panes resizable, gives the code its own colours inside the
green and red, turns the sidebar into a tree you can collapse, puts the whole
keymap one `?` away, and lets the mouse do what a mouse is for.

None of it changes what a review *is*. No new state reaches `.review/`.

## 2. Requirements

From the user, verbatim in intent:

1. Resize the TUI panels.
2. A `?` popup that shows the controls.
3. Syntax highlighting in the diff — green/red backgrounds, but the code itself
   coloured by syntax.
4. The file sidebar as a tree or a list, togglable.
5. Mouse: move around, resize, drag.
6. Each file row tinted by the shape of its change — all green at 100% additions,
   split in proportion otherwise, meeting at a tight light seam.
7. A zellij-like theme, with a powerline status bar along the bottom.
8. A focused pane colours its border.
9. Alerts as a small floating panel at the top with an orange border, gone after
   about five seconds.

## 3. The layout is computed once

This is the load-bearing decision, and everything else in the document depends on
it.

Today `ui::draw` computes its rectangles inline. Once the mouse can click things,
a second place has to answer "what is at column 41, row 12?" — and if that answer
is computed separately from the drawing, clicks land on the wrong thing. This
project has already shipped exactly that bug once, when the pane and the anchor
disagreed about which line a comment belonged to, and the fix was to make both
read from one function.

So:

```rust
pub struct Layout {
    pub sidebar: Rect,
    pub divider: Rect,     // the draggable column between the panes
    pub diff: Rect,
    pub bar: Rect,         // along the bottom, under both panes
    pub popup: Option<Rect>,
    pub toast: Option<Rect>,   // floating, top-centre, over whatever is beneath
}

pub struct Chrome { pub bar_rows: u16, pub help_open: bool, pub toast: bool }

pub fn layout(area: Rect, split: Split, chrome: Chrome) -> Layout;
pub fn hit(layout: &Layout, column: u16, row: u16) -> Option<Target>;

pub enum Target {
    SidebarRow(usize),
    DiffRow(usize),        // an index into the row plan, not a diff line
    Divider,
    Bar,
    Popup,
}
```

`layout` takes a `Chrome` rather than a `&Mode` so it stays a function of the
*view*, not of the app: the bar's height is the only thing the mode decides, and
passing the number of rows keeps the layout independent of what a mode means.

There is no `Target::Toast`. The toast is drawn over the panes but is not a click
target — see §9, where it takes no key and no gesture.

`layout` is a pure function of the area and the app's view state. `draw` uses it;
`hit` uses it; nothing computes a rectangle anywhere else. **What you click is
what you see, by construction.**

`Target::DiffRow` is an index into the existing row plan from `rv::rows`, not a
diff line number — because a comment box occupies rows too, and clicking one
should select that comment rather than some line it happens to sit near. The row
plan already knows which is which.

## 4. Resizable panes

```rust
pub struct Split { ratio: u16 }   // percent of the width given to the sidebar
```

- Held in `App`, session-only. It is a view preference, not review state, and
  v1 has no config file — the same reasoning that keeps the fold set out of
  `.review/`.
- **Keyboard:** `<` and `>` move the split by 2% a press. Keyboard resize exists
  because the mouse is not always available (a terminal over ssh with reporting
  off, a user who does not use one), and a feature reachable only by mouse is a
  feature some users do not have.
- **Mouse:** press on the divider column and drag; the split follows the pointer
  until release.
- **Clamped so neither pane can become useless:** the sidebar is never narrower
  than 12 columns nor wider than the area minus 20, and both bounds give way to
  an even split when the terminal is too small to honour them. A pane that can be
  dragged to zero width is a pane a user can lose.

### Collapsing, and small screens

A reviewer on a phone over ssh has forty columns, and two panes do not fit in
forty columns no matter how the ratio is set. rv has to work there, because
"read this diff on my phone" is exactly when a reviewer reaches for a terminal
rather than a browser.

**The sidebar collapses.** `z` toggles it — zoom, as in tmux and zellij — and a
chevron in its top border does the same thing with a click:

```
╭▾ Files (57)──────────╮   expanded, chevron points down
╭▸────────────────────╮    collapsed to a single column, chevron points right
```

**Ruling — the chevron is a `Target`, not a special case.** `hit()` already
answers what is at a cell; the toggle becomes `Target::SidebarToggle` and a click
on it is routed like every other click. A second mechanism for "what did they
click" is how the pane and the pointer start disagreeing.

**Ruling — collapsing is reachable by key as well as by click.** A control that
exists only for the mouse is a control an ssh user without mouse reporting does
not have, and that user is precisely the one this feature is for.

**Below 80 columns rv shows one pane at a time.** The sidebar collapses itself,
the diff takes the full width, and `z` swaps which one you are looking at rather
than trying to show both. This is a breakpoint, not a preference: at forty columns
a 30% sidebar is twelve columns of truncated path beside twenty-six of truncated
code, which is two unreadable panes instead of one readable one.

**Ruling — the breakpoint overrides the split but does not overwrite it.** The
reviewer's ratio is remembered; widening the terminal past the breakpoint restores
exactly the layout they had. A responsive layout that silently discards a
preference makes the reviewer set it twice.

Everything else that has to give at forty columns already does: status segments
drop by priority with the `?` hint last, the keymap popup scrolls when even one
column will not fit, diff lines clip with a marker rather than silently, and
comment bodies wrap. What this section adds is the pane that has to disappear
entirely, and the control to bring it back.

## 5. The `?` popup

### The problem it solves

Today the status line *is* the whole bar. It starts out showing the keymap hint,
and the first time anything happens — `deleted comment at app.rs:42`, `jumped to
store.rs:238` — that message replaces the hint **and stays there for the rest of
the session**. So the one place a reviewer could learn the keys is destroyed by
the first thing they do, and never comes back.

That is the defect `?` exists to fix, and fixing it properly means two changes,
not one: the keymap gets a home that nothing can evict (this section), and the
status stops being able to evict anything (§9).

### What it looks like

A centred panel, laid out like helix's keymap: bindings grouped by what they act
on, in as many columns as the terminal can hold, with keys in the accent colour
and descriptions beside them.

```
        ╭─ keys ─────────────────────────────────────────────────────╮
        │                                                            │
        │  move                      comments                        │
        │    ↓  (j)  next row          c      comment on this line   │
        │    ↑  (k)  previous row      enter  open the comment stack │
        │    ]  [    next / prev file  d      delete (asks first)    │
        │                              s      fold                   │
        │  focus                                                     │
        │    ←  (h)  sidebar         view                            │
        │    →  (l)  diff              t      tree or list           │
        │    tab     files/comments    < >    resize the panes       │
        │  quit                                                      │
        │    q       quit              ?      close this             │
        │                                                            │
        ╰────────────────────────────────────────────────────────────╯
```

**Ruling — the arrows are the binding; `hjkl` are aliases shown in parentheses.**
Everywhere the keymap is presented — this popup, the status bar, the README — the
arrow leads and the vim key follows it in brackets. rv is a tool a reviewer may
open once a week, and the arrows are the keys someone can find without being
told. `h` and `l` are added as aliases for Left and Right so the vim set is
complete rather than half-present, but they are the alternative spelling, not the
name of the binding.

**It must fit without scrolling at 80×24**, which is the size a reviewer over ssh
actually has. That is what drives the column layout: a single list of twenty
bindings needs twenty rows and does not fit beside its own borders and headings,
while three columns of seven do. The panel takes as many columns as fit and falls
back to fewer — and only when even one column will not fit does it scroll.

**Bindings that do nothing right now are dimmed rather than hidden**: `d` in the
Files tab, `t` in the Comments tab. A reviewer learning the tool should see that
the key exists and be told why it is inert here, rather than wondering whether
they misread the manual.

### It follows what you are looking at

What a key does depends on what is selected, so the popup leads with the context
you are actually in — the diff, a file, a commit, a comment — and the status bar
names that context too.

```rust
pub enum Context { Files, Commit, Comments, Diff, Comment, Typing, Confirm }
```

| Where the cursor is | Context |
|---|---|
| Sidebar, Files tab, on a file | `Files` |
| Sidebar, Files tab, on a commit node | `Commit` |
| Sidebar, Comments tab | `Comments` |
| Diff pane | `Diff` |
| Inside a line's comment stack | `Comment` |
| Typing a comment | `Typing` |
| Answering a delete | `Confirm` |

**Ruling — the context is derived, never stored.** It is a function of the mode,
the focus, the sidebar tab and the kind of row under the cursor. A stored copy
would need invalidating on every one of those changes and nothing would be
watching — the same reasoning that keeps `outdated` derived in the storage spec.

**Ruling — the popup emphasises by context; it never hides by context.** The
group matching where you are comes first and is titled for it; every other group
still appears, and bindings inert here are dimmed. Hiding them would make the one
screen built for learning the tool teach less than it could: a reviewer wants to
know what `d` does *before* they move onto a comment, not after. Contextual menus
that hide are how a user ends up believing a feature does not exist.

**Ruling — the `contexts` field drives dispatch as well as display.** A `Binding`
records which contexts it applies in, `on_key_browse` consults that to decide
whether a key does anything, and the popup consults the same field to decide what
to emphasise and what to dim. So a key that is inert cannot be shown as active,
and a key shown as active cannot be inert — the same one-table discipline that
keeps the keymap and the help from drifting.

**Ruling — the popup is generated from the same table the key handler dispatches
from.** A help screen maintained by hand is a help screen that lies; this project
has already had a README claim the keymap "agrees exactly" with the code while
missing two bindings. One `const BINDINGS: &[Binding]`, where `Binding` carries
the keys, the action, and the one-line description; `on_key_browse` matches on it
and the popup renders it. A binding that exists cannot be undocumented, and a
documented binding that does not exist will not compile.

- `?` opens it. `?`, `Esc`, and `q` all close it — **`q` closes the popup rather
  than quitting rv**, because quitting from a help screen is a surprise, and the
  reviewer who opened help is the one least sure what the keys do.
- While it is open every other key is inert, so a keystroke aimed at the help
  screen cannot edit a review by accident.
- If the terminal is too small for the full list the popup takes what it can and
  scrolls with `j`/`k`. It never truncates silently — the same rule the diff pane
  now follows.

## 6. Syntax highlighting

### Where the colours come from

Tree-sitter, sharing the grammars the navigation spec already commits to for the
symbol index, using each grammar's bundled `highlights.scm`. One parse per file
per side serves both features; a second highlighting engine would be a second
parser to keep in step.

**A diff line cannot be parsed on its own** — it is one line pulled out of a file,
and a parser needs the file. So highlighting is computed from the blob, not from
the diff:

1. Parse the whole blob once per `(commit, path)`, lazily, on first render of
   that file, and cache it — the same laziness the diff pipeline already uses for
   blob reads, for the same reason.
2. Extract highlight spans as `(line, start_col, end_col, capture)`.
3. A diff line looks up its own line number **on its own side**: a `Removed` line
   takes its spans from the base blob at its `left` number, everything else from
   the head blob at its `right`. This is the same side rule as `anchored_side`,
   and it must come from that one function rather than a second copy.

A file with no grammar renders plain, and the pane title says so. The project's
standing rule holds: the tool never presents a guess as a fact.

### How the colours combine

A dim green or red wash behind the whole line, with syntax colours at full
strength on top.

```
   1     fn parse(s: &str) -> Result<Ast> {
   2 -       let raw = s.trim();              ← dim red wash, syntax at full strength
   2 +       let raw = s.trim_start();        ← dim green wash, syntax at full strength
   3         Ok(parse_inner(raw)?)
```

- The tint is a background; the syntax colours are foregrounds. They cannot
  collide because they never contend for the same channel.
- **The selected line is marked with a brighter background, not `REVERSED`.**
  Reversing swaps foreground and background, which on a tinted line turns the
  syntax colours into the wash and the wash into the text — legible in neither
  direction. This is a change to existing behaviour and it is deliberate.
- Comment boxes keep their blue borders and render their bodies as prose. Prose
  is not code, and highlighting it would be a category error.

### The theme is the terminal's, not rv's

**Ruling — code text is painted only in the 16 indexed ANSI colours, never in
RGB.** This is not a degradation strategy, it is the whole theming design.

When a program emits colour index 4, the terminal substitutes whatever *its*
scheme calls blue. The 16 indexed colours are therefore already a pass-through to
the user's preferences: their Solarized, their Gruvbox, their hand-tuned
one-off, applied automatically and identically to every other tool they run.
Emitting `38;2;r;g;b` does the opposite — it dictates exact colours and ignores
the scheme entirely, which is what makes a syntax theme something a user then has
to configure. rv should never need a theme option, because rv should never be the
thing deciding.

The mapping is therefore semantic rather than chromatic:

| Capture | Index | Why |
|---|---|---|
| Comment | 8 (bright black) | every scheme defines 8 as its muted tone against its own background — this is the one that must never be white |
| Keyword | 5 (magenta) | |
| Function | 4 (blue) | |
| Type, Constructor | 6 (cyan) | |
| String | 2 (green) | |
| Number, Constant | 3 (yellow) | |
| Punctuation, Variable, Other | default | unstyled text inherits the terminal's own foreground |

Light and dark terminals need no special handling: a scheme built for a light
background defines its own 8 as a dark grey, and one built for a dark background
defines it as a light grey. Asking rv to detect which is to reimplement, badly, a
decision the user already made.

**Ruling — rv does not query the terminal's palette.** It could: `OSC 4;n;?`
returns a palette entry and `OSC 10`/`11` the default foreground and background,
and most modern emulators answer. But it means writing an escape sequence and
then *waiting for a reply that a multiplexer may swallow and an older terminal
never sends*, inside a TUI that must not hang on startup. The indexed colours
give the same result without asking a question that can go unanswered.

**Ruling — chrome may use RGB where it genuinely must; code text may not.** The
change gradient cannot exist in 16 colours, so it uses truecolour and degrades to
the 256-colour cube and then to a hard split. That exception is bounded to the
sidebar's tint, the alert orange and the focus magenta — decorations rv owns. The
code a reviewer is reading belongs to their terminal.

**Ruling — themes are not configurable in v1**, and after the above they should
not need to be. A theme system is a config file, and v1 has no config file.

## 7. The sidebar: tree or list

`t` toggles between a flat list and a directory tree; the title says which.

- **Tree groups by directory**, holding only the files the review changed.
- **Single-child chains collapse into one row** — `docs/superpowers/specs/` is one
  row, not three. A 29-file review otherwise spends most of its rows on
  punctuation.
- A directory row collapses and expands with `s`, which is already the project's
  verb for *collapse the thing under the cursor*: a comment box in the diff, a
  comment in the browser, and now a directory in the sidebar. One key, one
  meaning, three places.
- The flat list stays the default, because it is faster for the small reviews that
  are most reviews.
- Tree state is session-only, like every other view preference here.

This interacts with the tabs from the inline-comments work: **`Tab` cycles
Files → Commits → Comments**, and `t` applies wherever files are listed — the
Files tab, and the files beneath each commit. In the Comments tab `t` does nothing
and says so.

**Ruling — the commits view is a sidebar tab, not a separate view mode.** An
earlier draft gave it `1` and `2` while `Tab` cycled the tabs, which is two
mechanisms answering one question: what is the left column showing. A reviewer
reaches for Tab, and having reached for it should arrive. `1` and `2` survive as
direct shortcuts for jumping to a tab without cycling.

That also removes a piece of state. **Scope follows the selection, not a mode:**
selecting a commit in that tab is what narrows the diff to that change against its
parent and narrows the symbol index with it. There is no separate view enum to
keep in step with the tab, and therefore no way for the two to disagree.

### A commit is a directory

The navigation spec adds a commits view — `1` for the whole bookmark, `2` for one
change at a time. In that view **a commit is simply another node in the same
tree**, holding the files it touched:

```
▾ ytskpxpw  close the jj_lib alias bypass
  ▾ rv-core/tests
      constraints.rs
▾ zmomvwzm  enforce the constraints mechanically
  ▾ rv-core
    ▾ src
        store.rs
      ▾ tests
          store.rs
```

**Ruling — one tree, three node kinds, not two widgets.** A commit node, a
directory node and a file node differ only in what they are labelled with and what
they contain. Building the commits view as a separate list would mean a second
selection model, a second collapse rule, and a second place for the sidebar's
gradient to be computed — and the two would drift, the way the pane and the anchor
drifted.

`s` collapses a commit for exactly the reason it collapses a directory or a
comment box: it is the project's one verb for *collapse the thing under the
cursor*, now in four places with one meaning.

**`Enter` and `Space` also fold a directory or a commit row.** Every file tree a
reviewer has used — an editor's explorer, a file manager, a forge's tree — opens
and closes a folder with Enter or Space, and `s` alone makes rv the odd one out.
On a *file* row both keys keep their existing meaning and move focus to the diff,
because a file is a thing to look at while a directory is a thing to open.

**Ruling — this replaces `Enter`-on-a-directory rather than adding to it.** Today
`Enter` on a directory row focuses the diff, which is close to meaningless: a
directory is not a file, so there is nothing to focus on. Nothing is lost by
giving the key the behaviour every other tree already has. `t` still chooses whether the files
*inside* a commit are a directory tree or a flat list, so the commits view has both
shapes too.

**Ruling — `NodeKind::Commit` is defined now, even though the commits view lands
with the navigation work.** A third node kind costs almost nothing while the tree
is being written and is a retrofit afterwards; the tree module should be built
knowing a commit is coming.

A commit row aggregates the change gradient of everything beneath it, so a change
that is mostly deletions reads that way before it is expanded.

### How big, and in what order

The gradient says what *shape* a change is. It does not say how *large*, and a
reviewer deciding what to read next needs both — a 900-line lockfile update and a
12-line logic change should not look alike.

**Every row carries its counts**, right-aligned, in the same green and red the
gradient uses:

```
▾ ytskpxpw  close the jj_lib alias bypass            +148  −12
  ▾ rv-core/tests                                     +148  −12
      constraints.rs                                  +148  −12
▾ zmomvwzm  enforce the constraints mechanically      +212  −4
  ▾ rv-core/src
      store.rs                                         +18  −4
    ▾ tests
        store.rs                                      +194  −0
```

A directory or commit row shows its subtree's total, for the same reason it shows
its subtree's gradient: a collapsed row that hides its own weight is a row you
have to expand to evaluate.

Counts come from the `Stat` already computed for the gradient, so this costs
nothing new — and it inherits the same honesty: they count **lines of text**, and
the pane remains the authority on meaning. Large numbers abbreviate (`+1.2k`) so
one enormous file cannot eat the row.

**Ruling — when the sidebar is too narrow, the counts go before the path does.**
The path is the row's identity; the counts are context. And when they are dropped
the gradient still conveys the ratio, so the information degrades rather than
disappears.

### Sorting

`o` cycles the order, and the sidebar's title names it, because a list whose
order you cannot see is a list you cannot trust.

| Order | Files | Commits |
|---|---|---|
| `natural` | path order | stack order, newest first |
| `added` | most additions first | most additions first |
| `removed` | most removals first | most removals first |

`natural` is the default and means "the order the thing already has" — file
structure for files, time for commits — which is why it is one mode rather than
two.

**Ruling — sorting applies within the current grouping, not across it.** In tree
mode siblings are sorted against each other and directories keep their children;
a directory sorts among its siblings by its aggregate. Sorting does not flatten
the tree, and the tree does not disable sorting — a reviewer asked for both and
they compose.

**Ruling — the order is session-only and does not touch the review.** Like every
other view preference here: not in `.review/`, not persisted, and it never
reorders anything a comment refers to.

### Every row is a change gradient

A file row is tinted across its width by the shape of its change: all green when
every changed line was added, all red when every one was removed, and split in
proportion between — a file with twice as many additions as deletions is green for
two thirds of its width and red for the last third.

```
 +  src/app.rs        ████████████████████████░░░░░░░░   82% added
 ~  src/ui.rs         ██████████████▒▒▒▒▒▒░░░░░░░░░░░░   45% added
 -  src/old.rs        ░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░    0% added
 →  docs/moved.md     (no line changes — neutral)
```

**Ruling — the gradient is a diverging scale through a tight light pivot: green →
near-white → red, interpolated in Oklab.**

Two problems, one answer. Interpolating green to red channel-by-channel in sRGB
passes through muddy olive, because sRGB is not perceptually uniform and the two
hues differ sharply in luminance. Moving to Oklab fixes the mud but not the
underlying issue: green and red sit at opposite ends of Oklab's `a` axis, so a
straight lerp between them crosses `a ≈ 0` — a dull mid-grey exactly where the eye
is trying to read the boundary.

Pivoting through a bright neutral turns that crossing into a feature. Each half
only ever desaturates toward the pivot and back, so no cell is ever a mixture of
the two hues, and the meeting point reads as a deliberate seam rather than a patch
where the colour got confused.

The pivot is **not pure white**, and its lightness is defined relatively rather
than absolutely: a fixed step above the lighter of the two endpoints in Oklab `L`,
capped just short of white. Absolute white is a bright flare on a dark terminal and
vanishes into the background on a light one; a relative step reads as a highlight
in both.

**The pivot band is tight** — a couple of columns, not a wash. The bar exists to
show a proportion, and a wide blend destroys the very thing it is drawing: you can
no longer see where two thirds ends and one third begins.

**Ruling — there is no background wash on a sidebar row at all.** The colour
lives in the counts: `+204` in the added green, `-12` in the removed red, as
foreground on the terminal's own background. Two rounds of looking at the running
tool settled this. A full-row wash reads as a selection and competes with the real
one; even a text-width wash destroys the thing the sidebar exists to show, because
in tree mode the structure *is* the indentation and the fold markers, and neither
survives being painted over. Thirty files became thirty slabs of green and the
tree stopped looking like a tree.

**The proportional gradient survives as a compact bar**, a few cells wide, drawn
beside the counts only when the row has the width to spare — dropped before the
counts, which are dropped before the path. So the shape of a change is still
visible at a glance, but it is a mark on the row rather than the row itself.

**The only full-row background in the sidebar is the selection.** One background,
one meaning.

**Ruling — one green and one red for the whole interface.** The gradient's
endpoints are the same dim washes the diff pane puts behind an added and a removed
line. Shipping a saturated sidebar green beside a near-black diff green meant the
same concept rendered at two wildly different intensities in two panes a reviewer
looks at together — the sidebar shouted while the diff whispered, and neither
looked deliberate. The palette declares them once and both panes import them.

**Ruling — a directory row in the tree aggregates its subtree.** A collapsed
directory whose children are mostly deletions should say so; otherwise collapsing
a directory hides the very thing the colour exists to convey.

**Ruling — counts come from the in-process fallback diff, computed for every file
at startup, and they measure text rather than meaning.** The pane's difftastic
view is per-file and lazy by design; asking it for counts would mean running a
subprocess per file before the first frame, which on a large review is seconds of
startup for a decoration. The `similar` path is in-process and costs microseconds
per file, so the sidebar can colour every row immediately. The consequence is
honest and documented: a file whose change is purely cosmetic — a reindentation
that difftastic calls no semantic change — still shows a gradient, because its
text did change. The README says the bar measures lines of text, and the pane
remains the authority on meaning.

**Ruling — no lazy recolouring.** The row's colour is decided once, at startup,
and does not change as the reviewer browses. A sidebar whose colours shift while
you move through it is worse than one that is slightly coarser, and the
alternative — recolouring each file once its difftastic diff is known — would
repaint rows behind the reviewer's back.

Colour depth degrades rather than guessing: truecolour when the terminal
advertises it, the 256-colour cube when it does not, and at 16 colours a hard
green/red split with no blend. A file with no line changes at all — a pure rename,
a binary, a mode change — renders neutral, since a gradient over zero lines would
be inventing a ratio.

## 8. Mouse

Mouse reporting is **on unconditionally**. No toggle, no flag.

The concern that motivated a toggle — that capturing the mouse takes away the
terminal's own drag-to-select — does not survive contact with how terminals
actually behave: every current emulator (iTerm2, Alacritty, kitty, WezTerm,
Ghostty, xterm) keeps **Shift-drag** as a bypass that selects text regardless of
what the application requested. This is how zellij and every other mouse-aware TUI
works. rv therefore does not implement its own selection or clipboard, and the
README documents Shift-drag in one line.

What the mouse does:

| Gesture | Effect |
|---|---|
| Click in the sidebar | focus the sidebar and select that row (a directory row toggles it) |
| Click a diff line | focus the diff and select that line |
| Click a comment box | focus the stack and select that comment |
| Drag the divider | resize, following the pointer until release |
| Scroll wheel | scroll the pane under the pointer, without moving focus |
| Click in the popup | nothing; the popup is dismissed by key |

**Ruling — scrolling does not move the selection, and clicking does.** Scrolling is
looking; clicking is choosing. Conflating them means a stray wheel nudge silently
re-aims the next `c` or `d` at a different line.

**Ruling — no gesture deletes anything.** There is no click target for `d`, and
dragging a comment does nothing. The confirmation exists because deletion is
unrecoverable, and a mis-click is exactly the accident it protects against.

`App::on_mouse(MouseEvent) -> Result<Action>` sits beside `on_key`, and is equally
terminal-free: it takes a crossterm event and consults `hit`, both of which are
plain data. The event loop grows one arm. The state machine stays testable without
a pty, which is the property that has made every previous wave's tests possible.

## 9. Chrome: the status bar, the borders, and alerts

The look is zellij's: rounded borders, a powerline status bar along the bottom, a
coloured border on whichever pane has focus, and a restrained palette where every
colour already means something.

### The status bar moves to the bottom

It is currently drawn above the panes. It belongs below them, where nvim, zellij,
tmux and every terminal multiplexer put it, because that is where a reader's eye
goes for state rather than content.

Segments, separated by powerline arrows:

```
 BROWSE  src/app.rs 3/29  trunk()..@  4 open ▏                        ? help 
```

- **mode** — the `Context` from §5: `FILES`, `COMMIT`, `COMMENTS`, `DIFF`,
  `COMMENT`, `TYPING`, `CONFIRM`, coloured per context. It names what the cursor
  is on rather than only what typing does, because that is what decides what the
  next keystroke means — and it is the same value the `?` popup leads with, so
  the bar and the help can never describe different worlds.
- **position** — the selected file and how far through the list it is.
- **scope** — the revset being reviewed.
- **comments** — how many are open.
- **status** — the last thing that happened, when there is one.
- **hint** — right-aligned, naming `?`.

**Ruling — the status is a segment, not the bar.** This is the fix for the defect
described in §5: today a status message replaces the entire bar, so `deleted
comment at app.rs:42` evicts the keymap hint permanently and the reviewer loses
their only in-app reference on the first thing they do. As a segment it sits
between the others and can no longer displace anything. **The `?` hint is never
droppable** — it is the last segment to be dropped when the bar is narrow, ahead
even of the mode, because a reviewer on a narrow terminal is exactly the one who
most needs to be told where the keys are.

**Ruling — a status expires; roughly eight seconds, then the segment empties.** A
line reading `deleted comment at app.rs:42` ten minutes later is not information,
it is a claim about the present that stopped being true. It uses the same
injected-clock machinery as alerts, so its expiry is an ordinary assertion rather
than a sleep. Alerts and statuses differ in what they are for, not in how they
age: a status is the last thing that happened, an alert is something that went
wrong.

In `Mode::Comment` the bar grows to hold the buffer, as it does today.

**Ruling — powerline separators by default, with `RV_ASCII=1` to turn them off.**
The arrow glyphs need a patched font, and rv cannot detect one; a terminal without
it shows tofu. Defaulting to arrows matches what the user asked for and what
zellij does, and the escape hatch follows the precedent `RV_NO_DIFFT` already set
in this project. `RV_ASCII=1` substitutes plain separators and changes nothing
else.

### Focus colours the border

A focused pane's border is drawn in an accent colour; the unfocused one stays
dim. This replaces the earlier ruling that focus be shown with weight alone.

**Ruling — the focus accent is magenta, not green, red, blue or orange.** Every
other colour in this interface already carries a meaning: green is an addition,
red a removal, blue a comment, orange an alert. A focus accent reusing any of them
would be ambiguous exactly when the reviewer is scanning quickly. Magenta is the
one strong hue left unclaimed.

**The `▸` title marker stays** even though the border is now coloured. It is a
redundant signal on purpose: a reviewer with a 16-colour terminal, or one who does
not distinguish magenta from red, still needs to know where their keystrokes are
going. Colour is an enhancement of that signal, never the only carrier of it.

Borders are rounded, which is the whole of the zellij resemblance that is not
already covered by the bar and the accent.

### Alerts float, briefly

A **status** describes state and lives in the bottom bar: `comment saved at
app.rs:42`, `no comments on this line`. An **alert** describes something that went
wrong and needs to be noticed: a blob that could not be read, an anchored file
that has left the range, an export that is stale. Alerts get a small floating
panel at the top of the screen with an orange border, and they leave on their own
after about five seconds.

```
        ╭──────────────────────────────────────────╮
        │ ⚠ src/old.rs is no longer in this range  │
        ╰──────────────────────────────────────────╯
```

**Ruling — the toast never steals focus and never takes a key.** It cannot be
dismissed, confirmed, or interacted with, and no keystroke is consumed by it. It
is a notification, not a dialog; anything that needs an answer is a confirmation
in the bar, where `d` already puts one.

**Ruling — the fade is an Oklab lightness ramp, and there is no fade at 16
colours.** Terminals cannot alpha-blend, so fading means stepping the border and
text down in lightness over the final second — four steps, using the Oklab
machinery the change gradient already needs. On a terminal without the colour
depth to show a ramp, the toast simply disappears at its deadline. A fade that
degrades into a flicker is worse than no fade.

**Ruling — time enters the app as a parameter, never as a call to the clock.**
This is the consequential one. `App` gains:

```rust
pub fn alert(&mut self, message: impl Into<String>, now: Instant);
pub fn expire_alerts(&mut self, now: Instant);
pub fn alerts(&self) -> &[Alert];
```

Nothing inside `App` calls `Instant::now()`. The event loop supplies the time, and
a test supplies whatever time it likes — so "the toast is gone after five seconds"
and "it is dim at four and a half" are ordinary assertions rather than sleeps.
Every state machine in this project has stayed testable by refusing to reach for
ambient input, and a clock is ambient input.

**The event loop must stop blocking forever.** It currently sits in
`event::read()` until a key arrives, which means a toast raised at t=0 would still
be on screen at t=∞ if the reviewer walked away. It becomes `event::poll(timeout)`
with the timeout derived from the nearest alert deadline — the next fade step, or
expiry — and infinite when no alert is live, so an idle rv with nothing to show
still costs nothing.

## 10. Scrolling reaches every row

### The defect

A comment taller than the diff pane cannot be read. Not "is awkward to read" —
cannot be read, because part of it is unreachable at every cursor position.

The pane anchors its window on the row of the **selected diff line**
(`ui.rs`'s `plan.row_of_line(line)`), and `j` moves the selection to the next
diff line. A comment box sits between two diff rows. So with a box taller than
the pane: on the line above it you see the box's top, on the line below it you
see the box's bottom, and no selection anywhere puts the middle rows inside the
window. Scrolling appears to "jump through" the comment because it is not
scrolling the comment at all — it is stepping over it.

### The fix: the cursor moves by row

`↓` and `↑` — and their `j`/`k` aliases — move a cursor over the **rows** of the
plan, not over diff lines. A comment box is rows, so a comment box is something
the cursor can walk into, and every row is reachable by construction.

The selection everything else depends on — `c`, `d`, `comments_for_line`, the
anchor a comment is saved against — becomes **the diff line that owns the row
under the cursor**: a diff row owns itself, and a box row is owned by the line
its box hangs from. So `c` on a comment box comments on the line that comment is
about, which is the only thing it could sensibly mean.

**Ruling — the row cursor is the state; the line index is derived.** The reverse
would leave two cursors to keep in step, and the whole reason this defect exists
is that the window's anchor and the user's cursor were different things. One
cursor, one anchor.

**Ruling — reachability is a property test, not an example.** For any plan and
any pane height, every row must appear in the window for some cursor position.
That is the assertion this defect would have failed, and no example test would
reliably have caught it, because it only bites when a box is taller than the
pane — which no fixture happened to build.

`]` and `[` still move by file, and `J`/`K` — or the next natural pair — may
later move by diff line for a reviewer who wants to skip comments entirely; that
is an addition, not a replacement, and it is not required to fix this.

## 11. Testing

- **Layout and hit-testing are one function's two consumers**, so the central
  property is round-tripping: for every cell of a rendered frame, `hit` returns
  the target that `draw` actually painted there. Generate terminal sizes and
  split ratios and assert it, including at the clamp boundaries.
- **Resize:** the split never leaves its bounds under any sequence of `<`, `>`,
  and drags; a terminal too small for the bounds still renders; a drag that leaves
  the window does not strand the divider.
- **Popup:** every binding in the table appears in the popup; every key the
  handler dispatches is in the table (the compiler enforces the second half, and a
  test asserts the first); `q` closes rather than quits; keys are inert while it
  is open; it renders at 20x6 without panicking.
- **Highlighting:** a `Removed` line takes its spans from the base blob and an
  `Added` line from the head blob — the test that catches a side-blind
  implementation needs a rewrite that does not move, same path and same line
  number on both sides, since a rename encodes the side in its path and cannot
  discriminate. A file with no grammar renders plain and says so. A blob that
  fails to parse renders plain rather than erroring.
- **Sidebar tree:** single-child chains collapse to one row; `s` toggles a
  directory; the tree lists exactly the files the flat list does, which is the
  conservation property — a tree that loses a file is worse than no tree.
- **Mouse:** clicking a row selects what was drawn there at that size (this falls
  out of the round-trip property); scrolling changes no selection; no gesture
  deletes; a drag on the divider changes only the split.
- **Panic sweep** over a range of sizes in every view state, as previous waves
  did. Note from experience: a 1x1 terminal does *not* exercise the box code,
  because the panes get zero height and nothing draws — the sizes that catch
  arithmetic errors are the small-but-nonzero ones like 2x5, 9x6 and 12x24.

## 12. Non-goals

- No config file, no themes, no persisted layout. Every preference here is
  session-only.
- No clipboard integration and no rv-implemented text selection; Shift-drag is the
  terminal's job.
- No mouse gesture that deletes, resolves, or edits.
- No syntax highlighting inside comment bodies.
- No horizontal scrolling — long lines are still clipped with a marker, because
  the row model needs one row per diff line.
- No split beyond the two panes; no third pane, no vertical stacking.

## 13. Risks

| Risk | Mitigation |
|---|---|
| Click targets drift from what is drawn | `layout` is one function with two consumers, and a property test round-trips every cell of a real frame |
| Highlighting takes its spans from the wrong side | The side comes from the shared `anchored_side`; the test uses a rewrite that does not move, which is the only shape that can catch it |
| Tinted backgrounds make code unreadable on some themes | Tint is dim and background-only; syntax stays foreground; 16-colour mapping so a limited terminal degrades rather than guesses |
| Parsing every blob slows rendering | One lazy parse per `(commit, path)`, cached, exactly as blob reads already are; review-sized files |
| Mouse reporting annoys users who want to select text | Shift-drag documented; no capture of the clipboard; nothing rv does is irreversible by mouse |
| The keymap grows past what a bar can show | That is why `?` exists, and why it is generated from the dispatch table rather than written by hand |
