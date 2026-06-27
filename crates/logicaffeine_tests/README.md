# logicaffeine-tests

The workspace's integration and differential test suite — the living specification for the whole pipeline. It exercises English → lexer → parser → AST → FOL → compile (Rust/C) → run, plus the proof kernel, the VM/JIT/AOT optimizer tiers, and the Z3-backed hardware-verification stack.

Part of the [Logicaffeine](../../NEW_README.md) workspace. `publish = false`; `src/lib.rs` is a doc shell only, so all behavior lives in `tests/*.rs`, each auto-discovered as its own integration-test binary. As dev-dependencies it pulls in nearly every major crate: `-base`, `-kernel` (serde), `-language`, `-compile`, `-proof`, `-data`, `-system` (full), `-lexicon`, and `-forge`/`-jit` (copy-patch JIT + regalloc, non-wasm targets only). The Z3-backed `-verify`/`-tv`/`-synth` are *optional* dependencies, gated behind the `verification` feature, so the default path needs no Z3 toolchain.

## What's here

Hundreds of test binaries (~500+ `.rs` files at the top of `tests/`) plus three shared harness modules and a snapshot dir. The layout:

- **Numbered phases — `phase1_*` … `phase143_*` (~177 files).** A complexity ladder. 1–20 cover linguistics (garden-path, polarity, aspect/tense, wh-movement, event semantics, degrees, ellipsis, ambiguity, MWEs, ontology, negation, plurality); 21–38 the imperative language layer (blocks, imperative verbs, ownership, types, codegen, structs/enums/generics, modules, CLI, stdlib); 41–57 semantics + distributed systems (DRS donkey anaphora, refinement, modal subordination, sessions, CRDTs, security, mesh, sync); 60–104 the proof-kernel ladder (engine, induction, oracle, CoC, inductive types, certifier, extraction, reflection, diagonal lemma, Gödel incompleteness, tactics, induction-vs-Z3); 105–143 advanced semantics/pragmatics (relational adjectives, performatives, counterfactuals, presupposition, binding, clefts, evidentials, optatives, implicature, gapping, mereology).
- **Named `phase_*` — ~204 files.** `phase_hw_*` (~66) hardware verification (SVA/IEEE-1800, IC3/PDR, k-induction, CEGAR, interpolation, RTL/knowledge-graph extraction, synthesis, FOL→SVA — mostly `verification`-gated); `phase_crdt_*`; `phase_pe_*` + `phase_futamura` (partial evaluation and the Futamura projections); `phase_simon_*`/`phase_puzzle_*`/`phase_studio_*` (logic-grid puzzles solved by the *general* prover); `phase_exodia_*` (EXODIA JIT); `phase_literate_*` (NL proof scripts); `phase_temporal_*`; and a large family of optimizer-lever suites (`phase_de_rc`, `phase_dense_i64_map`, `phase_{i32,i64}_map`, `phase_affine_array`, `phase_seq_scalarize`, `phase_unroll`, `phase_inline_*`, `phase_borrow_hoist`, `phase_narrow`, `phase_{lia,omega,polyhedral}`, `phase_symmetry`, oracle/bounds, `phase_optimize`, `phase_codegen_c`).
- **`e2e_*` — ~60 files (18 `e2e_codegen_*`).** Compile-and-run codegen for every datatype/feature, distributed-systems end-to-end (concurrency, crdt, gossip, mesh, multi-node, network-partition), and interpreter-gap suites.
- **VM/JIT/hotswap/AOT — ~55 files.** `jit_*` (~34) copy-patch JIT + forge regalloc; `hotswap_*` (~14) the tiering pipeline; `vm_*` (~5); `aot_native_corpus`/`aot_native_tier` (`#[ignore]`d AOT soundness gates that invoke `rustc` per program); `tco`, `tier_invariance`, `bench_corpus`, `runpath_*`, `opt_toggle_aot_differential`.
- **Differential gates.** The cross-tier invariant suites — `e2e_differential`, `jit_vm_differential`, `vm_opt_differential`, `vm_parity_matrix`, `phase_vm_differential`, `baseline_vm_differential`, `phase_pe_differential`, `concurrency_differential`, `tier_invariance` — prove the interpreter, VM, JIT, and AOT tiers all agree on a program's meaning, catching optimization miscompiles.
- **Harness + snapshots.** `common/mod.rs` is the heart: `parse_to_view`/`parse!` build an `ExprView` from leaked arenas; `run_logos`/`compile_logos`/`assert_c_output`/`compile_and_link_c` build each program in a temp project against a nextest-sharded shared `CARGO_TARGET_DIR`. The keystone is **`assert_compiled_equals_interpreted`** (+ `_seeded`/`_eq`): codegen output MUST match the interpreter reference (VM + tree-walker). `pe_support/mod.rs` triangulates tree-walker vs PE-residual; `extraction_common/mod.rs` runs kernel-extracted Rust. `assert_snapshot!` checks `snapshots/*.txt` (12 golden files; regenerate with `UPDATE_SNAPSHOTS=1`).
- **Misc.** Standalone linguistic suites (`aktionsart_tests`, `complex_combinations`, `integration_tests`, `torture_tests`, `modal_scope_tests`, `intensionality_tests`, `diagnostic_bridge`) and small `debug_*` probes.

## Running the tests

The default path needs no Z3:

```bash
cargo test -- --skip e2e                       # fast: skip the compile-and-run e2e suites
cargo test                                     # full default suite
cargo test --test phase1_garden_path           # one binary
cargo test --features verification -- --skip e2e   # adds the Z3-gated suites (~72 files)
```

For the full parity-proven sweep across every crate and feature (including the Z3 `verification` tests, `#[ignore]`d suites, and doctests), use the nextest runner from the workspace root:

```bash
./scripts/run-all-tests-fast.sh                # add --no-ignored to skip the fuzz/bench monsters
```

The `verification` feature turns on `logicaffeine-compile/verification` and `logicaffeine-proof/verification` and pulls in the optional `-verify`/`-tv`/`-synth` crates, which require a Z3 toolchain (`Z3_SYS_Z3_HEADER=/usr/include/z3.h` on Linux).

## Feature flags

| Feature | Default | Enables |
|---------|---------|---------|
| `verification` | off | Z3-based static verification: turns on `verification` in `-compile`/`-proof` and pulls in `-verify`, `-tv`, `-synth`. Gates ~72 test files (the `phase_hw_*` Z3 suites and verification phases). Requires Z3. |
| `ffi-link-tests` | off | The C/wasm FFI link tests in `phase_ffi_requires.rs` that actually invoke a C compiler/linker, rather than only checking generated source. |
| `web-tests` | off | Pulls in `dep:logicaffeine-web` and enables the web-facing suites (`struggle_tests`, `learn_state_tests`, `unlock_logic_tests`). |

## License

Business Source License 1.1 — see [LICENSE.md](../../LICENSE.md).

---
[Docs index](../../new_docs/README.md) · [Root README](../../NEW_README.md) · [Changelog](../../CHANGELOG.md)
