# logicaffeine-forge

Copy-and-patch JIT for LOGOS — the executable-memory layer and stencil runtime.

`JitPage` allocates page-aligned memory, copies machine code into it, makes it
executable, and hands back a callable function pointer. At runtime, compiling a
function is `memcpy(stencil bytes)` + patch relocations, then flip the page to
executable — no per-function assembler, just stencils stamped out and wired
together.

## W^X model

- **macOS / aarch64 (Apple Silicon):** `mmap(PROT_RWX, MAP_JIT)` + per-thread
  `pthread_jit_write_protect_np` toggling + a mandatory `sys_icache_invalidate`.
- **Other Unix (Linux, Intel macOS):** `mmap(RW)` → copy → `mprotect(RX)`, with an
  I-cache flush on aarch64 Linux.
- **Windows:** `VirtualAlloc(RW)` → copy → `VirtualProtect(EXECUTE_READ)` →
  `FlushInstructionCache`.

Native only. A copy-and-patch JIT emits raw machine code and cannot run in the
WASM sandbox; the browser uses the bytecode VM instead.

## License

Licensed under BUSL-1.1. See the workspace root for details.
