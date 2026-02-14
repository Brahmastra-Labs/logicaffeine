# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

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
