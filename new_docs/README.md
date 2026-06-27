# Logicaffeine Documentation

Code-grounded guides to LOGOS and the Logicaffeine workspace. Every claim here is derived from and
verifiable against the source — not from older narrative docs.

> **Staging note.** These guides live under `new_docs/` and the root draft is `NEW_README.md` while
> the documentation overhaul is in progress. On promotion, `new_docs/` becomes `docs/` and
> `NEW_README.md` becomes `README.md`; the relative links update accordingly.

## Guides

| Guide | What it covers |
|-------|----------------|
| [Imperative mode](imperative-mode.md) | The LOGOS programming language: values, types, collections, control flow, functions, ownership, I/O |
| [Logic mode](logic-mode.md) | English → First-Order Logic and the linguistic phenomena it handles |
| [Execution & performance](execution-and-performance.md) | The interpreter, bytecode VM, copy-and-patch JIT, and AOT codegen |
| [Proof & verification](proof-and-verification.md) | The kernel, the proof engine, Z3 verification, and translation validation |
| [Concurrency & distributed](concurrency.md) | Tasks, channels, agents, the 8 CRDTs, and networking |
| [Studio & Learn](studio-and-learn.md) | The browser IDE and the gamified curriculum |
| [The `largo` CLI](cli.md) | The build tool, the `Largo.toml` manifest, and the registry |
| [Architecture](architecture.md) | The pipeline, the crate tier graph, and the four invariants |

## Where the source of truth lives

- **Workspace + versions** — the root [`Cargo.toml`](../Cargo.toml) (lockstep version across members).
- **The living specification** — the test suite in [`crates/logicaffeine_tests/`](../crates/logicaffeine_tests/),
  organized into phase tests with end-to-end differential tests across execution tiers.
- **Per-crate detail** — each crate's own `README.md` under [`crates/`](../crates/).
- **Release history** — [`CHANGELOG.md`](../CHANGELOG.md).

## Every example is tested

The code examples in these guides aren't decorative. The
[`doc_examples`](../crates/logicaffeine_tests/tests/doc_examples.rs) harness extracts every `logos`
block and every `"English" → First-Order-Logic` pair from this docs set, compiles each program, and
checks each FOL pair against the live compiler — so an example that drifts from the implementation
fails the test suite. (Placeholders like `...` are rejected; the examples are real, runnable code.)

---
[Root README](../NEW_README.md) · [Changelog](../CHANGELOG.md)
