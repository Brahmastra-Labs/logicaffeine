# logicaffeine-compile

Core compilation pipeline for LOGOS, transforming natural language logic into executable Rust code or interpreted results.

Part of the [Logicaffeine](https://logicaffeine.com) project.

## Pipeline Architecture

```text
LOGOS Source (.md)
      │
      ▼
┌───────────────────┐
│  1. Lexer         │  Tokenize source
└─────────┬─────────┘
          ▼
┌───────────────────┐
│  2. Discovery     │  Type & policy definitions
└─────────┬─────────┘
          ▼
┌───────────────────┐
│  3. Parser        │  Build AST
└─────────┬─────────┘
          ▼
┌───────────────────────────────────┐
│        4. Analysis Passes         │
│  ┌─────────┐     ┌───────────┐   │
│  │ Escape  │     │ Ownership │   │
│  └─────────┘     └───────────┘   │
└─────────┬─────────────────────────┘
          ▼
┌───────────────────┐
│  5. CodeGen       │  Emit Rust source
└─────────┬─────────┘
          ▼
    Rust Source
```

## Features

| Feature | Default | Description |
|---------|---------|-------------|
| `codegen` | Yes | Rust code generation |
| `interpreter-only` | No | Minimal build for interpretation only |

```toml
[dependencies]
logicaffeine-compile = "0.6"

# Or interpreter-only (smaller build)
logicaffeine-compile = { version = "0.6", default-features = false, features = ["interpreter-only"] }
```

## Quick Start

### Basic Compilation

```rust
use logicaffeine_compile::compile::compile_to_rust;

let source = "## Main\nLet x be 5.\nShow x to show.";
let rust_code = compile_to_rust(source)?;
// Generates:
// fn main() {
//     let x = 5;
//     println!("{}", x);
// }
```

### With Ownership Checking

```rust
use logicaffeine_compile::compile::compile_to_rust_checked;

let source = "## Main\nLet x be 5.\nGive x to y.\nShow x to show.";
let result = compile_to_rust_checked(source);
// Returns Err: "x has already been given away"
```

### Interpretation (Async)

```rust
use logicaffeine_compile::interpret_for_ui;

let source = "## Main\nLet x be 5.\nShow x to show.";
let result = interpret_for_ui(source).await;
// result.lines contains ["5"]
```

## API Overview

### Compilation Functions

| Function | Analysis | Use Case |
|----------|----------|----------|
| `compile_to_rust` | Escape | Basic compilation |
| `compile_to_rust_checked` | Escape + Ownership | `--check` flag |
| `compile_project` | Multi-file | Projects with imports |
| `compile_and_run` | Full + Execute | Development workflow |

For Z3 verification, see the `logicaffeine-verify` crate.

### UI Integration

| Function | Description |
|----------|-------------|
| `compile_for_ui` | Returns tokens, AST, and readings for web display |
| `interpret_for_ui` | Async execution with output capture |
| `generate_rust_code` | Generate Rust without building |
| `compile_for_proof` | Proof-mode compilation |
| `verify_theorem` | Theorem verification |

## Modules

| Module | Description |
|--------|-------------|
| `compile` | Top-level compilation pipeline |
| `codegen` | AST to Rust code generation |
| `interpreter` | Tree-walking async interpreter |
| `analysis` | Static analysis (escape, ownership, discovery) |
| `extraction` | Kernel term extraction to Rust |
| `diagnostic` | Rustc error translation |
| `sourcemap` | Source location mapping |
| `loader` | Multi-file module loading |
| `ui_bridge` | Web interface integration |

For Z3-based verification, see the `logicaffeine-verify` crate.

## Dependencies

### Internal Crates

| Crate | Purpose |
|-------|---------|
| `logicaffeine-base` | Arena allocation, interning |
| `logicaffeine-language` | Lexer, parser, AST |
| `logicaffeine-kernel` | Type theory core |
| `logicaffeine-data` | Runtime data types |
| `logicaffeine-system` | I/O and persistence |
| `logicaffeine-proof` | Proof construction |

### External

- `async-recursion` - Async recursive functions
- `serde`, `serde_json` - Serialization
- `include_dir` - Embedded standard library

## Error Handling

Compilation errors are returned as `ParseError`, which includes:

- **Lexical errors** - Invalid tokens, unterminated strings
- **Syntax errors** - Grammar violations, unexpected tokens
- **Semantic errors** - Type mismatches, undefined variables
- **Ownership errors** - Use-after-move, escape violations

The `diagnostic` module translates rustc errors back to LOGOS source locations when compiling generated code.

## License

Business Source License 1.1 (BUSL-1.1)

- **Free** for individuals and organizations with <25 employees
- **Commercial license** required for organizations with 25+ employees offering Logic Services
- **Converts to MIT** on December 24, 2029

See [LICENSE](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/LICENSE.md) for full terms.
