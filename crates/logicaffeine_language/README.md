# logicaffeine-language

The English → first-order logic pipeline: lexer, parser, semantics, and transpiler that turn natural-language sentences into formal logic with neo-Davidsonian event semantics and ontology expansion.

Part of the [Logicaffeine](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/README.md) workspace. Tier 3 — depends on logicaffeine_base, logicaffeine_lexicon, logicaffeine_proof.

## Role in the workspace

This is the language front-end. It sits between the structural atoms (`logicaffeine-base` — arena, interner, spans), the vocabulary (`logicaffeine-lexicon`), and the proof engine (`logicaffeine-proof`), and is consumed by the compiler, CLI, LSP, and web app. See [logic-mode.md](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/docs/logic-mode.md).

```text
Input → Lexer → Parser (AST + DRS) → Semantics → Transpiler → FOL
```

- **Lexer** (`lexer`, `token`, `lexicon`) — two-stage: structural `LineLexer` then token `Lexer`, with morphology, multi-word-expression collapsing (`mwe`), and lexicon lookup.
- **Parser** (`parser/{clause,noun,verb,quantifier,modal,question,pragmatics}`) — hand-written recursive descent over arena-allocated AST, with discourse tracking via Discourse Representation Structures (`drs`). Declarative (NL) and imperative (LOGOS) modes selected by `ParserMode`.
- **AST** (`ast/{logic,stmt,theorem}`) — the arena-lifetime `LogicExpr<'a>` / `Term<'a>` logical form plus statement and theorem nodes.
- **Semantics** (`semantics/{axioms,kripke,knowledge_graph}`, `lambda`, `scope`) — axiom expansion, Kripke lowering for modals, intensional/event readings, and Montague-style lambda transforms.
- **Transpiler** (`transpile`, `formatter`) — renders the logical form to the target notation.

The parser handles both declarative natural language and the imperative LOGOS surface (selected by `ParserMode`), so recent imperative constructs — `## Define` definitional blocks and the `followed by` sequence-concatenation operator among them — flow through this same front-end; the full LOGOS surface is in the imperative-mode guide.

### Module map

Grouped by pipeline stage. `intern` and `arena` re-export the `logicaffeine-base` primitives so downstream crates need only depend on this one.

| Module | Role |
|--------|------|
| `token`, `lexer`, `lexicon`, `mwe` | tokenization: structural `LineLexer` → token `Lexer`, morphology, lexicon lookup, multi-word-expression collapsing |
| `parser`, `ast`, `arena_ctx` | recursive-descent parse into arena-allocated AST (`arena_ctx` owns the allocation context) |
| `drs`, `session`, `pragmatics` | discourse: Discourse Representation Structures, the incremental-evaluation `Session`, and post-parse pragmatic inference |
| `semantics`, `lambda`, `scope`, `ontology` | axiom expansion, Montague lambda transforms, quantifier-scope permutation, and sort/anaphora ontology checks |
| `transpile`, `formatter`, `view`, `style` | render the logical form; `view` is the owned serialization form, `style` adds ANSI coloring |
| `source_format` | the canonical LOGOS source formatter (`format_source`/`format_line`) — structural reindent (4 spaces per lexed nesting level), string/prose interiors untouched; one rule set shared by the LSP's formatting providers and `largo fmt` |
| `token_class` | the single token-classification truth (verbs=function, nouns=type, …) every highlighting surface derives from — LSP semantic tokens and the REPL's ANSI painting can never disagree |
| `teach` | the single teaching truth: a `ConstructDoc` lesson (one plain sentence + runnable example + socratic question/tip, all required by the type) for every taught keyword, every `##` block type, and the built-in types — LSP hover/completion docs and the REPL's `:explain` all derive from this table, ratcheted by `tests/teach_lock.rs`; also the literate-doc extractors (`module_doc`/`doc_for_header_at`/`extract_literate_docs`) that turn a `## Note` above a definition into its hover documentation |
| `analysis`, `registry`, `symbol_dict` | static discovery passes (`## Definition` scan), the symbol/type registries, and symbol-dictionary extraction |
| `optimization`, `proof_convert` | the shared `Opt` optimization bitset and the bridge from arena `LogicExpr` to the proof engine's owned `ProofExpr` |
| `ast_depth` | the nesting-depth gate: rejects programs whose AST nests deeper than every downstream walker (optimizer, codegen, interpreter, VM) can safely recurse — `AstTooDeep` at parse time instead of a stack overflow later |
| `error`, `suggest`, `visitor`, `debug`, `analysis` | parse-error types + Socratic explanations, spelling suggestions, the AST visitor, and interner-aware debug display |

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

Business Source License 1.1 — see [LICENSE.md](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/LICENSE.md).

---
[Docs index](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/docs/README.md) · [Root README](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/README.md) · [Changelog](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/CHANGELOG.md)
