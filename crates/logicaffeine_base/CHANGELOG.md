# Changelog

All notable changes to this crate will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

## [0.6.0] - 2026-01-17

Initial crates.io release.

### Added

- Arena allocation via bumpalo with stable AST references
- Symbol interning with O(1) equality comparison
- Span tracking for error reporting with merge support
- SpannedError base type with source locations
- WASM-compatible design (no platform-specific code)
