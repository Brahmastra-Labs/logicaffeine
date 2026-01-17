# logicaffeine-tests

Integration test suite and E2E harness for the Logicaffeine ecosystem.

## Overview

This crate contains no library code - all tests run from the `tests/` directory. It provides:

- **Phase tests** for linguistic phenomena and language features
- **E2E tests** for full compilation pipeline (LOGOS → Rust → binary)
- **Test harness** with parsing, compilation, and assertion utilities

## Running Tests

Default workflow (skips slow E2E tests):

```bash
cargo test --workspace -- --skip e2e
```

Run all tests including E2E:

```bash
cargo test --workspace
```

Persistent logging for long test runs:

```bash
cargo test --workspace -- --skip e2e 2>&1 | tee test_output.log
```

## Test Organization

| Category | Count | Description |
|----------|-------|-------------|
| Phase tests | 171 | Linguistic phenomena, type theory, proofs |
| E2E tests | 29 | End-to-end compilation and execution |
| Debug tests | 7 | Diagnostic and targeting helpers |
| Other tests | 15 | Specialized areas (interpreter, parser, etc.) |

### Phase Ranges

| Range | Domain |
|-------|--------|
| 1-9 | Core linguistics (garden paths, polarity, tense, aspect) |
| 10-19 | Advanced linguistics (ellipsis, metaphor, plurality) |
| 20-40 | Distributed systems (CRDT, gossip, mesh, consensus) |
| 41-50 | Verification and static analysis |
| 60-69 | Proof theory and derivations |
| 70-79 | Kernel type checking and inductive types |
| 80-99 | Advanced type theory (universes, tactics, decidability) |
| 100+ | Polymorphism, generics, summit challenges |

## Test Harness API

Located in `tests/common/mod.rs`.

### Result Types

```rust
pub struct E2EResult {
    pub stdout: String,
    pub stderr: String,
    pub success: bool,
    pub rust_code: String,
}

pub struct CompileResult {
    pub binary_path: PathBuf,
    pub stderr: String,
    pub success: bool,
    pub rust_code: String,
}

pub struct InterpreterTestResult {
    pub output: String,
    pub error: String,
    pub success: bool,
}
```

### Functions

**Parsing:**

```rust
let view = parse_to_view("Every cat sleeps.");
```

**Compilation (no execution):**

```rust
let result = compile_logos("print 42.");
assert!(result.success);
```

**Compilation + Execution:**

```rust
let result = run_logos("print 42.");
assert!(result.success);
assert!(result.stdout.contains("42"));
```

**Interpreter (no Rust compilation):**

```rust
let result = run_interpreter("print 42.");
assert!(result.success);
```

**Assertions:**

```rust
assert_output("print 42.", "42");
assert_runs("let x = 1.");
assert_panics("assert false.", "assertion");
```

### Macros

```rust
let view = parse!("Every cat sleeps.");

assert_snapshot!("my_test", actual_output);
```

## Snapshot Testing

Snapshots are stored in `tests/snapshots/*.txt`.

Update all snapshots:

```bash
UPDATE_SNAPSHOTS=1 cargo test
```

## Dependencies

| Crate | Purpose |
|-------|---------|
| `logicaffeine-base` | Arena, interner, span types |
| `logicaffeine-kernel` | Type checker verification |
| `logicaffeine-language` | Parser, lexer, AST |
| `logicaffeine-compile` | LOGOS → Rust compilation |
| `logicaffeine-proof` | Proof derivation tests |
| `logicaffeine-data` | Runtime data structures |
| `logicaffeine-system` | Distributed system primitives |
| `tempfile` | Temporary directories for E2E |
| `futures` | Async interpreter execution |
| `tokio` | Async runtime for tests |

## License

BUSL-1.1
