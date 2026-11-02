# Architecture

One parser reads two languages, and one program can run on a tree-walker, under a JIT, in a browser
tab or on a GPU. That stays tractable because every crate has a fixed place in a dependency graph,
and four dependencies are forbidden outright. This guide follows English source from the lexer to
each back-end, then lays out the crate tier graph, the four invariants, and the dual-AST model that
lets one parser serve both modes. Afterwards you will know which crate owns each stage, and which
dependency a change must never add.

## The pipeline

One front-end, two modes, many back-ends. The parser switches mode on block headers (`## Main` /
`## To …` select the imperative grammar; otherwise the declarative/logic grammar applies).

```text
                 English source
                      │
                 Lexer  (logicaffeine_language: lexer.rs)
                      │  tokens
                 Parser (logicaffeine_language: parser/)
                 ┌────┴────┐
      Declarative│         │Imperative
                 ▼         ▼
          LogicExpr AST   Stmt/Expr AST
          (ast/logic.rs)  (ast/stmt.rs)
                 │         │
                 ▼         ▼
          Transpile      Analysis (types, ownership)
          → FOL          │
    (Unicode/LaTeX/      ├─ Tree-walking interpreter
     SimpleFOL/Kripke)   ├─ Bytecode VM ──► EXODIA copy-and-patch JIT (native)
                         │    │  the same register bytecode also lowers to:
                         │    ├─► Direct WASM module (no rustc): programs and LGX pages
                         │    ├─► RISC-V assembly (RV64GC + a small C runtime)
                         │    └─► Silicon ISA listing
                         ├─ AOT Rust codegen ──► cargo ──► native binary or wasm32
                         ├─ AOT C codegen (one self-contained C file)
                         │    └─► CUDA and Tenstorrent bundles (the C body on the device)
                         ├─ Analog, Plasticine and RDU bundles (dataflow lowerings)
                         └─ SVA/PSL codegen (hardware verification)

  Hosts   Serve (HTTP)  the interpreter, the VM and the compiled Rust binary answer alike
          LGX page      one WASM module in the browser; tree-walker and VM in-process
                        for largo test
```

On the command line, `largo run --interpret` runs the interpreter or the VM, `largo build` takes
the Rust path, and `largo emit <target>` prints or writes the C, RISC-V, WASM and device targets
(every target is in [the CLI guide](cli.md#emit)).

The declarative side can also feed the **proof engine** (`logicaffeine_proof`) and, when enabled,
**Z3 verification** (`logicaffeine_verify`). See [Logic mode](logic-mode.md),
[Execution & performance](execution-and-performance.md), and
[Proof & verification](proof-and-verification.md).

## The dual-AST model

| Mode | Block header | AST | Output |
|------|-------------|-----|--------|
| Declarative | `## Theorem`, default | `LogicExpr` ([`ast/logic.rs`](../crates/logicaffeine_language/src/ast/logic.rs)) | First-Order Logic |
| Imperative | `## Main`, `## To …` | `Stmt` / `Expr` ([`ast/stmt.rs`](../crates/logicaffeine_language/src/ast/stmt.rs)) | Executable code |

The **Assert Bridge** connects them: an imperative program can `Assert that <logic>`, tying
verifiable logical claims to executable code.

## The crate tier graph

A crate's tier is the length of its longest chain of internal dependencies (normal, build and
target-specific edges, read from the `Cargo.toml` files). Every dependency therefore points to a
strictly lower tier, and publishing in tier order puts every dependency on crates.io first.

```
Tier 0   base · runtime · forge · verify · lexicon   (no internal deps)
Tier 1   kernel · data · synth                   → base | forge
Tier 2   system · proof                          → base, data | kernel, verify
Tier 3   language                                → base, lexicon, proof
Tier 4   compile                                 → tiers 0–3
Tier 5   jit · lsp · tv · wirebench              → compile, and below
Tier 6   cli · web                               → jit | lsp, and below
Tier 7   tests                                   → everything
```

| Crate | Tier | Role |
|-------|------|------|
| [base](../crates/logicaffeine_base/README.md) | 0 | Arenas, interning, spans, and exact numeric, money, time and UUID types |
| [runtime](../crates/logicaffeine_runtime/README.md) | 0 | Deterministic concurrency runtime (tokio-free, WASM-safe) |
| [forge](../crates/logicaffeine_forge/README.md) | 0 | Copy-and-patch JIT executable-memory layer (native) |
| [verify](../crates/logicaffeine_verify/README.md) | 0 | Z3 static verification |
| [lexicon](../crates/logicaffeine_lexicon/README.md) | 0 | English vocabulary types + compile-time lexicon |
| [kernel](../crates/logicaffeine_kernel/README.md) | 1 | Calculus of Constructions + decision procedures |
| [data](../crates/logicaffeine_data/README.md) | 1 | Runtime values, the wire format and CRDTs (IO-free) |
| [synth](../crates/logicaffeine_synth/README.md) | 1 | Z3 specifications and witness checks for the JIT's stencils |
| [system](../crates/logicaffeine_system/README.md) | 2 | Platform IO, networking, persistence |
| [proof](../crates/logicaffeine_proof/README.md) | 2 | Kernel-checked proof search, tactics and certified SAT |
| [language](../crates/logicaffeine_language/README.md) | 3 | The front end: English → logic, and the LOGOS and LGX parser |
| [compile](../crates/logicaffeine_compile/README.md) | 4 | LOGOS compilation, interpretation, and codegen |
| [jit](../crates/logicaffeine_jit/README.md) | 5 | Wires the forge JIT into the VM (native) |
| [lsp](../crates/logicaffeine_lsp/README.md) | 5 | Language Server Protocol |
| [tv](../crates/logicaffeine_tv/README.md) | 5 | Translation validation, source side |
| [wirebench](../crates/logicaffeine_wirebench/README.md) | 5 | Wire-codec benchmark against industry serializers |
| [cli](../apps/logicaffeine_cli/README.md) | 6 | `largo`, the build tool |
| [web](../apps/logicaffeine_web/README.md) | 6 | The browser IDE: Studio, Learn Logic, the Syntax Guide (Dioxus + WASM) |
| [tests](../crates/logicaffeine_tests/README.md) | 7 | Every test: the integration suites and each crate's unit-test bodies |

> `verify`, `tv`, `synth`, and `wirebench` are excluded from `default-members` in the root
> [`Cargo.toml`](../Cargo.toml), so a plain `cargo build` builds none of them and needs no Z3 toolchain.
> `verify`, `tv` and `synth` build when the `verification` feature pulls them in; `wirebench` builds as a
> dev-dependency of the test suite.

## The four invariants

Each is a deliberate *absent* dependency, enforced at the crate boundary and documented in the
crate's own source. They keep components reusable and portable:

| Invariant | Crate | The forbidden dependency | Why |
|-----------|-------|--------------------------|-----|
| **Milner** | `kernel` | no path to the lexicon ([`kernel/src/lib.rs`](../crates/logicaffeine_kernel/src/lib.rs), `kernel/Cargo.toml`) | the type theory never sees English words, so the lexicon can change without touching the kernel |
| **Liskov** | `proof` | no dependency on `language` ([`proof/Cargo.toml`](../crates/logicaffeine_proof/Cargo.toml)) | the proof engine is reusable across front-ends |
| **Lamport** | `data` | no IO dependencies ([`data/src/lib.rs`](../crates/logicaffeine_data/src/lib.rs), `data/Cargo.toml`) | data structures stay WASM-safe and clock-agnostic |
| **Tarski** | `verify` | verification IR decoupled from the main AST ([`verify/src/lib.rs`](../crates/logicaffeine_verify/src/lib.rs)) | verification is front-end-agnostic |

## Platform gating

- **Native-only**: `forge` and `jit` (executable-memory JIT), plus the work-stealing scheduler in
  `runtime`, all `cfg`-gated off `wasm32`.
- **WASM**: the `web` app and the full engine (parser, interpreter, VM, proof engine, codegen) run
  in the browser; `system` abstracts IO across native and WASM targets.

## Where to look next

- The execution tiers in detail → [Execution & performance](execution-and-performance.md)
- The test suite as the living spec → [`crates/logicaffeine_tests/`](../crates/logicaffeine_tests/)
- Each crate's own `README.md` for its public API

## Where this is tested

`tier_map_lock` recomputes every crate's tier from the `Cargo.toml` files, holds the tier diagram
and the crate table above to it, and holds the crates this guide names as left out of the plain
build to the root manifest.

---
[Docs index](README.md) · [Root README](../README.md) · [Changelog](../CHANGELOG.md)
