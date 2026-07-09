# Architecture

How Logicaffeine is put together: the compilation pipeline, the crate tier graph, the four
dependency invariants that keep it honest, and the dual-AST model that lets one parser serve both
modes.

## The pipeline

One front-end, two modes, many back-ends. The parser switches mode on block headers (`## Main` /
`## To …` select the imperative grammar; otherwise the declarative/logic grammar applies).

```
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
             SimpleFOL/Kripke)   ├─ Bytecode VM ──► copy-and-patch JIT (native)
                                 ├─ AOT Rust codegen
                                 ├─ Direct WASM backend (--emit wasm, no rustc)
                                 ├─ AOT C codegen (benchmark subset)
                                 └─ SVA/PSL codegen (hardware verification)
```

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

Crates are layered so dependencies only ever point *down* a tier. Tiers are reconstructed from the
actual `Cargo.toml` dependency edges.

```
Tier 0   base · runtime · forge                  (no internal deps)
Tier 1   lexicon · kernel · data                 → base
Tier 2   system · proof                          → base, (data | kernel)
Tier 3   language · compile                       → tiers 0–2
Tier 4   jit · lsp · cli · web                    → tiers 0–3
Z3-gated verify · tv · synth                       (opt-in; outside default-members)
```

| Crate | Tier | Role |
|-------|------|------|
| [base](../crates/logicaffeine_base/README.md) | 0 | Arenas, interning, spans, errors |
| [runtime](../crates/logicaffeine_runtime/README.md) | 0 | Deterministic concurrency runtime (tokio-free, WASM-safe) |
| [forge](../crates/logicaffeine_forge/README.md) | 0 | Copy-and-patch JIT executable-memory layer (native) |
| [lexicon](../crates/logicaffeine_lexicon/README.md) | 1 | English vocabulary types + compile-time lexicon |
| [kernel](../crates/logicaffeine_kernel/README.md) | 1 | Calculus of Constructions + decision procedures |
| [data](../crates/logicaffeine_data/README.md) | 1 | Runtime values + CRDTs (IO-free) |
| [system](../crates/logicaffeine_system/README.md) | 2 | Platform IO, networking, persistence |
| [proof](../crates/logicaffeine_proof/README.md) | 2 | Backward-chaining proof engine + Socratic hints |
| [language](../crates/logicaffeine_language/README.md) | 3 | English → First-Order Logic pipeline |
| [compile](../crates/logicaffeine_compile/README.md) | 3 | LOGOS compilation, interpretation, and codegen |
| [jit](../crates/logicaffeine_jit/README.md) | 4 | Wires the forge JIT into the VM (native) |
| [lsp](../crates/logicaffeine_lsp/README.md) | 4 | Language Server Protocol |
| [verify](../crates/logicaffeine_verify/README.md) | — | Z3 static verification |
| [tv](../crates/logicaffeine_tv/README.md) | — | SMT translation validation |
| [synth](../crates/logicaffeine_synth/README.md) | — | EXODIA stencil synthesis & witness checking |

> `verify`, `tv`, `synth`, and `wirebench` are excluded from `default-members` in the root
> [`Cargo.toml`](../Cargo.toml), so a plain `cargo build`/`cargo test` needs no Z3 toolchain. They
> build whenever the `verification` feature pulls them in.

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
  `runtime` — all `cfg`-gated off `wasm32`.
- **WASM**: the `web` app and the full engine (parser, interpreter, VM, proof engine, codegen) run
  in the browser; `system` abstracts IO across native and WASM targets.

## Where to look next

- The execution tiers in detail → [Execution & performance](execution-and-performance.md)
- The test suite as the living spec → [`crates/logicaffeine_tests/`](../crates/logicaffeine_tests/)
- Each crate's own `README.md` for its public API

---
[Docs index](README.md) · [Root README](../README.md) · [Changelog](../CHANGELOG.md)
