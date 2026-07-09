# logicaffeine-forge

The copy-and-patch JIT's executable-memory layer: page-aligned W^X executable
pages, build-time-baked machine-code stencils, the copy-and-patch assembler, the
J1 micro-op compiler, and the EXODIA contiguous register-allocating x86-64
codegen tier.

Part of the [Logicaffeine](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/README.md) workspace. Tier 0, native-only
(cfg-gated off wasm32); no internal dependencies. Wired into the VM by
logicaffeine_jit.

## Role in the workspace

`logicaffeine-jit` (the LOGOS native tier) depends on this crate and lowers hot
integer/float regions and recursive functions to machine code; anything outside
the supported subset declines (`None` / `JitCompileError::Unsupported`) and the
VM keeps interpreting. The whole crate is `#![cfg(not(target_arch = "wasm32"))]`
— a copy-and-patch JIT emits raw machine code and cannot run in the WASM sandbox,
so the browser falls back to the bytecode VM.

The model is copy-and-patch: compiling a function is `memcpy(stencil bytes)` +
patch relocations, then flip the page to executable. `build.rs` compiles the
`#![no_std]` `stencils/int_stencils.rs` (94 hand-written `logos_stencil_*` fns)
with `rustc --emit=obj` — no linker, so it works for any installed target — then
extracts each symbol's code + relocations via the `object` crate, normalizing
every reloc across Mach-O / ELF / COFF. Holes are undefined `extern` symbols
(`logos_hole_cont_N` continuations, `LOGOS_HOLE_I64_N` constants) LLVM cannot
fold. Two hard build gates: **leaf purity** (a reloc to a non-hole symbol is an
error) and **tail calls** (each continuation site must decode as an
unconditional `b`/`jmp`). The build also generates the EXODIA register-threading
variants and emits `STENCILS`, the per-stencil `ST_*` statics, and the
`V_BINOP`/`V_FBINOP`/`V_SQRTF`/`V_DIVF`/… variant tables (all `include!`d into
`lib.rs`). See [execution-and-performance.md](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/docs/execution-and-performance.md).

## Public API

Crate root (`lib.rs`):
- `JitPage` — a page-aligned block of executable memory; `Send + Sync` (sealed
  read/execute-only after construction). `JitPage::new(code) -> Result<JitPage,
  JitError>` maps, copies, and seals. `with_layout(len, fill: FnOnce(u64) ->
  Vec<u8>)` is two-phase for PATCHED code: map first to learn the base, fill
  bytes against it, then write+seal. `patch_word(offset, value)` rewrites one
  8-byte literal-pool word post-seal (the self-call entry; `Err(EmptyCode)` on
  MAP_JIT targets, where callers use the table-indirect call stencil instead).
  `unsafe as_fn_i64_i64() -> extern "C" fn(i64, i64) -> i64`, plus `as_ptr`,
  `len`, `alloc_len`, `is_empty`.
- `JitError` — `EmptyCode | Map(io::Error) | Protect(io::Error)`.
- Re-exported from `stencil_model` (private, `include!`d by both `build.rs` and
  the runtime so it stays dependency-free): `Stencil { name, code, relocs }`,
  `Reloc { offset, kind: RelocKind, target: HoleId, addend }`, `RelocKind`
  (`Branch26`, `Page21`, `PageOff12 { scale }`, `GotPage21`, `GotPageOff12`,
  `Rel32`, `GotRel32`, `Abs64`), `HoleId` (`Cont(u8)`, `ConstI64(u8)`).

`jit` — the J1 straight-line micro-op compiler. `MicroOp` is the int/float subset
of the VM's register ops (`Slot = u16`); `Cmp`/`IOp`/`FOp`/`AffOp`/`RmwOp`/`StrSrc`
are its operation tags. The compilers:
- `compile_straightline(ops) -> Result<CompiledChain, JitCompileError>` and
  `compile_straightline_with(ops, shared_status)` / `…_coded(ops, shared_status,
  deopt_codes, depth_addr)` (the latter adds a per-op PRECISE deopt table
  emitting `(pc << 2) | 3`).
- `compile_straightline_pinned(ops, pins)` / `…_pinned_with(ops, pins,
  shared_status)` / `…_pinned_float(ops, pins, fpins, shared_status)` thread up
  to 4 GP slots through r0..r3 and up to 6 float slots through XMM f0..f5,
  keeping hot values register-resident.
- `CompiledChain::run_with_frame(&mut [i64]) -> ChainOutcome` where
  `ChainOutcome` is `Return(i64) | Deopt(i64)` (checked `Div`/`Mod` side-exit on
  a zero divisor); plus `bytes`, `base`, `piece_count`, `patch_marked`,
  `has_patch_marks`. `JitCompileError` = `Empty | FallsOffTheEnd | BadJumpTarget
  | Assembly(String) | Unsupported(&str)`. `reference_eval(ops, frame, fuel)` is
  the differential oracle; `magic_eval` and `BAKED_CALL_DEPTH = 2_500` are exposed.

`buffer` — the copy-and-patch assembler. `JitBuffer::push_stencil(stencil,
holes: &[HoleValue]) -> Label`, `mark_patch_hole`, then `finish() ->
Result<JitChain, BufferError>` lays out code + an 8-byte literal pool + GOT-style
slots, maps a page, patches against the base, writes once, and seals.
`JitChain::{from_code, run, run_with_frame, bytes, base, piece_count,
patch_marked, has_patch_marks}`. The CPS entry ABI is `unsafe extern "C"
fn(base: *mut i64, sp: *mut i64, r0..r3: i64, f0..f5: f64) -> i64`. Also
`Label`, `HoleValue`, `BufferError`, `jit_canary_enabled()`.

`regalloc` *(x86-64 only)* — the EXODIA backend: emits a whole region/function as
ONE x86-64 function with a global per-slot assignment (INT in GPRs, FLOAT in XMM,
else frame), removing the per-piece frame round-trip. `compile_region_regalloc`,
`compile_region_regalloc_precise`, `compile_function_regalloc` (real SysV
recursive self-calls via a baked entry cell), `compile_function_regalloc_precise`
(list-param mode-B, per-op resume `(pc << 2) | 3` for in-place-mutating
functions) — each returns `Option<CompiledChain>` and is bit-identical to
`reference_eval`. `regalloc_enabled()` / `simd_enabled()` read the env toggles.

`x64asm` *(x86-64 only)* — the minimal encoder the regalloc backend emits
through: `Asm` with `Reg`/`Xmm`/`Cond`/`LabelId`, moves, three-operand
arithmetic, compares/`setcc`, late-bound `jmp`/`jcc`, frame load/store, `idiv`,
scalar SSE (`addsd`/`mulsd`/`sqrtsd`/…), and packed `addpd`/`mulpd`/`cmppd`/…;
`resolve() -> Vec<u8>` binds the labels.

`vectorize` — `recognize_elementwise_map(ops) -> Option<MapPlan>` proves a pure
element-wise float map (the only bit-exactly vectorizable shape; reductions
reassociate and are rejected); `emit_map_kernel` / `emit_packed_arith` lower it
to a 2-wide packed-double loop.

`patch` — pure relocation byte-math (`patch_aarch64_branch26`/`_page21`/
`_pageoff12`, `patch_x64_rel32`, `patch_abs64`, `is_indirect`; `PatchError`),
compiled and tested on every host so the x86-64 patchers run on an arm64 box and
vice versa. `segv_trace::install()` is an opt-in SIGSEGV/SIGBUS tracer dumping
the faulting address, RIP, and registers.

## Platform support

W^X executable memory, per platform:
- **macOS / aarch64 (Apple Silicon):** `mmap(PROT_RWX, MAP_JIT)` + per-thread
  `pthread_jit_write_protect_np` toggling + a mandatory `sys_icache_invalidate`.
- **Other Unix (Linux, Intel macOS):** `mmap(RW)` → copy → `mprotect(RX)`, with
  an inline-asm I-cache flush on aarch64 Linux (`mprotect` does not flush it).
- **Windows:** `VirtualAlloc(RW)` → `VirtualProtect(EXECUTE_READ)` →
  `FlushInstructionCache`.

All writes happen inside `JitPage::new` on the constructing thread before any
function pointer can escape, so there is no cross-thread W^X hazard. The
copy-and-patch stencil tier runs on both x86-64 and aarch64; the EXODIA
contiguous codegen (`regalloc`/`x64asm`) is x86-64 only.

## Dependencies

Internal: none. External: `libc` (Unix mmap/mprotect/icache), `windows-sys`
(Windows VirtualAlloc/VirtualProtect/FlushInstructionCache), and `object` (a
build-dependency for parsing the compiled stencil object files).

## License

Business Source License 1.1 — see [LICENSE.md](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/LICENSE.md).

---
[Docs index](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/docs/README.md) · [Root README](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/README.md) · [Changelog](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/CHANGELOG.md)
