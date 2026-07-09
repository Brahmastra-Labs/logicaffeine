# logicaffeine-compile

The LOGOS compilation pipeline: it takes a parsed LOGOS AST, runs the analysis
and optimization passes over it, then executes it across four tiers — the
tree-walking interpreter, the register-bytecode VM (with a native AOT tier),
Rust source generation, and the experimental C / hardware-SVA backends.

Part of the [Logicaffeine](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/README.md) workspace. Tier 3 — the apex
compiler crate; depends on base, language, kernel, data, system, proof, runtime
(and verify under the `verification` feature).

## Role in the workspace

LOGOS source is lexed, parsed, and analyzed by `logicaffeine-language`; this
crate owns everything downstream of the AST. It runs the static analysis passes
(escape, ownership, type/unify, liveness, call-graph), the optimizer (oracle
abstract-interp facts, GVN, LICM, DCE, inlining, scalarization, e-graph,
supercompilation — all behind one `OptimizationConfig`), then hands the result
to one of the execution tiers. It feeds the CLI, the web Studio (via
`ui_bridge`), and the native JIT, and houses proof-term extraction and the
hardware-verification (SVA/PSL) backend.

```text
AST ──▶ Analysis (escape, ownership, type/unify, liveness, call-graph)
    ──▶ Optimize (oracle facts, GVN, LICM, DCE, inline, scalarize, e-graph)
    ──▶ { Interpreter | Bytecode VM (+ AOT) | Rust codegen | C / SVA codegen }
```

See [imperative-mode.md](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/docs/imperative-mode.md) for the language the
pipeline compiles and [execution-and-performance.md](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/docs/execution-and-performance.md)
for how the tiers relate.

## Public API

The crate root re-exports the entry points; the rest live under `compile::` and
`ui_bridge::`.

```text
// AST → generated Rust source.
pub fn compile_to_rust(source: &str) -> Result<String, ParseError>;
pub fn compile_to_rust_deterministic(source: &str) -> Result<String, ParseError>;
pub fn compile_to_rust_with_proven(source: &str, proven: &str) -> Result<String, ParseError>;

// Full output: Rust + extracted crate deps + optional C/Python/TypeScript FFI.
pub fn compile_program_full(source: &str) -> Result<CompileOutput, ParseError>;

// Interpret directly (no codegen), for sub-second Studio feedback.
pub fn interpret_for_ui_sync(input: &str) -> InterpreterResult;            // { lines, error }
pub async fn interpret_streaming<F>(input: &str, on_output: Rc<RefCell<F>>) // streams each Show line
    -> InterpreterResult where F: FnMut(String) + 'static;

// Concurrency analysis (pure AST — no codegen feature needed).
pub fn classify_source(source: &str) -> Result<concurrency::Determinacy, ParseError>;
pub fn send_check_source(source: &str) -> Result<Vec<concurrency::SendDiagnostic>, ParseError>;
```

### Module map

The pipeline is spread across focused modules under the crate root. `intern`,
`arena`, `arena_ctx`, `registry`, and `style` re-export the corresponding
`logicaffeine-base` / `logicaffeine-language` items so downstream code depends on
this crate alone.

| Module | Role |
|--------|------|
| `loader` | multi-file LOGOS project loader |
| `analysis` | compile-time analysis passes (escape, ownership, type/unify, liveness, call-graph) |
| `optimize` | the optimizer (oracle facts, GVN, LICM, DCE, inlining, scalarization, e-graph, supercompilation) behind one `OptimizationConfig` |
| `interpreter`, `vm`, `semantics` | the execution tiers (see below) and the shared value-semantics kernel |
| `codegen`, `codegen_c`, `codegen_sva` | Rust / C / hardware-SVA source emission |
| `extraction` | program extraction from kernel proof terms to Rust |
| `concurrency` | the determinacy model + `Send`/classification analysis |
| `diagnostic`, `sourcemap` | translating generated-Rust errors back to LOGOS source positions |
| `ui_bridge`, `debug` | the Studio surface and the one-op-at-a-time bytecode debugger bridge |
| `repl` | `ReplSession`, the replay-based interactive session behind `largo repl` (accumulated source re-run through the real engine, output high-water mark, error rollback) |
| `defeasible`, `verification` | defeasible reasoning + Z3 static verification (`verification` feature) |

`compile_to_rust("## Main\nReturn 42.")` emits a module whose `fn main` launches
a large-stack (64 MiB) worker thread running `_logos_main`, whose body is
`return 42;`. Other reachable functions in `compile::` include
`compile_to_rust_checked` (escape + ownership), `compile_to_rust_verified`
(`verification` feature), `compile_project` (multi-file), `compile_and_run`
(shells out to `rustc`), `compile_to_c`, `interpret_program`, `vm_run_source`,
and `build_native_cdylib` / `build_native_wasm`. `ui_bridge` adds the Studio
surface: `compile_for_ui`, `verify_theorem`, `prove_theorem_trace`,
`solve_grid`, `answer_question`, and the concurrent VM runners.

## Execution tiers

| Tier | Module | Notes |
|------|--------|-------|
| Interpreter | `interpreter.rs` | Tree-walking async interpreter over `RuntimeValue`. |
| Bytecode VM | `vm/` | Register VM — the browser/WASM engine and the JIT substrate; `aot_tier.rs` dlopens an off-thread `rustc`-built cdylib (`bg_aot`/`bg_compile`), cached via `tier_cache`. |
| Direct WASM | `vm/wasm/` | The direct Logos→`.wasm` backend: emits a WebAssembly module with no `rustc` (`largo build --emit wasm`). Byte emission is feature-independent; only *running* JIT'd regions in-browser needs the `wasm-jit` feature. |
| Rust codegen | `codegen/` | AST → Rust source (the AOT path); plus C / Python ctypes / TypeScript FFI bindings. |
| C codegen | `codegen_c/` | Experimental benchmark-only C backend. |
| SVA codegen | `codegen_sva/` | SVA/PSL + Rust runtime monitors for hardware verification. |

`semantics/` is the single value-semantics kernel shared by the interpreter and
the VM so the two tiers cannot diverge; `tail_call.rs` is the one self-tail-call
definition shared by all tiers. The copy-and-patch JIT itself lives in the
`logicaffeine_jit` / `logicaffeine_forge` crates — the VM tiers up into it.

## Feature flags

| Feature | Default | Gates |
|---------|---------|-------|
| `codegen` | Yes | The `codegen` and `codegen_c` modules, the `loop_shape` recognizer, and the `generate_rust_code` re-exports — the entire Rust/C source-emitting AOT path. |
| `interpreter-only` | No | Marker for a minimal build that drops the codegen path and runs interpretation/VM only. |
| `narrow-value` | No | Makes the VM register cell wrap the 8-byte NaN-boxed `Narrow` (`vm/nanbox.rs`) instead of the default 16-byte `RuntimeValue`. Off by default. |
| `verification` | No | Pulls in `logicaffeine-verify` and `logicaffeine-proof/verification`; enables the `verification` + `defeasible` modules, `VerificationPass`, `compile_to_rust_verified`, and the `check_theorem_*` re-exports (Z3 static verification + defeasible reasoning). |
| `wasm-jit` | No | Pulls in `wasmi` and enables *running* JIT'd hot regions from the direct WASM backend (`vm/wasm/region_jit.rs`) inside the browser. Byte emission works without it; keep this pass scoped to its own test run. |

## Dependencies

Internal: `logicaffeine-base` (arena/interning), `-language` (lexer/parser/AST),
`-kernel` (proof terms), `-data` (runtime types), `-system` (I/O + persistence;
the thin WS `relay` on native), `-proof` (derivation trees), `-runtime`
(generated-program runtime), `-verify` (Z3, optional).

External: `async-recursion` / `futures` (async interpreter), `rustc-hash`,
`serde` / `serde_json`, `bincode`; the wire-codec compression dial
`miniz_oxide` / `lz4_flex` / `ruzstd` (plus native-only `zstd`); `include_dir`
(embedded `std` library); native-only `libloading` (AOT dlopen); and
`gloo-timers` for WASM async sleep.

## License

Business Source License 1.1 — see [LICENSE.md](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/LICENSE.md).

---
[Docs index](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/docs/README.md) · [Root README](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/README.md) · [Changelog](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/CHANGELOG.md)
