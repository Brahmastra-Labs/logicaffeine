# Logicaffeine

**Write English. Get Logic. Run Code.**

[![Tests](https://img.shields.io/badge/tests-1000%2B%20passing-brightgreen)]()
[![Version](https://img.shields.io/badge/version-1.0-blue)]()
[![Phases](https://img.shields.io/badge/linguistic%20phases-43-success)]()
[![License](https://img.shields.io/badge/license-BSL%201.1-blue)](LICENSE.md)

Logicaffeine is a natural language compiler with two modes:

| Mode | Input | Output |
|------|-------|--------|
| **Logic** | English sentences | First-Order Logic (∀, ∃, →, ∧) |
| **Imperative** | English programs | Executable Rust code |

```
┌─────────────────────────────────────────────────────────────────────────────┐
│  LOGIC MODE                           IMPERATIVE MODE                       │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                             │
│  "Every woman loves a man."           ## Main                               │
│           ↓                           Let x be 5.                           │
│  ∀x(Woman(x) → ∃y(Man(y) ∧ Love(x,y))) If x is less than 10:                │
│                                           Return true.                      │
│                                       Return false.                         │
│                                                ↓                            │
│                                       fn main() -> bool {                   │
│                                           let x = 5;                        │
│                                           if x < 10 { return true; }        │
│                                           false                             │
│                                       }                                     │
│                                                                             │
└─────────────────────────────────────────────────────────────────────────────┘
```

The programming language is called **LOGOS**.

---

## Table of Contents

- [Quick Start](#quick-start)
- [The Grand Challenge: Mergesort](#the-grand-challenge-mergesort)
- [Imperative Mode](#imperative-mode)
  - [Hello World](#hello-world)
  - [Variables & Types](#variables--types)
  - [Control Flow](#control-flow)
  - [Collections](#collections)
  - [Functions](#functions)
  - [Structs](#structs)
  - [Enums & Pattern Matching](#enums--pattern-matching)
- [Logic Mode](#logic-mode)
  - [Quantifiers](#quantifiers)
  - [Connectives](#connectives)
  - [Modal Operators](#modal-operators)
  - [Tense & Aspect](#tense--aspect)
  - [Wh-Questions](#wh-questions)
  - [Scope Ambiguity](#scope-ambiguity)
  - [Parse Forests](#parse-forests)
  - [Discourse & Pronouns](#discourse--pronouns)
- [Type System](#type-system)
- [API Reference](#api-reference)
- [Architecture](#architecture)
- [Testing](#testing)
- [Glossary](#glossary)
- [Further Reading](#further-reading)

---

## Quick Start

```bash
# Build the project
cargo build

# Launch the interactive web UI
cargo run

# Run the test suite (1000+ tests)
cargo test

# Run a specific phase
cargo test --test phase43_collections
```

### Library Usage

```rust
use logos::{compile, compile_to_rust, compile_all_scopes};

// Logic Mode: English → First-Order Logic
let fol = compile("All men are mortal.").unwrap();
// → ∀x(Man(x) → Mortal(x))

// Imperative Mode: English → Rust
let rust = compile_to_rust("## Main\nLet x be 5.\nReturn x.").unwrap();
// → fn main() -> i64 { let x = 5; x }

// Get all scope readings for ambiguous sentences
let readings = compile_all_scopes("Every woman loves a man.").unwrap();
// → [surface scope, inverse scope]
```

---

## The Grand Challenge: Mergesort

This is a complete, recursive mergesort algorithm written in LOGOS. It compiles to working Rust and executes correctly.

```logos
## To Merge (left: Seq of Int) and (right: Seq of Int) -> Seq of Int:
    Let result be a new Seq of Int.
    Let i be 1.
    Let j be 1.
    Let n_left be length of left.
    Let n_right be length of right.

    While i is at most n_left and j is at most n_right:
        Let l_val be item i of left.
        Let r_val be item j of right.

        If l_val is less than r_val:
            Push l_val to result.
            Set i to i + 1.
        Otherwise:
            Push r_val to result.
            Set j to j + 1.

    While i is at most n_left:
        Let v be item i of left.
        Push v to result.
        Set i to i + 1.

    While j is at most n_right:
        Let v be item j of right.
        Push v to result.
        Set j to j + 1.

    Return result.

## To MergeSort (items: Seq of Int) -> Seq of Int:
    Let n be length of items.
    If n is less than 2:
        Return copy of items.

    Let mid be n / 2.
    Let left_slice be items 1 through mid.
    Let right_slice be items (mid + 1) through n.

    Let sorted_left be MergeSort(copy of left_slice).
    Let sorted_right be MergeSort(copy of right_slice).

    Return Merge(sorted_left, sorted_right).

## Main
    Let numbers be a new Seq of Int.
    Push 3 to numbers.
    Push 1 to numbers.
    Push 4 to numbers.
    Push 1 to numbers.
    Push 5 to numbers.

    Let sorted be MergeSort(numbers).
    Show sorted.
```

**What this demonstrates:**
- Recursive function definitions
- Generic collection types (`Seq of Int`)
- Compound conditions (`and`)
- Comparison operators (`is less than`, `is at most`)
- 1-based indexing (`item 1 of items`)
- Inclusive slicing (`items 1 through mid`)
- Collection operations (`Push`, `length of`, `copy of`)
- Full compilation to executable Rust

---

## Imperative Mode

Write executable programs in natural English. LOGOS compiles to Rust.

### Hello World

```logos
## Main
Show "Hello, World!".
```

Compiles to:

```rust
fn main() {
    println!("{:?}", "Hello, World!");
}
```

### Variables & Types

```logos
## Main
Let x be 5.                      # Immutable binding
Let name be "Alice".             # Text (string)
Let flag be true.                # Boolean
Let pi be 3.14159.               # Float

Set x to 10.                     # Mutation (requires prior Let)

Let y: Int be 42.                # Explicit type annotation
```

| LOGOS Type | Rust Type | Example |
|------------|-----------|---------|
| `Int` | `i64` | `Let x be 5.` |
| `Bool` | `bool` | `Let flag be true.` |
| `Text` | `String` | `Let name be "Alice".` |
| `Float` | `f64` | `Let pi be 3.14.` |
| `Seq of T` | `Vec<T>` | `Let items be [1, 2, 3].` |

### Control Flow

```logos
## Main
Let x be 5.

# Conditionals
If x is less than 10:
    Show "small".
Otherwise:
    Show "large".

# While loops
Let i be 1.
While i is at most 5:
    Show i.
    Set i to i + 1.

# For-each loops
Let items be [1, 2, 3].
Repeat for item in items:
    Show item.

# Early return
If x equals 0:
    Return false.
Return true.
```

**Comparison Operators:**

| English | Symbol | Meaning |
|---------|--------|---------|
| `is less than` | `<` | Less than |
| `is greater than` | `>` | Greater than |
| `is at most` | `<=` | Less than or equal |
| `is at least` | `>=` | Greater than or equal |
| `equals` / `is` | `==` | Equality |
| `is not` | `!=` | Inequality |

**Logical Operators:**

| English | Symbol | Example |
|---------|--------|---------|
| `and` | `&&` | `If x > 0 and y > 0:` |
| `or` | `\|\|` | `If x is 0 or y is 0:` |
| `not` | `!` | `If not flag:` |

### Collections

LOGOS uses **1-based indexing** because that's how humans count.

```logos
## Main
# List literals
Let items be [10, 20, 30, 40, 50].

# Access (1-indexed)
Let first be item 1 of items.     # → 10
Let third be item 3 of items.     # → 30

# Slicing (inclusive)
Let slice be items 2 through 4.   # → [20, 30, 40]

# Length
Let n be length of items.         # → 5

# Create empty collection
Let numbers be a new Seq of Int.

# Push (append)
Push 100 to numbers.
Push 200 to numbers.

# Pop (remove last)
Pop from numbers into last.       # last = 200

# Copy (deep clone)
Let backup be copy of items.
```

### Functions

Functions are defined with `## To` blocks:

```logos
## To add (a: Int) and (b: Int) -> Int:
    Return a + b.

## To factorial (n: Int) -> Int:
    If n is at most 1:
        Return 1.
    Return n * factorial(n - 1).

## To greet (name: Text) -> Text:
    Return "Hello, " + name + "!".

## Main
    Let sum be add(3, 4).           # → 7
    Let fact be factorial(5).       # → 120
    Show greet("World").            # → "Hello, World!"
```

**Function Syntax:**
- `## To verb (param: Type) -> ReturnType:`
- Multiple parameters: `(a: Int) and (b: Int)`
- No parameters: `## To greet -> Text:`
- Void return: omit `-> Type`

### Structs

Define custom data types:

```logos
## Definition
A Point has:
    an x: Int.
    a y: Int.

## Main
Let p be a new Point with x 10 and y 20.
Show p's x.                        # → 10
Show p's y.                        # → 20

Set p's x to 15.                   # Mutation
```

### Enums & Pattern Matching

```logos
## Definition
A Shape is either:
    a Circle with radius: Int.
    a Rectangle with width: Int and height: Int.

## To area (s: Shape) -> Int:
    Inspect s:
        When Circle:
            Return 3 * s's radius * s's radius.
        When Rectangle:
            Return s's width * s's height.

## Main
Let c be a new Circle with radius 5.
Let r be a new Rectangle with width 4 and height 6.

Show area(c).                      # → 75
Show area(r).                      # → 24
```

---

## Logic Mode

Compile English sentences to First-Order Logic with full semantic analysis.

### Quantifiers

```
Input:  "All men are mortal."
Output: ∀x(Man(x) → Mortal(x))

Input:  "Some cats are black."
Output: ∃x(Cat(x) ∧ Black(x))

Input:  "No dogs are cats."
Output: ∀x(Dog(x) → ¬Cat(x))

Input:  "Most birds fly."
Output: MOST x(Bird(x), Fly(x))

Input:  "Three cats sleep."
Output: ∃=3x(Cat(x) ∧ Sleep(x))

Input:  "At least two dogs bark."
Output: ∃≥2x(Dog(x) ∧ Bark(x))
```

### Connectives

```
Input:  "John runs and Mary walks."
Output: Run(j) ∧ Walk(m)

Input:  "John runs or Mary walks."
Output: Run(j) ∨ Walk(m)

Input:  "John does not run."
Output: ¬Run(j)

Input:  "If John runs then Mary walks."
Output: Run(j) → Walk(m)

Input:  "John runs if and only if Mary walks."
Output: Run(j) ↔ Walk(m)
```

### Modal Operators

```
Input:  "John can swim."
Output: ◇Swim(j)

Input:  "John must leave."
Output: □Leave(j)

Input:  "John may enter."
Output: ◇Enter(j)       [deontic permission]

Input:  "John should help."
Output: ○Help(j)        [deontic obligation]
```

### Tense & Aspect

```
Input:  "John ran."
Output: PAST(Run(j))

Input:  "John will run."
Output: FUT(Run(j))

Input:  "John is running."
Output: PROG(Run(j))

Input:  "John has run."
Output: PERF(Run(j))

Input:  "John had been running."
Output: PAST(PERF(PROG(Run(j))))
```

### Wh-Questions

```
Input:  "Who loves Mary?"
Output: λx.Love(x, m)

Input:  "What does John love?"
Output: λx.Love(j, x)

Input:  "Who did John say Mary loves?"
Output: λx.Say(j, [Love(m, x)])
```

### Scope Ambiguity

Quantified sentences can have multiple readings:

```
Input: "Every woman loves a man."

Reading 1 (Surface Scope):
∀x(Woman(x) → ∃y(Man(y) ∧ Love(x, y)))
"Each woman loves some man (possibly different men)"

Reading 2 (Inverse Scope):
∃y(Man(y) ∧ ∀x(Woman(x) → Love(x, y)))
"There is one specific man whom every woman loves"
```

Use `compile_all_scopes()` to get all readings.

### Parse Forests

Ambiguous sentences produce multiple parses:

```
Input: "I saw the man with the telescope."

Reading 1 (Instrument):
∃e(See(e) ∧ Agent(e, i) ∧ Theme(e, m) ∧ Instrument(e, t))
"I used the telescope to see him"

Reading 2 (Modifier):
∃e(See(e) ∧ Agent(e, i) ∧ Theme(e, m)) ∧ With(m, t)
"I saw the man who has the telescope"
```

```
Input: "I saw her duck."

Reading 1 (Noun):
See(i, duck)
"I saw her pet duck"

Reading 2 (Verb):
See(i, [Duck(her)])
"I saw her perform a ducking motion"
```

### Discourse & Pronouns

```
Sentence 1: "John saw Mary."
Sentence 2: "He loves her."

Output: See(j, m) ∧ Love(j, m) ∧ Precedes(e₁, e₂)
```

Pronouns are resolved using gender, number, and discourse context.

**Donkey Anaphora:**

```
Input: "Every farmer who owns a donkey beats it."
Output: ∀x∀y((Farmer(x) ∧ Donkey(y) ∧ Own(x,y)) → Beat(x,y))
```

The indefinite "a donkey" receives universal (not existential) force due to DRS accessibility constraints.

---

## Type System

### Primitives

| Type | Description | Example |
|------|-------------|---------|
| `Int` | 64-bit integer | `5`, `-10`, `0` |
| `Bool` | Boolean | `true`, `false` |
| `Text` | String | `"hello"` |
| `Float` | 64-bit float | `3.14` |
| `Unit` | No value | (implicit) |

### Collections

```logos
Let ints: Seq of Int be [1, 2, 3].
Let texts: Seq of Text be ["a", "b", "c"].
Let nested: Seq of (Seq of Int) be [[1, 2], [3, 4]].
```

### User-Defined Types

**Structs (Product Types):**
```logos
## Definition
A Person has:
    a name: Text.
    an age: Int.
```

**Enums (Sum Types):**
```logos
## Definition
A Result is either:
    an Ok with value: Int.
    an Error with message: Text.
```

### Generics

```logos
## Definition
A Box of [T] has:
    a contents: T.

## Main
Let int_box be a new Box of Int with contents 42.
Let text_box be a new Box of Text with contents "hello".
```

### Refinement Types (Planned)

```logos
Let positive: Int where it > 0 be 5.
```

---

## API Reference

### Core Functions

```rust
// Logic Mode
pub fn compile(input: &str) -> Result<String, ParseError>
pub fn compile_all_scopes(input: &str) -> Result<Vec<String>, ParseError>
pub fn compile_forest(input: &str) -> Vec<String>

// Imperative Mode
pub fn compile_to_rust(input: &str) -> Result<String, ParseError>
pub fn compile_to_dir(input: &str, output: &Path) -> Result<(), CompileError>

// Output Formats
pub fn compile_with_options(input: &str, opts: CompileOptions) -> Result<String, ParseError>
```

### Output Formats

| Format | Example | Use Case |
|--------|---------|----------|
| `Unicode` | `∀x(Dog(x) → Bark(x))` | Display, terminals |
| `LaTeX` | `\forall x (Dog(x) \supset Bark(x))` | Academic papers |
| `SimpleFOL` | `ALL x (Dog(x) IMP Bark(x))` | ASCII-only |

```rust
let options = CompileOptions { format: OutputFormat::LaTeX };
let latex = compile_with_options("All cats sleep.", options).unwrap();
```

---

## Architecture

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                          Logicaffeine Pipeline                              │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                             │
│  English Input                                                              │
│       │                                                                     │
│       ▼                                                                     │
│  ┌─────────┐    ┌─────────┐    ┌─────────┐    ┌─────────────┐              │
│  │  Lexer  │───▶│   MWE   │───▶│ Parser  │───▶│     AST     │              │
│  │         │    │Pipeline │    │         │    │             │              │
│  └─────────┘    └─────────┘    └─────────┘    └──────┬──────┘              │
│                                                       │                     │
│                          ┌────────────────────────────┼───────────────┐     │
│                          │                            │               │     │
│                          ▼                            ▼               │     │
│                   ┌─────────────┐              ┌─────────────┐        │     │
│                   │  Semantics  │              │   Codegen   │        │     │
│                   │  (λ-calc)   │              │   (Rust)    │        │     │
│                   └──────┬──────┘              └──────┬──────┘        │     │
│                          │                            │               │     │
│                          ▼                            ▼               │     │
│                   ┌─────────────┐              ┌─────────────┐        │     │
│                   │ Transpiler  │              │   Compile   │        │     │
│                   │  (FOL)      │              │   (cargo)   │        │     │
│                   └──────┬──────┘              └──────┬──────┘        │     │
│                          │                            │               │     │
│                          ▼                            ▼               │     │
│                   ┌─────────────┐              ┌─────────────┐        │     │
│                   │   Logic     │              │  Executable │        │     │
│                   │   Output    │              │   Binary    │        │     │
│                   └─────────────┘              └─────────────┘        │     │
│                                                                             │
└─────────────────────────────────────────────────────────────────────────────┘
```

### Key Modules

| Module | Purpose |
|--------|---------|
| `lexer.rs` | Tokenization with POS tagging |
| `parser/` | Recursive descent parser (8 sub-modules) |
| `ast/` | Arena-allocated abstract syntax tree |
| `transpile.rs` | AST → FOL string conversion |
| `codegen.rs` | AST → Rust code generation |
| `compile.rs` | End-to-end compilation orchestration |
| `lambda.rs` | Scope enumeration via λ-calculus |
| `drs.rs` | Discourse Representation Structures |
| `logos_core/` | Runtime library for generated code |

### Design Highlights

- **Arena Allocation**: Uses `bumpalo` for efficient AST nodes
- **Symbol Interning**: All strings interned for fast comparison
- **ParserGuard**: RAII pattern for automatic backtracking
- **Parse Forests**: Up to 12 readings for ambiguous inputs
- **Neo-Davidsonian Events**: Thematic roles (Agent, Patient, Theme)

---

## Testing

Tests are organized by linguistic complexity across 43 phases:

| Phases | Focus |
|--------|-------|
| 1-5 | Core syntax: garden path, polarity, tense, movement |
| 6-14 | Advanced semantics: degrees, ontology, MWEs |
| 15-20 | Extended phenomena: negation, plurality, axioms |
| 21-29 | Code generation: blocks, scoping, types, runtime |
| 30-38 | Type system: collections, structs, functions, modules |
| 41-43 | Advanced: event adjectives, DRS, refinement types |

**End-to-End Tests:**
- `e2e_collections.rs` - Push, pop, length, slicing
- `e2e_functions.rs` - Recursion, multi-parameter
- `e2e_structs.rs` - User-defined types
- `e2e_enums.rs` - Pattern matching
- `grand_challenge_mergesort.rs` - Full algorithm compilation

```bash
# Run all tests
cargo test

# Run specific phase
cargo test --test phase43_collections

# Run with output
cargo test -- --nocapture
```

---

## Glossary

| Term | Definition |
|------|------------|
| **Arena Allocation** | Memory allocation strategy where objects are allocated in a contiguous region and freed all at once |
| **DRS** | Discourse Representation Structure - formal framework for tracking entities and relations across sentences |
| **First-Order Logic (FOL)** | Formal system using quantifiers (∀, ∃), predicates, and logical connectives |
| **Lambda Calculus** | Formal system for function abstraction and application, used for compositional semantics |
| **MWE** | Multi-Word Expression - phrases that behave as single units ("fire engine", "kick the bucket") |
| **Neo-Davidsonian** | Event semantics using event variables with thematic roles (Agent, Patient, Theme) |
| **NPI** | Negative Polarity Item - words like "any" that require negative/downward-entailing contexts |
| **Parse Forest** | Collection of all valid parse trees for an ambiguous sentence |
| **Scope Ambiguity** | When quantifiers can be ordered in multiple ways, yielding different meanings |
| **Symbol Interning** | Storing strings once and referring to them by index for efficiency |
| **Thematic Role** | Semantic relationship between verb and argument (Agent, Patient, Theme, Goal, etc.) |
| **Vendler Class** | Aspectual classification: State, Activity, Accomplishment, Achievement, Semelfactive |

---

## Further Reading

- **[SPECIFICATION.md](SPECIFICATION.md)** - Complete language specification (5000+ lines)
- **[LOGOS_DOCUMENTATION.md](LOGOS_DOCUMENTATION.md)** - Full technical documentation
- **[ROADMAP.md](ROADMAP.md)** - Development roadmap and version history
- **[CLAUDE.md](CLAUDE.md)** - AI assistant guidelines for contributors

---

## License

**Business Source License 1.1**

- **Licensor:** Brahmastra Labs LLC
- **Change Date:** 2029-12-24 (converts to MIT)
- **Additional Use Grant:** Free for individuals and organizations with fewer than 25 employees, except for commercial "Logic Service" offerings

See [LICENSE.md](LICENSE.md) for full terms.

---

*Logicaffeine: Where natural language meets formal logic meets executable code.*
