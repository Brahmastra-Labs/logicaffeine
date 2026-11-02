# Logicaffeine Documentation

Guides to LOGOS and the Logicaffeine workspace, written against the source and checked by the test suite.

**Start here:** [Getting started](getting-started.md) takes you from nothing installed to a tested program, a tested
web page and English turned into logic, in about fifteen minutes. The [quick guide](../LOGOS_QUICKGUIDE.md) is every
construct on one page; keep it open beside whatever you read next. A good order after that: Imperative mode, then
Frontend and Web for applications, Logic mode for the other half of the language, and Proof & verification for what
the kernel checks.

## Guides

| Guide | What it covers |
|-------|----------------|
| [Getting started](getting-started.md) | Install `largo`, run a first program and its test, serve a web page, and turn a sentence into logic |
| [Imperative mode](imperative-mode.md) | The LOGOS programming language: values, types, collections, control flow, functions, ownership, I/O |
| [Frontend](frontend.md) | Pages in LGX: state and turns, lists, components, forms, stores, triggers, fetching, layouts, styling, testing, and shipping a site |
| [Web](web.md) | The server half: typed routes, requests and responses, cookies, sessions and CSRF, static files, JSON, PostgreSQL, and the sockets underneath |
| [Logic mode](logic-mode.md) | English → First-Order Logic and the linguistic phenomena it handles |
| [Concurrency & distributed](concurrency.md) | Tasks, channels, agents, CRDTs, and networking |
| [Execution & performance](execution-and-performance.md) | The interpreter, bytecode VM, copy-and-patch JIT, AOT codegen, and the direct WASM backend |
| [Proof & verification](proof-and-verification.md) | The kernel, the proof engine, Z3 verification, and translation validation |
| [The `largo` CLI](cli.md) | The build tool, the `Largo.toml` manifest, and the registry |
| [Studio & Learn](studio-and-learn.md) | The browser IDE and the Learn roadmap |
| [Architecture](architecture.md) | The pipeline, the crate tier graph, and the four invariants |
| [LGX reference](lgx-reference.md) | Every LGX construct, one row each, generated from the compiler's own catalogues |
| [Web standard library](lgx-stdlib.md) | The `forms`, `ui` and `session` modules web programs call |
| [Error catalog](ERROR_CATALOG.md) | Every diagnostic code with its Socratic explanation, generated from the compiler |

Runnable example projects, each with a README, are indexed in [`examples/`](../examples/README.md).

## Project

| Document | What it covers |
|----------|----------------|
| [FAQ](project/FAQ.md) | What LOGOS is, what it trusts, and what it does not do yet |
| [Versioning](project/VERSIONING.md) | Lockstep SemVer, the crate hierarchy, changelogs, and the release process |
| [The challenge](project/CHALLENGE.md) | The public "break my code" challenge and its rules |

## Where the source of truth lives

- **Workspace + versions** — the root [`Cargo.toml`](../Cargo.toml) (lockstep version across members).
- **The living specification** — the test suite in [`crates/logicaffeine_tests/`](../crates/logicaffeine_tests/),
  organized into phase tests with end-to-end differential tests across execution tiers.
- **Per-crate detail** — each crate's own `README.md` under [`crates/`](../crates/).
- **Release history** — [`CHANGELOG.md`](../CHANGELOG.md).

## Every example is tested

The code examples in these guides, in the root README and in every crate's README are run by the test suite.
[`getting_started_lock`](../crates/logicaffeine_tests/tests/getting_started_lock.rs) replays Getting started and the
README's Quick start command by command, with nothing but `largo` on the `PATH`. The
[`doc_examples`](../crates/logicaffeine_tests/tests/doc_examples.rs) harness compiles every `logos`
block, runs every one followed by an `output` block and requires exactly that output, runs every
`## Test` a `logos` block carries, checks every `"English" → First-Order-Logic` pair against
the live compiler, and holds every `rust` block shown beside a `logos` block to what the compiler
emits. Every `logos-dom` block is a page: the
[`canonical_teaching_lock`](../crates/logicaffeine_tests/tests/canonical_teaching_lock.rs) mounts
it, renders it on the tree-walker and the VM (which must agree), compiles it for the browser, and
runs its tests — and fails any fence that holds LOGOS without a tag that runs it. Placeholders like
`...` are rejected; the examples are real, runnable code.

---
[Root README](../README.md) · [Changelog](../CHANGELOG.md)
