# Changelog

All notable changes to this crate will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

## [0.6.0] - 2026-01-17

Initial crates.io release.

### Added

- Calculus of Inductive Constructions (CIC) implementation
- Unified Term representation (types and values are terms)
- Bidirectional type checking with universe cumulativity
- Beta, iota, and fix normalization
- Syntactic guard condition for termination checking
- Strict positivity checking for inductive types
- Standard library: Entity, Nat, Bool, Eq, And, Or, Ex, lists
- Decision procedures: ring, lia, omega, cc, simp
- Tactic combinators: orelse, then, try, repeat, first
- Deep embedding: Univ, Syntax, Derivation types
- Milner Invariant: no path to lexicon (recompile isolation)
