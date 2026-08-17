# rv

A terminal code reviewer for [Jujutsu](https://jj-vcs.github.io/jj/) stacks.

`rv` reads a range of jj changes out of your local repository, shows you the
diff, and lets you attach comments to individual lines. Those comments are
written to `.review/REVIEW-FEEDBACK.md`, a plain markdown file designed to be
handed to a coding agent: the agent fixes the code, appends a `**Reply:**` under
each comment, and `rv` folds the replies back in without losing them.

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
- Rust 1.89 or newer, edition 2024 (both forced by `jj-lib` 0.44).
- [difftastic](https://difftastic.wilfred.me.uk/) (`difft`) on `PATH`, optional.
  With it you get structural, language-aware diffs. Without it `rv` silently
  falls back to a line diff and labels the pane `fallback` so you always know
  which one you are reading.

## Build

```sh
cargo build --release        # ./target/release/rv
cargo install --path rv      # or put it on your PATH
```

## Usage

Run `rv` from the **workspace root** — the directory holding `.jj/`. See
[Current limits](#current-limits).

| Command | What it reviews |
| --- | --- |
| `rv` | `trunk()..@` — everything on your stack that trunk does not have |
| `rv <bookmark>` | `trunk()..<bookmark>`. The target may be a bookmark, a change id or a commit id |
| `rv --from <rev> --to <rev>` | An explicit range, e.g. `rv --from main --to my-feature` |
| `rv render` | Writes `.review/REVIEW-FEEDBACK.md` for that range and exits — no terminal needed |
| `rv status` | Prints the range, its changes, its changed files and its comment counts |
| `rv status --json` | The same report as JSON, for scripting |
| `rv --repo <path> …` | Reviews the workspace at `<path>` instead of the current directory |

`--to` overrides the positional target when both are given. That is also the
escape hatch for the one name collision: a bookmark literally called `render` or
`status` has to be passed as `rv --to status`, since subcommands share the first
positional slot.

`rv`'s idea of `trunk()` is its own built-in one — the latest of
`main`/`master`/`trunk` on `origin`/`upstream`, falling back to the repository
root. It is not read from your config, so in a repository with no remotes the
default range is your entire history.

Every command exits 0 on success and 1 on failure, printing the whole error
chain as a sentence rather than a backtrace.

### Example

```
$ rv status
revset  trunk()..@
base    0000000000000000000000000000000000000000
head    5deca3b318df5aa050f08ae19f65e4805c3f9975

changes (16)
  uspywkpvwqtypzsqrwmptwpymlvvrtok 5deca3b318df5aa050f08ae19f65e4805c3f9975 (no description set)
  mspsnktlqkkmpyyqunnolnwqnnuvwkuv c960123dbb78d6cec5c61e0406fe66ce0301efad fix(rv): widen comment ids
  …

files (27)
  added     rv-core/src/anchor.rs
  modified  rv-core/src/lib.rs
  …

comments  1 open, 0 awaiting verification, 0 resolved, 0 outdated
```

Change ids are shown the way `jj log` shows them (the reverse-hex `z`–`k`
alphabet), so you can paste one straight back into a jj command.

## The reviewer

A bare `rv` opens the terminal UI: a status line over a file sidebar and a diff
pane.

```
┌────────────────────────────────────────────────────────────┐
│ j/k line  [/] file  c comment  q quit                      │
├──────────────────────┬─────────────────────────────────────┤
│ Files (27)           │ rv-core/src/anchor.rs — difftastic  │
│ +  Cargo.lock        │    47 +    let start = index.satu…  │
│ +  rv-core/src/an…   │    48 +    let end = (index + 5)…   │
└──────────────────────┴─────────────────────────────────────┘
```

The sidebar marks each file by how it changed: `+` added, `-` removed, `~`
modified, `->` renamed. The diff pane's title says where its lines came from —
`difftastic (Rust)`, `fallback`, or `binary` — so a degraded diff is never
mistaken for a structural one. Diffs are computed lazily, for the selected file
only, and cached, so stepping back to a file does not re-run difftastic.

### Keybindings

**Browsing**

| Key | Action |
| --- | --- |
| `j` / `↓` | Next diff line |
| `k` / `↑` | Previous diff line |
| `]` | Next file |
| `[` | Previous file |
| `c` | Comment on the highlighted line |
| `q` | Quit |

**Typing a comment**

| Key | Action |
| --- | --- |
| any character | Append to the comment |
| `Backspace` | Delete the last character |
| `Enter` | Save; the status line reports `path:line` |
| `Esc` | Discard |

Comments are single-line in this milestone. Saving one writes it through to disk
immediately — `comments.json`, its snapshot, and a rewritten
`REVIEW-FEEDBACK.md` — so a comment survives the process being killed the
instant after `Enter`.

A comment anchors to the side of the diff its line belongs to: a removed line
anchors to the base revision, added and context lines to the head. The line
number shown in the pane is always the one the anchor stores. If the highlighted
line has no number on that side, `rv` refuses to save rather than anchoring it
somewhere approximate, and says so in the status line.

## The `.review/` directory

```
.review/
├── session.toml          the range under review and every change in it
├── comments.json         the authority on which comments exist
├── snapshots/<id>        the lines around each comment, as they were when made
└── REVIEW-FEEDBACK.md    the human- and LLM-readable projection of the above
```

`REVIEW-FEEDBACK.md` is a *projection*: it is rebuilt from `comments.json` on
every write. `comments.json` is the file that decides what exists.

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

## The LLM loop

This is what the tool is for.

1. You review in the TUI and leave comments. `rv` writes
   `.review/REVIEW-FEEDBACK.md`.
2. You point a coding agent at that file. The document opens with a `For LLMs:`
   block stating the protocol, so the file explains itself.
3. The agent fixes the code and appends a `**Reply:**` block under each comment
   it addressed.
4. The next time `rv` writes the document — `rv render`, or the next comment you
   save in the TUI — it reads the existing file first and folds those replies
   back into `comments.json`. Rewriting the document cannot destroy them.

An entry looks like this:

````markdown
## Open (1)

### 1. `rv-core/src/anchor.rs:48`
<!-- rv:anchor id=8d985355 change=uspywkpv… commit=5deca3b3… side=right line=48 hash=35f7a6da… -->

  ```rust
      let end = (index + 5).min(lines.len() - 1);
  ```

**Comment:** lines.len() - 1 underflows when lines is empty.

**Reply:** Fixed by using unwrap_or(0).
````

Two rules the parser depends on, both stated in the document itself:

- **Put the reply inside the entry it answers** — after that entry's
  `<!-- rv:anchor … -->` marker and before the next `###` or `##` heading. A
  `**Reply:**` that lands outside every entry (appended to the end of the file,
  for instance) has no comment to bind to and is dropped, and the next rewrite of
  the document removes it. Binding it to some other comment would put words in a
  comment nobody looked at, so `rv` drops it instead.
- **Keep the `**Reply:**` marker at the start of the line.** Not indented, not
  inside a list item. Indented text is quoted content, not structure.

Do not edit the `<!-- rv: -->` markers, the headings or the section order. Do not
move comments between sections — see the limits below.

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

This is milestone 1 — the point at which `rv` can review its own development
stack. Known and deliberate gaps:

- **Line-scoped comments only.** One line, one comment, one line of text. No
  multi-line selections, no multi-line comment bodies, no symbol- or
  block-scoped comments.
- **No verification or accept flow.** Every comment is `Open` and stays `Open`.
  Adding a reply does *not* move a comment to `awaiting-verification`, and there
  is no key to resolve, accept or reopen one. The `## Awaiting verification`,
  `## Resolved` and `## Outdated` sections render, but nothing in milestone 1
  puts anything in them. Editing `comments.json` by hand is the only way to
  change a state today.
- **No tree-sitter.** No symbol outline, no node-scoped anchors, no changed
  symbol summaries, no fuzzy symbol picker. The sidebar is a flat file list.
- **No re-anchoring command.** Anchors carry a content hash and can survive an
  edit, but there is no `rv reanchor` to sweep them after you rewrite history.
- **Must be run from the workspace root.** `rv` does not walk up the directory
  tree looking for `.jj/`. From a subdirectory it fails with
  `no jj workspace at <dir>`. Pass `--repo <root>` if you need to run it from
  elsewhere.
- **Comments attribute to the first change in the range**, which for the default
  `trunk()..@` is usually the working copy. Per-change attribution comes later.
- **`rv` reviews the last snapshot of `@`, not your files on disk.** Because it
  is strictly read-only it never snapshots the working copy, so edits you made
  since the last jj command are invisible to it — a file you just created will
  not appear in the review at all. Run any jj command (`jj status` will do) to
  snapshot, then run `rv`.
- **A non-colocated repository is not protected.** The exclude mechanism is
  `.git/info/exclude` and nothing else. In a repository created with
  `git.colocate = false` there is no top-level `.git/`, so `rv` creates one
  containing just `info/exclude` — a file jj never reads — and reports success.
  `.review/` is then visible to jj and gets snapshotted into the change under
  review. Colocation is jj's default, so this is an edge case, but `rv` does not
  currently detect or warn about it. Check `jj status` after your first `rv` run
  in a new repository.
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
cargo test --workspace
cargo clippy --workspace --all-targets
cargo fmt --all --check
```
