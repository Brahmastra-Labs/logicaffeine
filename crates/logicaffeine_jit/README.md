# logicaffeine-jit

The LOGOS native tier — wires the copy-and-patch forge JIT into the bytecode VM so hot functions and loop regions tier up to native machine code.

Part of the [Logicaffeine](../../NEW_README.md) workspace. Tier 4, native-only (cfg-gated off `wasm32`). Workspace version 0.9.17, lockstep. Depends on `logicaffeine_compile` (the VM) and, off-wasm, on `logicaffeine_forge` (executable memory + stencils + register allocator). The execution-tier story is in [execution-and-performance.md](../../new_docs/execution-and-performance.md).

## Role in the workspace

The bytecode VM in `logicaffeine-compile` profiles function calls and Main-loop back-edges; when something goes hot it asks its installed `NativeTier` to compile it. **This crate is that tier.** `ForgeTier` translates VM bytecode (`Op`) into the forge's `MicroOp` subset — both whole **functions** (`ChainFn`) and hot loop **regions** (`RegionChain`) — compiles each to a native stencil chain or the contiguous register-allocating backend via `logicaffeine-forge`, and hands the compiled unit back.

Anything outside the supported integer/float subset BAILS (`compile_* -> None`) and stays on bytecode forever — the deopt contract. The whole crate is `#![cfg(not(target_arch = "wasm32"))]`, so WASM builds compile it to nothing and the browser runs pure bytecode.

```
logicaffeine-compile (VM)  ──tier seam──▶  logicaffeine-jit  ──backend──▶  logicaffeine-forge
```

It implements `compile`'s `vm::{NativeTier, NativeFn, RegionFn, Op, Constant, …}` traits and calls `install_native_tier`; off-wasm it drives forge's `jit::{compile_straightline_*, MicroOp, CompiledChain}`, `regalloc::*`, and `segv_trace`. Consumed by `apps/logicaffeine_cli` and the differential `logicaffeine-tests`. No feature flags; library target only.

## The deopt contract (soundness)

Soundness rests on three legs, all differentially tested in `logicaffeine-tests`:
- the kind-inference dataflow (`kind_flow`): params are Int via the entry guard, comparisons are Bool, arithmetic requires Int operands; flow-sensitive join at merges (`Unknown ⊔ k = k`, conflict → `Mixed`, bail on use);
- per-call / per-entry guards: a non-Int argument routes the call back to bytecode;
- the region write-back contract: incoming-dead scratch needs no guard; writes re-box by inferred kind.

Two deopt flavors: classic discard-and-replay-from-head (sound for replay-idempotent regions; the VM rolls pinned buffers back to entry length), and PRECISE `DeoptAt`, which materializes the native frame and resumes AT the faulting op — required for push + in-place `SetIndex` worklist shapes. A `const` build assert pins the kernel's `MAX_CALL_DEPTH` to the forge's baked call-depth so deep native recursion can never side-exit at the wrong depth.

## Public API

- `pub fn install() -> &'static ForgeTier` — install the forge tier process-wide. Idempotent (`OnceLock`); call once at startup. `LOGOS_NATIVE_TIER=0` returns the tier without wiring it into the VM.
- `pub fn segv_trace_install()` — arm the diagnostic SIGSEGV/SIGBUS tracer (no-op unless `LOGOS_SEGV_TRAP` is set).
- `pub struct ForgeTier` (`impl NativeTier`) — the forge-backed tier with compile observability: `function_counts() -> (u32, u32)`, `region_counts() -> (u32, u32)`, `pinned_self_call_count() -> u32`, `regalloc_region_count() -> u32`, `regalloc_precise_region_count() -> u32`, `regalloc_function_count() -> u32`, and `runtime_stats() -> (u64, u64, u64, u64)` (entries, completions, deopts, precise_deopts — actual VM↔native crossings).
- `pub fn adapt(ops: &[Op], constants: &[Constant], register_count: usize) -> Option<(Vec<MicroOp>, usize)>` — translate a whole-program Main body (terminal `Show; Halt`) into the micro subset.
- `pub fn native_alloc_registry_len() -> usize`, `pub static REGION_RUNS`/`REGION_DEOPTS: AtomicU64`, `pub const LOGOS_MEMMEM_DEOPT: i64 = i64::MIN` (the memmem-failure sentinel).
- `logos_rt_*` `pub unsafe extern "C"` runtime helpers the stencils call by baked address (no relocations): `alloc_list_i64`, `list_triple`, `push_{i64,i32,f64,bool}`, `clear_{i64,i32,f64,bool}`, `map_{get_ii,set_ii,has_i}`, `str_append` (mirrors the VM's `add_assign` COW exactly), `memmem`/`memmem_frame`.

## Internals

A single ~7.7k-line `lib.rs`. `ChainFn::call` runs the chain on a 2²¹-slot thread-local arena, handling `Return` / classic `Deopt` / precise `DeoptAt` (`materialize` walks the live native frames). `RegionChain` carries guard / free / write / array sets, hoist guards, and per-resume-pc re-box kinds. `adapt_function` (modes A/B) and `adapt_region` lower to `MicroOp` over the narrow `Kind` lattice (Int/Bool/Float, pinned Int/Float/Bool lists incl. half-width `i32`, maps, ASCII text-as-bytes, mutable-text), run a loop-depth-weighted linear-scan pin selector (`select_pins`), apply array/float fusions (RMW, two-buffer load, FMA, cond/uncond swap, affine index), then prefer the regalloc backend over the per-piece stencil tier. A thread-local `ALLOC_REGISTRY` owns every fresh list — success detaches the returned one, deopt drains all, so replay is leak-free.

## Usage

```rust
logicaffeine_jit::segv_trace_install();
if std::env::var_os("LOGOS_NO_JIT").is_none() {
    logicaffeine_jit::install();
}
```

(`LOGOS_NO_JIT` is the caller-side gate shown above — `apps/logicaffeine_cli`'s convention; the crate's own kill-switch is `LOGOS_NATIVE_TIER=0` inside `install()`.) Live VM constructors in `logicaffeine-compile` then pick the tier up for every program they run.

## Tests & env knobs

33 `#[test]`s live in five `#[cfg(test)]` modules inside `lib.rs` (there is no `tests/` directory): `select_pins_tests` (pin-budget soundness — e.g. a float result reused as an integer array index must NOT be float-pinned, the spectral_norm SIGSEGV regression), `fuse_rmw_tests`, `fuse_ld2_tests`, `fuse_affine_tests` (the array/float fusion peepholes), and `bug002_hazard_tests` (the cross-branch array-value-store hazard detector). The end-to-end VM↔native byte-equivalence and deopt-replay tests live in `logicaffeine-tests`.

```bash
cargo nextest run -p logicaffeine-jit     # this crate's unit tests (native-only, no Z3)
cargo test -p logicaffeine-jit            # same, via cargo test
./scripts/run-all-tests-fast.sh           # full workspace, parity-proven nextest runner
```

Every env var the crate honors is a diagnostic or a kill-switch; all behavior-changing switches fall back **byte-identically** to the per-piece tier (or to bytecode):

| Variable | Effect |
|----------|--------|
| `LOGOS_NATIVE_TIER=0` | `install()` skips wiring the tier into the VM — isolates pure-bytecode wall time. |
| `LOGOS_SEGV_TRAP` (set) | arm the SIGSEGV/SIGBUS tracer from `segv_trace_install()` to localize faults in JIT'd code. |
| `LOGOS_JIT_REGALLOC=0` / `LOGOS_REGALLOC=0` | disable the WS-G contiguous register-allocating backend (function + region). |
| `LOGOS_REGALLOC_PRECISE=0` | disable the precise (list-param, in-place `SetIndex`) regalloc path (Mode B). |
| `LOGOS_LEVERB=0` | disable the Lever B region path. |
| `LOGOS_MEMMEM=0` | disable the `memmem` substring idiom. |
| `LOGOS_COPYPROP`/`LOGOS_RMW`/`LOGOS_LD2`/`LOGOS_FMA`/`LOGOS_CONDSWAP`/`LOGOS_SWAP`/`LOGOS_AFFINE` `=0` | disable the matching fusion / copy-prop pass (all default ON). |
| `LOGOS_NO_PINNED_ARGS` (set) | disable pinned-argument passing into compiled functions. |
| `LOGOS_ARRPTR` (set) | allow array-pointer GP pinning when fewer than 4 GP pins are in use. |
| `LOGOS_BUG002_BAIL=0` | disable the cross-branch array-value-store hazard stopgap (to reproduce the raw crash). |
| `LOGOS_JIT_TRACE` (set) | trace tier-up decisions. |
| `LOGOS_RDIAG` (set) | region adapter diagnostics. |
| `LOGOS_FUSE_TRACE` (set) | trace the fusion passes. |
| `LOGOS_DUMP_REGION=<head_pc>` | dump the region whose head pc matches. |
| `LOGOS_DUMP_MICRO=<head_pc>` | dump the lowered `MicroOp` stream for that head pc. |

## Dependencies

- Internal: `logicaffeine-compile` (the VM, trait seam) on every target; `logicaffeine-forge` (executable memory, stencils, regalloc, segv_trace) only under `cfg(not(target_arch = "wasm32"))`.
- External: none — no third-party crates beyond the standard library.
- Native-only: the entire crate is gated off `wasm32`; WASM builds it to nothing.

## License

Business Source License 1.1 — see [LICENSE.md](../../LICENSE.md).

---
[Docs index](../../new_docs/README.md) · [Root README](../../NEW_README.md) · [Changelog](../../CHANGELOG.md)
