# Contributing to Logicaffeine

Thank you for your interest in contributing to Logicaffeine! This document provides guidelines for contributing to the project.

## Getting Started

### Prerequisites

- Rust (latest stable)
- For verification features: Z3 SMT solver

### Building

```bash
# Clone the repository
git clone https://github.com/Brahmastra-Labs/logicaffeine.git
cd logicaffeine

# Build the project
cargo build

# Run the test suite
cargo test -- --skip e2e
```

### Running Tests

```bash
# Run all tests (excluding e2e)
cargo test -- --skip e2e

# Run a specific phase file
cargo test -p logicaffeine-tests --test phase1_garden_path

# Filter tests by name
cargo test -p logicaffeine-tests garden_path

# Run with output
cargo test -- --nocapture

# Full suite via the nextest fast runner
./scripts/run-all-tests-fast.sh
```

## Development Workflow

We follow **Test-Driven Development (TDD)**:

1. **RED** - Write a failing test first
2. **GREEN** - Write minimal code to make it pass
3. **REFACTOR** - Clean up while keeping tests green

### Important Rules

- **Never modify failing tests** - Tests define the spec. If a test fails, fix the implementation, not the test.
- **Keep changes focused** - Don't add unrelated improvements in the same PR.
- **No over-engineering** - Only add what's needed for the current task.

## Code Style

- Arena allocation for all AST nodes
- Symbol interning for all strings
- Guard pattern for parser backtracking
- Visitor pattern for AST traversal

## Pull Request Process

1. Fork the repository
2. Create a feature branch (`git checkout -b feature/my-feature`)
3. Make your changes following TDD
4. Ensure all tests pass
5. Submit a pull request

### PR Guidelines

- Include a clear description of what changes were made and why
- Reference any related issues
- Keep commits focused and atomic
- Update documentation if needed

## Project Structure

```
logicaffeine/
├── crates/
│   ├── logicaffeine_base/       # Arenas, tokens, spans, value types
│   ├── logicaffeine_lexicon/    # English vocabulary tables
│   ├── logicaffeine_kernel/     # Calculus of Constructions kernel (no lexicon)
│   ├── logicaffeine_data/       # Runtime values + CRDTs (IO-free)
│   ├── logicaffeine_system/     # Platform IO, networking, persistence
│   ├── logicaffeine_language/   # English → FOL: lexer, parser, AST, transpiler
│   ├── logicaffeine_proof/      # Proof engine: solvers, tactics, certificates
│   ├── logicaffeine_compile/    # Compilation: parse → analyze → codegen / interpret / VM
│   ├── logicaffeine_forge/      # Copy-and-patch JIT (native)
│   ├── logicaffeine_jit/        # Wires the forge JIT into the VM (native)
│   ├── logicaffeine_runtime/    # Deterministic concurrency runtime
│   ├── logicaffeine_lsp/        # Language Server Protocol
│   ├── logicaffeine_verify/     # Z3 static verification (verification feature)
│   ├── logicaffeine_tv/         # SMT translation validation
│   ├── logicaffeine_synth/      # Offline Z3 stencil proofs
│   └── logicaffeine_tests/      # The integration suite (phase-organized)
├── apps/
│   ├── logicaffeine_cli/        # largo — the LOGOS build tool
│   └── logicaffeine_web/        # Dioxus web IDE
├── assets/std/                  # LOGOS stdlib prelude
├── docs/                        # Code-grounded documentation guides
└── scripts/                     # Test runners, doc generators, release tooling
```

## Test Phases

The suite lives in `crates/logicaffeine_tests/tests/` (600+ files), phase-organized by linguistic and logical complexity — garden paths, polarity, tense/aspect, wh-movement, and on through the higher phases — alongside the execution-tier, concurrency, wire-codec, and proof suites.

## Questions?

- Open an issue for bugs or feature requests
- Check existing issues before creating new ones

## License

By contributing, you agree that your contributions will be licensed under the project's BSL-1.1 license.
