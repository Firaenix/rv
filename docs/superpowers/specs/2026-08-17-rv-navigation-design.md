# rv — view switching and symbol navigation

**Status:** design approved in conversation, awaiting spec review
**Date:** 2026-08-17
**Builds on:** `2026-08-17-rv-branch-reviewer-design.md` (§6 views, §8 node model),
`2026-08-17-rv-inline-comments-design.md` (focus model, keymap, collapse verb)

## 1. Purpose

Reviewing a stack means two different questions — "what does this branch do?" and
"what did this commit do?" — and answering either one means moving through code by
its structure rather than by scrolling. This design makes both cheap: a toggle
between the whole-bookmark view and the per-commit view, and symbol-level
navigation across whatever is in scope.

The governing value is stated plainly because it drives several rulings below:
**this must be ergonomic for someone who has never used rv before.** Where a
choice trades power for immediacy, immediacy wins.

## 2. Requirements

From the user, verbatim in intent:

1. Jumping around code is easy and ergonomic for anyone.
2. Switch seamlessly between viewing files and viewing commits.
3. Symbol search.
4. Jump between symbols across all currently diffed files.
5. Scope follows the view: on a commit, symbol jumps stay inside that commit; on
   the whole bookmark (the default view), jumps cross every file in the range.

## 3. Two views over one range

The original design already named these; this makes them concrete.

| Key | View | Sidebar | A file's diff is… |
|---|---|---|---|
| `1` | **Bookmark** (default) | flat list of every changed file in the range | range base → range head |
| `2` | **Commits** | two-level tree: change → the files it touched | that change's parent → that change |

Both views describe the same range; neither is derived from the other. A comment
records which view and which change it came from, so the two never have to be
reconciled — that was settled in the original design and nothing here changes it.

**Ruling — the commits view sidebar is a two-level tree, not a separate
change-picker screen.** A reviewer moving between "this commit's version of
`store.rs`" and "the next commit's version" should not leave the diff. Change
nodes collapse and expand with `s`, the same verb that collapses a comment box:
one key that means *collapse the thing under the cursor*, wherever the cursor is.

**Ruling — switching views preserves position where it can.** Toggling from
bookmark to commits view with `store.rs` selected lands on `store.rs` inside the
newest change that touched it; toggling back selects `store.rs` in the flat list.
Landing at the top of an unrelated list is the single most disorienting thing a
view toggle can do.

Per-change diffs need no new rv-core surface: `Repository::files(base, head)` and
`read_blob(commit, path)` already take arbitrary commit ids, so a change's diff is
its parent's tree against its own.

## 4. Symbols

### What is indexed

A **symbol** is a named definition with a location:

```rust
struct Symbol {
    name: String,
    kind: SymbolKind,   // Function, Struct, Enum, Trait, Impl, Module, Constant, Type, Macro
    file: String,
    line: u32,          // 1-based, in the side being viewed
    change_id: Option<String>,  // set in the commits view, None in the bookmark view
}
```

Extraction is tree-sitter plus each grammar's bundled `tags.scm`, via
`tree-sitter-tags` — the crates and their version pinning were verified in the
original design (§4): `tree-sitter` 0.26, `tree-sitter-tags` 0.26, grammars
depending on `tree-sitter-language` ^0.1, so there is no ABI skew.

**Ruling — this design indexes symbols for *navigation* only. It does not
implement the `NodePath` anchoring model.** Node-scoped comments and node-tier
anchors remain a separate concern with their own risks (path totality, round-trip
through markdown). Conflating them would put a comment-durability change inside a
navigation feature. `SymbolKind` and the extraction pipeline are chosen so the
anchoring work can build on them later without rework.

**Ruling — symbols come from the side you are looking at:** the head-side blob for
added, modified, and renamed files; the base-side blob for removed files. You
navigate code as it will exist, except where it will not exist at all.

**Ruling — only files in the diff are indexed, never the whole repository.** The
requirement is jumping between symbols in the *diffed* files, and a whole-repo
index would be a different feature with a different cost.

### Scope

Scope is exactly the current view, which is what makes the rule learnable:

- **Bookmark view** → every changed file in the range.
- **Commits view** → only the files of the selected change.

The status bar always names the scope in words (`scope: all 29 files` /
`scope: change ytskpxpw · 4 files`), because a jump that silently searches a
different set than the reviewer assumes is worse than no jump.

### Navigating

Two mechanisms, deliberately: search when you know what you want, stepping when
you are reading.

- **`/` — fuzzy symbol search.** Opens a picker over the in-scope symbols,
  filtered as you type with `nucleo`. Each row shows kind, name, and file. `Enter`
  jumps: select the file (loading its diff, switching change if needed) and put
  the cursor on the symbol's line. `Esc` cancels and restores the previous
  position. Matching is on `name` primarily and `file` secondarily, so typing
  `store write` finds `write_markdown` in `store.rs`.
- **`n` / `N` — next / previous symbol in scope.** Ordered by file, then by line,
  and **crossing file boundaries**, loading the next file's diff as needed. This
  is the "jump between symbols in all the current diffed files" requirement: `n`
  at the last symbol of a file moves to the first symbol of the next file rather
  than stopping.

`gd`-style definition jumps and reference search are explicitly not here — see
non-goals.

### Degradation

A file with no available grammar yields no symbols. It is skipped by `n`/`N`,
absent from the picker, and **labelled** in the sidebar and status bar as having
no symbol index. The original design's rule holds: the tool never presents a
guess as a fact, and silently behaving as though a file has no symbols when the
truth is "rv cannot parse this" is exactly such a guess.

**Grammar order:** Rust first, because dogfooding needs it first. Then
TypeScript/TSX, Python, Go, Markdown, TOML, YAML, Bash, SQL, Nix. Each is an
additive dependency plus one registry line.

### Performance

Parse lazily per file on first need, cache by `(commit_id, path)` — the same
laziness the diff pipeline already uses, for the same reason. The picker needs the
whole in-scope index, so entering `/` parses every in-scope file that is not yet
cached; at review sizes that is milliseconds, and the alternative (indexing up
front on launch) would delay startup for a feature the reviewer may not use.

## 5. Ergonomics

The keymap has outgrown what a new user can discover by poking at it, so:

- **`?` opens a help overlay** listing every binding, grouped by what it acts on,
  including the view toggle and the symbol keys. This was already an item in the
  original design's keymap and is now load-bearing.
- **The status bar always answers "where am I?"**: view, scope, file position
  (`file 3/29`), line position, and the enclosing symbol when one is known
  (`in fn write_markdown`). A reviewer should never have to reconstruct their
  position from the diff text.
- **Every navigation action reports what it did** in the status line — which
  symbol, in which file, in which change. Jumps that move the cursor invisibly are
  how a reviewer loses their place.
- **`Esc` always means "back out one step"** — out of the picker, out of the
  comment stack, out of a confirmation — and never destroys work.

## 6. Testing

- **Symbol extraction:** for each grammar, a fixture file yields the expected
  names, kinds, and lines, including nested items (a method inside an `impl`) and
  duplicate names in different scopes. A file the grammar cannot parse yields an
  empty index and the labelled-degraded state, not an error.
- **Scope:** in the commits view, the in-scope symbol set equals exactly the
  selected change's files' symbols; in the bookmark view, exactly the range's.
  Switching views recomputes it. This is the requirement's core assertion.
- **`n`/`N` crossing files:** `n` at a file's last symbol lands on the next file's
  first symbol and loads that diff; `N` at the first symbol of the first file
  stays put rather than wrapping (ruling: no wraparound — silent wrapping makes a
  reviewer think they have seen everything when they have looped).
- **Picker:** filtering narrows as typed; `Enter` lands on the right file and
  line; `Esc` restores the exact prior position, including view and change.
- **View toggle:** position preservation both directions, including a file that
  several changes touched, and a file that only one change touched.
- **Property tests:** the in-scope symbol set is exactly the union of the
  per-file sets (no symbol invented, none lost); `n` applied `len()` times from
  the first symbol visits every symbol exactly once; parsing never panics on
  arbitrary bytes, including invalid UTF-8 and truncated source.
- **Rendering** via `TestBackend`: the picker overlays without corrupting the
  panes; the help overlay lists every binding the app actually handles — a test
  that reads the key table from one place so help and behaviour cannot drift.

## 7. Non-goals

- **Reference search, `gd`/`gr`, and the symbol timeline.** These are the original
  design's milestone 4, they need a repository walk and reference
  classification, and none of them is required to jump between symbols.
- **`NodePath` anchoring and node-scoped comments** — see the ruling in §4.
- **Whole-repository symbol index or project-wide search.**
- **Line-to-change attribution between the two views.** The original design ruled
  each view an independent diff; nothing here needs that to change.
- **Mouse support.**

## 8. Risks

| Risk | Mitigation |
|---|---|
| tree-sitter grammar breadth becomes a maintenance tax | Degradation is specified and labelled; Rust ships first and each further grammar is additive |
| The two views drift into two code paths that disagree | Both build on the same `Repository::files`/`read_blob` calls with different commit pairs; the view is a scope parameter, not a second implementation |
| A jump silently changes scope and the reviewer searches the wrong set | Scope is named in words in the status bar at all times, and every jump reports where it landed |
| The keymap becomes unlearnable | `?` overlay generated from the same table the app dispatches on, so it cannot go stale |
| Parsing cost on entering the picker | Lazy per-file parse with a `(commit, path)` cache; measured at review sizes before shipping |
| Symbol index and diff disagree about a line number | Symbols are extracted from the same blob the diff was computed from, keyed by the same `(commit, path)` pair |

---

## Implementation status (audited 2026-08-19)

- **§3 view switching preserves position** — shipped 2026-08-19: `Tab` (and
  `1`/`2`/`3`, which jump to a tab directly) lands the cursor on the selected
  file's row in the list that appears — in the commits tab, under the newest
  change that touched it.
- **§3 `1` = Bookmark / `2` = Commits** — the views are sidebar tabs
  (viewport §7); `1`/`2`/`3` shipped as direct jumps to them.
- **§4 the `/` picker** — shipped without `nucleo`: rv deliberately carries
  no fuzzy-matching dependency, and the matcher is its own — every word of
  the query must match the name first or the file second, so `store write`
  finds `write_markdown` in `store.rs`; rows show the symbol's kind in its
  language's own keyword. Sub-word fuzziness (`stwr`) is **not** provided.
- **§4 grammar order** — symbols ship for Rust, Go, Python,
  JavaScript/TypeScript/TSX; Markdown, TOML, YAML and Bash are highlight-only
  and SQL and Nix carry no grammar yet: **open** as grammars get added.
- **§4 "labelled in the sidebar and status bar as having no symbol index"**
  — the label lives on the diff pane's title (`— no highlighting`), and the
  status line says "no symbols in this scope" when a symbol key is pressed;
  the sidebar carries no per-file mark, by choice — a column of "no index"
  marks on a mostly-unindexed review is noise.
- **§5 the status bar** — the scope segment is the *revset* (viewport §9),
  with the cursor's change as its own segment; the file:line position and the
  enclosing symbol (`in fn write_markdown`, whenever the index is already
  warm — the bar never builds one) shipped 2026-08-19. A jump's status names
  `symbol — path:line`; the change is on the bar already and is not repeated
  in the message.
