# rv

A jj-native terminal branch reviewer. Two crates: `rv-core` is terminal-free
(vcs, diff, anchor, store, markdown, highlight, symbols); `rv` is the CLI and
TUI over it.

Designs live in `docs/superpowers/specs/`, implementation plans in
`docs/superpowers/plans/`, and the running account of what was built and what
reviewing it found is `docs/rv-implementation-history.md`.

## Rules

- Source files must be under 400 lines — split them.
- Doc comments explain non-obvious constraints only; rationale belongs in the spec, not stamped into every file.
- Code documents itself: functions and variables are named so they read like prose. A comment is for a reason that is not clear from the code — and where the reason is unclear, first ask whether the code is necessary at all, or whether it could be refactored into something clearer to follow. Do not comment every module, struct, field and variable.
