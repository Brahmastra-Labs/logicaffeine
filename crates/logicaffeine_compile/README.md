# logicaffeine-compile

LOGOS compilation pipeline - codegen and interpreter.

Transforms LOGOS AST into executable Rust code through discovery, escape analysis, ownership analysis, and code generation phases.

## Compilation Pipeline

```
LOGOS Source → Discovery → Parsing → Escape Analysis → Ownership Analysis → CodeGen → Rust
```

| Phase | Description |
|-------|-------------|
| Discovery | Scans for type/policy definitions, builds registries, resolves module imports |
| Parsing | Builds arena-allocated AST with type context |
| Escape Analysis | Zone containment checking ("Hotel California" rule) |
| Ownership Analysis | Use-after-move detection via data-flow analysis |
| CodeGen | Transforms AST to Rust source code |

## Features

- Multi-pass analysis pipeline
- Dual compilation modes (direct Rust gen vs kernel extraction)
- Tree-walking interpreter for rapid execution
- Ownership system (Give/Show semantics)
- Zone safety checking
- Async VFS support
- Optional Z3-based verification

## Public API

### High-level UI Functions

```rust
use logicaffeine_compile::{
    compile_for_ui,          // Compilation for web interface
    compile_for_proof,       // Proof term extraction
    compile_theorem_for_ui,  // Theorem compilation
    verify_theorem,          // Z3 verification (requires verification feature)
    interpret_for_ui,        // Tree-walking interpretation
    generate_rust_code,      // Rust code generation (requires codegen feature)
};
```

### Result Types

- `CompileResult` - Standard compilation result with AST and tokens
- `ProofCompileResult` - Compilation result with proof terms
- `TheoremCompileResult` - Theorem compilation with extracted kernels
- `AstNode`, `TokenInfo`, `TokenCategory` - UI-friendly AST representation

### Re-exports

```rust
// Base types
pub use logicaffeine_base::{Arena, Interner, Symbol, SymbolEq};

// Language types
pub use logicaffeine_language::{ast, drs, error, lexer, parser, token};

// Analysis types
pub use logicaffeine_language::analysis::{
    TypeRegistry, DiscoveryPass, PolicyRegistry, PolicyCondition
};

// Module loading
pub use loader::{Loader, ModuleSource};
```

## Feature Flags

| Feature | Default | Description |
|---------|---------|-------------|
| `codegen` | Yes | Rust code generation support |
| `interpreter-only` | No | Interpreter without codegen |
| `verification` | No | Z3-based static verification (requires logicaffeine-verify) |

```toml
[dependencies]
# Default (with codegen)
logicaffeine-compile = { path = "../logicaffeine_compile" }

# Interpreter only
logicaffeine-compile = { path = "../logicaffeine_compile", default-features = false, features = ["interpreter-only"] }

# With Z3 verification
logicaffeine-compile = { path = "../logicaffeine_compile", features = ["verification"] }
```

## Module Structure

| Module | Purpose |
|--------|---------|
| `compile` | Main compilation pipeline |
| `codegen` | AST to Rust code generation |
| `interpreter` | Tree-walking async interpreter |
| `analysis/escape` | Zone escape checking |
| `analysis/ownership` | Linear type / ownership analysis |
| `analysis/discovery` | Type discovery with imports |
| `extraction/` | Kernel term extraction for proofs |
| `loader` | Module resolution (file:, logos: URIs) |
| `ui_bridge` | High-level UI-friendly API |
| `diagnostic` | Error bridging from rustc |
| `sourcemap` | Generated Rust to LOGOS source mapping |
| `verification` | Z3 verification bridge (optional) |

## Dependencies

### Internal Crates

| Crate | Purpose |
|-------|---------|
| `logicaffeine-base` | Arena allocation, symbol interning |
| `logicaffeine-language` | AST, lexer, parser, analysis |
| `logicaffeine-kernel` | Inductive types, formal definitions |
| `logicaffeine-data` | Runtime data structures |
| `logicaffeine-system` | VFS, persistence |
| `logicaffeine-proof` | Proof derivation trees |
| `logicaffeine-verify` | Z3 wrapper (optional) |

### External Crates

| Crate | Purpose |
|-------|---------|
| `async-recursion` | Async recursion support for interpreter |
| `serde` / `serde_json` | Serialization |
| `include_dir` | Embedded standard library |

## Usage

### Basic Compilation

```rust
use logicaffeine_compile::compile_for_ui;

let source = r#"
type Person { name: String }
let p = Person { name: "Alice" }
"#;

let result = compile_for_ui(source);
match result {
    Ok(compiled) => {
        println!("AST: {:?}", compiled.ast);
        println!("Tokens: {:?}", compiled.tokens);
    }
    Err(e) => eprintln!("Compilation error: {}", e),
}
```

### With Verification

```rust
#[cfg(feature = "verification")]
use logicaffeine_compile::verify_theorem;

let theorem_source = "theorem example: forall x. P(x) -> P(x)";
let result = verify_theorem(theorem_source);
```

## License

BUSL-1.1
