# Changelog

All notable changes to this crate will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

## [0.10.0] - 2026-07-08

### Added
- `satcli` module: the SAT command-line driver (DIMACS in, `s SATISFIABLE`/`s UNSATISFIABLE` + model out, DRAT/DPR/SR certificate export, competition exit codes) extracted from the `logos-sat` binary with injected output streams — shared verbatim by `logos-sat` and `largo sat`, and directly testable.
- Modal translation and independent verification.
- CDCL core with an incremental grid solver, grounding, and trust-tiers; label/PP convergence.

### Changed
- **`prove_unsat` — the expensive certified cuts are escalation-only.** The Lyapunov collapse, Nullstellensatz, Polynomial Calculus, and recursive certified-symmetry rungs ran on every call, taxing microsecond equivalence/BMC obligations ~50 ms each. The cheap structural recognizers stay upfront; the complete CDCL search then runs under a conflict budget (`EASY_SEARCH_CONFLICTS`), and only an instance that exhausts it escalates to the certified cuts, finishing on a fresh solver so the refutation certificate stays self-contained. Verdicts unchanged; the native-vs-Z3 speed locks hold again and the hard symmetric families still refute through the escalation path.

### Fixed
- **`cert_farkas` kernel reconstruction on doubled constants** — the proof-producing normalizer had no proof path for like monomials recombining to coefficient 1, and a merge that cancelled an entire prefix concluded with an unproven `add 0 x` residue the kernel rejected. Both close (locked by `arith::tests`), and `probe_double_constant_le_via_auto` runs un-ignored.

See the root CHANGELOG for the cross-crate history.

## [0.8.12] - 2026-02-14

Synced to workspace version 0.8.12. See root CHANGELOG for full history.

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
