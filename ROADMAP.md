# LOGOS Roadmap

**The Language That Reads Like English, Runs Like Rust**

---

## Current Release: v0.5.3

### 828 Tests Passing | All 6 Gates Met | Production Ready Core

---

## What's Working Now

### The Logic Kernel (Complete)

Write formal logic in plain English. No symbols required.

```
All men are mortal.
Socrates is a man.
Therefore, Socrates is mortal.
```

**Compiles to:** `∀x(Man(x) → Mortal(x)) ∧ Man(socrates) → Mortal(socrates)`

| Feature | Status |
|---------|--------|
| Universal Quantifiers (`All`, `Every`, `Each`) | ✅ |
| Existential Quantifiers (`Some`, `A`, `An`) | ✅ |
| Negative Quantifiers (`No`, `None`) | ✅ |
| Cardinals (`Two dogs`, `At least 3`) | ✅ |
| Modal Logic (`must`, `can`, `should`, `may`) | ✅ |
| Temporal Operators (`always`, `eventually`) | ✅ |
| Identity (`is equal to`, `is identical to`) | ✅ |
| Relative Clauses (`dogs that bark`) | ✅ |
| Reflexives (`himself`, `herself`, `themselves`) | ✅ |
| Reciprocals (`each other`, `one another`) | ✅ |

### The Imperative Engine (Complete)

Write executable code in natural language.

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

| Feature | Status |
|---------|--------|
| Variable Binding (`Let x be 5`) | ✅ |
| Mutation (`Set x to 10`) | ✅ |
| Conditionals (`If...Otherwise`) | ✅ |
| Loops (`While...`) | ✅ |
| Function Calls (`Call process with data`) | ✅ |
| Return Statements (`Return x`) | ✅ |
| Assert Bridge (`Assert that x > 0`) | ✅ |

### 1-Indexed Arrays (Just Shipped)

English speakers count from 1. So does LOGOS.

```
Let first be item 1 of list.
Let middle be items 2 through 5 of sequence.
```

**Compiles to:** `list[0]` and `&sequence[1..5]`

| Feature | Status |
|---------|--------|
| Single Index (`item 1 of list`) | ✅ |
| Slice Ranges (`items 2 through 5`) | ✅ |
| Zero Index Guard (helpful error) | ✅ |

### Boolean Precedence (Just Shipped)

`And` binds tighter than `Or`. Just like math.

```
A cat runs or a dog walks and a bird flies.
```

**Parses as:** `A or (B and C)` — the intuitive reading.

---

## The Dual-AST Architecture

LOGOS maintains two parallel AST systems:

| Mode | Block Header | AST | Output |
|------|-------------|-----|--------|
| **Declarative** | `## Theorem` | `LogicExpr` | First-Order Logic |
| **Imperative** | `## Main` | `Stmt` | Rust Code |

The **Assert Bridge** connects them:

```
## Main

Let x be input.
Assert that x is greater than 0.
Return x times 2.
```

Logic verification meets imperative execution.

---

## What's Next

### Coming Soon: Ownership Semantics

Natural language memory management.

```
Give the data to the processor.     // Move semantics
Show the report to the validator.   // Immutable borrow
Let the sorter modify the list.     // Mutable borrow
```

**Compiles to:**
```rust
processor.process(data);           // data moved
validator.validate(&report);       // &report
sorter.sort(&mut list);            // &mut list
```

### On The Horizon

| Feature | Priority | Description |
|---------|----------|-------------|
| **Ownership Verbs** | High | Give/Show/Let modify semantics |
| **Use-After-Move Detection** | High | Catch ownership errors at compile time |
| **Socratic Errors** | Medium | Story/State/Suggestion error format |
| **logos_core Runtime** | Medium | Standard library types |
| **Z3/SMT Integration** | Future | Static verification (v0.6+) |

---

## Test Coverage

```
828 tests passing
 28 test phases
  0 failures
```

| Phase | Coverage |
|-------|----------|
| Garden Path Sentences | ✅ |
| Negative Polarity Items | ✅ |
| Tense & Aspect | ✅ |
| Wh-Movement | ✅ |
| Quantifier Scope | ✅ |
| Verb Gapping | ✅ |
| Control Theory | ✅ |
| Imperative Blocks | ✅ |
| Index Access | ✅ |
| Boolean Precedence | ✅ |

---

## Why LOGOS?

**For Developers:** Write self-documenting code that non-programmers can read.

**For Logicians:** Express formal proofs in natural language.

**For Teams:** Bridge the gap between specification and implementation.

---

## Quick Start

```rust
use logos::compile;

// Logic mode
let proof = compile("All men are mortal.");
// → ∀x(Man(x) → Mortal(x))

// Imperative mode
use logos::compile::compile_to_rust;

let code = compile_to_rust("## Main\nReturn 42.");
// → fn main() -> i64 { 42 }
```

---

## Links

- [Full Specification](SPECIFICATION.md) — 5000+ lines of language design
- [Test Suite](tests/) — 828 executable examples
- [Implementation Plan](IMPLEMENTATION_PLAN.md) — Technical roadmap

---

*"In the beginning was the Word, and the Word was with Logic, and the Word was Code."*
