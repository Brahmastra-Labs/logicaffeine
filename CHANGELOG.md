# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [0.8.17] - 2026-02-15

### Added
- **C codegen backend** — `compile_to_c()` produces self-contained C files with embedded runtime (Seq, Map, Set, string helpers, IO). Compiles with `gcc -O2`. Supports integers, floats, booleans, strings, collections, control flow, functions.
- **Constant propagation** optimizer pass — forward substitution of immutable constants, chained with fold and DCE. Safety: skips Index/Slice expressions to preserve swap/vec-fill pattern detection.
- **334 new E2E tests** across 20 new test files:
  - 16 Rust codegen mirror files (181 tests) — every interpreter-only feature now also tested through the Rust codegen pipeline
  - `e2e_codegen_gaps.rs` (64 tests) — floats, modulo, options, nothing, collection type combos, struct/enum patterns, control flow, functions, escape blocks, strings
  - `e2e_codegen_optimization.rs` (15 tests) — TCO, constant propagation, DCE, vec-fill, swap, fold, index simplification
  - `e2e_interpreter_gaps.rs` (60 tests) — interpreter counterparts for gap coverage
  - `e2e_interpreter_optimization.rs` (14 tests) — interpreter counterparts for optimization correctness

### Fixed
- **For-range guard for complex expressions** — `While i is at most length of items` no longer produces `_` in generated Rust. Added `is_simple_expr()` guard.
- **For-range post-loop value for empty loops** — empty loops now correctly keep the counter at its start value using `max(start, limit)`.
- **Vec-fill pattern relaxed mutability** — `Let items be a new Seq of Bool` (without explicit `mutable`) now matches the vec-fill optimization.
- **C codegen missing Set variants** — `SetI64` and `SetStr` added to `c_type_str()`.
- **Interpreter float comparison** — `apply_comparison` now handles Float-Float, Int-Float, and Float-Int comparisons.

## [0.8.16] - 2026-02-15

### Fixed
- **For-range loop regression**: `RangeInclusive` (`..=`) has a known per-iteration overhead in Rust due to internal bookkeeping for edge cases. In O(n^2) inner loops like bubble sort, this compounded to a 41.4% regression vs the `while` loop it replaced. All inclusive ranges now emit exclusive form: `for i in 1..=n` becomes `for i in 1..(n + 1)`. For literal limits, the addition is computed at compile time (e.g. `for i in 1..6` instead of `for i in 1..=5`).

## [0.8.15] - 2026-02-15

### Added
- **TIER 1 codegen optimizations** — five peephole-level improvements targeting array-heavy benchmark performance
  - **For-range loop emission**: `Let i be 1. While i <= n: ... Set i to i + 1` compiles to `for i in 1..(n + 1)` instead of `while (i <= n)`, enabling LLVM trip count recognition, unrolling, and vectorization
  - **Iterator-based loops**: `Repeat for x in items` emits `.iter().copied()` instead of `.clone()` for Copy-type collections (`Vec<i64>`, `Vec<f64>`, `Vec<bool>`), eliminating full-collection copies
  - **Direct array indexing for list literals**: `Let items be [10, 20, 30]` now registers element type, enabling direct `arr[(idx-1) as usize]` instead of `LogosIndex` trait dispatch
  - **Vec fill exclusive bound**: `While i < n: Push 0 to items` now optimizes to `vec![0; n]` (previously only `<=` was matched)
  - **Swap pattern equality comparisons**: `If a equals b: swap` and `If a is not b: swap` now optimize to `arr.swap()`
- 24 new optimizer tests across all 5 TIER 1 items (codegen assertions + E2E correctness)

## [0.8.14] - 2026-02-15

### Added
- **TIER 0 optimizer bedrock** — deep expression recursion, unreachable-after-return DCE, algebraic simplification
  - Constant folding now recurses into all 26 Expr variants (function call args, list literals, index expressions, struct constructors, Option/Maybe Some, Contains, Closure bodies)
  - Statements after `Return` are eliminated as dead code
  - Algebraic identity/annihilator rules for both int and float: `x + 0 → x`, `x * 1 → x`, `x * 0 → 0`, `x - 0 → x`, `x / 1 → x`
- **Maybe syntax** — `Maybe Int` and `Maybe of Int` as dual syntax for `Option of Int`
  - Supports direct Haskell-style `Maybe T` (no "of" required) and consistent `Maybe of T`
  - Works in all positions: variable annotations, return types, nested generics
- 33 new optimizer and type expression tests

### Fixed
- Publish workflow: skip already-published crates instead of failing

## [0.8.13] - 2026-02-14

### Added
- Accumulator introduction: converts `f(n-1) + k` and `n * f(n-1)` into zero-overhead loops
- Automatic memoization: pure multi-branch recursive functions get thread-local HashMap cache
- Mutual tail call optimization: pairs like `isEven`/`isOdd` merged into single loop with tag dispatch
- Purity analysis: fixed-point dataflow to identify side-effect-free functions
- 14 new optimizer tests (accumulator, memoization, mutual TCO — codegen + E2E correctness)

## [0.8.12] - 2026-02-14

### Changed
- Optimizer updates

## [0.8.11] - 2026-02-14

### Added
- Peephole optimizer: vec fill pattern (`vec![val; count]` instead of push loop)
- Peephole optimizer: swap pattern (`arr.swap()` instead of temp variable assignments)
- Copy-type elision: skip `.clone()` on Vec/HashMap indexing for primitive types
- HashMap equality optimization: `map.get()` instead of `map[key].clone()` for comparisons

### Changed
- Release profile: `opt-level = 3`, `codegen-units = 1`, `panic = "abort"`, `strip = true`
- `#[inline]` on Value arithmetic, LogosDate/LogosMoment accessors, parseInt/parseFloat
- Variable type tracking threaded through all expression codegen paths

## [0.8.10] - 2026-02-14

### Changed
- Direct collection indexing codegen for known Vec/HashMap types (avoids trait dispatch)
- `#[inline(always)]` on all Showable, LogosContains, LogosIndex trait impls
- `get_unchecked` after validated bounds in Vec indexing (removes redundant bounds check)
- LTO enabled in release profile for generated projects

## [0.8.9] - 2026-02-14

### Changed
- Frontend deploy now triggers after benchmarks commit fresh results
- Deploy checkout always uses latest main HEAD

### Fixed
- Benchmark CI checkout failure when latest.json is dirty
- Benchmark CI `actions: write` permission for deploy trigger

## [0.8.8] - 2026-02-14

(skipped — missing actions:write permission)

## [0.8.7] - 2026-02-14

(skipped — benchmark CI fix landed after tag)

## [0.8.6] - 2026-02-13

### Added
- Benchmark suite with programs in 10 languages (C, C++, Rust, Go, Zig, Nim, Python, Ruby, JS, Java)
- Benchmark CI workflow
- Benchmarks web page with interactive results
- Interpreter improvements and map key support
- nextest configuration

### Fixed
- Removed `.cargo/config.toml` mold linker config (breaks CI)

## [0.8.3] - 2026-02-12

### Fixed
- FFI C-linkage tests gated behind `ffi-link-tests` feature (CI compatibility)
- Platform-aware linker flags for C ABI tests (macOS + Linux)

## [0.8.2] - 2026-02-12

### Added

- Optimizer infrastructure with constant folding and dead code elimination
- Interpreter mode (`largo run --interpret`) for sub-second development feedback
- Map insertion syntax (`Set X at KEY to VALUE`)

### Changed

- FFI/C export safety: thread-local error cache, panic boundaries, null handle checks, dynamic `logos_version()`
- `LogosHandle` from `*const c_void` to `*mut c_void`
- Text/String excluded from C ABI value types

### Fixed

- UTF-8 string indexing in interpreter (`.len()` → `.chars().count()`)
- Rust keyword escaping in generated FFI code

## [0.8.1] - 2026-02-12

### Fixed

- Added `@types/node` to VSCode extension devDependencies

## [0.8.0] - 2026-02-10

### Added

- LSP server with full language intelligence (definitions, references, hover, completions, rename, semantic tokens, diagnostics)
- VSCode extension with bundled LSP binaries for 5 platforms
- FFI/C export system for cross-language interop
- CI/CD workflows for release, publish, and deployment

### Changed

- Compiler improvements and bug fixes

## [0.7.0] - 2026-02-01

### Added

- Escape analysis for memory safety
- Concurrency and async runtime
- Syntactic sugar features
- E2E test suite expansion (77 new tests)

### Fixed

- 10 compiler bugs across codegen, parser, and map handling
- Async/concurrency correctness issues

### Changed

- Web platform: studio IDE, mobile responsiveness, homepage redesign

## [0.6.0] - 2026-01-17

Initial crates.io release with lockstep versioning.

### Changed

- Synchronized all crate versions to 0.6.0 (lockstep versioning)
- Added CHANGELOG.md to every crate
- Added VERSIONING.md with release process documentation

## [0.5.5] - 2026-01-01

First public release.

### Added

**Compiler**
- Z3 SMT solver integration for static verification (`logos_verification` crate)
- Refinement type syntax with `where` clauses
- DRS (Discourse Representation Structures) for donkey anaphora
- Event adjective analysis ("Olga is a beautiful dancer")
- Escape analysis for memory safety
- Diagnostic system with source mapping

**Runtime (`logos_core`)**
- Standard library: `env`, `file`, `random`, `time` modules
- CRDT support: GCounter, LWW registers
- Memory zones for region-based allocation

**Tooling**
- CLI tool (`largo`) for project management
- Package registry with publish/download
- GitHub Actions for CI/CD deployment
- Rust code formatter for generated output

**Web Platform**
- Learning platform with interactive curriculum
- Vocabulary reference component
- User profile page
- Universal navigation

**Tests**
- End-to-end test suite (collections, functions, structs, enums, etc.)
- Phase 41: Event adjectives
- Phase 42: DRS
- Phase 50: Security policy analysis
- Phase 85: Memory zones
- Grand challenge: Mergesort example

### Core Features (v0.5.5)

**Logic Mode** - English → First-Order Logic
- Quantifiers: universal, existential, negative, cardinal
- Modal operators: necessity, possibility, deontic
- Temporal logic: tense, aspect
- Wh-questions, relative clauses, reflexives, reciprocals
- Scope ambiguity resolution
- Parse forests for structural ambiguity

**Imperative Mode** - English → Rust
- Variables, mutation, control flow
- Functions with typed parameters
- Structs and enums with pattern matching
- Collections with 1-based indexing
- Generics (`Seq of Int`, `Box of [T]`)
