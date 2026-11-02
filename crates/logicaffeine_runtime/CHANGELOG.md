# Changelog

All notable changes to this crate will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

## [0.11.0] - 2026-11-02

### Added
- **`RtPayload::ComplexF` and `RtPayload::Crdt`.** A floating-point complex value (`ComplexF { re, im }`) and a CRDT can cross a task or pipe boundary. A CRDT ships its full kind-tagged convergent state rather than its shared handle; the receiving side builds a fresh replica and merges it, so a redelivered payload never double-counts. The marshalling is in `crates/logicaffeine_compile/src/concurrency/marshal.rs`.
- **`scheduler::narrate_deadlock(&SchedSnapshot) -> String`.** It renders a proven deadlock as prose: each blocked task with what it waits on, and each channel's fill, blocked senders and receivers, and closed state. `crates/logicaffeine_tests/unit/logicaffeine_runtime/scheduler/deadlock_narration_tests.rs`.
- **`RtPayload::{Exact, Scaled, Depends, Converging}`.** An exact value crosses a task or pipe boundary in its canonical encoding: a real algebraic number, computable real or ordinal by its bytes, a hyperreal or infinite integer with the name and residues of its infinitesimal, a residue-dependent value branch by branch, and a provisional answer with its evidence.

### Breaking changes
- **`RtPayload` gained `ComplexF` and `Crdt`.** The enum is not `#[non_exhaustive]`, so an exhaustive `match` stops compiling. Migration: add the two arms.

### Changed

- Licensed under the MIT License, which replaces the Business Source License 1.1, and the package ships it as `LICENSE.md`.

- **The published crate carries only its source.** An anchored `include` allowlist ships `src/`, the README and the CHANGELOG; the `executor` and `scheduler` integration suites moved to `crates/logicaffeine_tests/tests/` and the unit-test bodies to `crates/logicaffeine_tests/unit/logicaffeine_runtime/`, so no test code ships. The declared `rust-version` is 1.88 (0.10.1 declared 1.75).

### Fixed
- **A `Select` that names one pipe in two arms no longer takes that pipe's next value.** A woken select withdraws every registration it made, on the pipe that woke it too; the second registration used to outlive the select, and the pipe's next value resumed the task inside an unrelated `Receive`. `Close` wakes such a select once, not twice. Tree-walker and VM both, through this scheduler. `select_same_pipe_twice_lock`.

## [0.10.0] - 2026-07-08

### Added
- Initial release. The deterministic concurrency runtime — the operational semantics of LOGOS concurrency for the interpreter and VM.
- Task scheduler, FIFO channels, `Select`, a logical-clock timer wheel, and the seed/trace machinery. A run is a deterministic function of `(program, seed)` and replays bit-for-bit from `(program, trace)`.
- Single `Chooser::decide` choke point for every nondeterministic scheduling decision: record mode draws from a seeded SplitMix64 RNG and logs `ChoicePoint`s; replay mode re-issues the recorded choice and asserts the decision shape still matches.
- Pure `std`, WASM-safe, tokio-free. By charter never linked into AOT-compiled binaries (the compiled path uses `logicaffeine-system`); the zero-dependency constraint enforces that boundary. See the root CHANGELOG for cross-crate context.
