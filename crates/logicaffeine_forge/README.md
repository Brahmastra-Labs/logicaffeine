# logicaffeine-forge

The machine-code half of EXODIA, the LOGOS copy-and-patch JIT: W^X executable memory, machine-code stencils baked at build time, the assembler that copies and patches them, and a register-allocating x86-64 backend. Copy-and-patch compiles by copying pre-built stencils and patching their holes, so tier-up costs a copy, not an optimizer run; the price is that each stencil is optimized alone, which `regalloc` wins back by emitting a region as one register-allocated function. It decides nothing about what to compile: the VM finds hot code, and `logicaffeine-jit` translates its bytecode and owns the deopt contract.

Part of the [Logicaffeine](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/README.md) workspace, tier 0. Native-only: it compiles on x86-64 System V (Linux and macOS) and to nothing on every other target, and it has no internal dependencies.

## Role in the workspace

`logicaffeine-jit` depends on this crate and lowers hot loop regions and recursive functions through it. Anything outside the supported micro-op subset declines (`None` / `JitCompileError::Unsupported`) and the VM keeps interpreting.

The whole crate is `#![cfg(all(target_arch = "x86_64", not(target_os = "windows")))]`. Stencil extraction needs rustc to emit tail calls at continuation sites, which holds only under the x86-64 System V ABI. On Windows x64 and aarch64 rustc emits a plain `call` there, so a stencil chain would grow the stack; wasm32 cannot run generated machine code at all. Those targets run the bytecode VM, and `build.rs` emits an empty stencil table without extracting. See [execution-and-performance.md](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/docs/execution-and-performance.md) for where the JIT sits among the execution tiers.

## How stencils are made

A stencil is an ordinary Rust function, compiled once at build time into machine code with holes in it. `build.rs` compiles the `#![no_std]` `stencils/int_stencils.rs` (the hand-written `logos_stencil_*` functions) with `rustc --emit=obj`, with no linker, so it works for any installed target. It then extracts each symbol's code and relocations through the `object` crate, normalizing every relocation across Mach-O, ELF and COFF. Holes are undefined `extern` symbols LLVM cannot fold: `logos_hole_cont_N` continuations and `LOGOS_HOLE_I64_N` constants.

Two build gates are hard errors: *leaf purity* (a relocation to a non-hole symbol) and *tail calls* (each continuation site must decode as an unconditional `b`/`jmp`). The build also generates the register-threading variants and emits `STENCILS`, the per-stencil `ST_*` statics, and the `V_BINOP`/`V_FBINOP`/`V_SQRTF`/`V_DIVF`/`V_POPCNT`/`V_LZCNT`/`V_TZCNT`/`V_FLOORF`/`V_CEILF`/`V_ROUNDF`/`V_CHRGUARD`/`V_UTF8LEN`/… variant tables, all `include!`d into `lib.rs`. The bit-count and rounding variants are inline `popcnt`/`lzcnt`/`tzcnt`/`roundsd`, and `logicaffeine-jit` lowers them only on a CPU that reports the instruction.

At run time, compiling a function is `memcpy(stencil bytes)` plus patched relocations, then the page flips to executable.

## Public API

Crate root (`lib.rs`):
- `JitPage`, a page-aligned block of executable memory; `Send + Sync` (sealed read/execute-only after construction). `JitPage::new(code) -> Result<JitPage, JitError>` maps, copies, and seals. `with_layout(len, fill: FnOnce(u64) -> Vec<u8>)` is two-phase for patched code: map first to learn the base, fill bytes against it, then write and seal. `patch_word(offset, value)` rewrites one 8-byte literal-pool word after sealing (the self-call entry; `Err(EmptyCode)` on `MAP_JIT` targets, where callers use the table-indirect call stencil instead). `unsafe as_fn_i64_i64() -> extern "C" fn(i64, i64) -> i64`, plus `as_ptr`, `len`, `alloc_len`, `is_empty`.
- `JitError`: `EmptyCode | Map(io::Error) | Protect(io::Error)`.
- Re-exported from `stencil_model` (private, `include!`d by both `build.rs` and the runtime so it stays dependency-free): `Stencil { name, code, relocs }`, `Reloc { offset, kind: RelocKind, target: HoleId, addend }`, `RelocKind` (`Branch26`, `Page21`, `PageOff12 { scale }`, `GotPage21`, `GotPageOff12`, `Rel32`, `GotRel32`, `Abs64`), `HoleId` (`Cont(u8)`, `ConstI64(u8)`).

`jit`, the straight-line micro-op compiler. `MicroOp` is the subset of the VM's register ops the native tier lowers, over `i64` frame slots (`Slot = u16`; a `Float` is its bit pattern):
- integers: exact `Int` `Add`/`Sub`/`Mul` (overflow side-exits so the VM promotes), `Div`/`Mod` and their constant-divisor forms (`DivPow2`, `MagicDivU`), comparisons and branches, bitwise ops, `Shl`, arithmetic `Shr` and logical `Ushr`, and the bit counts `Popcnt`/`Lzcnt`/`Tzcnt`;
- floats: IEEE `Float` arithmetic with `SqrtF`, `FmaF` and `IntToFloat`, and float-to-`Int` rounding (`FloorToInt`, `CeilToInt`, `RoundToInt` with ties away from zero);
- text: `chr` code points (`ChrGuard` side-exits on an invalid Unicode scalar, `Utf8Len` is the byte length), string append, and `MemMem`;
- memory: array loads, stores, read-modify-writes and swaps (bounds-checked, or unchecked where the optimizer proved the index in range), map get/set/has, and list construction, push and clear;
- runtime helpers: `Host` calls `helper(frame, dst, src, count, imm)` over the frame, which reads the slots `src..src + count`, writes `dst` and answers 0 to side-exit (the native tier builds, tests and releases enum nodes through it);
- calls and self-calls.

`Cmp`/`IOp`/`FOp`/`AffOp`/`RmwOp`/`StrSrc` are its operation tags. The compilers:
- `compile_straightline(ops) -> Result<CompiledChain, JitCompileError>` and `compile_straightline_with(ops, shared_status)` / `compile_straightline_coded(ops, shared_status, deopt_codes, depth_addr)` (the latter adds a per-op precise deopt table emitting `(pc << 2) | 3`).
- `compile_straightline_pinned(ops, pins)` / `compile_straightline_pinned_with(ops, pins, shared_status)` / `compile_straightline_pinned_float(ops, pins, fpins, shared_status)` thread up to 4 general-purpose slots through r0..r3 and up to 6 float slots through XMM f0..f5, keeping hot values in registers.
- `CompiledChain::run_with_frame(&mut [i64]) -> ChainOutcome`, where `ChainOutcome` is `Return(i64) | Deopt(i64)`. A checked op side-exits before any effect: `Int` overflow on `Add`/`Sub`/`Mul`, a zero divisor (and `i64::MIN / -1` on `Div`), an out-of-bounds checked array access, an invalid scalar at `ChrGuard`. The `Deopt` payload is `1` (replay from the entry state) or `(pc << 2) | 3` (a precise exit: resume at bytecode `pc`). Plus `bytes`, `base`, `piece_count`, `patch_marked`, `has_patch_marks`. `JitCompileError` = `Empty | FallsOffTheEnd | BadJumpTarget | Assembly(String) | Unsupported(&str)`. `reference_eval(ops, frame, fuel)` is the differential oracle; `magic_eval` and `BAKED_CALL_DEPTH = 2_500` are exposed.

`buffer`, the copy-and-patch assembler. `JitBuffer::push_stencil(stencil, holes: &[HoleValue]) -> Label`, `mark_patch_hole`, then `finish() -> Result<JitChain, BufferError>` lays out code, an 8-byte literal pool and GOT-style slots, maps a page, patches against the base, writes once, and seals. `JitChain::{from_code, run, run_with_frame, bytes, base, piece_count, patch_marked, has_patch_marks}`. The CPS entry ABI is `unsafe extern "C" fn(base: *mut i64, sp: *mut i64, r0..r3: i64, f0..f5: f64) -> i64`. Also `Label`, `HoleValue`, `BufferError`, `jit_canary_enabled()`.

`regalloc` *(x86-64 only)*, the register-allocating backend. It emits a whole region or function as one x86-64 function with a global per-slot assignment (`Int` in general-purpose registers, `Float` in XMM, everything else in the frame), removing the frame round-trip each stencil piece pays. `compile_region_regalloc`, `compile_region_regalloc_precise`, `compile_function_regalloc` (real System V recursive self-calls through a baked entry cell) and `compile_function_regalloc_precise` (list-parameter functions that mutate in place, with a resume point `(pc << 2) | 3` at every op) each return `Option<CompiledChain>` and are bit-identical to `reference_eval`; the two region entries take the frame slots the caller reads back after the chain returns (`live_out`), which every exit leaves as `reference_eval` would. `regalloc_enabled()` / `simd_enabled()` read the environment knobs below.

`x64asm` *(x86-64 only)*, the minimal encoder the register-allocating backend emits through. `Asm` with `Reg`/`Xmm`/`Cond`/`LabelId` covers moves, three-operand arithmetic, compares and `setcc`, late-bound `jmp`/`jcc`, frame load/store, `idiv`, shifts by immediate and by `cl` (`shl_cl`/`sar_cl`/`shr_cl`), the bit counts `popcnt_rr`/`lzcnt_rr`/`tzcnt_rr`, scalar SSE (`addsd`/`mulsd`/`sqrtsd`/…, `roundsd_rri`, `cvtsi2sd`, `cvttsd2si_rr`), and packed `addpd`/`mulpd`/`cmppd`/…; `resolve() -> Vec<u8>` binds the labels.

`vectorize`: `recognize_elementwise_map(ops) -> Option<MapPlan>` proves a pure element-wise float map (the only shape that vectorizes bit-exactly; reductions reassociate and are rejected), and `emit_map_kernel` / `emit_packed_arith` lower it to a 2-wide packed-double loop. It is off by default: `LOGOS_SIMD=1` turns it on for regions, and it stays off until a bit-identical differential gate certifies it.

`patch`, pure relocation byte-math (`patch_aarch64_branch26`/`_page21`/`_pageoff12`, `patch_x64_rel32`, `patch_abs64`, `is_indirect`; `PatchError`): functions over byte buffers that do not depend on the host architecture. `segv_trace::install()` is an opt-in SIGSEGV/SIGBUS tracer that dumps the faulting address, RIP, and registers (Linux; a no-op elsewhere).

## Environment knobs

Each kill switch falls back to an emission that computes the same result, the two opt-ins (`LOGOS_SIMD`, `LOGOS_LINEAR_SCAN`) are off by default, and the diagnostics only write to stderr or arm guards. [`logicaffeine-jit`](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/crates/logicaffeine_jit/README.md) reads its own knobs on top of these.

| Variable | Default | Effect |
|----------|---------|--------|
| `LOGOS_REGALLOC` | on | `=0` turns off the register-allocating backend (`regalloc_enabled()`); regions and functions fall back to the stencil tier. |
| `LOGOS_SIMD` | off | `=1` compiles a recognized element-wise float map region to a 2-wide packed-double loop (`vectorize`). |
| `LOGOS_LINEAR_SCAN` | off | `=1` lets float slots share a register across disjoint live ranges, in regions with no calls. Measured slower on float-heavy regions, so it stays off. |
| `LOGOS_CALL_WEIGHT` | on | `=0` drops the ranking bonus that puts values live across a call in callee-saved registers. |
| `LOGOS_NO_INLINE_PUSH` | unset | Set to anything but `0`, a list push always calls the runtime helper instead of taking the inline fast path. |
| `LOGOS_NO_OFF_CSE` | unset | `=1` recomputes the scaled index for every array access instead of reusing it across arrays read at the same index. |
| `LOGOS_NO_PTR_HOIST` | unset | `=1` reloads an array's pointer and length from the frame on every access instead of hoisting them out of the loop. |
| `LOGOS_NO_CONST_HOIST` | unset | `=1` materializes a loop-invariant constant on every iteration instead of once in the loop pre-header. Read on every compile, so one process can compare both. |
| `LOGOS_NOTHREAD` | unset | Set, the stencil compiler stops jump threading (branching past an unconditional `Jump` straight to its target). |
| `LOGOS_JIT_CANARY` | unset | `=1` arms sentinel canaries past the operand stack and the region frame, so an overrun fails where it happens. |
| `LOGOS_DUMP_CLASS` | unset | Set to anything but `0`, prints a slot-class and register-placement histogram to stderr for each compiled region that touches a float. |
| `LOGOS_DUMP_LS` | unset | Set together with `LOGOS_LINEAR_SCAN=1`, prints each linear-scan region's ops and live ranges to stderr. |
| `LOGOS_SEGV_TRAP` | unset | Set, `segv_trace::install()` arms the SIGSEGV/SIGBUS tracer. |

## Platform support

The crate builds for x86-64 System V only (Linux and Intel macOS), where W^X executable memory is `mmap(RW)`, then copy, then `mprotect(RX)`. The source also carries the Apple Silicon path (`mmap(PROT_RWX, MAP_JIT)` with per-thread `pthread_jit_write_protect_np` and `sys_icache_invalidate`), the aarch64 Linux I-cache flush, and the Windows path (`VirtualAlloc(RW)`, `VirtualProtect(EXECUTE_READ)`, `FlushInstructionCache`); the crate-level `cfg` compiles them out.

All writes happen inside `JitPage::new` on the constructing thread before any function pointer can escape, so there is no cross-thread W^X hazard. Both the stencil tier and the register-allocating backend (`regalloc`, `x64asm`) run on x86-64 System V.

## Dependencies

Internal: none. External: `libc` (Unix `mmap`/`mprotect`), `object` (a build-dependency for parsing the compiled stencil object files), and `windows-sys`, declared for Windows, where the crate compiles to nothing.

## License

MIT License — see [LICENSE.md](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/LICENSE.md).

---
[Docs index](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/docs/README.md) · [Root README](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/README.md) · [Changelog](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/crates/logicaffeine_forge/CHANGELOG.md)
