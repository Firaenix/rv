# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [1.7.3](https://github.com/Firaenix/rv/compare/v1.7.2...v1.7.3) - 2026-09-09

### Fixed

- *(rv)* a jump holds the place it landed on, not the row

## [1.7.2](https://github.com/Firaenix/rv/compare/v1.7.1...v1.7.2) - 2026-09-09

### Fixed

- *(rv)* a view toggle re-clamps the cursor to the plan it just shortened

### Other

- *(rv)* the click's row is the row selected — a correctness property for the sidebar mouse

## [1.7.1](https://github.com/Firaenix/rv/compare/v1.7.0...v1.7.1) - 2026-09-08

### Fixed

- *(rv)* mouse clicks on commit-tab files opened the wrong diff

## [1.7.0](https://github.com/Firaenix/rv/compare/v1.6.1...v1.7.0) - 2026-09-08

### Added

- *(rv)* whole-file diff fallback when difftastic can't pair a reformatted region

## [1.6.1](https://github.com/Firaenix/rv/compare/v1.6.0...v1.6.1) - 2026-09-08

### Fixed

- *(rv)* full-file context for commit-view diffs, and a stuck-Pending merge

## [1.6.0](https://github.com/Firaenix/rv/compare/v1.5.0...v1.6.0) - 2026-09-02

### Added

- auto-refresh when the repository moves
- several reviews at once — the store keys by reviewed head

### Other

- rewrite the README for a first look, with a recorded demo

## [1.5.0](https://github.com/Firaenix/rv/compare/v1.4.0...v1.5.0) - 2026-09-02

### Added

- rv config and rv keymap each open their own file
- split, ascii and engine defaults in Config.toml
- the status bar says when the diff on screen is the line-based fallback
- the comment browser conforms to tree mode and says what a comment is

### Fixed

- read keybindings.toml and Config.toml from ~/.config/rv on every platform

## [1.4.0](https://github.com/Firaenix/rv/compare/v1.3.0...v1.4.0) - 2026-09-02

### Added

- session defaults in ~/.config/rv/Config.toml

## [1.3.0](https://github.com/Firaenix/rv/compare/v1.2.0...v1.3.0) - 2026-09-02

### Added

- configurable keybindings via ~/.config/rv/keybindings.toml

## [1.2.0](https://github.com/Firaenix/rv/compare/v1.1.0...v1.2.0) - 2026-09-01

### Added

- *(rv)* tree navigation, Space mode-leader, and a layers-first ?
- *(rv)* PgUp/PgDn/Home/End navigation and a Jump & scroll keymap section

### Other

- *(rv)* unify commits-view diff loading with the file list; add direct i key

## [1.1.0](https://github.com/Firaenix/rv/compare/v1.0.0...v1.1.0) - 2026-08-21

### Added

- *(rv)* full-file diff context, cached rendering, keybind ergonomics, and a bash-grammar SIGSEGV guard ([#8](https://github.com/Firaenix/rv/pull/8))
- *(rv)* close every open spec item — confidence, before/after, grouping, hunks, one file

### Fixed

- *(rv)* give the rv-core path dependency a version so cargo package passes
