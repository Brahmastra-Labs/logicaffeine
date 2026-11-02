# logicaffeine-jit

The native execution tier for the LOGOS bytecode VM: it translates hot functions and loop regions into forge micro-ops and runs them as x86-64 machine code. Where the VM dispatches one boxed op at a time, a tiered loop runs as one native chain over unboxed slots, and anything the tier cannot prove sound stays on bytecode, so tiering changes speed, not output. Machine code and executable memory belong to `logicaffeine-forge`; this crate is the bridge and its soundness contract.

Part of the [Logicaffeine](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/README.md) workspace, tier 5. Native-only: it compiles on x86-64 System V (Linux and macOS) and to nothing elsewhere, where the VM runs bytecode. The execution-tier story is in [execution-and-performance.md](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/docs/execution-and-performance.md).

## Role in the workspace

The bytecode VM in `logicaffeine-compile` profiles function calls and Main-loop back-edges; when something goes hot it asks its installed `NativeTier` to compile it. **This crate is that tier.** `ForgeTier` translates VM bytecode (`Op`) into the forge's `MicroOp` subset, both whole **functions** (`ChainFn`) and hot loop **regions** (`RegionChain`), compiles each to a native stencil chain or the register-allocating backend via `logicaffeine-forge`, and hands the compiled unit back.

Anything outside the supported subset (see [What tiers](#what-tiers)) returns `None` from `compile_*` and stays on bytecode for good: the deopt contract.

```text
logicaffeine-compile (VM)  ──tier seam──▶  logicaffeine-jit  ──backend──▶  logicaffeine-forge
```

It implements `compile`'s `vm::{NativeTier, NativeFn, RegionFn, Op, Constant, …}` traits and calls `install_native_tier`; it drives forge's `jit::{compile_straightline_*, MicroOp, CompiledChain}`, `regalloc::*`, and `segv_trace`. Consumed by `apps/logicaffeine_cli` and the differential `logicaffeine-tests`.

## What tiers

A value rides a native slot unboxed, by kind:
- `Int` (exact: `+`/`-`/`*` overflow side-exits and the VM promotes to a big integer), `Bool`, `Float`;
- `Word32`/`Word16` as the zero-extended value and `Word64` as its bit pattern, re-boxed through `RegBox::Word32`/`RegBox::Word64` at an exit, a deopt or a function's return. Same-width modular `+`/`-`/`*` (including `acc += x`), `xor`, `&` and `|` stay in the Word; a `Word16` is consumed only by `intOfWord16`;
- a `chr(code)` result as its code point, behind a `ChrGuard` that side-exits on an invalid Unicode scalar, re-boxed to a one-character `Text`;
- pinned lists of `Int` (full or half-width `i32` storage), `Word32`, `Float` and `Bool`, an `Int`-keyed map, ASCII text carried as bytes, and a mutable-text append accumulator (appending a byte, an `Int`'s decimal text, or a constant);
- a node of one of the program's own enums, as the VM's own `Rc<InductiveValue>` pointer: `a new Node with …`, an `Inspect` arm's test and bindings (`Int`, `Bool`, `Float` and node fields, the kind read from the declared variant and checked at run time), and an enum parameter or return, through runtime helpers the `Host` micro-op calls.

Builtin calls lowered natively: `sqrt`; `abs`, `min`, `max` on `Int`; `count_ones`, `leading_zeros`, `trailing_zeros`; `floor`, `ceil`, `round` of a `Float` to an `Int` (`round` ties away from zero); `pow` with an `Int` base and a constant exponent in `0..=32` (unrolled checked multiplies); `word32`, `word64`, `word16`, `word_and`, `word_or`, `word_not`, `word64And`, `intOfWord16`, `intOfWord32`, `intOfWord64`, `word64Shl`, `word32Shr`, `word64Shr`, `rotl`, `rotr`; and `chr`, with `length of chr(k)` as the UTF-8 byte length. An op whose instruction the CPU lacks (`popcnt`, `lzcnt`, `bmi1`, `sse4.1`) declines to the VM. `word8`/`intOfWord8` decline. `crates/logicaffeine_tests/tests/forge_builtin_compliance_ratchet.rs` classifies every `BuiltinId` as tiered, a gap, or host-only, and runs a hot ground-truth program for each tiered one.

## The deopt contract (soundness)

Soundness rests on these legs, all differentially tested in `logicaffeine-tests`:
- the kind-inference dataflow (`kind_flow`): forward and flow-sensitive per pc; a parameter starts at its declared kind, constants carry their pool type, comparisons are `Bool`, arithmetic promotes as the semantics core does (`Int`×`Int`→`Int`, any `Float`→`Float`, same-width Word→Word); merges join per slot (`Unknown ⊔ k = k`, conflict → `Mixed`, bail on use). List-ness is read per pc (`holds_list_at`), so a register the bytecode compiler reuses for an `Int` and a list is never treated as a list where it holds the `Int`;
- per-call / per-entry guards: an argument whose discriminant does not match the declared parameter kind routes the call back to bytecode;
- the region write-back contract: incoming-dead scratch needs no guard; writes re-box by inferred kind;
- list ownership: a store into a list that may be shared (a parameter's, or a self-call's result) first copies it through `logos_rt_list_own`, the native twin of the VM's copy-on-write; at a deopt every alias of one native-owned list re-boxes to one shared value;
- a region that reads a list element and can `Return` out of the function declines to the VM;
- node ownership (`inductive.rs`), a forward dataflow per register: a parameter, and a field read out of one, is borrowed and costs no refcount traffic; a node the function builds, a call's result and a field of an owned node carry one strong count, which moves into a new node or the return value at its last use, or is released on the control-flow edge where its register dies (a landing pad on a branch's taken edge). Where a borrowed node and an owned one meet in one register, the borrowed copies take a count of their own; an owned node meeting a value that is not one keeps the function on bytecode. A side exit hands every node back to the VM: an owned one transfers its count, a borrowed one is shared.

Two deopt flavors: classic discard-and-replay-from-head (sound for replay-idempotent regions; the VM rolls pinned buffers back to entry length), and precise `DeoptAt`, which materializes the native frame and resumes at the faulting op, as push and in-place `SetIndex` worklist shapes require. Every reachable pc of a precise function is a resume point, so an `Int` overflow anywhere in a list-parameter or enum function resumes on the VM, and a reachable `ReturnNothing` (the fall-through after an `Inspect` whose arms cover every variant) side-exits so the VM returns Nothing from the same pc. A `const` build assert pins the semantics core's `MAX_CALL_DEPTH` to the forge's baked call-depth so deep native recursion can never side-exit at the wrong depth.

## Public API

- `pub mod image`: build a self-contained, versioned bytecode image with `image::build`, then decode and execute it through a caller-supplied `ForgeTier` with `image::run`; native machine code is never serialized.
- `pub fn install() -> &'static ForgeTier`: install the forge tier process-wide. Idempotent (`OnceLock`); call once at startup. `LOGOS_NATIVE_TIER=0` returns the tier without wiring it into the VM.
- `pub fn segv_trace_install()`: arm the diagnostic SIGSEGV/SIGBUS tracer (no-op unless `LOGOS_SEGV_TRAP` is set).
- `pub struct ForgeTier` (`impl NativeTier`; `ForgeTier::new()`), the forge-backed tier with compile observability: `function_counts() -> (u32, u32)`, `region_counts() -> (u32, u32)`, `pinned_self_call_count() -> u32`, `regalloc_region_count() -> u32`, `regalloc_precise_region_count() -> u32`, `regalloc_function_count() -> u32`, and `runtime_stats() -> (u64, u64, u64, u64)` (entries, completions, deopts, precise_deopts: actual VM↔native crossings).
- `pub fn adapt(ops: &[Op], constants: &[Constant], register_count: usize) -> Option<(Vec<MicroOp>, usize)>`: translate a whole-program Main body (terminal `Show; Halt`) into the micro subset.
- `pub fn native_alloc_registry_len() -> usize`, `pub static REGION_RUNS`/`REGION_DEOPTS: AtomicU64`, `pub const LOGOS_MEMMEM_DEOPT: i64 = i64::MIN` (the memmem-failure sentinel).
- `logos_rt_*` `pub unsafe extern "C"` runtime helpers the stencils call by baked address (no relocations): `alloc_list_i64`, `list_triple`, `list_own` (copy-on-write of a shared list), `push_{i64,i32,f64,bool}`, `clear_{i64,i32,f64,bool}`, `map_{get_ii,set_ii,has_i}`, `str_append` (mirrors the VM's `add_assign` COW exactly), `memmem`/`memmem_frame`, and `side_exit` (a `Host` helper that always side-exits).

## Internals

`lib.rs`, plus `inductive.rs` for the enum lane (the ownership dataflow, the payload-kind seeding from the program's declared enum types, and the node helpers). `ChainFn::call` runs the chain on a 2²¹-slot thread-local arena, handling `Return` / classic `Deopt` / precise `DeoptAt` (`materialize` walks the live native frames). `RegionChain` carries guard / free / write / array sets, hoist guards, and per-resume-pc re-box kinds. `adapt_function` (scalar and precise list-parameter forms) and `adapt_region` lower to `MicroOp` over the narrow `Kind` lattice (Int/Bool/Float, Word32/Word64/Word16, a `chr` code point, pinned Int/Word32/Float/Bool lists incl. half-width `i32`, maps, ASCII text-as-bytes and single ASCII bytes, mutable-text and constant-text append operands). They run a loop-depth-weighted linear-scan pin selector (`select_pins`), apply array/float fusions (RMW, two-buffer load, FMA, cond/uncond swap, affine index), then prefer the regalloc backend over the per-piece stencil tier. A thread-local `ALLOC_REGISTRY` owns every fresh list: success detaches the returned one, deopt drains all, so replay is leak-free.

## Usage

```rust,no_run
logicaffeine_jit::segv_trace_install();
if std::env::var_os("LOGOS_NO_JIT").is_none() {
    logicaffeine_jit::install();
}
```

`LOGOS_NO_JIT` is the caller-side gate (`apps/logicaffeine_cli`'s convention); the crate's own kill switch is `LOGOS_NATIVE_TIER=0` inside `install()`. Live VM constructors in `logicaffeine-compile` then pick the tier up for every program they run.

## Tests & env knobs

The unit tests live in the `#[cfg(test)]` modules of `lib.rs`, their bodies in `crates/logicaffeine_tests/unit/logicaffeine_jit/` behind one-line `#[path]` stubs: `memmem_safety_tests` (substring-scan memory-safety guards), `select_pins_tests` (pin-budget soundness, e.g. a float result reused as an integer array index must not be float-pinned), and `fuse_rmw_tests`, `fuse_ld2_tests`, `fuse_affine_tests` (the array/float fusion peepholes). The end-to-end VM↔native byte-equivalence and deopt-replay tests live in `logicaffeine-tests`.

```bash
cargo nextest run -p logicaffeine-jit     # this crate's unit tests (native-only, no Z3)
./scripts/run-all-tests-fast.sh           # full workspace, parity-proven nextest runner
```

The crate's own knobs are below; forge's are in [its README](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/crates/logicaffeine_forge/README.md). Each is a diagnostic or a kill switch, and every kill switch falls back byte-identically to the per-piece tier (or to bytecode):

| Variable | Effect |
|----------|--------|
| `LOGOS_NATIVE_TIER=0` | `install()` skips wiring the tier into the VM, isolating pure-bytecode wall time. |
| `LOGOS_SEGV_TRAP` (set) | arm the SIGSEGV/SIGBUS tracer from `segv_trace_install()` to localize faults in JIT'd code. |
| `LOGOS_JIT_REGALLOC=0` / `LOGOS_REGALLOC=0` | disable the register-allocating backend (function + region). |
| `LOGOS_REGALLOC_PRECISE=0` | disable its precise list-parameter path (in-place `SetIndex`). |
| `LOGOS_LEVERB=0` | stop tiering regions that call a list-parameter function. |
| `LOGOS_JIT_NODES=0` | keep every function that handles enum values on bytecode. |
| `LOGOS_MEMMEM=0` | disable the `memmem` substring idiom. |
| `LOGOS_COPYPROP`/`LOGOS_RMW`/`LOGOS_LD2`/`LOGOS_FMA`/`LOGOS_CONDSWAP`/`LOGOS_SWAP`/`LOGOS_AFFINE` `=0` | disable the matching fusion / copy-prop pass (all default on). |
| `LOGOS_NO_PINNED_ARGS` (set) | disable pinned-argument passing into compiled functions. |
| `LOGOS_ARRPTR` (set) | allow array-pointer GP pinning when fewer than 4 GP pins are in use. |
| `LOGOS_JIT_TRACE` (set) | trace tier-up decisions. |
| `LOGOS_RDIAG` (set) | region adapter diagnostics. |
| `LOGOS_FUSE_TRACE` (set) | trace the fusion passes. |
| `LOGOS_DUMP_REGION=<head_pc>` | dump the region whose head pc matches. |
| `LOGOS_DUMP_MICRO=<head_pc>` | dump the lowered `MicroOp` stream for that head pc. |

## Dependencies

- Internal: `logicaffeine-compile` (the VM, trait seam) on every target; `logicaffeine-forge` (executable memory, stencils, regalloc, segv_trace) only under `cfg(all(target_arch = "x86_64", not(target_os = "windows")))`.
- External: none beyond the standard library.

## License

MIT License — see [LICENSE.md](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/LICENSE.md).

---
[Docs index](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/docs/README.md) · [Root README](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/README.md) · [Changelog](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/crates/logicaffeine_jit/CHANGELOG.md)
