# Changelog

All notable changes to this crate will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

## [0.9.17] - 2026-06-11

### Added
- Initial release. The deterministic concurrency runtime — the operational semantics of LOGOS concurrency for the interpreter and VM.
- Task scheduler, FIFO channels, `Select`, a logical-clock timer wheel, and the seed/trace machinery. A run is a deterministic function of `(program, seed)` and replays bit-for-bit from `(program, trace)`.
- Single `Chooser::decide` choke point for every nondeterministic scheduling decision: record mode draws from a seeded SplitMix64 RNG and logs `ChoicePoint`s; replay mode re-issues the recorded choice and asserts the decision shape still matches.
- Pure `std`, WASM-safe, tokio-free. By charter never linked into AOT-compiled binaries (the compiled path uses `logicaffeine-system`); the zero-dependency constraint enforces that boundary. See the root CHANGELOG for cross-crate context.
