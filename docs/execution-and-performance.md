# Execution & performance

You should not have to pick between a sub-second edit loop and a compiled binary, so LOGOS puts
several back-ends behind one front-end. A program can be interpreted for instant feedback, run on
a bytecode VM that tiers up to native machine code, compiled ahead-of-time to Rust, emitted
directly as a self-contained WebAssembly module, or lowered to C, RISC-V, CUDA and accelerator
targets. The VM with its JIT is benchmarked against V8, and the compiled Rust tier against C and
the rest of a cross-language suite. Every other target is held by its own lock to the VM's
answers, or, for a modeled target, to its own contract.

After this guide you will know which tier runs your program, how to steer the optimizer from the
source, and what keeps the tiers in agreement.

## The five execution tiers

| Tier | Where | Target | Notes |
|------|-------|--------|-------|
| **Tree-walking interpreter** | [`interpreter.rs`](../crates/logicaffeine_compile/src/interpreter.rs) | native + WASM | Direct AST evaluation; sub-second feedback (`largo run --interpret`) |
| **Bytecode VM** | [`vm/`](../crates/logicaffeine_compile/src/vm/) (`compiler.rs`, `machine.rs`) | native + WASM | Register-based; portable; JIT-eligible; the browser engine |
| **Copy-and-patch JIT (EXODIA)** | [`forge/`](../crates/logicaffeine_forge/) + [`jit/`](../crates/logicaffeine_jit/) | x86-64 System V (Linux, macOS) | Tiers up from hot VM functions/loops (`vm/native_tier.rs`); elsewhere the VM runs bytecode |
| **AOT Rust codegen** | [`codegen/`](../crates/logicaffeine_compile/src/codegen/) | native + WASM | The tier benchmarked against C and the cross-language suite (`largo build`) |
| **Direct WASM backend** | [`vm/wasm/`](../crates/logicaffeine_compile/src/vm/wasm/) | WebAssembly | Emits a self-contained `.wasm` module with no `rustc`/`cargo`/`wasm-bindgen` in the loop — milliseconds (`largo build --emit wasm`); `--emit wasm-linked` links the real Rust runtime via `rust-lld` for exact arbitrary-precision integers and the exact extension's values |

Serving HTTP is one of the places the tiers meet. A `Serve` statement runs on the interpreter,
the bytecode virtual machine and the compiled Rust tier, and a differential lock holds the three
byte-identical: they answer the same request with the same bytes. They share one production
lifecycle (bounded admission, body and read-deadline caps, graceful drain) read from one place. It
is refused on the device backends, which have no host sockets. See [Web](web.md).

## Other targets

Beside the tiers, `largo emit <target>` writes the program for another machine; the
[CLI guide](cli.md) lists every target's flags and output files. The lock named in each row holds the
target to the VM, or a modeled target to its own contract. The halves that execute an artifact need
that target's toolchain or device (a C compiler, `qemu-riscv64`, `nvcc` and a GPU, TT-Metalium,
ngspice, the Spatial toolchain). `op_backend_matrix_lock` classifies every bytecode op against C,
Rust, RISC-V, WASM, CUDA, Tenstorrent and the spatial fabric and holds each backend to a coverage
floor. A program the exact extension reaches runs on C, linked WebAssembly and RISC-V, which link the
extension's value runtime beside it; the Silicon ISA, CUDA, Tenstorrent, analog, Plasticine and RDU tiers
refuse one by name, since nothing they run on can call that runtime (`exact_on_devices_lock`).

| Target | Where | Command | Notes |
|--------|-------|---------|-------|
| **C** | [`codegen_c/`](../crates/logicaffeine_compile/src/codegen_c/) | `largo emit c` | One self-contained C file with its runtime embedded; a general backend, described below the table (`c_numeric_tower_lock`, `crdt_backend_lock`) |
| **RISC-V** | [`codegen_riscv/`](../crates/logicaffeine_compile/src/codegen_riscv/) | `largo emit riscv`, `largo build --emit riscv` | RV64GC assembly plus a small C runtime (`logos_rt.c`); link with a RISC-V GCC, run under `qemu-riscv64` (`riscv_aot_lock`). A program the exact extension reaches also links that extension's value runtime, built for RISC-V and linked statically (`exact_on_riscv_lock`) |
| **Silicon ISA** | [`vm/isa.rs`](../crates/logicaffeine_compile/src/vm/isa.rs) | `largo build --emit isa` | The register-VM opcode set as a chip ISA listing, every instruction tagged with its reversibility class, written as `<entry>.isa` (`isa_aot_lock`) |
| **WASM DOM** | [`vm/wasm/`](../crates/logicaffeine_compile/src/vm/wasm/) | `largo emit wasm-dom` | A self-contained `.wasm` page plus the `.html`/`.js` host that wires its `dom_*` imports to the browser (see [Frontend](frontend.md)); the page's DOM operations match the tree-walker's and the VM's (`wasm_dom`, `wasm_dom_emit`) |
| **CUDA** | [`codegen_cuda/`](../crates/logicaffeine_compile/src/codegen_cuda/) | `largo emit cuda` | One `.cu` running the whole program on one GPU thread (the C tier's body lifted to `__device__`), a host launcher, a `Makefile` and the C oracle it is checked against; a concurrent program runs on the stepped scheduler (`cuda_aot_lock`). Tensor sentences also lower to parallel GPU kernels through the library entry points |
| **Tenstorrent** | [`codegen_tt/`](../crates/logicaffeine_compile/src/codegen_tt/) | `largo emit tenstorrent` | A single-Tensix-core TT-Metalium program: the C tier's compute body as a kernel on one baby RISC-V core, a host orchestrator, CMake glue and the C oracle. Tasks, pipes, select and sleep are refused (`tenstorrent_aot_lock`) |
| **Analog** | [`codegen_ana/`](../crates/logicaffeine_compile/src/codegen_ana/) | `largo emit analog` | A modeled mixed-signal bundle for a float-dataflow region: tile netlist, gain schedule, range certificate, itemized error budget and a behavioral ngspice netlist; an integer-only program is refused (`analog_target_lock`) |
| **Plasticine** | [`codegen_sva/`](../crates/logicaffeine_compile/src/codegen_sva/) | `largo emit plasticine` | A counted loop as Spatial source for the open RDU toolchain, a regression app checking the VM's answer, a placement and a modeled report (`emit_plasticine`) |
| **RDU** | [`spatial/`](../crates/logicaffeine_compile/src/spatial/) | `largo emit rdu` | The SambaNova RDU lane chosen by program shape (tensor, loop or whole program): a certified neutral placement, its certificate and a modeled report; vendor configuration is gated on the vendor SDK (`emit_rdu_lanes`) |

**The C backend.** `largo emit c` writes one self-contained C file carrying the stdlib prelude and
the exact numeric tower: an `Int` promotes to a big integer on provable overflow, in growing
accumulators and in every `**`, beside rationals, complex and modular values. It also carries money,
quantities, UUIDs, words and temporal values, collections, records and enums, CRDT counters, and
concurrency on the ported scheduler ([Concurrency](concurrency.md)). Where C cannot prove an
`Int`'s operands bounded, its arithmetic wraps; that divergence is ledgered in
[`c_known_divergences.txt`](../crates/logicaffeine_tests/tests/gauntlet/c_known_divergences.txt). A
Rust-only native kernel, or a CRDT kind C does not lower, is refused by name.

The multi-core and accelerator lowerings (Tensix meshes, the persistent CUDA kernel, the
reconfigurable fabric and the RDU) are backends of one spatial IR
([`spatial/`](../crates/logicaffeine_compile/src/spatial/)): stages over bounded buffers joined by
typed handshakes and placed on a described mesh. Each lowering refuses by name what it cannot
place. The
[`codegen_sva/`](../crates/logicaffeine_compile/src/codegen_sva/) module is also English →
SystemVerilog Assertions and runtime monitors for hardware verification (see
[Proof & verification](proof-and-verification.md)), and generates Verilog RTL for the hardware
targets: the soft cores, the reconfigurable fabric and the whole-program engine.

## How tier-up works

The VM runs bytecode. When a function or loop gets hot, **EXODIA**, the copy-and-patch JIT
(`ForgeTier`, installed via [`logicaffeine_jit::install`](../crates/logicaffeine_jit/src/lib.rs)),
compiles it to native machine code by stitching pre-built stencils together and patching in the
live operands, so compilation stays fast and execution is native. A register-allocating x86-64
backend (`regalloc.rs`) sits above per-stencil dispatch. JIT regions carry **deoptimization
guards**: if a region sees data it didn't specialize for, it bails safely back to bytecode. Set
`LOGOS_NO_JIT` to stay on the VM. (`vm/native_tier.rs`, `vm/aot_tier.rs`.)

In the browser, the interpreter and VM run in WebAssembly and produce byte-identical results to
native; that is what powers the in-page [Studio](studio-and-learn.md). The direct WASM backend is
the complement: instead of running the engine *in* WASM, it compiles the program *to* a WASM module
(locked to VM/interpreter behavior by the `wasm_aot_lock` differentials); the `wasm-jit` feature
additionally lets JIT'd hot regions execute in-browser (`vm/wasm/region_jit.rs`).

## The optimizer

Optimizations are registry-driven. Each pass is registered in
[`logicaffeine_language/src/optimization.rs`](../crates/logicaffeine_language/src/optimization.rs)
(`REGISTRY`, one row per optimization: its keyword, the paths it touches, its cost, whether it emits
`unsafe`, and what it requires, conflicts with and preempts) and toggled through a single
`OptimizationConfig`. The passes themselves live in
[`compile/src/optimize/`](../crates/logicaffeine_compile/src/optimize/) and the code generators in
[`codegen/`](../crates/logicaffeine_compile/src/codegen/), covering bounds-check elimination, loop
transforms, scalarization, de-`Rc`, inlining, dense-map specialization, and more.

Source decorators select them by keyword, and every tier's compile path honors them: AOT-Rust
(`largo build`, `largo emit rust`), C, the VM, WASM and the Silicon ISA.

| Decorator | Effect |
|-----------|--------|
| `## No <keyword>` | Turns the optimization off for the function that follows, or program-wide when it sits before `## Main`; `## No Optimize` turns every one off |
| `## Require <keyword>` | Compilation fails, naming the keyword, when the optimization did not fire for the program, so a dependence on an optimization becomes an error. `largo run --interpret` refuses it up front: the check belongs to the compiled path (`largo run`, `largo build`, `largo emit`) |
| `## Tier <keyword> <pin>` | Pins the hotness tier at which the run path's tiered optimizer pays for it: `eager`, `never`, or a tier `t0`–`t3`. `## No` wins over any pin |

An unknown keyword is ignored. The ambient equivalents are `LOGOS_OPT_OFF=<kw>,<kw>` (and
`LOGOS_OPT=off` for all), and `LOGOS_TIER_PIN=<kw>:<pin>,…`, which a `## Tier` decorator overrides.
The keywords, in the groups the registry files them under (the headings `largo opts` prints):

| Group | Keywords |
|-------|----------|
| Inlining & calls | `memo`, `tco`, `specialize`, `comptime`, `inline`, `unfold`, `defunctionalize`, `foldfuse` |
| Peephole | `peephole` |
| Arrays & memory | `borrow`, `scalarize`, `affine`, `interleave`, `unbox`, `hoistborrows`, `regpromote`, `variantbox`, `bitpack`, `stridefill` |
| Loops | `loophoist`, `unroll`, `loopsplit`, `closedform`, `fuse`, `loopcse`, `interchange`, `tile`, `accumbound`, `divmodfuse`, `specializeloop`, `sortnest`, `divisorsieve`, `tracememo` |
| Redundancy | `cse`, `deadcode`, `sccp`, `probefuse` |
| Number representation | `narrow`, `narrowvm`, `narrowmap`, `densemap`, `elemtype`, `fastdiv`, `floatstrength`, `intstrength` |
| Bounds & checks | `oracle`, `oraclehints`, `unchecked`, `hintversion` |
| Strings & SIMD | `simd`, `cascade`, `indexstring`, `capscale`, `asciiclassmask` |
| Search-space | `symmetry`, `popcount`, `saturate`, `supercompile` |
| Bignum | `bignumbyref`, `bignuminplace` |
| Bits | `bitintrinsics` |
| Text | `textbytesview`, `stringaccumreuse`, `textfrombytesbuilder`, `textslicereturn`, `stringconcatreuse`, `stringrepeatloop`, `radixdigitbuffer` |
| Math | `simdreduce` |
| Arithmetic | `strengthreducepow2`, `branchlessaccumulate`, `wideaccumulator`, `masknarrow`, `modwrap`, `branchlessselect` |
| Bytes | `byteslicereturn` |

```logos
## Require memo

## To fib (n: Int) -> Int:
    If n is less than 2:
        Return n.
    Return fib(n - 1) + fib(n - 2).

## Main
Show fib(10).
```

Memoization fires on the double recursion, so this compiles; the same decorator over a program with
nothing to memoize is a compile error.

To see exactly which optimizations *fired* for a program:

```bash
largo opts src/main.lg          # human-readable
largo opts src/main.lg --json   # keyword list
```

## Benchmarks

The [`benchmarks/`](../benchmarks/) directory holds the harness and recorded results
([`benchmarks/results/`](../benchmarks/results/)); its [README](../benchmarks/README.md) explains how
each suite runs and the fairness rules every implementation follows. LOGOS is measured two ways:

- **AOT**: `largo build --release` against C, C++, Rust, Zig, Go, Java, JavaScript, Python, Ruby
  and Nim on a shared set of programs, every output verified before anything is timed.
- **VM + JIT**: `largo run --interpret` against Node.js (V8), both running the same naive
  algorithm, to track the non-compiled path.

The current charts are in the root README's [Benchmarks](../README.md#benchmarks) section,
regenerated from the JSON under `benchmarks/results/`, and the interactive suite is on the
[`/benchmarks`](https://logicaffeine.com/benchmarks) page. This guide quotes no figures, because
they move with every optimization. One distinction to read them with: where the optimizer proves a
shortcut, compiled LOGOS skips work that C still executes, so the charts report those programs
apart from the same-algorithm comparisons.

## Verifying the compiler itself

Three mechanisms keep the tiers honest:

- **Differential tests** (`e2e_differential`, `e2e_feature_matrix`) assert the tree-walker, the VM
  and the compiled Rust agree on output; `op_backend_matrix_lock` and the per-target locks above
  extend the same contract to every other backend.
- **Tier divergence as a verdict.** `tier_differential` runs a program on the tree-walker and the VM
  and classifies any disagreement as a compiler bug with a ready-to-file reproduction
  (`tier_divergence_drill`); `largo test --tiers all` applies it to every passing `## Test`.
- **Translation validation** ([`logicaffeine_tv`](../crates/logicaffeine_tv/README.md)) symbolically
  executes the LOGOS source into bitvector/boolean terms and proves with Z3 that the encoding agrees
  with the tree-walker on the ordered `Show` outputs and the error outcome, over straight-line
  `Int`/`Bool` programs and a determinate concurrency fragment. The emitted-Rust side of the
  equivalence has no symbolic executor, so source ↔ emitted-Rust equivalence is not closed.

## See also

- The language being executed → [Imperative mode](imperative-mode.md)
- The crate layout and gating → [Architecture](architecture.md)

## Where this is tested

The execution tiers live in [`crates/logicaffeine_compile/src/`](../crates/logicaffeine_compile/src/),
the JIT in [`crates/logicaffeine_forge/`](../crates/logicaffeine_forge/README.md) and
[`crates/logicaffeine_jit/`](../crates/logicaffeine_jit/README.md), and the harness in
[`benchmarks/`](../benchmarks/). Each target's lock is named in its table row above, and
`op_backend_matrix_lock` holds every backend to its coverage floor. `decorator_through_largo`
holds the optimization decorators on the real `largo` compile path and on the C, ISA and WASM
compilers, and `e2e_require_optimization` holds `## Require` to failing when its optimization does
not fire.

---
[Docs index](README.md) · [Root README](../README.md) · [Changelog](../CHANGELOG.md)
