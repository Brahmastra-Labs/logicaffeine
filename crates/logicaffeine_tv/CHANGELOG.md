# Changelog

All notable changes to this crate will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

## [0.9.17] - 2026-06-11

### Added
- Initial release. SMT translation validation — proves the Rust emitted by the compiler is observationally equivalent to its LOGOS source, per compile.
- Symbolically executes both sides into the shared `logicaffeine-verify` domain (bitvector/boolean) and discharges the equivalence with Z3.
- `check_encoder_sound` cross-validates the LOGOS encoder against the tree-walking interpreter — the trust anchor that catches a buggy encoder rather than letting it prove two wrong things equal.
- Translation validation at rung 3–4: the trust boundary is the encoders, Z3, and rustc. The verifiable core is straight-line `Int`/`Bool` (no loops/functions/calls). Z3-backed and gated behind the `verification` feature. See the root CHANGELOG for cross-crate context.
