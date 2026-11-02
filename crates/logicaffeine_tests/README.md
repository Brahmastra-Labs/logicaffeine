# logicaffeine-tests

Every test in the workspace lives in this one crate, so none of it ships: integration and differential suites are `tests/*.rs` binaries of this package, and each crate's white-box unit tests have their bodies under `unit/<crate dir>/<module path>.rs`. A crate keeps only a one-line `#[cfg(test)] #[path = "…"] mod tests;` stub, so its unit tests keep private access and their names, and still run under `cargo nextest run -p <crate>`. Together they are the specification for the whole pipeline: English → lexer → parser → AST → FOL, compilation to Rust and C, every execution tier, the proof kernel, and the Z3-backed hardware-verification stack.

Part of the [Logicaffeine](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/README.md) workspace, tier 7. It is `publish = false`, and `src/lib.rs` is a doc shell: the behavior lives in `tests/`, `unit/` and the `src/bin/` shims.

## Role in the workspace

The crate sits at the top of the dependency graph so it can reach every crate below it. `test_home_lock` and `unit_home_lock` keep every test here: no other crate has a `tests/` directory, and no `src/` holds a test body. A new test goes in one of two places: a `tests/<suite>.rs` binary for integration and differential work, or `unit/<crate dir>/<module path>.rs` behind the crate's stub for white-box work.

`src/bin/` rebuilds two shipped binaries from their library entries: `largo-under-test` calls `logicaffeine_cli::largo_main` and `logos-sat-under-test` calls `logicaffeine_proof::satcli::main`. The CLI suites spawn those, so they test exactly what ships.

## Test layout

`tests/` holds one binary per suite, plus their helpers and fixtures; a crate's suites keep their own names, with a crate prefix where two would collide. The phase-organized core:

- **Numbered phases — `phase1_*` … `phase143_*`.** Ordered roughly by complexity. Phases 1–20 cover linguistics (garden-path, polarity, aspect/tense, wh-movement, event semantics, degrees, ellipsis, ambiguity, MWEs, ontology, negation, plurality). Phases 21–38 cover the imperative language layer (blocks, imperative verbs, ownership, types, codegen, structs/enums/generics, modules, CLI, stdlib). Phases 41–57 cover semantics and distributed systems (DRS donkey anaphora, refinement, modal subordination, sessions, CRDTs, security, mesh, sync). Phases 60–104 cover the proof kernel (engine, induction, oracle, CoC, inductive types, certifier, extraction, reflection, diagonal lemma, Gödel incompleteness, tactics, induction versus Z3). Phases 105–143 cover advanced semantics and pragmatics (relational adjectives, performatives, counterfactuals, presupposition, binding, clefts, evidentials, optatives, implicature, gapping, mereology).
- **Named `phase_*`.** `phase_hw_*` is hardware verification (SVA/IEEE-1800, IC3/PDR, k-induction, CEGAR, interpolation, RTL/knowledge-graph extraction, synthesis, FOL→SVA), mostly `verification`-gated. Beside it sit `phase_crdt_*`; `phase_pe_*` + `phase_futamura` (partial evaluation and the Futamura projections); `phase_simon_*`/`phase_puzzle_*`/`phase_studio_*` (logic-grid puzzles solved by the *general* prover); `phase_exodia_*` (EXODIA, the copy-and-patch JIT); `phase_literate_*` (NL proof scripts); and `phase_temporal_*`. The optimizer-pass suites are `phase_de_rc`, `phase_dense_i64_map`, `phase_{i32,i64}_map`, `phase_affine_array`, `phase_seq_scalarize`, `phase_unroll`, `phase_inline_*`, `phase_borrow_hoist`, `phase_narrow`, `phase_{lia,omega,polyhedral}`, `phase_symmetry`, oracle/bounds, `phase_optimize` and `phase_codegen_c`.
- **`e2e_*`, including `e2e_codegen_*`.** Compile-and-run codegen for every datatype and feature, distributed systems end to end (concurrency, crdt, gossip, mesh, multi-node, network-partition), and interpreter-gap suites.
- **VM, JIT, hotswap, AOT.** `jit_*` and `forge_*` for the JIT and forge's register allocation; `hotswap_*` for the tiering pipeline; `vm_*`; `aot_native_corpus`/`aot_native_tier` (`#[ignore]`d AOT soundness gates that invoke `rustc` per program); `tco`, `tier_invariance`, `bench_corpus`, `runpath_*`, `opt_toggle_aot_differential`.
- **Differential gates.** The cross-tier invariant suites (`e2e_differential`, `jit_vm_differential`, `vm_opt_differential`, `vm_parity_matrix`, `phase_vm_differential`, `baseline_vm_differential`, `phase_pe_differential`, `concurrency_differential`, `tier_invariance`) prove the interpreter, VM, JIT and AOT execution tiers agree on a program's meaning, which is what catches an optimization miscompile.
- **Harness and snapshots.** `common/mod.rs` is the shared harness: `parse_to_view`/`parse!` build an `ExprView` from leaked arenas, and `run_logos`/`compile_logos`/`assert_c_output`/`compile_and_link_c` build each program in a temp project against a shared `CARGO_TARGET_DIR`, sharded under nextest. The central check is **`assert_compiled_equals_interpreted`** (with `_seeded` and `_eq` variants): codegen output must match the interpreter reference (VM and tree-walker). `pe_support/mod.rs` triangulates the tree-walker against the partial-evaluation residual; `extraction_common/mod.rs` runs kernel-extracted Rust. `assert_snapshot!` checks the golden files in `tests/snapshots/` (regenerate with `UPDATE_SNAPSHOTS=1`).
- **Misc.** Standalone linguistic suites (`aktionsart_tests`, `complex_combinations`, `integration_tests`, `torture_tests`, `modal_scope_tests`, `intensionality_tests`, `diagnostic_bridge`).

## Running the tests

This crate builds one binary per suite, so the question is always what cargo will build, not just what will run. From the workspace root, the full suite runs on cargo-nextest: every crate, the Z3 `verification` tests, the `#[ignore]`d suites and the doctests.

```bash
./scripts/run-all-tests-fast.sh                                                      # the full suite
./scripts/run-all-tests-fast.sh --no-ignored                                         # skip the multi-minute #[ignore]d fuzz and bench suites
cargo nextest run -p logicaffeine-tests --test phase1_garden_path                    # one suite
cargo nextest run -p logicaffeine-tests --test phase1_garden_path -E 'test(<name>)'  # one test in that suite
cargo nextest run -p logicaffeine-kernel                                             # one crate's unit tests
```

Name the suite with `--test <file>` whenever you run part of this crate. A `-E` filter only chooses which tests run: cargo still builds every test binary it is pointed at, and for `-p logicaffeine-tests` that is every suite in `tests/`, hundreds of gigabytes of build output. A bare `cargo test` in the workspace builds all of them too.

The research probes and the other on-demand classes (gate-level silicon simulations, qemu corpus differentials, and the like) are left out of the full run and have their own runners, indexed in [scripts/README.md](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/scripts/README.md).

The default path needs no Z3. The `verification` feature turns on `verification` in `logicaffeine-cli`, `logicaffeine-compile` and `logicaffeine-proof` and pulls in the optional `-verify`/`-tv`/`-synth` crates, which need a Z3 toolchain (`Z3_SYS_Z3_HEADER=/usr/include/z3.h` on Linux; the fast runner sets it itself):

```bash
Z3_SYS_Z3_HEADER=/usr/include/z3.h cargo nextest run -p logicaffeine-tests --features verification --test phase_verification
```

## Feature flags

| Feature | Default | Enables |
|---------|---------|---------|
| `verification` | off | Z3-based static verification: turns on `verification` in `-cli`/`-compile`/`-proof` and pulls in `-verify`, `-tv`, `-synth`. Gates the Z3 suites (the `phase_hw_*` Z3 suites, the verification phases, `phase_exodia_forge`, `phase_tv_encoder_sound`). Requires Z3. |
| `ffi-link-tests` | off | The C/wasm FFI link tests in `phase_ffi_requires.rs` that invoke a C compiler and linker, rather than only checking generated source. |
| `web-tests` | off | Pulls in `dep:logicaffeine-web` (with `dioxus`, `dioxus-ssr`, `dioxus-history`) and enables the web crate's suites (`struggle_tests`, `learn_state_tests`, `guide_*`, `ssr_render`, `code_examples_all_tiers`, and more). The full-suite runner and CI turn it on. |
| `codegen` | on | Forwards `logicaffeine-compile/codegen` (compile's default): the compile crate's codegen-backend suites in the `cuda_*`, `sass_*`, `spatial_*`, `tt_*` and `tensor_*` families gate on it. |
| `wasm-jit` | on | Forwards `logicaffeine-compile/wasm-jit` and pulls in `wasmi`: the browser WASM-JIT and AOT-WASM suites (`scripts/test/wasm-jit-filter.txt`). On by default because the CLI dependency already turns it on in compile. |
| `serde` | on | Forwards `logicaffeine-kernel/serde` (the kernel's audit-trail and certificate suites). |
| `http` | on | Forwards `logicaffeine-system/http` (the system crate's HTTP lifecycle and engine suites). |
| `persistence` | on | Forwards `logicaffeine-system/persistence`. |
| `concurrency` | on | Forwards `logicaffeine-system/concurrency`. |
| `io-uring` | off | Forwards `logicaffeine-system/io-uring` (the io_uring HTTP engine suites; off by default, as in the system crate). |
| `nbe` | off | Forwards `logicaffeine-kernel/nbe` (the NbE checker suites). |
| `capnproto` | off | Forwards `logicaffeine-wirebench/capnproto` and pulls in `capnp`: the Cap'n Proto head-to-heads (needs the `capnp` compiler). |
| `handed-reds` | off | The RED suites handed out with the fusion book's tickets, each behind `#![cfg(feature = "handed-reds")]` until its ticket lands; `scripts/run-handed-reds-tests.sh` runs them on purpose. |

## Dependencies

- **Internal, native:** `logicaffeine-cli`, `logicaffeine-proof` and `logicaffeine-jit` as dependencies (the `*-under-test` binaries and the CLI suites); `logicaffeine-base`, `logicaffeine-kernel` (`serde`, `lean`), `logicaffeine-language` (`lexicon-reference`), `logicaffeine-compile`, `logicaffeine-data`, `logicaffeine-system` (`full`), `logicaffeine-lexicon`, `logicaffeine-runtime`, `logicaffeine-forge`, `logicaffeine-lsp`, `logicaffeine-wirebench` and `wiki-trace` as dev-dependencies.
- **Internal, optional:** `logicaffeine-web` behind `web-tests`; the Z3-backed `logicaffeine-verify`, `logicaffeine-tv` and `logicaffeine-synth` behind `verification`.
- **Oracles (dev-only, never shipped):** reference crypto (`ml-kem`, `ml-dsa`, `libcrux-ml-kem`, `libcrux-ml-dsa`) for bit-exactness, and `postgres-protocol`, `pgwire` and `postgresql_embedded` for the LOGOS Postgres client.
- **Harness (dev-only):** `rcgen`, `rustls` and `tokio-rustls` for hermetic TLS; `syn`, `quote` and `proc-macro2` for the source-reading locks; `wasmi` to validate compiled WASM structurally; `tower-lsp` to drive the language server; serializer peers (`postcard`, `rmp-serde`, `bincode`); and `wasm-bindgen-test` for the browser suites.

## License

MIT License — see [LICENSE.md](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/LICENSE.md).

---
[Docs index](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/docs/README.md) · [Root README](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/README.md) · [Changelog](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/crates/logicaffeine_tests/CHANGELOG.md)
