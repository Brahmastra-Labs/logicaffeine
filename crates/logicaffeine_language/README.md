# logicaffeine-language

The English → first-order logic pipeline: lexer, parser, semantics, and transpiler that turn natural-language sentences into formal logic with neo-Davidsonian event semantics and ontology expansion.

Part of the [Logicaffeine](../../NEW_README.md) workspace. Tier 3 — depends on logicaffeine_base, logicaffeine_lexicon, logicaffeine_proof.

## Role in the workspace

This is the language front-end. It sits between the structural atoms (`logicaffeine-base` — arena, interner, spans), the vocabulary (`logicaffeine-lexicon`), and the proof engine (`logicaffeine-proof`), and is consumed by the compiler, CLI, LSP, and web app. See [../../new_docs/logic-mode.md](../../new_docs/logic-mode.md).

```
Input → Lexer → Parser (AST + DRS) → Semantics → Transpiler → FOL
```

- **Lexer** (`lexer`, `token`, `lexicon`) — two-stage: structural `LineLexer` then token `Lexer`, with morphology, multi-word-expression collapsing (`mwe`), and lexicon lookup.
- **Parser** (`parser/{clause,noun,verb,quantifier,modal,question,pragmatics}`) — hand-written recursive descent over arena-allocated AST, with discourse tracking via Discourse Representation Structures (`drs`). Declarative (NL) and imperative (LOGOS) modes selected by `ParserMode`.
- **AST** (`ast/{logic,stmt,theorem}`) — the arena-lifetime `LogicExpr<'a>` / `Term<'a>` logical form plus statement and theorem nodes.
- **Semantics** (`semantics/{axioms,kripke,knowledge_graph}`, `lambda`, `scope`) — axiom expansion, Kripke lowering for modals, intensional/event readings, and Montague-style lambda transforms.
- **Transpiler** (`transpile`, `formatter`) — renders the logical form to the target notation.

## Public API

All fallible entry points return `Result<_, ParseError>`; `compile_forest` is infallible (it returns whatever readings parse).

| Function | Signature | Purpose |
|----------|-----------|---------|
| `compile` | `(&str) -> Result<String, ParseError>` | Unicode FOL (default) |
| `compile_simple` | `(&str) -> Result<String, ParseError>` | ASCII SimpleFOL |
| `compile_kripke` | `(&str) -> Result<String, ParseError>` | modals as explicit world quantification |
| `compile_pragmatic` | `(&str) -> Result<String, ParseError>` | adds scalar-implicature enrichment (`some` ⇝ `∃ +> ¬∀`) |
| `compile_with_options` | `(&str, CompileOptions) -> Result<String, ParseError>` | choose `format` + `pragmatic` |
| `compile_all_scopes` | `(&str) -> Result<Vec<String>, ParseError>` | every quantifier-scope permutation |
| `compile_forest` | `(&str) -> Vec<String>` | every parse reading (noun/verb, PP attachment) |
| `compile_ambiguous` | `(&str) -> Result<Vec<String>, ParseError>` | readings × scopes |
| `compile_discourse` | `(&[&str]) -> Result<String, ParseError>` | batch with shared discourse context |
| `compile_with_discourse` | `(&str, &mut WorldState, &mut Interner) -> Result<String, ParseError>` | thread your own discourse state |
| `compile_theorem` | `(&str) -> Result<String, ParseError>` | parse + prove a `Given:`/`Prove:` block |

Each entry point has a `*_with_options` companion taking `CompileOptions`. `MAX_FOREST_READINGS` (= 12) caps forest size to bound combinatorial blowup. Lower-level variants (`compile_kripke_with`, `compile_with_world_state{,_options}`, `compile_with_world_state_interner_options`) expose the `WorldState` / `Interner` directly.

Key types and re-exports:

- `OutputFormat` — `Unicode` (default), `LaTeX`, `SimpleFOL`, `Kripke`.
- `CompileOptions { format: OutputFormat, pragmatic: bool }` — defaults to `{ Unicode, false }`.
- `Session` — multi-sentence discourse: `new` / `with_format`, `eval`, `history`, `turn_count`, `world_state{,_mut}`, `reset`. Carries anaphora state across `eval` calls.
- Pipeline types: `Lexer`, `LineLexer`, `LineToken`, `Token`/`TokenType`, `Parser`, `ParserMode`, `NegativeScopeMode`, `QuantifierParsing`, `Drs`/`WorldState`, `ParseError`/`ParseErrorKind`/`socratic_explanation`, `TypeRegistry`, `SymbolRegistry`, `AstContext`, `TranspileContext`.
- `proof_convert` (`logic_expr_to_proof_expr`, `term_to_proof_term`) bridges the arena `LogicExpr<'a>` to the proof engine's owned `ProofExpr`/`ProofTerm`; `compile_theorem` proves via `logicaffeine_proof`.
- `optimization` is the single source of truth for compiler optimization toggles (an `OptimizationConfig` `u64` bitset over the 40-pass `Opt` enum), shared across the AOT/run/VM/JIT/codegen paths. It lives here because the parser maps `## No <X>` source decorators onto `Opt`s.

## Quick example

```rust
use logicaffeine_language::compile;

let fol = compile("Every man is mortal.").unwrap();
assert_eq!(fol, "∀x((Man(x) → Mortal(x)))");
```

Discourse across sentences:

```rust
use logicaffeine_language::Session;

let mut s = Session::new();
s.eval("A man walked in.").unwrap();
s.eval("He sat down.").unwrap();   // "He" resolves to "a man"
let transcript = s.history();
```

## Feature flags

| Feature | Default | Description |
|---------|---------|-------------|
| `dynamic-lexicon` | off | Enables runtime lexicon loading (`logicaffeine-lexicon/dynamic-lexicon`), re-exported as the `runtime_lexicon` module. |

A `build.rs` step compiles `assets/lexicon.json` into generated Rust tables (lexicon, multi-word expressions, ontology, axioms) under `OUT_DIR`; changing the lexicon requires a rebuild.

## Dependencies

Internal: `logicaffeine-base` (arena, interner, symbols, spans — re-exported as `Arena`/`Interner`/`Symbol`), `logicaffeine-lexicon` (vocabulary), `logicaffeine-proof` (theorem proving).

External: `bumpalo` (arena allocation), `serde` + `serde_json` (hardware/ontology types and build-time lexicon compilation).

## License

Business Source License 1.1 — see [LICENSE.md](../../LICENSE.md).

---
[Docs index](../../new_docs/README.md) · [Root README](../../NEW_README.md) · [Changelog](../../CHANGELOG.md)
