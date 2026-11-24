# Changelog

All notable changes to this crate will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Added

- **`rust`: the emitted Rust validated against its source.** `validate(source)` proves, for every input, that each
  function `compile_to_rust` emits shows what its source shows and ends as it ends, or stops with a loud integer
  overflow where the source holds a value past 64 bits (`ProvenUpToOverflow`: it never computes a wrong answer). The
  program is parsed with `syn`; release-build `i64` arithmetic wraps, and the `logos_*_i64`/`*_exact` helpers,
  `expect_i64` and `logos_cold_path` are taken as `logicaffeine_data::ops` defines them. The proof is `bytecode`'s,
  now generic over its target (`product::Side`), with loop heads paired as the two sides reach them when the nests
  differ, so a loop the emitter versions on a guard is proved against its one source loop. A seeded miscompile of the
  emitted text is refuted with an input, a silent wrap included (`rust_tv_lock`). An `i64` parameter is assumed to
  hold an `i64` value (`Side::assumes`): a caller holding a larger one stops before the call.
- **A loop compiled to its closed form is proved for every input.** When the target runs no loop where the source does
  (the emitter's `n·(n+1)/2` behind a guard with an exact cold path, Faulhaber's sums of squares and cubes, a
  `## Main` folded to a constant; the run-path optimizer's closed forms on the VM), each of the target's paths is proved
  against the source loop under an invariant per source head over the head's state and the inputs, assuming the
  path's condition (`bytecode::product::closed`). The candidates come from concrete runs of the source on sampled
  inputs (`product::sample`): the polynomial relations among a head's integers and the inputs, found exactly over the
  rationals by increasing degree (`product::relation`), and the bounds the runs kept on each integer and each
  difference of two, filed by the target path the inputs take; Houdini keeps the inductive ones. The lockstep proof
  offers the same candidates and every comparison over the inputs a path tested, so a guard before a versioned loop
  is known inside it: the emitter's plain-`+` accumulation of `3·i + 2` is proved through `n₀ ≤ 1000000000` and
  `3·i² + i = 2·total + 4`. A division a polynomial always meets exactly is given to Z3 as a lemma, decided by
  residues (`product::divides`). The source is shown to leave each loop, through a bound the invariant keeps on a
  quantity every iteration raises, since the target always ends. The Gauss sum is proved through `i² = 2·total + i`
  and `i − n₀ ≤ 1`; a changed coefficient in the closed form is refuted with an input.
- At a pair of loop heads reached from the entry, the lockstep proof also offers each location's entry term: the
  term over the inputs it holds there (`product::sample::entry_terms`), kept where the loop never disturbs it. The
  bound of the emitter's `for _ in a..(b + 1)` is `b₀ + 1` at every head, so a loop it walks to an inclusive bound
  is proved against the source's `i ≤ b` (`rust_tv_lock`).
- Z3's work per query is bounded in resource units rather than milliseconds, so a verdict does not depend on how
  loaded the machine is, and a query the incremental engine abandons as incomplete is decided again by a fresh solver
  (simplification, solved-equation elimination, then the SMT core; not Z3's default nonlinear tactic, which ignores
  the resource limit). Each validation runs its proofs in a Z3 context of its own, so a verdict does not depend on
  what the process asked Z3 before.
- **`bytecode`: the register VM's bytecode validated against its source, for every input.** `validate(source, target)`
  checks `## Main` and every function against the `CompiledProgram` the target runs: `Target::Vm`, the program as
  written compiled with the oracle's facts, or `Target::Run`, the program after the run-path optimizer. Both sides run
  symbolically over exact integers. The source side is a control-flow graph whose meaning is the tree-walker's. The
  bytecode side takes each op as the VM's handler computes it, including on operands the compiler's own facts exclude:
  `MagicDivU` on a negative dividend computes what its reciprocal sequence computes on the `u64` reading, and `BitAnd`
  follows the runtime's mask rule on a big integer. The two are paired at their loop heads, Houdini infers the
  invariant relating registers to variables (equalities, constants, lower bounds), and Z3 shows every pair of paths
  that can run together ends alike, having shown the same values. Loops are proved for every trip count, not
  unrolled. A tail call the compiler turned into a jump is proved as that loop; a recursive or looping callee is an
  uninterpreted function named in `assumed`. A reciprocal sequence is decided exact for every `u64` by the
  Granlund–Montgomery error bound, computed exactly (`value::magic_exact`); one that is not exact is modeled step by
  step. A failed obligation is `Refuted` only when an input, replayed concretely through both sides, shows the two
  apart, and `Unproven` otherwise. `validate_edited` validates the bytecode a seeded edit leaves, and `check_executors`
  holds both symbolic executors to the tree-walker and the VM (`bytecode_tv_lock`).

### Fixed

- The source encoder takes `and`/`or` as the runtime does: the truth of each operand, a `Bool` result, and a right
  operand that raises counted only when it runs. It modeled them as bitwise on `Int` and counted the right operand's
  errors always, so `6 and 3` and `false and 1 / z > 0` were reported as disagreements with the interpreter
  (`phase_tv_encoder_sound`).

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
