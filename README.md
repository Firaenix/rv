# rv

A terminal code reviewer for [Jujutsu](https://jj-vcs.github.io/jj/) stacks.

`rv` reads a range of jj changes out of your local repository, shows you the
diff, and lets you attach comments to individual lines. Those comments live in
`.review/`, and the same CLI is a coding agent's whole interface to them: the
agent reads the review with `rv comments --json`, fixes the code, answers with
`rv reply`, and ticks work off with `rv resolve` — no file parsing anywhere.

It is a review tool for the work sitting on your disk right now — before it
becomes a pull request, or instead of ever becoming one.

## What rv is not

- **No forge.** It does not know what GitHub, GitLab, Gerrit or Forgejo are. It
  never reads or writes a pull request, a merge request or a change list.
- **No network.** Nothing `rv` does makes an outbound connection.
- **No authentication.** There is no account, no token, no config file to hold
  one.
- **No model calls.** `rv` never talks to an LLM. The handoff is a file you give
  to whatever agent you already use.
- **No history mutation.** `rv` opens the repository read-only. It never starts
  a transaction and never writes inside `.jj/`. The only things it writes are
  `.review/` and one line appended to `.git/info/exclude`.
- **No user configuration.** `rv` deliberately ignores your jj config — your
  `revset-aliases`, your `ui.diff.tool`, all of it. It behaves the same on a
  fresh jj install as on a heavily customised one. The one knob is the
  `RV_NO_DIFFT` environment variable below.

## Requirements

- A jj repository **colocated with git** — one with a top-level `.git/`. This is
  the default for `jj git init` as of jj 0.44. `rv` needs `.git/info/exclude` to
  keep `.review/` out of the change you are reviewing; see
  [Current limits](#current-limits) for what happens without it.
- Rust 1.89 or newer, edition 2024 (both forced by `jj-lib` 0.44) — to build
  from source. The prebuilt binaries below need none of it.
- [difftastic](https://difftastic.wilfred.me.uk/) (`difft`) on `PATH`, optional.
  With it you get structural, language-aware diffs. Without it `rv` silently
  falls back to a line diff and labels the pane `fallback` so you always know
  which one you are reading.

## Install

```sh
# macOS and Linux
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/Firaenix/rv/releases/latest/download/rv-installer.sh | sh

# Windows
powershell -c "irm https://github.com/Firaenix/rv/releases/latest/download/rv-installer.ps1 | iex"

# Homebrew
brew install Firaenix/tap/rv
```

Prebuilt binaries are published for macOS (Apple Silicon and Intel), Linux
(x86-64 and arm64) and Windows (x86-64). Each release also carries the archives
and their checksums directly, if you would rather not pipe a script into a
shell.

### From source

```sh
cargo build --release        # ./target/release/rv
cargo install --path rv      # or put it on your PATH
```

`rv` is not on crates.io: the name belongs to an unrelated project there, so
`cargo install rv` installs something else. Install from this repository, or
use one of the binaries above.

## Usage

Run `rv` from the **workspace root** — the directory holding `.jj/`. See
[Current limits](#current-limits).

| Command | What it reviews |
| --- | --- |
| `rv` | `trunk()..@` — everything on your stack that trunk does not have |
| `rv <bookmark>` | `trunk()..<bookmark>`. The target may be a bookmark, a change id or a commit id |
| `rv --from <rev> --to <rev>` | An explicit range, e.g. `rv --from main --to my-feature` |
| `rv comment <file> --line <n> [--side left] -m <text>` | Adds a comment exactly as the TUI would — anchor and id handled. `-m -` reads the body from stdin. The reviewer agent's way in |
| `rv comments [--json] [--state open]` | Lists the comments — id, state, body, reply, anchor and excerpt. The agent's read channel |
| `rv reply <id> -m <text>` | Stores an answer on the comment (`-m -` for stdin). A second reply replaces the first; state is untouched |
| `rv resolve <id>` / `rv abandon <id>` | Settles a comment, recording who (`--by agent` is the default; the TUI's `r`/`a` record `user`). Re-applying reopens it |
| `rv diff [<file>] --json` | The changes in rv's own side-aware coordinates — the numbers `rv comment --line` accepts |
| `rv render [--out <path>]` | Prints the review as markdown, a view nothing reads back; `--out` writes it to a file |
| `rv status` | Prints the range, its changes, its changed files and its comment counts |
| `rv status --json` | The same report as JSON, for scripting |
| `rv status --check` | Exit 1 while any comment is open, 0 otherwise, nothing printed — the worker's poll and a CI gate |
| `rv --repo <path> …` | Reviews the workspace at `<path>` instead of the current directory |
| `rv --no-difft …` | Diffs with the in-process engine instead of difftastic: line-based rather than structural, with context lines. What a reviewer with no `difft` on `PATH` sees |

### Nix

```sh
nix run .            # run rv without installing anything
nix build            # ./result/bin/rv, with difftastic wrapped onto its PATH
nix develop          # a shell with cargo, jj, difftastic and the rust toolchain
```

The packaged binary carries its own difftastic, so structural diffs work even
where `difft` is not installed.

`--to` overrides the positional target when both are given. That is also the
escape hatch for the one name collision: a bookmark literally called `render` or
`status` has to be passed as `rv --to status`, since subcommands share the first
positional slot.

`rv`'s idea of `trunk()` is its own built-in one — the latest of
`main`/`master`/`trunk` on `origin`/`upstream`, falling back to the repository
root. It is not read from your config, so in a repository with no remotes the
default range is your entire history.

Exit codes: **0** on success (including `--help` and `--version`), **1** when
`rv` itself fails — an unreadable workspace, an unresolvable revision, an empty
range, no terminal — and **2** for a command line clap rejects, such as an
unknown flag. Failures print the whole error chain as a sentence rather than a
backtrace.

### Example

Real output from `rv` reviewing its own repository, with the middle of each list
elided at the `…` markers:

```
$ rv status
revset  trunk()..@
base    0000000000000000000000000000000000000000
head    2b9795343e3de90321d23e92e13c8efb6a30e613

changes (17)
  ntoxqqukosxynuuymkrsllvymmsnsmxp 2b9795343e3de90321d23e92e13c8efb6a30e613 (no description set)
  uspywkpvwqtypzsqrwmptwpymlvvrtok fbad2ee3477848ec4350606e4961374f8bd24bb5 docs(rv): README and milestone-1 dogfood verification
  mspsnktlqkkmpyyqunnolnwqnnuvwkuv c960123dbb78d6cec5c61e0406fe66ce0301efad fix(rv): widen comment ids and display the anchored line number
  …
  nowwnlnmvkwonnvtrspxrrxnprsupkvs 38c8c68bf0bd9f88aa93cacce4dbd9512a670df0 docs: rv design spec, milestone-1 implementation plan, and handoff

files (28)
  added     Cargo.lock
  added     Cargo.toml
  added     README.md
  …
  added     rv/tests/cli.rs

comments  0 open, 0 awaiting verification, 0 resolved, 0 abandoned, 0 outdated
```

Every file reads `added` there only because this repository's whole history is
one stack with no merge base — `trunk()` degrades to the root commit when there
are no remotes. In a normal repository you will see `modified`, `removed` and
`renamed` too.

Change ids are shown the way `jj log` shows them (the reverse-hex `z`–`k`
alphabet), so you can paste one straight back into a jj command.

## The reviewer

A bare `rv` opens the terminal UI: a sidebar and a diff pane over a status
line, with your comments drawn in the diff itself, under the lines they are
about.

```
┌──────────────────────┬──────────────────────────────────────────────────────┐
│ Files (27)           │ ▸ rv-core/src/anchor.rs — difftastic (Rust)          │
│ +  Cargo.lock        │    47 +    let start = index.satu…                   │
│ +  rv-core/src/an…   │    48 +    let end = (index + 5)…                    │
│ ~  rv/src/ui.rs      │        ╭─ 8d985355 · open ────────────────╮          │
│                      │        │ lines.len() - 1 underflows       │          │
│                      │        │ reply: fixed with unwrap_or(0)   │          │
│                      │        ╰──────────────────────────────────╯          │
├──────────────────────┴──────────────────────────────────────────────────────┤
│ ↓↑ move  ←→ focus  [/] file  c comment  g goto  v view  ? help  q quit │
└─────────────────────────────────────────────────────────────────────────────┘
```

The sidebar marks each file by how it changed: `+` added, `-` removed, `~`
modified, `->` renamed. `Tab` switches it from that file list to a list of every
comment in the review, and `Enter` on one of those rows opens the code it is
about. The diff pane's title says where its lines came from — `difftastic
(Rust)`, `fallback`, or `binary` — so a degraded diff is never mistaken for a
structural one. Diffs are computed lazily, for the selected file only, and
cached, so stepping back to a file does not re-run difftastic.

The pane the next keystroke lands in is marked with a `▸` on its title and a
bold border, never with colour — the chrome's colours already mean something
else: a blue border is a *comment*, a green background an *addition*, a red one
a *removal*.

**The code's own colours are your terminal's, not `rv`'s.** Syntax highlighting
emits only the 16 indexed ANSI colours, which every scheme redefines for itself,
so `rv` shows your Solarized or your Gruvbox rather than a palette it picked —
which is why there is no theme setting and no need for one. The chrome and the
code never collide because they use different channels: a diff's green and red
are *backgrounds*, and a syntax colour is always a *foreground*.

### Keybindings

**Browsing**

Movement is the **arrows** (and the mouse) — there are no `hjkl` aliases. The
rest of the keymap is layered under leaders, each opening a small menu of what
the next key does. `Space` is the **contextual** menu — it shows whichever
actions suit the mode you are in; `m` jumps to a **mode**; `g` **goes**
somewhere; `c` acts on the **comment** under the cursor; and `v` is the full,
stable list of **view** toggles. When only one thing makes sense — `c` on a line
with no comment yet — rv skips the menu and does it, naming the choice in the
status bar. `?` shows the leaders; `?` again unrolls the whole map.

| Key | Action |
| --- | --- |
| `↓` | Next row, file or comment — whichever the focused pane is listing |
| `↑` | The previous one |
| `←` | Out / up a level: the diff hands the focus back to the sidebar, a comment stack back to the diff, and in the file or commits list it climbs one level of the tree you drilled into |
| `→` | Into / open: in the file or commits list it drills into the directory or change under the cursor, opens the file under it (moving the focus to the diff), and in the Comments tab jumps to the comment's code |
| `PgDn` | Move the cursor a screenful forward in the focused pane |
| `PgUp` | A screenful back |
| `Home` | Jump the cursor to the first row of the focused pane |
| `End` | Jump it to the last row |
| `Shift`+`←` | Scroll the focused pane's text sideways toward the start of the line |
| `Shift`+`→` | The other way, to read the tail of lines wider than the pane; a trackpad's sideways flick does the same |
| `]` | Next file, from whichever pane the cursor is in |
| `[` | Previous file, likewise |
| `Enter` | To the diff for the highlighted item: opens the file under the sidebar cursor (moving the focus to the diff), steps into the selected diff line's comment stack, or from the Comments tab jumps to the comment's code. It no longer fires on a directory or change row — `→` drills into those |
| `Tab` | To the next mode, looping: Files → Commits → Comments → Diff. A file row under a change shows *that change's* diff of it |
| `s` | Fold a comment box away, or a directory in the file list — again to unfold |
| `f` | On the diff: toggle full-file context (also `v` `f` from anywhere) |
| `i` | In the **Commits** list: put the change details away, or bring them back (also `v` `i` from anywhere) |
| `E` | Open the selected file at the cursor's line in `$EDITOR`, and come back to the review when it exits. Unset `$EDITOR` is reported in the status line rather than guessed at |
| `+` | Widen the sidebar |
| `_` | Narrow it |
| `Esc` | Leave the comment stack, or back out of a zoomed directory |
| `?` | What the keys do **here**: a contextual tip in the corner above the bar. `?` again unrolls the whole keymap; `Esc` or `q` closes either |
| `q` | Quit |
| `Ctrl+C` | Quit from anywhere, including out of a half-typed comment |
| `Space` `t` | (Files/Commits) Switch the list between a flat list and a tree |
| `Space` `o` | (Files/Commits) Cycle the list's order: by path, additions, deletions |
| `Space` `#` | (Files/Commits) Show or hide the `+n -n` counts |
| `Space` `c` | (Files/Commits) Tint the names by their change |
| `Space` `g` | (Diff) Group the diff by side instead of interleaving |
| `Space` `b` | (Diff) Cycle before ｜ diffed ｜ after |
| `Space` `f` | (Diff) Toggle full-file context |
| `Space` `d` | (on a comment) Delete it, after a `y`/`n` confirmation |
| `Space` `r` | (on a comment) Resolve it — again to reopen |
| `Space` `a` | (on a comment) Abandon it — again to reopen |
| `m` `f` | Go to the **Files** mode: the sidebar's file list |
| `m` `c` | Go to the **Commits** mode: the list of changes |
| `m` `o` | Go to the **Comments** mode: the review's comment browser |
| `m` `d` | Go to the **Diff** mode: the focus lands on the diff |
| `g` `↓` | Next hunk: the first line of the next run of changes, skipping the context between them |
| `g` `↑` | The previous hunk |
| `g` `n` | Next symbol in scope — every changed file, or one change's files from the Commits tab |
| `g` `N` | The previous symbol |
| `g` `/` | Find a symbol by name: type, `Enter` jumps to the best match, `Esc` cancels |
| `c` `c` | Comment on the highlighted line |
| `c` `d` | Delete a comment, after a `y`/`n` confirmation |
| `c` `r` | Resolve a comment — press it again to reopen |
| `c` `a` | Abandon a comment, dropping it without fixing it — again to reopen |
| `v` `f` | Toggle full-file context. Default on: the pane shows the whole file with the changed lines highlighted. Off restores the difftastic-only view of just the changes |
| `v` `g` | Group the diff: each hunk's removed lines before its added ones, the way a unified diff prints — instead of difftastic's interleaving of the two sides |
| `v` `b` | Cycle the diff between showing both sides, the **before** side alone (context and removals), and the **after** side alone (context and additions) |
| `v` `t` | Switch the file list between a flat list and a tree |
| `v` `o` | Cycle the file list's order: by path, by additions, by deletions |
| `v` `c` | Tint the sidebar names by their change — each name runs green through a light seam to red, split where its additions end — or turn the tint off |
| `v` `#` | Show or hide the sidebar's `+n -n` counts |
| `v` `z` | Hide the sidebar, or bring it back — the `‹` in the bottom-left corner does the same by pointer |
| `v` `<` | Narrow the sidebar |
| `v` `>` | Widen it |
| `v` `i` | Put the change details away, or bring them back. They show whenever a change is highlighted in the **Commits** tab: both ids, the whole description, and every file it touched |
| `v` `r` | Refresh: re-resolve the range against the repository as it now stands — after a push, a rebase, or a bookmark move |

In the diff, `↓` and `↑` move by **row** rather than by diff line, so a comment
box is something the cursor walks through rather than over — see [Inline
comments](#inline-comments). `v <`, `v >` and the fold state are preferences of
the running session: nothing about how you arranged your screen is written to
`.review/`.

`J` and `K` walk *hunks* — a run of added or removed lines with unchanged
context either side — so a long file with three edits in it is three presses
rather than a scroll. Neither wraps: at the last hunk `J` says so in the status
line and leaves the cursor where it is, because a jump back to the top would
look exactly like a jump that did nothing.

Which comment `d` and `s` act on follows the cursor: the box you are on inside a
stack, the comment the browser is showing on the **Comments** tab, and otherwise
the selected diff line's — the newest of them for `d`, all of them together for
`s`. The **Files** tab selects a file rather than a comment, so `d` there
deletes nothing and says so, while `s` still folds the line the diff is on.

**Typing a comment**

| Key | Action |
| --- | --- |
| any character | Append to the comment |
| `Backspace` | Delete the last character |
| `Enter` | Save; the status line reports `path:line` |
| `Esc` | Discard |

Comments are single-line for now. Saving one writes it through to
`comments.json` immediately, so a comment survives the process being killed
the instant after `Enter`.

A comment anchors to the side of the diff its line belongs to: a removed line
anchors to the base revision, added and context lines to the head. The line
number shown in the pane is always the one the anchor stores. If the highlighted
line has no number on that side, `rv` refuses to save rather than anchoring it
somewhere approximate, and says so in the status line.

### Inline comments

A saved comment is drawn **beneath the line it is anchored to**, in a blue
bordered box indented to the diff's gutter, titled with the comment's id and its
state. Several comments on one line stack under it, oldest first. The boxes are
part of the diff pane: `↓` and `↑` move the cursor by **row**, so they walk
*through* a box rather than over it, and the pane scrolls with them. A comment
longer than the pane is tall is therefore read the way any other content is, by
scrolling. While the cursor is inside a box, the line the box belongs to stays
the selected one — so `c` there comments on the code the box is about.

`Enter` steps the cursor *into* the stack under the selected line, where `j` and
`k` move between the boxes rather than between the lines; `Esc` or `←` steps
back out. The selected box is drawn brighter and bold, so `d` and `s` visibly
have a target.

A reply — stored by a coding agent with `rv reply` — renders **inside the same
box**, beneath the comment body, prefixed `reply:` and dimmed. It is part of the same conversation, and
dimming it is what tells your own words from the answer to them at a glance.

`s` folds a box down to a single row and unfolds it again. **Folding is a view
preference of this session only.** It is never written to `.review/`, so nothing
another reviewer — or an LLM reading the export — sees depends on how you
arranged your screen; reopening `rv` gives you every box back. A comment that is
no longer open starts folded, and greyed, since it is not one asking for an
answer.

**Deleting a comment is permanent.** `d` asks first — the status line reads
`delete comment at <path>:<line>? (y/n)` — and only a lowercase `y` goes
through; every other key keeps the comment. What `y` removes is the comment's
entry in `comments.json`, with nothing that undoes it.

`REVIEW-FEEDBACK.md` is a **view**, produced only on request — the TUI's `e`
or `rv render --out` — and read back by nothing. Saving, settling and replying
never touch it; `comments.json` is what says which comments exist right now.

## The `.review/` directory

```
.review/
├── session.toml          the range under review and every change in it
├── comments.json         the authority on which comments exist — each entry
│                         carries the lines around its comment, as they were
└── REVIEW-FEEDBACK.md    a rendered view of the above, only ever written on request
```

`REVIEW-FEEDBACK.md` is a *view*: rebuilt from `comments.json` when you ask for
it, never read back. `comments.json` is the file that decides what exists.

**`.review/` is kept out of version control automatically.** On its first run in
a repository, `rv` appends `/.review/` to `.git/info/exclude` — never to
`.gitignore`, which is shared and would affect every clone. This is correctness,
not tidiness: an untracked-but-visible `.review/` would be snapshotted by jj into
the very change you are reviewing, on every single jj command. After running
`rv`, both `git status` and `jj status` should stay silent about it. Every write
under `.review/` is atomic (write to a temp file in the same directory, fsync,
rename), so a reader never sees half a file. One side effect of that scheme: the
files `rv` writes end up mode `0600`, including `.git/info/exclude`, which it
rewrites rather than owns. More restrictive than the `0644` git leaves, never
less, but worth knowing if something else on your machine reads that file.

Deleting `.review/` throws away your comments and nothing else; the next `rv`
run recreates it.

## The agent loop

This is what the tool is for, and it is CLI end to end.

The worker's loop:

```sh
rv status --check                    # is there work? (exit 1 = open comments)
rv comments --json --state open      # what exactly?
# …fix the code…
rv reply 6ce52206 -m "Widened to 8 hex; prop_store pins the width."
rv resolve 6ce52206
rv status --json                     # open down, resolved up
```

The reviewer's loop:

```sh
rv status --json                     # scope: changes, files
rv diff --json                       # what changed, in rv's own coordinates
# …read the head files as needed…
rv comment rv-core/src/store.rs --line 238 -m - <<'EOF'
`content_hash` is computed from the untrimmed line, so re-indenting breaks
every anchor — hash the trimmed text.
EOF
```

Every command either succeeds or exits 1 with a reason on stderr; nothing in
the loop parses a document. The `-m -` form reads the body from stdin — the
`git commit -F -` convention — so a finding full of backticks and quotes never
meets the shell. Two skills in this repository teach the loop to agents:
`rv-reviewer` and `rv-worker` under `.claude/skills/`.

`rv render` still produces the markdown for human reading, and a reply an agent
wrote into a pre-CLI export is rescued into the store once, the first time this
version loads the review.

## `RV_ASCII`

The status bar's powerline arrows and the sidebar's folder and file icons are
nerd-font glyphs. They need a patched font, rv cannot detect one, and a font
without the patch shows tofu — so one switch turns both off:

```sh
RV_ASCII=1 rv
```

As with `RV_NO_DIFFT`, *presence* is the switch: `RV_ASCII=0` turns the glyphs
off like any other value.

## `RV_NO_DIFFT`

Set `RV_NO_DIFFT=1` to skip difftastic entirely and use the built-in line diff:

```sh
RV_NO_DIFFT=1 rv
```

Use it when difftastic is missing, when it is slow on a pathological file, or
when you want to check whether a confusing diff is difftastic's alignment or
your code. The diff pane title changes from `difftastic (Rust)` to `fallback` so
the two are never confused. `rv` also falls back on its own, silently and with
the same label, if `difft` is absent or returns something unexpected.

## Current limits

This is 1.0.0 — the first release. `rv` reviews its own development stack, and
has been doing so for the whole of its own history. Known and deliberate gaps:

- **Line-scoped comments only.** One line, one comment, one line of text. No
  multi-line selections, no multi-line comment bodies, no symbol- or
  block-scoped comments.
- **No verification flow.** Comments resolve (`r`, `rv resolve`), abandon
  (`a`, `rv abandon`) and derive `outdated` when their code moves on — but a
  reply does not move a comment to `awaiting-verification`, and nothing
  automated verifies a fix. The human's pass in the TUI is the verification.
- **No node-scoped anchors.** Symbols index for `n`/`N` and the `/` picker, but
  a comment still anchors to a line, not a syntax node.
- **No re-anchoring command.** Anchors carry a content hash and can survive an
  edit, but there is no `rv reanchor` to sweep them after you rewrite history.
- **Must be run from the workspace root.** `rv` does not walk up the directory
  tree looking for `.jj/`. From a subdirectory it fails with
  `no jj workspace at <dir>`. Pass `--repo <root>` if you need to run it from
  elsewhere.
- **`rv` reviews the last snapshot of `@`, not your files on disk.** Because it
  is strictly read-only it never snapshots the working copy, so edits you made
  since the last jj command are invisible to it — a file you just created will
  not appear in the review at all. Run any jj command (`jj status` will do) to
  snapshot, then run `rv`.
- **A workspace with no top-level `.git/` is not protected.** The exclude
  mechanism is `.git/info/exclude` and nothing else, and `rv` creates that path
  unconditionally rather than checking whether a git repository is really there.
  So in any workspace root without a sibling `.git/`, `rv` invents one
  containing a single `info/exclude` file that jj never reads, reports success,
  and leaves `.review/` visible — where jj then snapshots it straight into the
  change you are reviewing. This is not only the `git.colocate = false` case: it
  covers repositories where colocation was turned off with
  `jj git colocation disable`, and **every secondary workspace created by
  `jj workspace add`**, which has its own `.jj/` and no `.git/` of its own.
  `rv` does not currently detect or warn about any of them. After your first
  `rv` run in a new workspace, check that `jj status` does not list `.review/`.
- **Not a TTY, not a review.** A bare `rv` needs a terminal; piped or redirected
  it prints `could not start the terminal` and exits 1. Use `rv render` or
  `rv status` in scripts.

## Layout

| Crate | Contents |
| --- | --- |
| `rv-core` | Everything terminal-free: jj-lib access, diff production, anchoring, `.review/` I/O, markdown render and reply parsing |
| `rv` | The CLI and the ratatui TUI |

`rv-core` must never depend on `ratatui`, `crossterm` or `tui-textarea`, and
`jj-lib` is imported in exactly one file, `rv-core/src/vcs.rs`. The first rule is
what makes anchoring testable without a terminal; the second bounds the blast
radius of a jj-lib upgrade to one module.

## Development

```sh
nix develop          # cargo, jj and difftastic at the versions CI uses

cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
```

The suite spawns a real jj workspace per fixture and shells out to `difft`, so
both must be on `PATH` — `nix develop` is the shortest way to have exactly the
versions CI runs. CI runs those three commands inside that same shell, so a
green run locally means a green run there.

## Licence

MIT or Apache-2.0, at your option. See [LICENSE-MIT](LICENSE-MIT) and
[LICENSE-APACHE](LICENSE-APACHE).
