# logicaffeine-language

Natural language to first-order logic transpilation pipeline.

This crate provides a complete system for parsing English sentences and producing
formal logical representations in various notations. It handles quantifier scope,
pronoun resolution, modal logic, and ambiguity in natural language.

Part of the [Logicaffeine](https://logicaffeine.com) project.

## Quick Start

```rust
use logicaffeine_language::compile;

let fol = compile("Every philosopher is wise.").unwrap();
// → "∀x(Philosopher(x) → Wise(x))"

let fol = compile("Socrates is a philosopher.").unwrap();
// → "Philosopher(socrates)"
```

## Output Formats

The crate supports multiple output formats for different contexts:

| Format | Function | Example | Use Case |
|--------|----------|---------|----------|
| Unicode | `compile()` | `∀x(P(x) → Q(x))` | Terminal, documentation |
| LaTeX | `compile_with_options()` | `\forall x(P(x) \to Q(x))` | Academic papers |
| SimpleFOL | `compile_simple()` | `Ax(P(x) -> Q(x))` | ASCII-only environments |
| Kripke | `compile_kripke()` | Explicit world quantification | Modal logic analysis |

```rust
use logicaffeine_language::{compile_simple, compile_kripke, compile_with_options};
use logicaffeine_language::{CompileOptions, OutputFormat};

// ASCII output
let ascii = compile_simple("Every cat sleeps.").unwrap();
// → "Ax(Cat(x) -> Ez(Sleep(z) & Agent(z, x)))"

// LaTeX output
let opts = CompileOptions { format: OutputFormat::LaTeX };
let latex = compile_with_options("Some dog barks.", opts).unwrap();

// Modal logic with explicit worlds
let kripke = compile_kripke("Necessarily, every truth is necessary.").unwrap();
```

## Multi-Sentence Discourse

For pronoun resolution and anaphora tracking across multiple sentences, use `Session`:

```rust
use logicaffeine_language::Session;

let mut session = Session::new();
session.eval("A man walked in.").unwrap();
session.eval("He sat down.").unwrap(); // "He" resolves to "a man"

// Access the accumulated logical form
let result = session.result();
```

For finer control over discourse state, use the lower-level API:

```rust
use logicaffeine_language::{compile_with_discourse, WorldState, Interner};

let mut world_state = WorldState::new();
let mut interner = Interner::new();

let fol1 = compile_with_discourse("A philosopher entered.", &mut world_state, &mut interner).unwrap();
let fol2 = compile_with_discourse("She was wise.", &mut world_state, &mut interner).unwrap();
```

## Handling Ambiguity

Natural language is inherently ambiguous. This crate provides strategies for each type:

### Structural & Lexical Ambiguity

`compile_forest` returns all valid parse readings for lexical ambiguity (noun/verb)
and structural ambiguity (PP attachment):

```rust
use logicaffeine_language::compile_forest;

let readings = compile_forest("I saw the man with the telescope.").unwrap();
// Returns both: "saw using telescope" and "man has telescope"
```

### Quantifier Scope Ambiguity

`compile_all_scopes` returns all quantifier scope permutations:

```rust
use logicaffeine_language::compile_all_scopes;

let scopes = compile_all_scopes("Every woman loves a man.").unwrap();
// Surface scope: ∀x(Woman(x) → ∃y(Man(y) ∧ Loves(x, y)))
// Inverse scope: ∃y(Man(y) ∧ ∀x(Woman(x) → Loves(x, y)))
```

### Combined Ambiguity

`compile_ambiguous` handles both structural and scope ambiguity together:

```rust
use logicaffeine_language::compile_ambiguous;

let all_readings = compile_ambiguous("Every student read a book about logic.").unwrap();
```

## Core API Reference

### Single Sentence

| Function | Description |
|----------|-------------|
| `compile(input)` | Parse and transpile to Unicode FOL |
| `compile_simple(input)` | Parse and transpile to ASCII FOL |
| `compile_kripke(input)` | Modal logic with explicit world quantification |
| `compile_with_options(input, opts)` | Custom output format |
| `compile_theorem(input)` | Parse as a theorem for proof engine |

### Ambiguity

| Function | Description |
|----------|-------------|
| `compile_forest(input)` | All parse readings (lexical/structural) |
| `compile_all_scopes(input)` | All quantifier scope permutations |
| `compile_ambiguous(input)` | All readings × all scopes |

### Discourse

| Function | Description |
|----------|-------------|
| `compile_with_discourse(input, world, interner)` | Single sentence with DRS tracking |
| `compile_discourse(sentences)` | Multiple sentences with shared context |
| `Session::new()` | REPL-style incremental evaluation |

## Architecture

The pipeline consists of four stages:

```
Input → Lexer → Parser → Semantics → Transpiler → FOL Output
```

1. **Lexer** (`lexer`) - Tokenizes natural language input, handles vocabulary lookup
   and morphological analysis via the lexicon.

2. **Parser** (`parser`) - Constructs a logical AST with discourse tracking via
   Discourse Representation Structures (`drs`). Uses arena allocation for efficiency.

3. **Semantics** (`semantics`) - Applies axiom expansion, Kripke lowering for
   modal logic, and intensional readings. Neo-Davidsonian event semantics.

4. **Transpiler** (`transpile`) - Renders the AST to the target notation format.

### Key Types

- `Token` - Lexical tokens with spans and semantic features
- `Parser` - Configurable recursive descent parser
- `Drs` / `WorldState` - Discourse representation for pronoun tracking
- `Session` - Stateful multi-sentence evaluation
- `OutputFormat` - Unicode, LaTeX, SimpleFOL, or Kripke

## Feature Flags

```toml
[dependencies]
logicaffeine-language = "0.1"

# Or with dynamic lexicon loading
logicaffeine-language = { version = "0.1", features = ["dynamic-lexicon"] }
```

| Feature | Description |
|---------|-------------|
| `dynamic-lexicon` | Runtime lexicon loading via `runtime_lexicon` module |

## Dependencies

This crate builds on:

- `logicaffeine-base` - Arena allocation, symbol interning
- `logicaffeine-lexicon` - Vocabulary database with verb frames, noun features

## Re-exported Types

For convenience, key types from dependencies are re-exported:

```rust
use logicaffeine_language::{Arena, Interner, Symbol};
```

## License

Business Source License 1.1 (BUSL-1.1)

- **Free** for individuals and organizations with <25 employees
- **Commercial license** required for organizations with 25+ employees offering Logic Services
- **Converts to MIT** on December 24, 2029

See [LICENSE](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/LICENSE.md) for full terms.
