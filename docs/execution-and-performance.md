# Execution & performance

One front-end, several back-ends. A LOGOS program can be interpreted for instant feedback, run on a
bytecode VM that tiers up to native machine code, compiled ahead-of-time to Rust, or emitted
directly as a self-contained WebAssembly module. All of it is benchmarked.

Source of truth: [`crates/logicaffeine_compile/src/`](../crates/logicaffeine_compile/src/) (the
tiers), [`crates/logicaffeine_forge/`](../crates/logicaffeine_forge/README.md) +
[`crates/logicaffeine_jit/`](../crates/logicaffeine_jit/README.md) (the JIT), and
[`benchmarks/`](../benchmarks/).

## The five execution tiers

| Tier | Where | Target | Notes |
|------|-------|--------|-------|
| **Tree-walking interpreter** | [`interpreter.rs`](../crates/logicaffeine_compile/src/interpreter.rs) | native + WASM | Direct AST evaluation; sub-second feedback (`largo run --interpret`) |
| **Bytecode VM** | [`vm/`](../crates/logicaffeine_compile/src/vm/) (`compiler.rs`, `machine.rs`) | native + WASM | Register-based; portable; JIT-eligible; the browser engine |
| **Copy-and-patch JIT (EXODIA)** | [`forge/`](../crates/logicaffeine_forge/) + [`jit/`](../crates/logicaffeine_jit/) | native only | Tiers up from hot VM functions/loops (`vm/native_tier.rs`) |
| **AOT Rust codegen** | [`codegen/`](../crates/logicaffeine_compile/src/codegen/) | native + WASM | The performance gold standard (`largo build`) |
| **Direct WASM backend** | [`vm/wasm/`](../crates/logicaffeine_compile/src/vm/wasm/) | WebAssembly | Emits a self-contained `.wasm` module with no `rustc`/`cargo`/`wasm-bindgen` in the loop — milliseconds (`largo build --emit wasm`); `--emit wasm-linked` links the real `logicaffeine_base::BigInt` runtime via `rust-lld` for exact arbitrary-precision integers |

Two special-purpose emitters sit beside the tiers:

| Emitter | Where | Notes |
|---------|-------|-------|
| **AOT C codegen** | [`codegen_c/`](../crates/logicaffeine_compile/src/codegen_c/) | Experimental benchmark/embedding subset |
| **SVA/PSL codegen** | [`codegen_sva/`](../crates/logicaffeine_compile/src/codegen_sva/) | English → SystemVerilog Assertions + runtime monitors for hardware verification (see [Proof & verification](proof-and-verification.md)) |

## How tier-up works

The VM runs bytecode; when a function or loop gets hot, the **copy-and-patch JIT** — the tier is
named **EXODIA** — (`ForgeTier`, installed via
[`logicaffeine_jit::install`](../crates/logicaffeine_jit/src/lib.rs)) compiles it to native machine
code by stitching pre-built stencils and patching in the live operands — fast compilation, native
execution — with a register-allocating x86-64 backend (`regalloc.rs`) above per-stencil dispatch. JIT regions carry **deoptimization guards**: if a region sees
data it didn't specialize for, it bails safely back to bytecode. Set `LOGOS_NO_JIT` to stay on the
VM. (`vm/native_tier.rs`, `vm/aot_tier.rs`.)

In the browser, the interpreter and VM run in WebAssembly and produce byte-identical results to
native — that is what powers the in-page [Studio](studio-and-learn.md). The direct WASM backend is
the complement: instead of running the engine *in* WASM, it compiles the program *to* a WASM module
(locked to VM/interpreter behavior by the `wasm_aot_lock` differentials); the `wasm-jit` feature
additionally lets JIT'd hot regions execute in-browser (`vm/wasm/region_jit.rs`).

## The optimizer

Optimizations are registry-driven. Each pass is registered in
[`logicaffeine_language/src/optimization.rs`](../crates/logicaffeine_language/src/optimization.rs) and
toggled through a single `OptimizationConfig`, with per-program control via `## No <X>` decorators in
the source. The passes themselves live in
[`compile/src/optimize/`](../crates/logicaffeine_compile/src/optimize/) and the code generators in
[`codegen/`](../crates/logicaffeine_compile/src/codegen/) — covering bounds-check elimination, loop
transforms, scalarization, de-`Rc`, inlining, dense-map specialization, and more.

To see exactly which optimizations *fired* for a program:

```bash
largo opts src/main.lg          # human-readable
largo opts src/main.lg --json   # keyword list
```

## Benchmarks

The [`benchmarks/`](../benchmarks/) directory holds the harness and recorded results
([`benchmarks/results/`](../benchmarks/results/)). LOGOS is measured two ways:

- **AOT** (compiled Rust/C) against C, Rust, Zig, Go, Java, JavaScript, Python, Ruby, and Nim on a
  shared set of compute kernels.
- **Interpreter + VM + JIT** against V8, to track the non-compiled path.

Results are published live on the [`/benchmarks`](https://logicaffeine.com/benchmarks) page; the JSON
under `benchmarks/results/` is the raw data. Because numbers move with every optimization, this guide
points at the live data rather than freezing a figure here. Broadly: the AOT path reaches C-level
performance on same-algorithm kernels (and wins outright where it can prove a better algorithm),
while the VM+JIT path is competitive with V8 and beats it on float-heavy work.

## Verifying the compiler itself

Two mechanisms keep the tiers honest:

- **Differential tests** (`e2e_differential`, `e2e_feature_matrix`) assert the interpreter, VM, JIT,
  and AOT paths agree on output.
- **Translation validation** ([`logicaffeine_tv`](../crates/logicaffeine_tv/README.md)) uses SMT to
  prove the emitted Rust matches the LOGOS source.

## See also

- The language being executed → [Imperative mode](imperative-mode.md)
- The crate layout and gating → [Architecture](architecture.md)

---
[Docs index](README.md) · [Root README](../README.md) · [Changelog](../CHANGELOG.md)
