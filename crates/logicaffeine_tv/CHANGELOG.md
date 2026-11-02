# Changelog

All notable changes to this crate will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

## [0.11.0] - 2026-11-02

### Added

- `tensor::discharge` proves a rewrite of the one tensor IR: the lemmas of the obligation
  `logicaffeine_compile::tensor_graph::smt` prints for two graphs, each loaded into Z3. `Proven` when every lemma's
  premises are satisfiable and its premises and goal together are not; `Refuted` with the counterexample; `Vacuous` when
  a lemma's premises assume what no input satisfies. The crate depends on `z3` directly.

### Changed

- Licensed under the MIT License, which replaces the Business Source License 1.1, and the package ships it as `LICENSE.md`.

- **The published crate carries only its source.** An anchored `include` allowlist ships `src/`, the README and the CHANGELOG, so the `concurrency_tv` integration suite that the 0.10.1 tarball carried in `tests/` is no longer published; it runs from `crates/logicaffeine_tests/tests/concurrency_tv.rs`. The declared `rust-version` is 1.88 (0.10.1 declared 1.75). The API is unchanged.

## [0.10.0] - 2026-07-08

### Added
- Initial release. SMT translation validation — proves the Rust emitted by the compiler is observationally equivalent to its LOGOS source, per compile.
- Symbolically executes both sides into the shared `logicaffeine-verify` domain (bitvector/boolean) and discharges the equivalence with Z3.
- `check_encoder_sound` cross-validates the LOGOS encoder against the tree-walking interpreter — the trust anchor that catches a buggy encoder rather than letting it prove two wrong things equal.
- Translation validation at rung 3–4: the trust boundary is the encoders, Z3, and rustc. The verifiable core is straight-line `Int`/`Bool` (no loops/functions/calls). Z3-backed and gated behind the `verification` feature. See the root CHANGELOG for cross-crate context.
