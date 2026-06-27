# Changelog

All notable changes to this crate will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

## [0.9.17] - 2026-06-11

### Added
- Initial release. EXODIA Phase 2 — the Forge's offline proof tooling for the copy-and-patch JIT. Runs at development and CI time; never on the production runtime path.
- Z3 specifications for the JIT's integer micro-operations over 64-bit bitvectors, with satisfiability and algebraic-property gates.
- Three-way witness harness: runs Z3-chosen inputs through the real compiled stencil and cross-checks against the specification.
- Naming note: here "synth" means stencil spec/template synthesis, unrelated to the hardware SVA synthesis in `logicaffeine_verify`. See the root CHANGELOG for cross-crate context.
