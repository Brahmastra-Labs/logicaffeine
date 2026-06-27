# Changelog

All notable changes to this crate will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

## [0.9.17] - 2026-06-11

### Added
- Initial release. The native execution backend of the LOGOS copy-and-patch JIT.
- `JitPage` executable-memory layer: allocates page-aligned memory, copies machine code into it, flips the page to executable, and returns a callable function pointer. Compiling a function at runtime is `memcpy(stencil bytes)` followed by relocation patching.
- Build-time-baked stencil runtime: stencils extracted from `rustc --emit=obj` over `object` with a relocation whitelist and tail-call/leaf-purity gates, byte-exact per-arch patchers, and literal-pool constant holes (Mach-O arm64 has no MOVW/MOVK relocations).
- J1 micro-op compiler and the EXODIA contiguous register-allocating region/function x86-64 codegen tier (`regalloc.rs`, `x64asm.rs`), replacing per-stencil-piece dispatch with allocated registers and contiguous code.
- Platform-correct W^X: macOS/aarch64 (`MAP_JIT` + per-thread `pthread_jit_write_protect_np` + `sys_icache_invalidate`), other Unix (`mmap(RW)` → `mprotect(RX)` + aarch64 I-cache flush), Windows (`VirtualAlloc` → `VirtualProtect` → `FlushInstructionCache`).
- Native-only (`#![cfg(not(target_arch = "wasm32"))]`); behavior tuned by environment variables. See the root CHANGELOG for cross-crate context.
