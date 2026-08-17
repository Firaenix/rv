# rv — Milestone 1 Handoff

You are starting work on **rv**, a jj-native terminal branch reviewer.

## Read these two files first, in order

1. `docs/superpowers/specs/2026-08-17-rv-branch-reviewer-design.md` — design and constraints
2. `docs/superpowers/plans/2026-08-17-rv-milestone-1.md` — implementation plan (10 tasks)

## Execute with

Use superpowers:subagent-driven-development or superpowers:executing-plans. The plan's steps
use `- [ ]` checkboxes; tick each as completed.

## Environment facts

- Repo is jj-colocated (jj 0.44, git backend, branch `main`, no remotes).
- Commit with `jj describe -m "..." && jj new`. NEVER `git commit`.
- `jj-lib` 0.44 is the only VCS dependency; it is async — use `pollster` + `futures`.
- difft 0.70 is installed; JSON output needs `DFT_UNSTABLE=yes`.
- The plan is self-contained; the user's jj config is never read.
- Start at Task 1 and proceed in order. Report after each task.