# LOGOS

**The Language That Reads Like English, Runs Like Rust**

[![Tests](https://img.shields.io/badge/tests-828%20passing-brightgreen)]()
[![Version](https://img.shields.io/badge/version-0.5.3-blue)]()
[![Gates](https://img.shields.io/badge/gates-6%2F6-success)]()

LOGOS is a dual-mode natural language compiler:

- **Logic Mode:** Compile English to First-Order Logic (FOL)
- **Imperative Mode:** Compile English to executable Rust code

```
Logic:      "Every woman loves a man."
            → ∀x(Woman(x) → ∃y(Man(y) ∧ Love(x, y)))

Imperative: "Let x be 5. Return x plus 1."
            → let x = 5; x + 1
```

**[See the full ROADMAP](ROADMAP.md)** for what's done and what's next.

---

## Table of Contents

- [Quick Start](#quick-start)
- [Features](#features)
- [Output Formats](#output-formats)
- [Pipeline Architecture](#pipeline-architecture)
- [API Reference](#api-reference)
- [Lexicon System](#lexicon-system)
- [Project Structure](#project-structure)
- [Test Phases](#test-phases)
- [Gamification & Learning](#gamification--learning)
- [Design Decisions](#design-decisions)
- [Development](#development)
- [Examples](#examples)
- [Further Documentation](#further-documentation)

---

## Quick Start

```bash
# Build the project
cargo build

# Launch the interactive web UI
cargo run

# Run the test suite
cargo test

# Run specific test phase
cargo test --test phase5_wh_movement
```

### Library Usage

```rust
use logos::{compile, compile_all_scopes, compile_forest, OutputFormat, CompileOptions};

// Basic compilation (Unicode output)
let result = compile("All men are mortal.").unwrap();
// Output: ∀x(Man(x) → Mortal(x))

// Get all scope readings
let readings = compile_all_scopes("Every woman loves a man.").unwrap();
// Returns both: surface scope and inverse scope

// Get all parse trees for ambiguous sentences
let parses = compile_forest("I saw the man with the telescope.");
// Returns multiple readings for PP-attachment ambiguity

// LaTeX output
let options = CompileOptions { format: OutputFormat::LaTeX };
let latex = compile_with_options("All cats sleep.", options).unwrap();
// Output: \forall x (Cat(x) \supset Sleep(x))
```

---

## Features

### Linguistic Phenomena

LOGICAFFEINE handles 28+ phases of linguistic complexity:

#### Core Logic
- **Quantifiers**: Universal (∀), existential (∃), generalized (MOST, FEW, MANY)
- **Cardinals**: "Three dogs bark" → ∃=3x(Dog(x) ∧ Bark(x))
- **Bounded quantifiers**: At least N, at most N
- **Connectives**: And (∧), or (∨), not (¬), if-then (→), iff (↔)
- **Identity**: "Clark is equal to Superman" → C = S

#### Verbal Phenomena
- **Transitivity**: Intransitive, transitive, ditransitive
- **Thematic roles**: Agent, Theme, Beneficiary, Instrument
- **Passive voice**: "Mary was loved by John" → Love(J, M)
- **Tense**: Past, present, future with Reichenbachian temporal logic
- **Aspect**: Perfect, progressive, habitual, iterative

#### Modal Logic
- **Alethic**: Necessity (□), possibility (◇)
- **Deontic**: Obligation, permission
- **Modal vectors**: Force strength from 0.0 to 1.0

#### Movement & Binding
- **Wh-questions**: "Who did John see?" → λx.See(J, x)
- **Long-distance extraction**: "Who did John think Mary said Bill saw?"
- **Topicalization**: "The apple, John ate"
- **Reflexives**: "John loves himself" → Love(J, J)
- **Reciprocals**: "John and Mary love each other" → Love(J, M) ∧ Love(M, J)
- **Relative clauses**: Nested and recursive

#### Discourse & Pragmatics
- **Pronoun resolution**: Gender, number, case agreement
- **Definite reference**: "A dog barked. The dog ran." (same entity)
- **Bridging anaphora**: "I bought a car. The engine smoked."
- **Event ordering**: Temporal precedence across sentences

#### Ambiguity Handling
- **Lexical ambiguity**: "duck" (noun/verb) → multiple readings
- **PP-attachment**: "with the telescope" → instrument vs modifier
- **Scope ambiguity**: All quantifier orderings
- **Parse forests**: Up to 12 readings preserved

#### Advanced Semantics
- **Adjective types**: Intersective, subsective, privative
- **Metaphor detection**: Sort mismatch triggers metaphor wrapper
- **Coercion**: Noun-to-verb conversion ("John tabled the motion")
- **VP ellipsis**: "John runs. Mary does too."
- **Sluicing**: "Someone left. I know who."
- **Plurality**: Distributive vs collective readings

### Imperative Engine (NEW in v0.5)

Write executable code in natural English:

```
## Main

Let counter be 0.
While counter equals 10:
    Set counter to counter plus 1.
Return counter.
```

**Compiles to Rust:**
```rust
fn main() -> i64 {
    let mut counter = 0;
    while counter == 10 {
        counter = counter + 1;
    }
    counter
}
```

| Statement | Syntax | Output |
|-----------|--------|--------|
| Binding | `Let x be 5.` | `let x = 5;` |
| Mutation | `Set x to 10.` | `x = 10;` |
| Conditional | `If x equals 5:` | `if x == 5 {` |
| Loop | `While x equals 0:` | `while x == 0 {` |
| Return | `Return x.` | `return x;` |
| Index | `item 1 of list` | `list[0]` |
| Slice | `items 2 through 5 of list` | `&list[1..5]` |
| Assert | `Assert that x > 0.` | `debug_assert!(x > 0);` |

**Key Features:**
- **1-Indexed Arrays:** `item 1` is the first element (like humans count)
- **Boolean Precedence:** `And` binds tighter than `Or`
- **Assert Bridge:** Logic verification in imperative code

---

## Output Formats

| Format | Example | Use Case |
|--------|---------|----------|
| Unicode | `∀x(Dog(x) → Bark(x))` | Display, terminals |
| LaTeX | `\forall x (Dog(x) \supset Bark(x))` | Academic papers |
| SimpleFOL | `ALL x (Dog(x) IMP Bark(x))` | ASCII-only environments |

### Symbol Mapping

| Concept | Unicode | LaTeX |
|---------|---------|-------|
| Universal | ∀ | \forall |
| Existential | ∃ | \exists |
| Conjunction | ∧ | \wedge |
| Disjunction | ∨ | \vee |
| Negation | ¬ | \neg |
| Implication | → | \supset |
| Biconditional | ↔ | \equiv |
| Necessity | □ | \Box |
| Possibility | ◇ | \Diamond |

---

## Pipeline Architecture

```
┌─────────┐    ┌───────┐    ┌────────┐    ┌─────────┐    ┌───────────┐
│  Input  │───▶│ Lexer │───▶│   MWE  │───▶│ Parser  │───▶│    AST    │
│ String  │    │       │    │Pipeline│    │         │    │           │
└─────────┘    └───────┘    └────────┘    └─────────┘    └─────────┬─┘
                                                                   │
┌──────────────────────────────────────────────────────────────────┘
│
▼
┌───────────┐    ┌────────────┐    ┌────────────┐    ┌──────────┐
│ Semantics │───▶│ Pragmatics │───▶│ Transpiler │───▶│  Output  │
│  Axioms   │    │            │    │            │    │  String  │
└───────────┘    └────────────┘    └────────────┘    └──────────┘
```

### Stage Details

1. **Lexer** (`lexer.rs`): Tokenizes input with part-of-speech tagging
2. **MWE Pipeline** (`mwe.rs`): Collapses multi-word expressions ("fire engine" → FireEngine)
3. **Parser** (`parser/`): Modular recursive-descent parser with 8 sub-modules
4. **AST** (`ast/`): Arena-allocated abstract syntax tree
5. **Semantics** (`semantics/`): Applies axioms (entailments, privatives)
6. **Pragmatics** (`pragmatics.rs`): Discourse-level processing
7. **Transpiler** (`transpile.rs`): Converts AST to formatted output

---

## API Reference

### Core Functions

```rust
/// Basic compilation to Unicode
pub fn compile(input: &str) -> Result<String, ParseError>

/// Compile with format options
pub fn compile_with_options(input: &str, options: CompileOptions) -> Result<String, ParseError>

/// Compile with SimpleFOL output
pub fn compile_simple(input: &str) -> Result<String, ParseError>
```

### Scope & Ambiguity

```rust
/// Returns all scope permutations for quantified sentences
/// "Every woman loves a man" returns both surface and inverse scope
pub fn compile_all_scopes(input: &str) -> Result<Vec<String>, ParseError>

/// Returns all valid parse trees for ambiguous sentences
/// Handles lexical, structural, and plurality ambiguity
pub fn compile_forest(input: &str) -> Vec<String>

/// Returns readings for PP-attachment ambiguity
pub fn compile_ambiguous(input: &str) -> Result<Vec<String>, ParseError>
```

### Discourse

```rust
/// Compile with discourse context for pronoun resolution
pub fn compile_with_context(
    input: &str,
    ctx: &mut DiscourseContext
) -> Result<String, ParseError>

/// Batch compile sentences with temporal ordering
/// "John ran. He stopped." → Run(J) ∧ Stop(J) ∧ Precedes(e1, e2)
pub fn compile_discourse(sentences: &[&str]) -> Result<String, ParseError>
```

### UI Integration

```rust
/// Rich result with AST, tokens, and all readings
pub fn compile_for_ui(input: &str) -> CompileResult

/// Tokenize input for syntax highlighting
pub fn tokenize_for_ui(input: &str) -> Vec<TokenInfo>
```

### Types

```rust
pub enum OutputFormat {
    Unicode,    // Default
    LaTeX,
    SimpleFOL,
}

pub struct CompileOptions {
    pub format: OutputFormat,
}

pub struct CompileResult {
    pub logic: Option<String>,
    pub ast: Option<AstNode>,
    pub readings: Vec<String>,
    pub tokens: Vec<TokenInfo>,
    pub error: Option<String>,
}
```

---

## Lexicon System

The vocabulary is defined in `assets/lexicon.json`.

### Structure

```json
{
  "keywords": {
    "all": "All", "every": "All", "some": "Some",
    "and": "And", "or": "Or", "not": "Not"
  },
  "verbs": {
    "love": {
      "lemma": "love",
      "class": "State",
      "transitive": true,
      "features": []
    },
    "give": {
      "lemma": "give",
      "class": "Achievement",
      "transitive": true,
      "ditransitive": true,
      "features": []
    }
  },
  "nouns": {
    "man": { "lemma": "man", "plural": "men", "gender": "Masculine" },
    "woman": { "lemma": "woman", "plural": "women", "gender": "Feminine" }
  }
}
```

### Vendler Aspect Classes

| Class | Example | Properties |
|-------|---------|------------|
| State | know, love, be | Static, durative, atelic |
| Activity | run, swim, walk | Dynamic, durative, atelic |
| Accomplishment | build, write | Dynamic, durative, telic |
| Achievement | find, win, die | Dynamic, punctual, telic |
| Semelfactive | knock, cough | Dynamic, punctual, atelic |

### Verb Features

- **Transitive/Ditransitive**: Argument structure
- **SubjectControl/ObjectControl**: "John persuaded Mary to leave"
- **Factive**: Presupposes truth of complement
- **Performative**: Speech acts
- **Opaque/Intensional**: de dicto readings
- **Collective/Mixed**: Plurality behavior

### Adding Vocabulary

```json
"verbs": {
  "teleport": {
    "lemma": "teleport",
    "class": "Achievement",
    "transitive": true,
    "features": []
  }
}
```

After modifying lexicon.json, run `cargo build` to regenerate.

---

## Project Structure

```
logos/
├── assets/
│   └── lexicon.json           # Vocabulary database
├── src/
│   ├── lib.rs                 # Public API and exports
│   ├── main.rs                # Web UI entry point
│   │
│   ├── lexer.rs               # Tokenization
│   ├── token.rs               # Token types (60+)
│   ├── lexicon.rs             # Lexicon access
│   ├── runtime_lexicon.rs     # Dynamic lexicon
│   │
│   ├── parser/                # Modular parser
│   │   ├── mod.rs            # Main parser logic
│   │   ├── clause.rs         # Clause parsing
│   │   ├── noun.rs           # Noun phrases
│   │   ├── verb.rs           # Verb phrases
│   │   ├── quantifier.rs     # Quantifier handling
│   │   ├── question.rs       # Wh-questions
│   │   ├── modal.rs          # Modal operators
│   │   ├── pragmatics.rs     # Parser pragmatics
│   │   └── common.rs         # Shared utilities
│   │
│   ├── ast/                   # Abstract syntax tree
│   │   ├── mod.rs
│   │   ├── logic.rs          # LogicExpr, Term, NounPhrase
│   │   └── stmt.rs           # Statement types
│   │
│   ├── semantics/             # Semantic processing
│   │   ├── mod.rs
│   │   └── axioms.rs         # Entailments, privatives
│   │
│   ├── transpile.rs           # AST to string output
│   ├── formatter.rs           # Unicode/LaTeX/SimpleFOL
│   │
│   ├── context.rs             # Discourse context
│   ├── pragmatics.rs          # Pragmatic processing
│   ├── lambda.rs              # Scope enumeration
│   ├── ontology.rs            # Sort system, bridging
│   │
│   ├── mwe.rs                 # Multi-word expressions
│   ├── intern.rs              # Symbol interning
│   ├── arena.rs               # Arena allocation
│   ├── arena_ctx.rs           # AST context
│   ├── registry.rs            # Symbol registry
│   │
│   ├── error.rs               # Parse errors
│   ├── scope.rs               # Variable scoping
│   ├── view.rs                # Expression views
│   ├── visitor.rs             # AST visitor pattern
│   │
│   ├── game.rs                # Gamification system
│   ├── achievements.rs        # Achievement definitions
│   ├── progress.rs            # User progress tracking
│   ├── srs.rs                 # Spaced repetition
│   ├── grader.rs              # Answer grading
│   ├── content.rs             # Learning content
│   ├── suggest.rs             # Suggestion engine
│   │
│   └── ui/                    # Dioxus web interface
│       ├── app.rs             # Main app component
│       ├── state.rs           # State management
│       ├── router.rs          # Page routing
│       ├── pages/             # Page components
│       └── components/        # Reusable components
│
└── tests/                     # Phase-organized tests
    ├── phase1_garden_path.rs
    ├── phase2_polarity.rs
    ├── ...
    └── phase28_precedence.rs
```

---

## Test Phases

Tests are organized by linguistic phenomenon:

| Phase | Name | Description |
|-------|------|-------------|
| 1 | Garden Path | Syntactically ambiguous sentences requiring reanalysis |
| 2 | Polarity Items | NPI "any" (existential) vs free choice "any" (universal) |
| 3 | Tense & Aspect | Reichenbachian temporal logic, perfect/progressive |
| 4 | Movement | Topicalization, reciprocals, filler-gap dependencies |
| 5 | Wh-Movement | Long-distance extraction across clause boundaries |
| 6 | Complex Tense | Temporal constraint extraction |
| 7 | Adjective Semantics | Subsective, intersective, gradable adjectives |
| 8 | Degrees | Comparatives, measurements, symbolic cardinality |
| 9 | Coercion | Noun-to-verb conversion |
| 10 | VP Ellipsis | "John runs. Mary does too." |
| 10b | Sluicing | "Someone left. I know who." |
| 11 | Metaphor | Sort mismatch detection (Human/Celestial) |
| 12 | Ambiguity | Parse forests for lexical/structural ambiguity |
| 13 | MWEs | Multi-word expressions, idioms, compounds |
| 14 | Ontology | Bridging anaphora, part-whole relations |
| 15 | Negation | Extended NPI licensing, temporal NPIs |
| 16 | Aspect Extended | Complex aspectual chains |
| 17 | Degrees Extended | Clausal comparatives, superlatives |
| 18 | Plurality | Distributive vs collective (mixed verbs) |
| 19 | Group Plurals | Cardinals with collective readings |
| 20 | Axioms | Analytic entailments, privative adjectives |
| 21+ | Advanced | Blocks, codegen, guards, precedence |

### Running Tests

```bash
# All tests
cargo test

# Specific phase
cargo test --test phase5_wh_movement

# Specific test
cargo test wh_embedded_clause

# With output
cargo test -- --nocapture
```

---

## Gamification & Learning

LOGICAFFEINE includes a complete learning system:

### XP & Levels

```rust
// XP calculation includes bonuses
pub struct XpReward {
    pub base: u64,           // 10 + (difficulty * 5)
    pub combo_bonus: u64,    // Up to +100% at 10 combo
    pub streak_bonus: u64,   // +2 per day, max 7 days
    pub first_try_bonus: u64,// +5 for first attempt
    pub critical_bonus: u64, // 10% chance for 2x
    pub total: u64,
}

// Level progression
pub fn level_title(level: u32) -> &'static str {
    match level {
        1 => "Novice",
        2..=4 => "Apprentice",
        5..=9 => "Student",
        10..=14 => "Scholar",
        15..=19 => "Adept",
        20..=29 => "Expert",
        30..=39 => "Master",
        40..=49 => "Sage",
        _ => "Grandmaster",
    }
}
```

### Streaks & Freezes

- **Streaks**: Consecutive days of practice
- **Streak freezes**: Protect streaks on missed days
- **Freeze grants**: Earned at level milestones (5, 10, 15...)

### Achievements

| Achievement | Requirement | Reward |
|------------|-------------|--------|
| First Blood | First correct answer | 50 XP |
| On Fire | 5-answer combo | 100 XP |
| Unstoppable | 10-answer combo | 250 XP + "Logic Machine" title |
| Week Warrior | 7-day streak | 200 XP + freeze + "Dedicated" title |
| Monthly Master | 30-day streak | 1000 XP + freeze + "Logician" title |
| Century | 100 correct answers | 500 XP + "Scholar" title |
| Millennium | 1000 correct answers | 2000 XP + "Sage" title |

### Spaced Repetition (SRS)

The SRS system schedules reviews based on performance:
- Correct answers increase interval
- Incorrect answers decrease interval
- Optimized for long-term retention

---

## Design Decisions

### Arena Allocation

Uses `bumpalo` for efficient AST node allocation:

```rust
pub struct AstContext<'a> {
    pub exprs: &'a Arena<LogicExpr<'a>>,
    pub terms: &'a Arena<Term<'a>>,
    pub nps: &'a Arena<NounPhrase<'a>>,
    // ...
}
```

Benefits:
- Fast allocation without individual heap calls
- Automatic cleanup when context drops
- Prevents memory fragmentation during parsing

### Symbol Interning

Strings are interned to reduce memory and enable fast comparison:

```rust
pub struct Interner {
    strings: Vec<String>,
    lookup: HashMap<String, Symbol>,
}

pub struct Symbol(u32);  // Compact, copyable
```

### ParserGuard (RAII Backtracking)

```rust
let guard = self.guard();
if let Some(result) = self.try_parse_quantifier() {
    guard.commit();
    return Some(result);
}
// guard drops, parser state restored
```

### Parse Forests

Ambiguous sentences produce multiple readings:

```rust
pub const MAX_FOREST_READINGS: usize = 12;

pub fn compile_forest(input: &str) -> Vec<String> {
    // Returns all valid parses up to limit
}
```

Ambiguity types:
1. Lexical (noun/verb: "duck")
2. PP-attachment ("with the telescope")
3. Plurality (distributive/collective)
4. Scope (quantifier ordering)

### Neo-Davidsonian Events

Predicates use event variables with thematic roles:

```
"John gave Mary a book"
→ ∃e(Give(e) ∧ Agent(e, J) ∧ Theme(e, B) ∧ Beneficiary(e, M))
```

---

## Development

### TDD Workflow

1. **RED**: Write a failing test first
2. **GREEN**: Write minimal code to pass
3. **REFACTOR**: Clean up while tests stay green

```bash
# Watch mode (install cargo-watch)
cargo watch -x test

# Run specific test with output
cargo test test_name -- --nocapture
```

### Code Style

- Arena allocation for all AST nodes
- Symbol interning for all strings
- Guard pattern for parser backtracking
- Visitor pattern for AST traversal

### Regenerating Documentation

```bash
./generate-docs.sh
```

---

## Examples

### Quantifiers

```
Input:  "All men are mortal."
Output: ∀x(Man(x) → Mortal(x))

Input:  "Some cats are black."
Output: ∃x(Cat(x) ∧ Black(x))

Input:  "No dogs are cats."
Output: ∀x(Dog(x) → ¬Cat(x))

Input:  "Most dogs bark."
Output: MOST x(Dog(x), Bark(x))

Input:  "Three cats sleep."
Output: ∃=3x(Cat(x) ∧ Sleep(x))
```

### Scope Ambiguity

```
Input: "Every woman loves a man."

Reading 1 (surface scope):
∀x(Woman(x) → ∃y(Man(y) ∧ Love(x, y)))
"Each woman loves some man (possibly different)"

Reading 2 (inverse scope):
∃y(Man(y) ∧ ∀x(Woman(x) → Love(x, y)))
"There is one man whom every woman loves"
```

### Wh-Questions

```
Input:  "Who loves Mary?"
Output: λx.Love(x, M)

Input:  "What does John love?"
Output: λx.Love(J, x)

Input:  "Who did John say Mary loves?"
Output: λx.Say(J, [Love(M, x)])
```

### Discourse

```
Sentence 1: "John saw Mary."
Sentence 2: "He loves her."

Combined: See(J, M) ∧ Love(J, M) ∧ Precedes(e1, e2)
```

### Metaphor Detection

```
Input:  "Juliet is the sun."
Output: Metaphor(Juliet, Sun)  // Sort mismatch: Human/Celestial

Input:  "The king is bald."
Output: Bald(K)  // No metaphor: compatible sorts
```

### Ambiguity (Parse Forest)

```
Input: "I saw the man with the telescope."

Reading 1 (instrument): See(I, M, Telescope)
Reading 2 (modifier): See(I, M) ∧ With(M, T)

Input: "I saw her duck."

Reading 1 (noun): See(I, Duck)
Reading 2 (verb): See(I, [Duck(her)])
```

### Plurality

```
Input: "The boys lifted the piano."

Reading 1 (distributive): *Lift(boys, piano)
"Each boy lifted it separately"

Reading 2 (collective): Lift(boys, piano)
"They lifted it together"
```

### VP Ellipsis

```
Input: "John runs. Mary does too."
Output: Run(J) ∧ Run(M)

Input: "John can swim. Mary can too."
Output: ◇Swim(J) ∧ ◇Swim(M)
```

---

## Further Documentation

- **[ROADMAP.md](ROADMAP.md)** - What's done, what's next, version status
- **[SPECIFICATION.md](SPECIFICATION.md)** - Complete language specification (5000+ lines)
- **[LOGOS_DOCUMENTATION.md](LOGOS_DOCUMENTATION.md)** - Full technical documentation
- **[CLAUDE.md](CLAUDE.md)** - AI assistant guidelines

---

## License

MIT
