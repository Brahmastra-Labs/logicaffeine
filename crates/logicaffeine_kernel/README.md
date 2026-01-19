# logicaffeine-kernel

Pure Calculus of Constructions (CoC) type theory implementation. This is the logical kernel for Logicaffeine—a language-agnostic type checker and proof assistant core.

## Overview

The kernel implements the Calculus of Inductive Constructions, a unified type system where terms and types inhabit the same syntactic category. Everything is a `Term`:

- Types are Terms: `Nat : Type 0`
- Values are Terms: `zero : Nat`
- Functions are Terms: `λx:Nat. x`
- Proofs are Terms: `refl : a = a`

### Milner Invariant

This crate has **no path to the lexicon**. Adding words to the English vocabulary never triggers a recompile of the type checker. The kernel is purely logical and language-agnostic.

```
Natural Language → Lexer → Parser → Compile → Kernel
                                                 ↑
                                   (no lexicon dependency)
```

## Core Types

### Term

The unified representation for all expressions:

```rust
pub enum Term {
    Sort(Universe),       // Type 0, Type 1, Prop
    Var(String),          // Local variable
    Global(String),       // Global definition
    Pi { ... },           // Π(x:A). B (dependent function type)
    Lambda { ... },       // λ(x:A). t (function)
    App(Box<Term>, Box<Term>),  // f x (application)
    Match { ... },        // Pattern matching
    Fix { ... },          // Recursive fixpoint
    Lit(Literal),         // i64, f64, String
    Hole,                 // Implicit argument
}
```

### Universe

The type hierarchy: `Prop : Type 1 : Type 2 : ...`

```rust
pub enum Universe {
    Prop,      // Propositions (proof-irrelevant)
    Type(u32), // Types at level n
}
```

Universe subtyping (cumulativity):
- `Prop ≤ Type(i)` for all `i`
- `Type(i) ≤ Type(j)` if `i ≤ j`

### Context

Typing context with three kinds of bindings:

- **Inductives**: Type definitions (`Nat : Type 0`)
- **Constructors**: Data constructors (`Zero : Nat`, `Succ : Nat → Nat`)
- **Definitions**: Named terms with their types

## Public API

### Type Checking

```rust
use logicaffeine_kernel::{Context, Term, infer_type, is_subtype, normalize};

let ctx = Context::new();

// Infer the type of a term
let ty = infer_type(&ctx, &term)?;

// Check subtyping with cumulativity
let sub = is_subtype(&ctx, &term_a, &term_b)?;

// Reduce to normal form
let nf = normalize(&ctx, &term);
```

### Standard Library

```rust
use logicaffeine_kernel::prelude::StandardLibrary;

let mut ctx = Context::new();
StandardLibrary::register(&mut ctx);
// Now ctx has: Entity, Nat, Bool, True, False, Not, Eq, And, Or, Ex, Int, Float, Text
```

**Standard types:**

| Type | Description |
|------|-------------|
| `Entity` | Domain of individuals for FOL |
| `Nat` | Natural numbers (`Zero`, `Succ`) |
| `Bool` | Booleans (`true`, `false`) |
| `True` | Unit proposition with constructor `I` |
| `False` | Empty proposition (no constructors) |
| `Not` | Negation: `Not P = P → False` |
| `Eq` | Propositional equality with `refl` |
| `And` | Conjunction with `conj` |
| `Or` | Disjunction with `left`, `right` |
| `Ex` | Existential with `witness` |
| `Int` | 64-bit signed integer (primitive) |
| `Float` | 64-bit floating point (primitive) |
| `Text` | UTF-8 string (primitive) |

## Decision Procedures

The kernel includes automated proof tactics:

| Module | Tactic | Proves |
|--------|--------|--------|
| `ring` | `try_ring` | Polynomial equalities: `x * (y + z) = x*y + x*z` |
| `lia` | `try_lia` | Linear inequalities: `x > 2 ∧ y ≥ 1 ⊢ x + y > 2` |
| `cc` | `try_cc` | Congruence closure: `x = y ⊢ f(x) = f(y)` |
| `omega` | `try_omega` | Integer arithmetic with floor/ceil: `3x ≤ 10 ⊢ x ≤ 3` |
| `simp` | `try_simp` | Rewriting simplification: constant folding, hypothesis substitution |

The `try_auto` tactic tries all procedures in sequence: `simp → ring → cc → omega → lia`.

## Reflection System

Deep embedding for metaprogramming with `Syntax` (quoted terms) and `Derivation` (proof trees):

```
Syntax constructors:
  SVar, SGlobal, SSort, SApp, SLam, SPi, SLit, SName

Derivation constructors:
  DAxiom, DModusPonens, DUnivIntro, DUnivElim, DRefl, DInduction,
  DCompute, DCong, DElim, DInversion, DRewrite, DDestruct, DApply
```

Operations: `syn_size`, `syn_max_var`, `syn_lift`, `syn_subst`, `syn_beta`, `syn_step`, `syn_eval`, `syn_quote`, `syn_diag`.

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                        Interface                            │
│  (term_parser, literate_parser, command)                    │
└─────────────────────────────────────────────────────────────┘
                             │
                             ▼
┌─────────────────────────────────────────────────────────────┐
│                      Type Checker                           │
│  infer_type, is_subtype, substitute                         │
└─────────────────────────────────────────────────────────────┘
                             │
              ┌──────────────┴──────────────┐
              ▼                             ▼
┌─────────────────────────┐   ┌─────────────────────────────┐
│      Reduction          │   │         Prelude             │
│  normalize, reduce      │   │  standard library types     │
└─────────────────────────┘   └─────────────────────────────┘
                                            │
              ┌─────────────┬───────────────┼───────────────┐
              ▼             ▼               ▼               ▼
┌───────────────┐ ┌───────────────┐ ┌───────────────┐ ┌───────────┐
│     ring      │ │     lia       │ │      cc       │ │   omega   │
│  polynomial   │ │    linear     │ │  congruence   │ │  integer  │
│   equality    │ │  arithmetic   │ │   closure     │ │ arithmetic│
└───────────────┘ └───────────────┘ └───────────────┘ └───────────┘
```

## Directory Structure

```
src/
├── lib.rs              # Crate root, public exports
├── term.rs             # Term, Universe, Literal types
├── context.rs          # Typing context with bindings
├── type_checker.rs     # Type inference and subtyping
├── reduction.rs        # Normalization and reduction
├── error.rs            # KernelError, KernelResult
├── ring.rs             # Ring decision procedure
├── lia.rs              # Linear integer arithmetic (Fourier-Motzkin)
├── cc.rs               # Congruence closure
├── omega.rs            # Integer arithmetic with floor/ceil
├── simp.rs             # Rewriting simplification
├── prelude.rs          # Standard library definitions
├── positivity.rs       # Strict positivity checking for inductives
├── termination.rs      # Termination checking for Fix
└── interface/          # REPL, parsing, literate syntax
```

## License

Business Source License 1.1 (BUSL-1.1)

- **Free** for individuals and organizations with <25 employees
- **Commercial license** required for organizations with 25+ employees offering Logic Services
- **Converts to MIT** on December 24, 2029

See [LICENSE](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/LICENSE.md) for full terms.
