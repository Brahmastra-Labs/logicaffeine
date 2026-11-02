# Changelog

All notable changes to this crate will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

## [0.11.0] - 2026-11-02

### Added
- **Functions over the program's own enums tier.** A node rides a frame slot as the VM's own `Rc<InductiveValue>` pointer; `a new Node with …`, an `Inspect` arm's test and bindings (`Int`, `Bool`, `Float` and node fields, the kind read from the declared variant and checked at run time) and an enum parameter or return lower through runtime helpers the new `Host` micro-op calls.
  - **Ownership** is a forward dataflow per register (`inductive.rs`). A parameter, and a field read out of one, is borrowed and costs no refcount traffic, so a tree walk like binary-trees' `check` takes and drops no counts. A node the function builds, a call's result and a field of an owned node carry one strong count, which moves into a new node or the return value at its last use or is released on the control-flow edge where its register dies (a landing pad on a branch's taken edge). Where a borrowed and an owned node meet in one register, the borrowed copies take a count of their own.
  - **Side exits** hand every node back to the VM: owned ones transfer their count, borrowed ones are shared. A reachable `ReturnNothing` (the fall-through after an `Inspect` whose arms cover every variant) side-exits so the VM returns Nothing from the same pc.
  - **binary-trees** at depth 16 runs in 2.16 s on the JIT tier against 5.04 s with the lane off (`LOGOS_JIT_NODES=0`), byte-identical. `jit_enum_lock` (14 rows: output against the tree-walker, compile floors, and allocations that do not grow with the work, across side exits and branch edges; 11 of 11 mutants killed), `bench_corpus` holds binary_trees at one native function.
- **27 builtins tier natively; 0.10.1 lowered `sqrt` alone.**
  - **Numeric:** `abs`, `min` and `max` on `Int` (branchless, no new stencil); `count_ones`, `leading_zeros` and `trailing_zeros`; `floor`, `ceil` and `round` of a `Float` to an `Int` (ties away from zero); and `pow` with an `Int` base and a constant exponent in `0..=32`, lowered to unrolled checked multiplies that deopt to the big-integer path.
  - **Word lane:** `word32`, `word64`, `word16`, `word_and`, `word_or`, `word64And`, `intOfWord16`/`intOfWord32`/`intOfWord64`, `word64Shl`, `word32Shr` and `word64Shr` (constant or variable count), `word_not`, `rotl` and `rotr`.
  - **Text:** `chr`, guarded by a `ChrGuard` that deopts on an invalid scalar, and `length of chr(k)` as the UTF-8 byte length, so the idiom runs native without allocating.
  - **CPU features:** an op whose instruction the CPU lacks (`popcnt`, `lzcnt`, `bmi1`, `sse4.1`) declines to the VM. `word8`/`intOfWord8` are the declared remaining gap.
  - **Locks:** `forge_builtin_compliance_ratchet` classifies all 234 `BuiltinId`s and holds the tiered floor at 27; `forge_numeric_perf_lock` and `forge_fnv_hash_lock` require a hot native region to beat the VM by at least 3× on x86_64.
- **A function can return a machine word, and a text accumulator can append an `Int`.** The kind dataflow accepted a `Return` of an `Int`, `Bool`, `Float`, list or node but not of a `Word32`/`Word64`, so every SHA-256 helper (`ssig0`, `ch`, `maj`, … each `-> Word32`) stayed on the VM; and `Set text to text + i` appends the decimal text of an `Int` through the same `StrAppend` helper the byte and constant forms use (`StrSrc::Int`), writing it with the VM's own `to_display_string`. sha256 compiles 6 of its 7 functions (0 before) and runs 1.48× the VM; strings tiers both loops and runs 1.78× the VM. `forge_word_return_lane`, `jit_strappend`.
- **Words ride unboxed.** A `Word32`/`Word16` is carried as its zero-extended value and a `Word64` as its bit pattern in an i64 slot; it enters a region behind a live-in guard and re-boxes at an exit or deopt, so a loop-carried hash state tiers. Same-width modular `+`/`-`/`*` (including `acc += x`), generic `xor`, `&` and `|`, and indexed reads of a pinned `Seq of Word32` all lower, so a SHA-256 compression round and FNV-1a run native, byte-identical to the tree-walker. `crates/logicaffeine_tests/tests/forge_word_arith_lane.rs`, `crates/logicaffeine_tests/tests/forge_word_state_lane.rs`, `crates/logicaffeine_tests/tests/forge_word_list_pin_lane.rs`, `forge_sha256_compression_shape_lock`, `forge_sha256_correctness_lock`.
- **A function over `Seq of Word32` tiers.** A list a function builds starts as a fresh-list site whose first `Int` or `Word32` push decides its packing (`Kind::FreshList`, `FreshSites`); pushes, item stores, `length of` and a `Return` of a `Word32` list run natively, and a list handed back to the VM (returned, written back, or held at a side exit) keeps the VM's packed representation (`NativeRet::ListByHandle`, `PinElem::Word32`). SHA-256's compression function runs natively (`forge_word_list_lane`). A function with an in-out enum parameter stays on the VM (`jit_enum_lock`).

### Changed

- Licensed under the MIT License, which replaces the Business Source License 1.1, and the package ships it as `LICENSE.md`.

- **The published crate carries only its source.** An anchored `include` allowlist ships `src/`, the README and the CHANGELOG; the unit-test bodies live in `crates/logicaffeine_tests/unit/logicaffeine_jit/` behind `#[cfg(test)] #[path]` stubs, so no test code ships. The declared `rust-version` is 1.88 (0.10.1 declared 1.75).

### Removed
- **The cross-branch array-store stopgap and its `LOGOS_BUG002_BAIL` switch.** The recursive in-place-store shape it refused now tiers, made correct by the flow-sensitive list kinds under Fixed; the variable is no longer read.

### Fixed
- **A region with a `Return` inside its loop exits with its return flag down when the loop runs out.** The region compile hands the register-allocating backend its live-out slots (the write-back registers and the return flag and value), which keeps the flag's write on the path that returns. `jit_region_in_loop_return_lock`.
- Native regions retain callee frames on precise exits. Host helpers with frame-resident operands remain eligible for precise region register allocation.
- Native register swaps retain support for mixed scalar representations alongside pinned same-element list swaps; flow-sensitive kinds preserve subsequent arithmetic and exit conversion.
- Scalar `Swap` lowers to native register moves while carrying type, liveness and ownership information through the adapter. Pinned collections stay on the VM path. Integer, real, boolean and word exchanges, mixed-kind round trips and overflow recovery are covered by `jit_scalar_swap_lock`, `jit_scalar_swap_edges_lock` and adapter unit tests.
- Mutable native arguments return their final scalar values and owned list buffers through direct, forwarded and recursive calls. Empty loan slots and recursive overflow deoptimization preserve VM state; newly created aliases trigger copy-on-write. Integer-list literals use native allocation, and return kinds follow each program point despite register reuse. Locked by `jit_in_out_storage_lock` and `jit_list_params`.
- **List discipline in native code.**
  - **Flow-sensitive list kinds:** list typing is per pc, so a register the bytecode compiler reuses no longer plants a list triple from an `Int` (a quicksort `SIGABRT`).
  - **Copy-on-write:** a store into a list that may be a parameter's or a self-call's result copies it first, as the VM does, through the new `logos_rt_list_own` runtime entry; the caller's list used to change under native code.
  - **Deopt rows:** every reachable pc has one, so an `Int` overflow inside a list-parameter function resumes on the VM instead of panicking, and aliases of one native list re-box to one shared value.
  - Locks: `jit_register_kind_reuse_lock`, `vm_cow_scaling_timing_lock`.
- **A list parameter's length is live through a region call.** A callee re-derives its pinned vector, pointer and length from the handle at entry; a hot `length of xs` reached through a region call read a stale slot, and bounded searches returned 0 or false. `crates/logicaffeine_tests/tests/jit_forge_pin_length_gauntlet.rs`, `crates/logicaffeine_tests/tests/jit_forge_random_fuzz.rs`.
- **A hot list search with an early `Return` stays correct.** A region that reads a list element and can return out of the function declines to the VM; it had returned a stale element. `jit_list_search_differential_lock`.

## [0.10.0] - 2026-07-08

### Added
- Initial release. The LOGOS native tier — the bridge from the bytecode VM's tier-up seam to the copy-and-patch forge JIT.
- `ForgeTier` translates VM bytecode (`Op`) into the forge's `MicroOp` subset for both whole functions (`ChainFn`) and hot loop regions (`RegionChain`), compiling each to a native stencil chain or the register-allocating backend via `logicaffeine-forge`.
- `install()` makes the tier process-wide; every live VM constructor picks it up, and `largo` installs it at startup.
- Deopt contract: anything outside the supported integer/float subset declines (`compile_* -> None`) and stays on bytecode, so correctness never depends on the JIT accepting a program.
- Native-only (`#![cfg(not(target_arch = "wasm32"))]`); WASM builds compile it to nothing and the browser runs pure bytecode. See the root CHANGELOG for cross-crate context.
