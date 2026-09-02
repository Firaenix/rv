# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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
