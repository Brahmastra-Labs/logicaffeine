# logicaffeine-language

The LOGOS front end: one lexer and parser that turn English sentences into first-order logic and read imperative LOGOS programs and LGX pages (LGX is the web framework) into the AST every execution tier runs. English is ambiguous where logic cannot be: `Every woman loves a man.` has two quantifier scopes, and `compile_all_scopes` hands back both readings instead of silently picking one. It stops at the AST and the logical form; running and compiling programs is `logicaffeine-compile`'s job.

Part of the [Logicaffeine](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/README.md) workspace, tier 3. A `build.rs` step compiles the vocabulary from `logicaffeine-lexicon` into generated lookup tables.

## Role in the workspace

This is the language front end. It sits on the structural atoms (`logicaffeine-base`: arena, interner, spans), the vocabulary (`logicaffeine-lexicon`) and the proof engine (`logicaffeine-proof`), and the compiler, CLI, LSP and web app consume it. See [logic-mode.md](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/docs/logic-mode.md).

```text
Input → Lexer → Parser (AST + DRS) → Semantics → Transpiler → FOL
```

- **Lexer** (`lexer`, `token`, `lexicon`): two stages, a structural `LineLexer` and then the token `Lexer`, with morphology, multi-word-expression collapsing (`mwe`) and lexicon lookup.
- **Parser** (`parser/{clause,noun,verb,quantifier,modal,question,pragmatics,math}`): hand-written recursive descent over an arena-allocated AST, tracking discourse with Discourse Representation Structures (`drs`). `ParserMode` selects declarative English or imperative LOGOS. `parser/math.rs` is the theorem surface's math layer: quantified arithmetic in English (`For all n, n plus 0 equals n.`), which `proof_convert` lowers to an `Int` identity the proof engine's ring oracle decides.
- **AST** (`ast/{logic,stmt,theorem}`, plus `ast/{component,view}` for LGX): the arena-lifetime `LogicExpr<'a>` / `Term<'a>` logical form plus statement, theorem, component and view nodes.
- **Semantics** (`semantics/{axioms,kripke,knowledge_graph}`, `lambda`): axiom expansion, Kripke lowering for modals, intensional and event readings, Montague-style lambda transforms, and quantifier-scope enumeration.
- **Transpiler** (`transpile`, `formatter`): renders the logical form in the target notation.

## Example

```rust
use logicaffeine_language::compile;

let fol = compile("Every man is mortal.").unwrap();
assert_eq!(fol, "∀x((Man(x) → Mortal(x)))");
```

Every quantifier-scope reading of an ambiguous sentence:

```rust
use logicaffeine_language::compile_all_scopes;

let readings = compile_all_scopes("Every woman loves a man.").unwrap();
assert!(readings.len() >= 2);
```

Discourse across sentences, where a pronoun resolves against an earlier turn (an unresolved pronoun would print as `?`):

```rust
use logicaffeine_language::Session;

let mut session = Session::new();
let first = session.eval("A man entered.").unwrap();
assert!(first.contains("Man"));

let second = session.eval("He sat.").unwrap();
assert!(second.contains("Sit"));
assert!(!second.contains("?"));
```

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

`compile_all_scopes`, `compile_forest`, `compile_ambiguous`, `compile_discourse` and `compile_with_world_state` each have a `*_with_options` companion taking `CompileOptions`. `MAX_FOREST_READINGS` (= 12) caps forest size to bound combinatorial blowup. Lower-level variants (`compile_kripke_with`, `compile_with_world_state{,_options}`, `compile_with_world_state_interner_options`) expose the `WorldState` / `Interner` directly.

Key types and re-exports:

- `OutputFormat`: `Unicode` (default), `LaTeX`, `SimpleFOL`, `Kripke`.
- `CompileOptions { format: OutputFormat, pragmatic: bool }`, defaulting to `{ Unicode, false }`.
- `Session`: multi-sentence discourse with `new` / `with_format`, `eval`, `history`, `turn_count`, `world_state{,_mut}`, `reset`. It carries anaphora state across `eval` calls.
- Pipeline types: `Lexer`, `LineLexer`, `LineToken`, `Token`/`TokenType`, `Parser`, `ParserMode`, `NegativeScopeMode`, `QuantifierParsing`, `Drs`/`WorldState`, `ParseError`/`ParseErrorKind`/`socratic_explanation`, `TypeRegistry`, `SymbolRegistry`, `AstContext`, `TranspileContext`.
- `proof_convert` (`logic_expr_to_proof_expr`, `term_to_proof_term`) bridges the arena `LogicExpr<'a>` to the proof engine's owned `ProofExpr`/`ProofTerm`; `compile_theorem` proves through `logicaffeine_proof`.
- `optimization` holds the compiler's optimization toggles: an `OptimizationConfig` `u64` bitset with one bit per `Opt` pass (`OPT_COUNT` of them), shared by the AOT, run, VM, JIT and codegen paths. It lives here because the parser maps the `## No <X>` and `## Require <X>` source decorators onto `Opt`s: `## No` turns a pass off, and `## Require` fails compilation unless the pass fired (`Parser::required_optimizations`).

## The imperative surface

The same parser reads the imperative LOGOS surface, so every imperative construct flows through this front end; [imperative-mode.md](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/docs/imperative-mode.md) is the guide to the core surface. Among the imperative forms it parses:

- modules: `Import Geometry.`, `Import Geometry and Colors.`, `Import Geometry from "shapes/geometry.lg".`, `Import Geometry as Geo.`, selective `Import double from Util.`, qualified `Geometry::Point`, and `private` definitions (`analysis::dependencies` resolves them; `import_format` sorts the block);
- the test framework: `## Test: <sentence>` blocks with `Expect that …`, `Expect this to fail …:`, `Expect output:`, `Advance time by …`, `Given a file …`, case tables (`For each of these cases:`) and seeded, shrinking properties (`For all integers a and b …:`);
- contracts and errors: `Fail with <reason>.`, `, otherwise <reason>` on `Assert`/`Require`/`Assume`, and dimensioned range contracts (`Assume v is between 0.2 volts and 0.7 volts.`);
- observation statements (`Note that …`, `Warn that …`, `Record that …`, `During <gerund>:`), `Swap x and y.`, `Reversibly:` blocks, `Converge remote into local.`, `Close ch.`, task joins (`Await h into r.`), `Solve … for x.` / `Minimize …`, and `every x in xs where …` selection;
- tensor sugar (`a R by C tensor of Real`, `matmul`, `softmax of`, `rmsnorm of`, `attention of … over … and …`, and more), desugared to row-major `Seq` loops every execution tier runs;
- LGX pages and components (`## Page`, `## Component`, `## Store`, `## Directive`, `## Route`, `## Before each request`, `## Redirect`), parsed into first-class AST by `parser/component.rs` and lowered to LOGOS by `parser/lgx_lower`;
- tuple and record types in signatures (`(Int, Text)`, `(quotient: Int, remainder: Int)`) with `Let (a, b) be …` destructuring;
- `## Define` definitional blocks and the `followed by` sequence-concatenation operator.

A theorem's `Proof:` line is read as a sketch: `compile::run_proof_sketch` runs it through the proof engine's tactic-script language and discharges the obligations it leaves with the certified automation, reporting how many. A step that errors is never swept.

## Module map

Grouped by pipeline stage. `intern` and `arena` re-export the `logicaffeine-base` primitives, so downstream crates need only depend on this one.

| Module | Role |
|--------|------|
| `token`, `lexer`, `lexicon`, `mwe` | tokenization: structural `LineLexer` → token `Lexer`, morphology, lexicon lookup, multi-word-expression collapsing |
| `parser`, `ast`, `arena_ctx` | recursive-descent parse into an arena-allocated AST (`arena_ctx` owns the allocation context) |
| `lint` | post-parse warnings over an already-built `Stmt` body: `CopyLintScope`, the value-semantics copy lint (a local copied from a field or another local, mutated, then never read or written back) |
| `lgx` | the LGX template layer. `html` is the flat HTML tokenizer, with the generated WHATWG `entities` table. `template` builds the `{…}` tag tree: `{if}`/`{for each}`/`{inspect}`/`{let}`/`{extends}`/`{block}`/`{children}`/`{raw}`, holes with filters, components and directives. `css` scopes CSS to its component, `raw` maps raw-block offsets, and `deps` is the static read/write dependency graph a fine-grained DOM patcher is compiled from. `check` and `selector` are the structural checks a template passes before any LOGOS expression in it is parsed. Every embedded expression stays text plus its offset; `parser/component.rs` re-lexes it with the imperative lexer (`Lexer::new_imperative`) and parses it with the real expression parser into the view IR (`ast::view`) |
| `drs`, `session`, `pragmatics` | discourse: Discourse Representation Structures, the incremental-evaluation `Session`, and post-parse pragmatic inference |
| `semantics`, `lambda`, `ontology` | axiom expansion, Montague lambda transforms and quantifier-scope enumeration (`lambda::enumerate_scopings`), and the part-whole and sort lookups behind bridging anaphora and metaphor detection |
| `scope` | a stack of variable bindings and their ownership states for imperative code generation |
| `transpile`, `formatter`, `view`, `style` | render the logical form; `view` is the owned serialization form, and `style` adds ANSI coloring |
| `printer` | render an imperative `Expr` (and its subexpressions) back to English, the basis of power-assert failure annotations, property counterexamples, and mutation-survivor reports |
| `source_format`, `import_format` | the canonical LOGOS source formatter (`format_source`/`format_line`): structural reindent at 4 spaces per lexed nesting level, with string and prose interiors untouched, and one rule set shared by the LSP's formatting providers and `largo fmt`. Formatting never changes the token stream, so `import_format` (`normalize_imports`), which sorts and de-duplicates a module's `Import` block, runs as a separate `largo fmt` pass |
| `token_class` | the token classification (verbs as functions, nouns as types, and so on) that every highlighting surface derives from, so LSP semantic tokens and the REPL's ANSI painting cannot disagree |
| `teach` | the lesson table: a `ConstructDoc` (one plain sentence, a runnable example, and a Socratic question or tip, all required by the type) for every taught keyword, every `##` block type, and the built-in types. LSP hover and completion docs and the REPL's `:explain` all read from it, and `crates/logicaffeine_tests/tests/teach_lock.rs` ratchets it. It also holds the literate-doc extractors (`module_doc`/`doc_for_header_at`/`extract_literate_docs`) that turn a `## Note` above a definition into its hover documentation |
| `analysis`, `registry`, `symbol_dict` | static discovery passes (`## Definition` scan, import scan), the type and symbol registries, and symbol-dictionary extraction |
| `optimization`, `proof_convert` | the shared `Opt` optimization bitset, and the bridge from arena `LogicExpr` to the proof engine's owned `ProofExpr` |
| `ast_map` | the exhaustive rebuilding traversal of the imperative AST. A pass overrides the hooks for the nodes it rewrites and walks the rest. Every `Stmt`/`Expr`/`TypeExpr` variant is named with no wildcard arm, so a new variant does not compile until someone decides how it is traversed |
| `ast_depth` | the nesting-depth gate: it rejects programs whose AST nests deeper than every downstream walker (optimizer, codegen, interpreter, VM) can safely recurse, reporting `AstTooDeep` at parse time instead of a stack overflow later |
| `error`, `suggest`, `visitor`, `debug` | parse-error types with Socratic explanations, spelling suggestions, the AST visitor, and interner-aware debug display |
| `diagnostic` | the canonical machine-readable diagnostic surface (`logos-diagnostic/1`): a stable kebab-case code, a severity and a one-line headline for every `ParseErrorKind`, decided in wildcard-free matches so a new kind does not compile undecided. Its `Diagnostic` JSON shape is shared by `largo check --json`, `largo run --json` and the MCP `logos_check` tool, the language server uses the same codes and severities, and the rendered `catalog()` backs `largo explain` and the generated error catalog |

## Feature flags

| Feature | Default | Enables |
|---------|---------|---------|
| `dynamic-lexicon` | off | Runtime lexicon loading (`logicaffeine-lexicon/dynamic-lexicon`), re-exported as the `runtime_lexicon` module. |
| `lexicon-reference` | off | Test-only: `lexicon::reference`, every generated word table written as a linear `match`, the oracle the word-table differential checks the generated tables against. |

The `build.rs` step compiles the vocabulary database (`logicaffeine_lexicon::LEXICON_JSON`, taken as a build-dependency) into generated Rust tables for the lexicon, multi-word expressions, ontology and axioms under `OUT_DIR`, so changing the lexicon requires a rebuild.

## Dependencies

Internal: `logicaffeine-base` (arena, interner, symbols, spans, re-exported as `Arena`/`Interner`/`Symbol`), `logicaffeine-lexicon` (vocabulary), `logicaffeine-proof` (theorem proving).

External: `bumpalo` (arena allocation), `serde` + `serde_json` (hardware and ontology types, and build-time lexicon compilation).

## License

MIT License — see [LICENSE.md](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/LICENSE.md).

---
[Docs index](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/docs/README.md) · [Root README](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/README.md) · [Changelog](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/crates/logicaffeine_language/CHANGELOG.md)
