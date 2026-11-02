# Changelog

All notable changes to this crate will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

## [0.11.0] - 2026-11-02

### Added
- **`StrSrc::Int`, an `Int` appended to a text accumulator as its decimal text**, beside the byte and constant forms, in the register-allocating backend; `StrSrc::slot`/`slot_mut` name the frame slot a source reads.
- **`MicroOp::Host`, a call to a runtime helper over the frame.** `helper(frame, dst, src, count, imm)` reads the slots `src..src + count`, writes `dst` and answers 0 to side-exit, through one new frame stencil (`logos_stencil_host`) on the per-piece and precise tiers; the precise register-allocating function backend preserves frame operands and caller-saved registers across the helper, then materializes the precise exit when it answers 0. One helper per operation the stencil set has no shape for: the native tier's enum lane builds, tests, reads, retains and releases nodes through it.
- **Bit-count, logical-shift, float→int rounding and `chr` micro-ops.** `jit::MicroOp` gains `Ushr`, `Popcnt`, `Lzcnt`, `Tzcnt`, `FloorToInt`, `CeilToInt`, `RoundToInt` (ties away from zero), `ChrGuard` (side-exits on an invalid Unicode scalar) and `Utf8Len`.
  - **Stencils:** each has a frame stencil, taking `crates/logicaffeine_forge/stencils/int_stencils.rs` from 94 to 103 `logos_stencil_*` functions. On x86_64 the bit counts and rounding are the real `popcnt`, `lzcnt`, `tzcnt` and `roundsd` instructions in inline asm.
  - **Register-threaded tables:** `V_POPCNT`, `V_LZCNT`, `V_TZCNT`, `V_UTF8LEN`, `V_CHRGUARD`, `V_FLOORF`, `V_CEILF` and `V_ROUNDF`; `V_BINOP` gains a thirteenth family, `ushr`.
  - **Register-allocating backend:** `x64asm::Asm` gains `popcnt_rr`, `lzcnt_rr`, `tzcnt_rr`, `shr_cl`, `roundsd_rri` and `cvttsd2si_rr`, and the backend emits `Ushr`, `Popcnt`, `Lzcnt`, `Tzcnt`, `FloorToInt` and `CeilToInt` directly; `RoundToInt`, `ChrGuard` and `Utf8Len` stay on the stencil tier.
  - `reference_eval` covers every new op. Tests: `crates/logicaffeine_tests/tests/forge_word64_rotate_lane.rs`, `crates/logicaffeine_tests/tests/forge_chr_lane.rs`, `forge_numeric_perf_lock`.

### Breaking changes
- **`regalloc::compile_region_regalloc` and `compile_region_regalloc_precise` take a trailing `live_out: &[Slot]`**, the frame slots the caller reads after the chain returns. Migration: pass the slots read back from the frame; `&[]` for a chain observed only through its return value.
- **`jit::MicroOp` gained nine variants, and `V_BINOP` changed type.** `V_BINOP` went from `[[[[&Stencil; 5]; 5]; 5]; 12]` to `[[[[&Stencil; 5]; 5]; 5]; 13]`. `MicroOp` is not `#[non_exhaustive]`, so an exhaustive `match` stops compiling. Migration: add the arms; the new logical right-shift family (`ushr`) is at index 12, after the twelve 0.10.1 families.

### Changed

- Licensed under the MIT License, which replaces the Business Source License 1.1, and the package ships it as `LICENSE.md`.

- **The published crate carries only what builds it.** An anchored `include` allowlist ships `src/`, `build.rs`, `stencils/`, the README and the CHANGELOG; the unit-test bodies live in `crates/logicaffeine_tests/unit/logicaffeine_forge/` behind `#[cfg(test)] #[path]` stubs, so no test code ships. The declared `rust-version` is 1.88 (0.10.1 declared 1.75).

### Fixed
- **A loop-invariant constant moves to the pre-header only when its slot is dead at the loop head.** `liveness` (the backward dataflow behind `liveness_after`) takes the slots every `Return` reads, and `loop_invariant_const_hoists` requires the hoisted slot dead at the head under it, so a read before the write in the loop body, an exit before the first write, or an exit block inside the loop's span that sets a caller-read flag keeps the value the slot entered with. Tests: `regalloc::tests` (`a_constant_read_before_its_write_in_the_loop_is_not_hoisted`, `a_region_leaves_every_live_out_slot_as_the_reference_does` and three more).
- Recursive allocation-scope helpers can execute in register-allocated native functions. Host-call controls cover aliased operands, live integer and float registers, and precise failure materialization (regalloc::tests::host_calls).

## [0.10.0] - 2026-07-08

### Added
- Initial release. The native execution backend of the LOGOS copy-and-patch JIT.
- `JitPage` executable-memory layer: allocates page-aligned memory, copies machine code into it, flips the page to executable, and returns a callable function pointer. Compiling a function at runtime is `memcpy(stencil bytes)` followed by relocation patching.
- Build-time-baked stencil runtime: stencils extracted from `rustc --emit=obj` over `object` with a relocation whitelist and tail-call/leaf-purity gates, byte-exact per-arch patchers, and literal-pool constant holes (Mach-O arm64 has no MOVW/MOVK relocations).
- J1 micro-op compiler and the EXODIA contiguous register-allocating region/function x86-64 codegen tier (`regalloc.rs`, `x64asm.rs`), replacing per-stencil-piece dispatch with allocated registers and contiguous code.
- Platform-correct W^X: macOS/aarch64 (`MAP_JIT` + per-thread `pthread_jit_write_protect_np` + `sys_icache_invalidate`), other Unix (`mmap(RW)` → `mprotect(RX)` + aarch64 I-cache flush), Windows (`VirtualAlloc` → `VirtualProtect` → `FlushInstructionCache`).
- Native-only (`#![cfg(not(target_arch = "wasm32"))]`); behavior tuned by environment variables. See the root CHANGELOG for cross-crate context.
