# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0](https://github.com/Firaenix/rv/releases/tag/v0.1.0) - 2026-08-19

### Added

- *(rv)* the agent loop is the CLI, and the markdown is a view
- *(rv)* R refreshes the review, and a nix flake ships it
- *(rv)* resolve and abandon a comment, and say who did
- *(rv)* a sidebar you can read at a glance, and a bar that keeps its hint
- *(rv)* the terminal's own colours, readable long comments, and nine grammars
- *(rv)* layout, highlighting, the sidebar tree model and the palette
- *(rv)* fold browsed comments, dim replies, and pin the docs to the code
- *(rv-core)* remove a comment and its snapshot
- *(rv)* ratatui review TUI with line comments
- *(rv-core)* REVIEW-FEEDBACK.md render and reply parsing
- *(rv-core)* write-through .review/ store with exclude management
- *(rv-core)* content-hash anchors with resolution cascade
- *(rv-core)* difft JSON diff production with similar fallback
- *(rv-core)* file enumeration, rename detection, and blob reads
- *(rv-core)* jj-lib repository handle and stack enumeration

### Fixed

- *(rv-core)* the snapshots folder is gone, because nothing ever read it
- *(rv)* say when trunk() was not a trunk
- *(rv-core)* the excerpt says which of its lines the comment is about
- *(rv)* outdated comments were reported as open, and the export forbade what rv does
- *(rv)* complete the chunk ordering and stop the pane hiding lines you can comment on
- *(rv-core)* bound fence scanning at body markers and document the column-0 rule
- *(rv-core)* harden reply parsing against marker and body mangling
- *(rv-core)* atomic temp-file-plus-rename writes in the review store
- *(rv-core)* fail anchors safe on blank-line collisions and out-of-range creation
- *(rv-core)* label whole-file creations and deletions as difftastic output

### Other

- *(release)* licences, CI, cross-platform release automation
- code documents itself, and a rule saying so
- every file in the rv crate is under the rule now
- *(rv)* colour arrives after the code, not before it
- property suites, and the three source defects they found
- *(rv-core)* pin that comments upsert by id, not by change_id
- *(rv-core)* close the jj_lib alias bypass and prove the vacuity guard fails
- *(rv-core)* enforce the architectural constraints mechanically
