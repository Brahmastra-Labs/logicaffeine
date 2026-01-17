# Changelog

All notable changes to this crate will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

## [0.6.0] - 2026-01-17

Initial crates.io release.

### Added

- Backward-chaining proof engine implementing Curry-Howard correspondence
- Robinson's unification algorithm with occurs check
- Higher-order pattern unification (Miller patterns)
- Alpha-equivalence for quantifiers and lambda expressions
- Beta-reduction to weak head normal form
- Comprehensive inference rules (modus ponens, induction, etc.)
- Structural induction over inductive types
- Modal and temporal logic support
- Socratic hint generation for pedagogical guidance
- DerivationTree to Kernel Term certification
- Optional `verification` feature for Z3 oracle fallback
- Liskov Invariant: no dependency on language crate
