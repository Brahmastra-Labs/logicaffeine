# Changelog

All notable changes to this crate will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

## [0.6.0] - 2026-01-17

Initial crates.io release.

### Added

- WASM-safe data structures (no IO dependencies)
- CRDT implementations: GCounter, PNCounter, LWWRegister, MVRegister
- Complex CRDTs: ORSet (with AddWins/RemoveWins bias), ORMap
- Sequence CRDTs: RGA, YATA
- Vector clocks and Dot contexts for causal ordering
- Delta-state synchronization via DeltaCrdt trait
- LogosIndex trait with 1-based indexing convention
- LogosContains trait for unified membership checking
- Lamport Invariant: no path to system IO
