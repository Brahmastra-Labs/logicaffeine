# Changelog

All notable changes to this crate will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

## [0.11.0] - 2026-10-04

### Added
- **`reversible`: Z3 bijectivity proofs for the reversible opcodes.** `all_rev_specs` pairs `RevAdd`, `RevSub`, `RevXor` and `RevNeg` with the inverses the ISA declares for them. `prove_bijective` discharges both `∀x,d. G(F(x,d),d) = x` and `∀x,d. F(G(x,d),d) = x` over BV64 by refutation, and `prove_swap_involution` proves `RevSwap` is its own inverse; a deliberately wrong inverse is rejected, so the harness can fail. It is the semantic complement of the structural pairing in `reverse_op_lock`. `RevSpec`, `all_rev_specs`, `prove_bijective` and `prove_swap_involution` are re-exported at the crate root. `crates/logicaffeine_tests/unit/logicaffeine_synth/reversible/tests.rs`.

## [0.10.0] - 2026-07-08

### Added
- Initial release. EXODIA Phase 2 — the Forge's offline proof tooling for the copy-and-patch JIT. Runs at development and CI time; never on the production runtime path.
- Z3 specifications for the JIT's integer micro-operations over 64-bit bitvectors, with satisfiability and algebraic-property gates.
- Three-way witness harness: runs Z3-chosen inputs through the real compiled stencil and cross-checks against the specification.
- Naming note: here "synth" means stencil spec/template synthesis, unrelated to the hardware SVA synthesis in `logicaffeine_verify`. See the root CHANGELOG for cross-crate context.
