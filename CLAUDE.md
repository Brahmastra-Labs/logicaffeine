# Logos - AI Assistant Guidelines

## Critical Rules

1. **NEVER RUN GIT COMMANDS** - Do not use git under any circumstances
2. **STAY IN logos/** - Work only in this directory, ignore parent friendslop
3. **USE TDD** - Follow RED/GREEN test-driven development
4. **NEVER MODIFY RED TESTS** - Do not update failing tests without stopping and asking the user first. The test defines the spec; if a test fails, fix the implementation, not the test.
5. **RUNNING TESTS**
  Use `cargo test -- --skip e2e` when running tests unless asked to run all tests. BY DEFAULT, skip the e2e tests.
  When asked to run all tests run `cargo test`.
  When running tests, don't tail or head the outputs, just read the entire thing.
  During development, we will develop the RED test, then work until that passes, then run all our tests.
  For large refactors, we can selectively run existing tests to ensure we didn't break things.

## Specification Guidelines

1. Code snippets should not have comments unless they explain implementation detail
2. Clean quirky names with a technical voice but do not change the details
3. If you see issues, ask questions in planning mode
4. Do not leave snippets with random comments - only add things that provide value
5. Leave no trace when editing - do not mention what was wrong, do not keep old code "for legacy compatibility"
6. Do not add unrequested changes - if you have an idea, mention it but do not assume it is wanted
7. Doing things right is not scope creep - prefer the best possible approach over the fastest or easiest

## TDD Workflow

1. **RED** - Write a failing test first
2. **GREEN** - Write minimal code to make it pass
3. **REFACTOR** - Clean up while keeping tests green

```bash
cargo test                           # Run all tests
cargo test --test phase1_garden_path # Run specific phase
cargo test test_name                 # Run specific test
```

## Project Overview

Logos is an English-to-First-Order-Logic transpiler. It parses natural language and outputs formal logic in Unicode or LaTeX.

**Pipeline:** Input → Lexer → Parser → AST → Transpiler → FOL Output

## Key Directories

```
logos/
├── assets/lexicon.json    # Vocabulary database
├── src/
│   ├── lexer.rs          # Tokenization
│   ├── parser/           # Parser modules
│   ├── ast.rs            # AST types
│   ├── transpile.rs      # Output generation
│   ├── compile.rs        # Compilation pipeline
│   └── cli.rs            # CLI (largo) commands
├── logos_core/           # Runtime library for compiled programs
├── logos_verification/   # Z3 static verification (optional)
└── tests/                # Phase-organized tests
```

## Test Phases

Tests are organized by linguistic complexity:
- Phase 1: Garden path sentences
- Phase 2: Polarity items
- Phase 3: Tense & Aspect
- Phase 4: Movement & Reciprocals
- Phase 5: Wh-movement
- Phase 6-14: Advanced phenomena
- Phase 42: Z3 Static Verification (requires `verification` feature)

## Lexicon System

The `assets/lexicon.json` file defines all vocabulary:
- **Verbs**: Vendler class, transitivity, control features
- **Nouns**: Animacy, gender, number
- **Adjectives**: Intersective, subsective, gradable

Changes to lexicon.json require `cargo build` to regenerate.

## Code Patterns

- **Arena allocation**: AST nodes use bumpalo arenas
- **ParserGuard**: RAII pattern for parser backtracking
- **Symbol interning**: Strings interned for efficiency

## Commands

```bash
cargo test           # Run tests
cargo build          # Build
cargo run            # REPL mode
./generate-docs.sh   # Regenerate docs
```

## Feature Flags

| Feature | Description |
|---------|-------------|
| `cli` | Enables the `largo` CLI tool |
| `verification` | Enables Z3-based static verification (requires Z3 installed) |

```bash
# Build with CLI
cargo build --features cli

# Build with verification (requires Z3)
cargo build --features verification

# Build with both
cargo build --features cli,verification
```

## Z3 Static Verification

The `logos_verification` crate provides Z3-based static verification. It requires Z3 to be installed on the system.

### Setup (macOS)

```bash
brew install z3

# Set environment variables for building
export Z3_SYS_Z3_HEADER=/opt/homebrew/include/z3.h
export BINDGEN_EXTRA_CLANG_ARGS="-I/opt/homebrew/include"
export LIBRARY_PATH="/opt/homebrew/lib"
```

### Running Verification Tests

```bash
# Tests WITHOUT verification (default, no Z3 needed)
cargo test -- --skip e2e

# Tests WITH verification (requires Z3)
cargo test --features verification -- --skip e2e

# Only verification tests
cargo test --features verification --test phase_verification
```

### Crate Structure

```
logos_verification/
├── Cargo.toml
└── src/
    ├── lib.rs        # Public API
    ├── solver.rs     # Z3 Verifier wrapper
    ├── license.rs    # Stripe license validation
    └── error.rs      # Socratic error messages
```

### License Gating

Verification is gated by license. Valid license keys are Stripe subscription IDs (`sub_*` format) validated against `api.logicaffeine.com/validate`. Only Pro, Premium, Lifetime, and Enterprise plans can use verification.

## Updating Documentation (generate-docs.sh)

When asked to update documentation, follow this process:

### 1. Audit for Missing Features

Compare source code against documentation:
- Check `tests/` for new phase files (phase13_*, phase14_*, etc.)
- Check `src/` for new modules not in add_file calls
- Grep for new functions/patterns in lexer.rs, parser/mod.rs
- Look for new Token types, Expr variants, Term variants

### 2. Sections to Update (in order)

| Section | Location | What to Update |
|---------|----------|----------------|
| Table of Contents | Lines 40-55 | Add new phases, sections |
| Key Design Decisions | Lines 109-150 | Add architectural bullets |
| Word Classification Priority | Line ~176 | Add rows for new ambiguity patterns |
| Lexical Ambiguity | Line ~185 | Add new ambiguity patterns |
| Linguistic Phenomena | Lines 480-720 | Add new linguistic features |
| Glossary | Lines 1000-1230 | Add implementation terms |
| Test Descriptions | Lines 1245-1380 | Add `add_test_description` calls |
| Source Modules | Lines 1520-1680 | Add `add_file` calls for new .rs files |
| Lexer Description | Line ~1530 | Update with new lexer features |
| Parser Description | Line ~1550 | Update with new parser features |

### 3. Checklist

Before running `./generate-docs.sh`:
- [ ] Table of Contents matches actual phases
- [ ] All test phases have `add_test_description` entries
- [ ] All src/*.rs files have `add_file` entries
- [ ] New glossary terms added for new concepts
- [ ] Lexer/Parser descriptions mention new features
- [ ] Linguistic Phenomena covers new syntax patterns
- [ ] Design decisions include new architectural patterns

### 4. Verification

After running `./generate-docs.sh`:
```bash
# Verify new content appears
grep -n "Phase 13\|Phase 14\|<new-feature>" LOGOS_DOCUMENTATION.md
```
