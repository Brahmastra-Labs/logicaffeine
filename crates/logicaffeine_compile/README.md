# logicaffeine-compile

The LOGOS compiler: it takes a program `logicaffeine-language` has parsed, checks and optimizes it, then runs it on an execution tier or translates it for another machine. Every lane is built to one rule: agree with the VM or refuse the program by name, so a backend that cannot lower a construct names it instead of shipping code that computes something else. It does not lex or parse, and the JIT's machine code lives in `logicaffeine-jit` and `logicaffeine-forge`.

Part of the [Logicaffeine](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/README.md) workspace, tier 4. It builds for native targets and `wasm32` alike; the Z3-backed verification pass sits behind the `verification` feature.

## Role in the workspace

Follow one program through. `logicaffeine-language` lexes and parses the English into an AST. This crate assembles it with its imports and the standard-library functions it reaches (`loader`, `analysis::assemble`), then runs the analysis passes the path needs: type checking, escape, ownership, dimensions, liveness, the call graph, called-name resolution and import hygiene. The optimizer comes next, every pass behind one `OptimizationConfig`: abstract-interpretation facts, GVN, LICM, DCE, inlining, scalarization, the e-graph and supercompilation. From there the program is either run or translated.

```text
AST ──▶ Assemble (imports, prelude) ──▶ Analysis (type check, escape, ownership, liveness, call graph, …)
    ──▶ Optimize (abstract-interpretation facts, GVN, LICM, DCE, inline, scalarize, e-graph)
    ──▶ run:       Interpreter | Bytecode VM (+ native JIT, background AOT)
    ──▶ translate: Rust | C | WASM | RISC-V | CUDA | Tenstorrent | spatial IR (analog, Plasticine, RDU) | SVA / RTL
```

The tests hold each lane to the VM. The tree-walker and the VM run the same programs to byte-identical output (`strict_engine_lock`). `riscv_aot_lock` and `cuda_aot_lock` classify every VM op as lowered or refused for their backend and, where the toolchain is present, run a corpus against the tree-walker. A program outside a backend's fragment gets a typed refusal naming the construct. The analog lane is the one approximate target: it agrees with the VM within an error budget the compiler emits beside the netlist. One known hole remains: AOT-Rust does not lower `Stream` yet, and leaves a marker comment where the send would be instead of refusing.

The crate feeds `largo`, the web Studio (through `ui_bridge`), the language server, translation validation, and the native JIT. See [imperative-mode.md](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/docs/imperative-mode.md) for the language the pipeline compiles and [execution-and-performance.md](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/docs/execution-and-performance.md) for how the execution tiers relate.

## Example

Run one program on the tree-walker and on the VM, then hand the VM a program it cannot run:

```rust
use logicaffeine_compile::{run_strict_tw, run_strict_vm, EngineId};

let src = concat!(
    "## Main\n",
    "    Let mutable total be 0.\n",
    "    Repeat for i from 1 to 5:\n",
    "        Set total to total + i.\n",
    "    Show total.\n",
    "    Show \"done\".\n",
);
let tw = run_strict_tw(src);
let vm = run_strict_vm(src);
assert_eq!(tw.engine_ran, Some(EngineId::TreeWalker));
assert_eq!(vm.engine_ran, Some(EngineId::Vm));
assert_eq!(tw.output, "15\ndone");
assert_eq!(vm.output, tw.output);

let sleepy = concat!("## Main\n", "    Show \"before\".\n", "    Sleep 1.\n", "    Show \"after\".\n");
let refused = run_strict_vm(sleepy);
assert!(refused.refused());
assert_eq!(refused.output, "");
```

The strict runners never stand in for each other: `engine_ran` says which engine executed, and a program the VM cannot run (`Sleep` needs the async runtime; the VM is sync-only) comes back with no output and the reason in `error`. The everyday entry, `interpret_for_ui_sync`, runs the VM and falls back to the tree-walker for what the VM rejects, returning an `InterpreterResult { lines, error }`.

## Public API

The crate root re-exports the entry points; the rest live under `compile::` and `ui_bridge::`.

```text
// AST → generated Rust source.
pub fn compile_to_rust(source: &str) -> Result<String, ParseError>;
pub fn compile_to_rust_deterministic(source: &str) -> Result<String, ParseError>;
pub fn compile_to_rust_with_proven(source: &str, proven: &str) -> Result<String, ParseError>;

// Full output: Rust + extracted crate deps + optional C/Python/TypeScript FFI.
pub fn compile_program_full(source: &str) -> Result<CompileOutput, ParseError>;

// Strict single-engine runs: the outcome plus the engine that actually ran.
pub fn run_strict_tw(source: &str) -> EngineOutcome;
pub fn run_strict_vm(source: &str) -> EngineOutcome;

// Interpret directly (no codegen), for sub-second Studio feedback.
pub fn interpret_for_ui_sync(input: &str) -> InterpreterResult;            // { lines, error }
pub async fn interpret_streaming<F>(input: &str, on_output: Rc<RefCell<F>>) // streams each Show line
    -> InterpreterResult where F: FnMut(String) + 'static;

// Concurrency analysis (pure AST; no codegen feature needed).
pub fn classify_source(source: &str) -> Result<concurrency::Determinacy, ParseError>;
pub fn send_check_source(source: &str) -> Result<Vec<concurrency::SendDiagnostic>, ParseError>;
```

Other entry points in `compile::` include `compile_to_rust_checked` (escape and ownership), `compile_to_rust_verified` (`verification` feature), `compile_project` (multi-file), `compile_and_run` (shells out to `rustc`), `compile_to_c`, `compile_to_wasm`, `interpret_program`, `vm_run_source`, and `build_native_cdylib` / `build_native_wasm`. `compile_to_rust("## Main\nReturn 42.")` emits a `fn main` that runs `_logos_main` on a 64 MiB-stack worker thread. `ui_bridge` adds the Studio surface: `compile_for_ui`, `verify_theorem`, `prove_theorem_trace`, `solve_grid`, `answer_question`, and the concurrent VM runners.

### Module map

`intern`, `arena`, `arena_ctx`, `registry` and `style` re-export the matching `logicaffeine-base` / `logicaffeine-language` items so downstream code can depend on this crate alone.

| Module | Role |
|--------|------|
| `loader` | Module resolution (`Loader`, `DepRoot` dependency roots) and the standard-library prelude. The `assets/std/*.lg` modules are embedded with `include_str!`, spliced in on demand (`apply_prelude`), and pruned on the compiled tiers to the functions the program reaches |
| `analysis` | Compile-time passes: `assemble` (a multi-module project, imports resolved transitively, as one source), `check` (bidirectional type checking), escape, ownership, `dimension_check`, liveness, call graph, `known_calls` (a call to a name defined nowhere is a check error), `import_hygiene` (unused, duplicate and self imports), `pipes` (which parameters carry a pipe), `relay` (whether a program's pages talk to the relay) |
| `optimize` | The optimizer (abstract-interpretation facts, GVN, LICM, DCE, inlining, scalarization, e-graph, supercompilation) behind one `OptimizationConfig` |
| `interpreter`, `vm`, `semantics` | The execution tiers (below) and the semantics core both of them share |
| `compile` | The top-level entry points (`compile_to_rust`, `compile_project`, the optimization graph, dependency extraction) that drive every backend |
| `codegen`, `codegen_c`, `codegen_riscv`, `codegen_cuda`, `codegen_tt`, `codegen_sva` | Rust / C / RISC-V / CUDA / Tenstorrent (TT-Metalium) / hardware SVA emission |
| `codegen_sass` | The core of an sm_86 SASS assembler: finds a kernel's machine code in a cubin, decodes each instruction's control field, re-encodes it and patches it back |
| `program_bindings` | The ahead-of-time lift for program bindings. The root bindings a function reads or writes become the fields of one `__Program` record, handed to each such function as a `mutable` parameter, so a program whose functions share `## Main`'s bindings builds on AOT-Rust. A call that can reach a binding before its `Let`, and a task launched on a function that reaches one, are refused by name |
| `monomorphize` | Generics for the tiers without their own: one concrete function per (generic function, type arguments) a program calls and one registered type per applied generic, from the type arguments the checker solved at each site. It lowers `some`/`none` to the core `Option` enum where a backend has no `Option` |
| `codegen_ana` | The analog spatial-fabric lane, for an FPAA-style mixed-signal coprocessor. Float dataflow becomes Kirchhoff-sum, transconductance and OTA-C tiles behind a digital control plane, agreeing with the VM within a compiler-emitted error budget. Anything else is a typed refusal, kept apart from `codegen_sva`'s exact regime |
| `codegen_mx` | The mixed-signal partitioner: splits a program into certificate-gated analog regions (through `codegen_ana`), each with a typed refusal and a digital fallback, so removing every analog binding still leaves a program that agrees with the VM |
| `accel` | The accelerator backends. A scalar function of the optimized program becomes a region, a control-flow graph of checked cells held to the VM, and every cell runs exactly on the lane words a device actually has: a multi-word integer core and an IEEE-754 binary64 softfloat built from lane operations, parameterized by the device's word and multiplier widths. The device model is generated as C from the regions and their routines, with a driver that counts every call it answers or hands back |
| `codegen_dram` | The DRAM compute-in-memory target (ComputeDRAM bulk bitwise and RowClone): sense-amplifier ops proven equal to the VM by the `codegen_sva::dram` oracle, a host control plane for everything else, and typed refusals |
| `spatial` | The spatial IR every spatial backend lowers from: a graph of stages over bounded buffers with typed handshakes, placed onto a described mesh. Its executable token semantics (`spatial::eval`) are the reference, and each program gets a `ChoreographyCertificate` (deadlock freedom, buffer bounds, per-PE SAT). The buffering, reader-fusion and semaphore-choreography passes run once over this IR, never per backend. Each target carries its roofs (`spatial::roofs`): the bandwidths and engine rates a program's work is held against, each cited. An all-integer program evaluates in checked `i64`, and `spatial::value_range` proves the interval every tensor holds, which sizes the fabric's words and datapaths. `spatial::lower::fabric_board` makes a fabric program a board image: a host port over the board's serial line and the autoregressive loop on the chip. `spatial::passes::forward` hands a result only the next statement reads from its producer's compute to its consumer's, where the caller declares what it reads (`compile_program_to_spatial_returning`) |
| `aot_unsupported` | The AOT refusal classifier, written once: what the device backends (`codegen_tt`, `codegen_cuda`, `codegen_ana`, `codegen_dram`) decline, and the browser-only surface the AOT-Rust and C tiers refuse, so the backends cannot drift apart |
| `tensor_recognize` | The backend-agnostic tensor-op recognizer: LOGOS AST to typed op specs (matmul, elementwise, reduction, SwiGLU) that `codegen_cuda`, `codegen_tt` and the `spatial` lowerings emit from |
| `model` | Models as data. `model::spec` reads a checkpoint's architecture from its GGUF metadata into a `ModelSpec` (Qwen3 and Qwen3-MoE, Qwen3.5's Gated DeltaNet hybrid, Llama 3, Gemma 3, gpt-oss, DeepSeek V2's latent attention); `model::gguf` reads the whole file, its metadata typed, and writes one; `model::quant` dequantizes the GGUF weight formats bit-for-bit as ggml defines them; `model::tokenizer` runs the checkpoint's vocabulary (byte-pair under every llama.cpp pre-tokenizer, and SentencePiece) as llama.cpp runs it; `model::author` writes a spec as a LOGOS program in each deployment form (the forward, a decode step, a prefill chunk, a batch, and each cut per layer); `model::bind` fills that program's weights from the checkpoint; `model::reference` evaluates the spec in f64, the oracle every lane is measured against |
| `bringup` | Model bring-up: a checkpoint's one program lowered by every chip that runs it. `model::program` names each cut of the program `model::author` writes (the decode step, a prefill chunk, a batch, one layer, the embedding or the head) and its identity (the cut and the SHA-256 of its text). A `bringup::Backend` lowers the cuts it runs and names the cut behind each artifact, or refuses the model by name: the CUDA lane (one fused program), the D5005 decode engine (the sequencer's program), the SN50 (a certified bundle). `bring_up` runs a checkpoint through the ones asked for |
| `codegen_cpu` | The CPU tier: a checkpoint's one program for the host's cores. `codegen_cpu::shim` answers the shared optimizer for each instruction set (x86 AVX2 with AVX-VNNI linked, the others refused by name until their libraries exist) at two numerics points, a product's activation in one word a 32-value block or as its values; `codegen_cpu::link` emits every op as a run of phases of the CPU library's units (`codegen_cpu::kernels`), a fused op one run of its parts' phases, so it is its parts to the bit; `codegen_cpu::model` links a checkpoint's decode step and prefill chunks into one C++ program over shared buffers, the checkpoint mapped and its rows read in place |
| `tensor_graph` | Tensor programs as graphs of ops, and the transforms over them. `tensor_graph::vjp` differentiates a tensor program in reverse mode and writes its backward pass as ordinary LOGOS — tensor sugar where the language has the form, row-major loops where it does not — so every backend takes it and the VM checks it against central differences; an op without a rule is refused by name, never differentiated as if absent |
| `crypto_accel` | Native SHA-256, HMAC-SHA256 and PBKDF2-HMAC-SHA256 for the tree-walker and VM, byte-identical to the pure-LOGOS reference in `assets/std/crypto.lg` (the version the AOT tiers compile) |
| `extraction` | Program extraction from kernel proof terms to Rust |
| `concurrency` | The determinacy model and `Send` analysis, the task drivers that put interpreter and VM tasks on `logicaffeine-runtime`'s scheduler, and the wire codec (below) |
| `diagnostic`, `sourcemap` | Translating generated-Rust errors back to LOGOS source positions |
| `ui_bridge`, `debug` | The Studio surface and the one-op-at-a-time bytecode debugger |
| `web_host` | The browser host for a page written in LGX, the LOGOS web framework, once it is compiled to WebAssembly (below) |
| `page_host` | Drives a page's `## Test` across the tiers that can execute the page, and compares their per-statement `PageTrace`s |
| `debug_session`, `frame_eval` | The `DebugSession` facade (DAP vocabulary) that `largo dap` and the Studio drawer both sit on, and the paused-frame expression evaluator (`evaluate`, conditional breakpoints, the Prove tab) behind every debug surface |
| `observe` | The observation channel: `Note`/`Warn`/`Record`/`During` events, separate from program output. On the compiled tiers a crate-private pass removes those statements before the optimizer runs, so the emitted Rust is byte-identical to the same program without them (`obs_zero_cost_codegen_lock`) |
| `repl` | `ReplSession`, the replay-based session behind `largo repl`: accumulated source re-run through the real engine, an output high-water mark, rollback on error |
| `replay` | The `.replay` failure artifact: a failing run captures a self-contained JSON bundle (embedded source, scheduler seed and decision trace, input transcript, failure coordinate) that `largo replay --verify` re-executes byte-identically |
| `testrun` | The `largo test` runner core, reused by the Studio: discovers `## Test` blocks, runs each in a fresh sealed world with record-and-continue expectations and deterministic op budgets, and renders name-ordered, byte-identical reports |
| `defeasible`, `verification` | Defeasible reasoning and Z3 static verification (`verification` feature) |

## Execution tiers and backends

| Lane | Module | Notes |
|------|--------|-------|
| Interpreter | `interpreter.rs` | Tree-walking async interpreter over `RuntimeValue`, the reference semantics. |
| Bytecode VM | `vm/` | Register VM: the browser engine and what the native JIT tiers up from. On native targets a very hot all-`Int` function is compiled by `rustc` on a worker thread (`bg_aot`) and loaded as a cdylib (`aot_tier.rs`); `tier_cache` persists optimized function bytecode so a re-run skips re-optimization. `vm::peephole` is the copy-propagation and dead-move pass for the backends that lower bytecode with no optimizing assembler after them. `vm::reach` computes which functions and ops can run. `vm::session` holds a program open so a host can call back into it (event handlers, fetch completions, timers). `vm::reverse`, `vm::journal` and `vm::pebble` classify every op's reversibility, record the history tape and prune it, so `Vm::run_reverse` restores the initial state. `vm::isa` / `vm::binisa` emit the bytecode as Silicon ISA assembly and its binary form (`compile_to_isa`). |
| Direct WASM | `vm/wasm/` | LOGOS to `.wasm` with no `rustc` (`compile_to_wasm`, `largo build --emit wasm`), and the path an LGX page takes to the browser. Emission needs no feature; only *running* JIT-compiled hot regions in the browser needs `wasm-jit`. |
| Rust codegen | `codegen/` | AST to Rust source (the `largo build` path), plus C / Python ctypes / TypeScript FFI bindings. `compile_to_rust_deterministic` builds a binary whose tasks replay the interpreter's schedule on `logicaffeine_system::replay`; `compile_to_rust_observed` keeps the observation events the release profile removes. |
| C codegen | `codegen_c/` | `compile_to_c` (tasks as coroutines), `compile_to_c_stepped` (tasks as step functions over heap frames, the form a GPU thread runs), and `compile_to_c_via_bytecode` (lowers the optimized bytecode; a typed refusal for an op it does not lower yet). Host C, RISC-V and CUDA schedule a concurrent program on `codegen_c::SCHED_CORE`, a port of the runtime scheduler, matching the interpreter at every seed (`c_family_scheduler_conformance_lock`). |
| RISC-V | `codegen_riscv/` | RV64GC assembly from the optimized bytecode (`compile_to_riscv`, `largo build --emit riscv`); `riscv_aot_lock` classifies every VM op and holds the deferred-gap list empty (host sockets and `Serve` are refused by name). |
| CUDA | `codegen_cuda/` | A single-file CUDA program lifting the C tier's program body onto the device (`compile_to_cuda`), plus the tensor kernels (`compile_to_cuda_matmul`, the forward-pass entry points); `cuda_aot_lock` classifies every VM op and holds the gap list empty. |
| CPU | `codegen_cpu/` | A checkpoint's decode step and prefill chunks as one C++17 program for the host's cores (`codegen_cpu::model::compile_model`), each op a kernel of the CPU library run in phases on a thread pool; `cpu_shim_lock` holds every tiny architecture's cuts to the evaluator. |
| Tenstorrent | `codegen_tt/` | TT-Metalium kernel and host for a Tensix core (`compile_to_tenstorrent`), the tensor ops, and the concurrency mesh. |
| Spatial | `spatial/`, `codegen_ana/`, `codegen_mx/`, `codegen_dram/` | The spatial IR and its lowerings: analog (`compile_to_analog`), Plasticine (`compile_to_plasticine`), the SambaNova RDU (`compile_to_rdu`), the mixed-signal partitioner and the DRAM compute-in-memory target, each a certified artifact or a named refusal. |
| SVA / RTL | `codegen_sva/` | SVA/PSL and Rust runtime monitors for hardware verification, and RTL generation (soft cores, the decision fabric, the whole-program engine, Caravel wrappers). |

`semantics/` is the semantics core the interpreter and the VM share, so the two cannot diverge on arithmetic, comparison, collections or builtins; `tail_call.rs` is the one self-tail-call definition every execution tier uses. EXODIA, the copy-and-patch JIT, lives in `logicaffeine-jit` and `logicaffeine-forge`, and the VM tiers up into it.

## LGX: pages and servers

LGX is parsed by `logicaffeine-language`; this crate is the half that makes it run. A page compiles through the direct WASM backend to one module, and `web_host` is its browser side. It supplies the real DOM behind the module's `dom_*` imports, the `__logos_call_closure` trampoline that events and fetch completions enter through, and the default theme, favicon and build-error overlay. `largo emit wasm-dom`, `largo serve` and the Studio's live preview share it, so a page renders the same in all three.

The same page runs in-process on the tree-walker and the VM, which is where `largo test` works it. `page_host` carries each driving statement (`Click`, `Type … into`, `Navigate to`) out on whichever tier executes the page and compares the tiers' traces after every statement. On the server side, a backend's `## Route` handlers and its `Serve` statement run on the tree-walker, the VM and compiled Rust, over the HTTP server in `logicaffeine-system`. A route the VM cannot lower is refused by name. The [Frontend](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/docs/frontend.md) and [Web](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/docs/web.md) guides cover the language side.

## Device bundles

`largo emit tenstorrent -o DIR` and `largo emit cuda -o DIR` write a bundle rather than one file. It holds the device program (a TT-Metalium kernel and host orchestrator with CMake glue, or one `.cu` holding the kernel and its launcher, with a Makefile) and `logos_oracle.c`. The oracle is the C tier's translation of the same program, the body the device code is derived from. Built with gcc and run on the host, it prints the VM's output, so you can check a bundle before any card or toolkit is present. `compile_to_tenstorrent` and `compile_to_cuda` return the parts (`TtArtifact`, `CudaArtifact`, each with its `oracle_c`). The analog, Plasticine and RDU targets write bundles of their own, listed in the [CLI guide](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/docs/cli.md).

## The wire codec

When a value leaves its task, through a channel or to a peer over the relay, `concurrency::marshal` carries it. `materialize` and `rebuild` move a value out of one task's heap into a `Send` payload (`logicaffeine_runtime::RtPayload`) and back into another's. `message_to_wire` / `message_from_wire` frame the whole value (scalars, collections, structs, inductives) for the network, as a single-pass tagged-varint binary by default or JSON for interop. A header byte records the codec, an optional checksum and the compression (deflate, LZ4, zstd or context mixing), so any receiver decodes any sender without being told. `message_to_wire_auto` runs the encodings against a large payload and ships the smallest, and two peers `negotiate` the strongest compression both understand.

Around it sit `concurrency::stream` (length-prefixed framing over a byte stream), `concurrency::fec` (Reed-Solomon erasure coding: any `k` of `n` shards rebuild the message) and `concurrency::channel` (an encryption envelope that never inspects the encoded bytes). `logicaffeine-wirebench` measures the codec against other formats.

## Feature flags

| Feature | Default | Enables |
|---------|---------|---------|
| `codegen` | on | The `codegen` and `codegen_c` modules, the counted-loop recognizer, the device and spatial backends (`codegen_tt`, `codegen_cuda`, `codegen_cpu`, `spatial`, `codegen_sass`, `codegen_ana`, `codegen_dram`, `codegen_mx`, `aot_unsupported`, `tensor_graph`), and the `generate_rust_code` re-exports. `codegen_riscv`, `codegen_sva` and `tensor_recognize` build without it. |
| `interpreter-only` | off | Names an interpretation-and-VM build; pair it with `default-features = false`, which is what leaves the `codegen` backends out. |
| `narrow-value` | off | Makes the VM register cell wrap the 8-byte NaN-boxed `Narrow` (`vm/nanbox.rs`) instead of the 16-byte `RuntimeValue`. |
| `verification` | off | Pulls in `logicaffeine-verify` and `logicaffeine-proof/verification`; enables the `verification` and `defeasible` modules, `VerificationPass`, `compile_to_rust_verified`, and the `check_theorem_*` re-exports (Z3 static verification and defeasible reasoning). |
| `wasm-jit` | off | Pulls in `wasmi` and enables *running* JIT-compiled hot regions from the direct WASM backend (`vm/wasm/region_jit.rs`) in the browser. Emission works without it; run its tests in a pass of their own. |

## Dependencies

Internal: `logicaffeine-base` (arena, interning), `-language` (lexer, parser, AST), `-kernel` (proof terms, with its `serde` feature), `-data` (runtime types), `-system` (I/O and persistence; on native also its `relay` and `http` features), `-proof` (derivation trees), `-runtime` (the interpreter and VM scheduler), `-verify` (Z3, optional).

External: `async-recursion` / `futures` (the async interpreter), `bumpalo`, `indexmap`, `rustc-hash`, `serde` / `serde_json`, `bincode`, `fancy-regex`; the wire codec's compressors `miniz_oxide` / `lz4_flex` / `ruzstd` (plus native-only `zstd`); native-only `libloading` (loading the background AOT build); `gloo-timers`, `js-sys` and `wasm-bindgen` on wasm32; and `wasmi` under `wasm-jit`. The standard library in `assets/std/` is embedded with `include_str!`.

## License

MIT License — see [LICENSE.md](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/LICENSE.md).

---
[Docs index](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/docs/README.md) · [Root README](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/README.md) · [Changelog](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/crates/logicaffeine_compile/CHANGELOG.md)
