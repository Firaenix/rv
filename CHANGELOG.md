# Changelog

## 1.0.0 - 2026-08-19

The first release of `rv`: a jj-native terminal branch reviewer, and the CLI
an agent loop runs on.

### The review

- Review any revset range (`trunk()..@` by default) straight off your local
  jj repository: no forge, no network, no accounts, no model calls.
- Structural diffs via difftastic, with an in-process fallback and an
  async fast-then-refine pipeline so a keystroke never waits on a spawn.
- Tree-sitter syntax highlighting in the terminal's own colours, parsed off
  the drawing thread.
- Line-anchored comments that survive edits and history rewrites: content-hash
  anchors with a moved/weak/outdated resolution cascade, rename-following,
  and `outdated` derived on every load — never stored.
- Comment lifecycle with an audit trail: open, resolved, abandoned — each
  recording *who* settled it.

### The TUI

- Two panes over a powerline bar that names the context you are in
  (`FILES` / `COMMITS` / `DIFF` / `STACK`), your `file:line`, the enclosing
  symbol, the revset, and the open-comment count.
- A contextual `?` tip in the corner — what the keys do *here* — with the
  whole keymap one more press away.
- File list as a flat list or a tree with single-child chains merged, zoom
  into any directory or change (`Enter`, `Shift+→`), fold with `Space`,
  reorder with `o`, tint names by their change's add/remove gradient (`g`),
  toggle counts (`#`), and nerd-font icons behind the `RV_ASCII` switch.
- A commits tab: every change in the stack with its own per-change diffs,
  `i` for full change details.
- Symbol navigation: `n`/`N` stepping and a `/` picker that matches every
  query word against name then file.
- Horizontal scrolling (`H`/`L`, `Shift+←/→`, sideways wheel), mouse support
  throughout, and colours resolved from the terminal's own theme.

### The agent loop

- `rv comments --json` — the read channel: bodies, anchors, excerpts,
  confidence and resolved line per comment.
- `rv reply <id> -m` — the answer channel; `-m -` reads bodies from stdin.
- `rv comment`, `rv resolve`, `rv abandon` — anchored writes with validation
  a program can act on.
- `rv diff --json` — the range's changes in rv's own side-aware coordinates,
  so agents comment with numbers rv itself issued.
- `rv status --check` — the worker's poll and a CI gate in one exit code.
- `rv render` — the review as markdown, a view nothing reads back.
- Reviewer and worker skills under `.claude/skills/` teach the loop to
  coding agents.
