# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0](https://github.com/Firaenix/rv/releases/tag/v0.1.0) - 2026-08-19

### Added

- *(rv)* nerd-font folder and file icons, behind the glyph switch
- *(rv)* the agent loop is the CLI, and the markdown is a view
- *(rv)* the interface learns where you are — context bar, ? tip, theme colours, text gradient, sideways scroll, tree zoom
- *(rv)* R refreshes the review, and a nix flake ships it
- *(rv)* comment, resolve and abandon from the command line
- *(rv)* the keystroke stops waiting for difftastic, and a change describes itself
- *(rv)* i shows a change in full, and the two ids are two colours
- *(rv)* the description under the list, and a tree with three tiers
- *(rv)* a file under a change shows that change's diff of it
- *(rv)* symbol jumping, short commit ids, and a comment list you can act on
- *(rv)* put the sidebar away, and fold a row with Enter or Space
- *(rv)* a commits tab, where a change is a directory of the files it touched
- *(rv)* export the review without leaving it, and a popup that still fits
- *(rv)* resolve and abandon a comment, and say who did
- *(rv)* the mouse, floating alerts, and an index of the symbols in scope
- *(rv)* a sidebar you can read at a glance, and a bar that keeps its hint
- *(rv)* the terminal's own colours, readable long comments, and nine grammars
- *(rv)* resize the panes, a ? keymap popup, and syntax colours in the diff
- *(rv)* layout, highlighting, the sidebar tree model and the palette
- *(rv)* fold browsed comments, dim replies, and pin the docs to the code
- *(rv)* show comments in the diff, stop clipping silently, browse comments
- *(rv)* comment stacks, deletion behind a confirmation, and folding
- *(rv)* load comments, focus panes, abort with ctrl-c, keep your place
- *(rv)* row model for diff lines and their comment boxes
- *(rv)* ratatui review TUI with line comments
- *(rv)* session assembly with render and status subcommands
- *(rv-core)* jj-lib repository handle and stack enumeration

### Fixed

- *(rv-core)* the snapshots folder is gone, because nothing ever read it
- *(rv)* what a fresh-eyes review of the newest commits caught
- *(rv)* the review loop, run on rv's own review — and what it found
- *(rv)* say when trunk() was not a trunk
- *(rv-core)* the excerpt says which of its lines the comment is about
- *(rv)* a query stops rewriting the review it is querying
- *(rv)* outdated comments were reported as open, and the export forbade what rv does
- *(rv)* complete the chunk ordering and stop the pane hiding lines you can comment on
- *(rv)* widen comment ids and display the anchored line number

### Other

- *(release)* licences, CI, cross-platform release automation
- *(rv)* session.rs back under the rule
- *(rv)* the files this session grew go back under the rule
- code documents itself, and a rule saying so
- *(rv)* one constructor taking an engine, and a flag a reviewer can reach
- every file in the rv crate is under the rule now
- *(rv)* colour arrives after the code, not before it
- *(rv)* the renderer is eleven files, one per thing it draws
- split the four biggest files into modules that fit in your head
- property suites, and the three source defects they found
- *(rv)* paste verification output and correct README scope
- *(rv)* README and milestone-1 dogfood verification
