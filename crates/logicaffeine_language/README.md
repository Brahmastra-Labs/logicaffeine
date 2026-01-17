# logicaffeine-language

Natural language to first-order logic pipeline.

This crate is the core NL→FOL transpiler. It parses English sentences and outputs formal logic representations in multiple formats (Unicode, LaTeX, SimpleFOL, Kripke).

**Key capabilities:** quantifier scope resolution, modal logic, tense/aspect, anaphora resolution, discourse tracking.

## Quick Start

```rust
use logicaffeine_language::compile;

let fol = compile("Every student reads a book.").unwrap();
// Output: ∀x(Student(x) → ∃y(Book(y) ∧ Read(x, y)))
```

## Compilation API

### Basic Functions

| Function | Description |
|----------|-------------|
| `compile(input)` | NL→FOL with Unicode output |
| `compile_simple(input)` | SimpleFOL format |
| `compile_kripke(input)` | Modal semantics with explicit world quantification |
| `compile_with_options(input, options)` | Custom output format |

### Discourse & State

| Function | Description |
|----------|-------------|
| `compile_with_world_state(input, world_state)` | Persistent discourse state |
| `compile_with_world_state_options(input, world_state, options)` | WorldState + output format |
| `compile_with_discourse(input, world_state, interner)` | Full control for cross-sentence anaphora |
| `compile_with_world_state_interner_options(input, ws, interner, opts)` | Full control with options |
| `compile_discourse(sentences)` | Multiple sentences with temporal ordering |
| `compile_discourse_with_options(sentences, options)` | Discourse with output format |

### Ambiguity Resolution

| Function | Description |
|----------|-------------|
| `compile_all_scopes(input)` | All quantifier scope orderings |
| `compile_all_scopes_with_options(input, options)` | Scope orderings with output format |
| `compile_forest(input)` | All parse trees (structural/lexical ambiguity) |
| `compile_forest_with_options(input, options)` | Parse forest with output format |
| `compile_ambiguous(input)` | PP attachment ambiguity handling |
| `compile_ambiguous_with_options(input, options)` | PP ambiguity with output format |

### Theorem Proving

| Function | Description |
|----------|-------------|
| `compile_theorem(input)` | Compile and prove a theorem block |

### Constants

| Constant | Value | Description |
|----------|-------|-------------|
| `MAX_FOREST_READINGS` | 12 | Maximum parse readings to prevent exponential blowup |

## Session API

For REPL-style incremental evaluation with persistent discourse state:

```rust
use logicaffeine_language::Session;

let mut session = Session::new();
let out1 = session.eval("The boys lifted the piano.").unwrap();
let out2 = session.eval("They smiled.").unwrap();  // "They" resolves to "the boys"

assert_eq!(session.turn_count(), 2);
let full_discourse = session.history();  // Includes Precedes relations
```

| Method | Description |
|--------|-------------|
| `new()` | Create session with default settings |
| `with_format(format)` | Create session with specific output format |
| `eval(input)` | Parse one sentence, returns FOL for that sentence |
| `history()` | Full accumulated logic with temporal constraints |
| `turn_count()` | Number of sentences processed |
| `world_state()` | Direct access to WorldState |
| `world_state_mut()` | Mutable access to WorldState |
| `reset()` | Clear discourse state |

## Output Formats

```rust
use logicaffeine_language::{compile_with_options, CompileOptions, OutputFormat};

let options = CompileOptions { format: OutputFormat::LaTeX };
let latex = compile_with_options("Every dog barks.", options).unwrap();
```

| Format | Example | Use Case |
|--------|---------|----------|
| `Unicode` (default) | `∀x(P(x) → Q(x))` | Display, documentation |
| `LaTeX` | `\forall x (P(x) \to Q(x))` | Academic papers |
| `SimpleFOL` | `forall x . P(x) -> Q(x)` | Interop with provers |
| `Kripke` | Explicit world quantification | Modal logic analysis |

## Parser Configuration

The parser can be configured for handling ambiguity and linguistic phenomena:

### Parser Modes

```rust
use logicaffeine_language::ParserMode;
```

| Mode | Description |
|------|-------------|
| `Declarative` (default) | Propositions, NeoEvents, ambiguity allowed |
| `Imperative` | Statements, strict scoping, deterministic |

### Negative Scope Mode

Controls scope of negation for lexically negative verbs (lacks, miss):

```rust
use logicaffeine_language::NegativeScopeMode;
```

| Mode | Semantics | Example |
|------|-----------|---------|
| `Narrow` (default) | `∃y(Key(y) ∧ ¬Have(x,y))` | "User is missing some key" |
| `Wide` | `¬∃y(Key(y) ∧ Have(x,y))` | "User has no keys" |

### Modal Preference

Controls interpretation of polysemous modals (may, can, could):

```rust
use logicaffeine_language::ModalPreference;
```

| Mode | Description |
|------|-------------|
| `Default` | may=Permission, can=Ability, could=PastAbility |
| `Epistemic` | may=Possibility (wide scope), could=Possibility |
| `Deontic` | can=Permission (narrow scope, deontic domain) |

### Parser Setter Methods

These methods configure the parser for specific ambiguity readings:

| Method | Description |
|--------|-------------|
| `set_pp_attachment_mode(bool)` | Toggle PP attachment (verb vs noun) |
| `set_noun_priority_mode(bool)` | Prefer noun reading for ambiguous tokens |
| `set_collective_mode(bool)` | Enable collective readings for cardinals |
| `set_event_reading_mode(bool)` | Event-modifying adjective interpretation |
| `set_negative_scope_mode(mode)` | Wide vs narrow negation scope |
| `set_modal_preference(pref)` | Epistemic vs deontic modal reading |

## AST Types

### LogicExpr

The main expression type with 30+ variants:

| Category | Variants |
|----------|----------|
| **Core Logic** | `Predicate`, `Identity`, `Quantifier`, `BinaryOp`, `UnaryOp`, `Atom` |
| **Modal** | `Modal` (with ModalVector: domain, force, flavor) |
| **Temporal** | `Temporal` (Past, Future), `Aspectual` (Progressive, Perfect, Habitual, Iterative) |
| **Event Semantics** | `NeoEvent` (Neo-Davidsonian), `Event`, `Voice` (Passive) |
| **Discourse** | `Presupposition`, `Focus`, `SpeechAct`, `Imperative` |
| **Comparatives** | `Comparative`, `Superlative` |
| **Control** | `Control`, `Lambda`, `App`, `Intensional`, `Scopal` |
| **Questions** | `Question`, `YesNoQuestion` |
| **Advanced** | `Counterfactual`, `Causal`, `Metaphor`, `Categorical`, `Relation` |
| **Plurality** | `GroupQuantifier`, `Distributive`, `TemporalAnchor` |

### Term

8 variants for representing individuals and values:

| Variant | Description |
|---------|-------------|
| `Constant(Symbol)` | Named entities (John, Paris) |
| `Variable(Symbol)` | Bound variables (x, y, e1) |
| `Function(Symbol, &[Term])` | Function application |
| `Group(&[Term])` | Collective terms |
| `Possessed { possessor, possessed }` | Genitive constructions |
| `Sigma(Symbol)` | Sum individual (Link's plurality theory) |
| `Intension(Symbol)` | De dicto predicates (^Unicorn) |
| `Proposition(&LogicExpr)` | Embedded clauses |
| `Value { kind, unit, dimension }` | Numeric values with units |

### QuantifierKind

9 quantifier types:

| Kind | Example |
|------|---------|
| `Universal` | every, all |
| `Existential` | a, some |
| `Most` | most |
| `Few` | few |
| `Many` | many |
| `Cardinal(n)` | three, five |
| `AtLeast(n)` | at least three |
| `AtMost(n)` | at most five |
| `Generic` | bare plurals |

### ThematicRole

10 Neo-Davidsonian roles:

`Agent`, `Patient`, `Theme`, `Recipient`, `Goal`, `Source`, `Instrument`, `Location`, `Time`, `Manner`

## WorldState & DRS

`WorldState` manages discourse state across sentences:

```rust
use logicaffeine_language::WorldState;

let mut ws = WorldState::new();

// Event tracking
let e1 = ws.next_event_var();  // "e1"
let e2 = ws.next_event_var();  // "e2"
ws.event_history();            // ["e1", "e2"]

// Temporal constraints
ws.add_time_constraint("e1".into(), TimeRelation::Precedes, "e2".into());
ws.time_constraints();

// Reference time (Reichenbach)
ws.next_reference_time();      // "r1"
ws.current_reference_time();   // "r1" or "S" (speech time)

// Sentence boundaries (telescoping)
ws.end_sentence();             // Mark boundary, collect candidates
ws.in_discourse_mode();        // true after first sentence

// Modal subordination
ws.enter_modal_context(is_epistemic, force);
ws.in_modal_context();
ws.exit_modal_context();
```

### DRS (Discourse Representation Structure)

Tracks referents and accessibility:

| Type | Description |
|------|-------------|
| `Drs` | Box hierarchy for scope tracking |
| `DrsBox` | Universe of referents with box type |
| `BoxType` | Main, ConditionalAntecedent, NegationScope, ModalScope, etc. |
| `Referent` | Variable + noun class + gender/number + source |
| `ReferentSource` | MainClause, ProperName, ConditionalAntecedent, NegationScope, etc. |

## Error Handling

```rust
use logicaffeine_language::{ParseError, ParseErrorKind, socratic_explanation};

match compile("All men mortal.") {
    Ok(fol) => println!("{}", fol),
    Err(e) => {
        // Rich error display with source context
        println!("{}", e.display_with_source("All men mortal."));

        // Pedagogical explanation
        let interner = Interner::new();
        println!("{}", socratic_explanation(&e, &interner));
    }
}
```

### ParseErrorKind Variants

| Category | Variants |
|----------|----------|
| **Syntax** | `UnexpectedToken`, `ExpectedContentWord`, `ExpectedCopula`, `ExpectedVerb` |
| **Semantics** | `UnknownQuantifier`, `UnknownModal`, `TypeMismatch` |
| **Discourse** | `UnresolvedPronoun`, `ScopeViolation` |
| **Comparatives** | `ExpectedThan`, `ExpectedComparativeAdjective`, `ExpectedSuperlativeAdjective` |
| **Imperative** | `UndefinedVariable`, `UseAfterMove`, `ZeroIndex` |
| **Grammar** | `GrammarError`, `StativeProgressiveConflict` |
| **Other** | `GappingResolutionFailed`, `RespectivelyLengthMismatch`, `Custom` |

## Token System

### TokenType

100+ token variants organized by category:

| Category | Examples |
|----------|----------|
| **Quantifiers** | `All`, `Some`, `No`, `Most`, `Few`, `Many`, `Cardinal(n)`, `AtLeast(n)`, `AtMost(n)` |
| **Modals** | `Must`, `Shall`, `Should`, `Can`, `May`, `Cannot`, `Would`, `Could`, `Might` |
| **Connectives** | `And`, `Or`, `If`, `Then`, `Not`, `Iff`, `Because` |
| **Imperatives** | `Let`, `Set`, `Return`, `While`, `For`, `Assert`, `Inspect` |
| **IO** | `Read`, `Write`, `Console`, `File` |
| **Collections** | `Push`, `Pop`, `At`, `Length`, `Add`, `Remove`, `Contains` |
| **Concurrency** | `Launch`, `Task`, `Pipe`, `Receive`, `Send`, `Await` |
| **CRDT** | `Shared`, `Merge`, `Increase`, `Decrease`, `SharedSet`, `SharedMap` |

### Special Types

| Type | Description |
|------|-------------|
| `BlockType` | Theorem, Main, Definition, Proof, Function, TypeDef, Policy |
| `PresupKind` | Stop, Start, Regret, Continue, Realize, Know |
| `FocusKind` | Only, Even, Just |

## Advanced Types

### AstContext

Arena bundle with allocation helpers:

```rust
use logicaffeine_language::AstContext;

let ctx = AstContext::new(&expr_arena, &term_arena, &np_arena, &sym_arena, &role_arena, &pp_arena);

// Convenience builders
ctx.predicate(name, args);
ctx.binary(left, TokenType::And, right);
ctx.quantifier(QuantifierKind::Universal, var, body, island_id);
ctx.temporal(TemporalOperator::Past, body);
ctx.modal(vector, operand);
```

### SymbolRegistry

Manages symbol abbreviations during transpilation:

```rust
use logicaffeine_language::SymbolRegistry;

let mut registry = SymbolRegistry::new();
let abbreviated = ast.transpile(&mut registry, &interner, OutputFormat::Unicode);
```

### TypeRegistry

Type definitions discovered during the discovery pass:

```rust
use logicaffeine_language::TypeRegistry;
```

### Visitor Trait

For AST traversal:

```rust
use logicaffeine_language::visitor::{Visitor, walk_expr, walk_term};

struct MyVisitor;

impl<'a> Visitor<'a> for MyVisitor {
    fn visit_expr(&mut self, expr: &'a LogicExpr<'a>) {
        // Process expression
        walk_expr(self, expr);  // Continue traversal
    }

    fn visit_term(&mut self, term: &'a Term<'a>) {
        // Process term
        walk_term(self, term);
    }
}
```

### TranspileContext

For advanced transpilation control:

```rust
use logicaffeine_language::TranspileContext;

let ctx = TranspileContext::new(&mut registry, &interner);
```

## Architecture

Pipeline stages:

1. **Tokenization** — `Lexer` converts text to tokens
2. **MWE Collapse** — Multi-word expressions collapsed
3. **Discovery** — Type definitions scanned
4. **Parsing** — `Parser` builds AST with discourse state
5. **Semantics** — Axioms applied, Kripke lowering for modal output
6. **Transpilation** — AST → FOL string in chosen format

## Design Patterns

| Pattern | Description |
|---------|-------------|
| **Arena Allocation** | AST nodes use bumpalo arenas for efficient memory management |
| **Symbol Interning** | Strings interned via `Interner` for deduplication |
| **RAII Backtracking** | `ParserGuard` pattern for safe parser state rollback |
| **Parse Forest** | Up to 12 readings for ambiguous sentences |
| **Visitor Pattern** | AST traversal via `Visitor` trait |
| **Two-Pass Parsing** | Discovery pass then parse pass |

## Feature Flags

| Feature | Description |
|---------|-------------|
| `dynamic-lexicon` | Runtime lexicon modification via `runtime_lexicon` module |

```rust
// With dynamic-lexicon feature enabled:
use logicaffeine_language::runtime_lexicon;
```

## Dependencies

Internal workspace crates:
- `logicaffeine-base` — Core types (Interner, Symbol, Arena)
- `logicaffeine-lexicon` — Vocabulary definitions, morphology
- `logicaffeine-proof` — Proof engine for theorem compilation

## License

BUSL-1.1
