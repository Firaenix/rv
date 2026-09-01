# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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
