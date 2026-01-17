# logicaffeine-kernel

Pure Calculus of Constructions type theory implementation.

## Milner Invariant

**This crate has NO path to the lexicon.** Adding words to the English vocabulary never triggers a recompile of the type checker. The kernel is the trusted core for proof verification, isolated from natural language processing.

## Overview

A unified type system where terms and types are the same syntactic category, based on the Calculus of Inductive Constructions (CIC). Everything is a `Term`:

- Types are Terms (`Nat : Type 0`)
- Values are Terms (`zero : Nat`)
- Functions are Terms (`λx:Nat. x`)
- Proofs are Terms (`refl : a = a`)

## Usage

```toml
[dependencies]
logicaffeine-kernel = { path = "../logicaffeine_kernel" }
```

## Public API

```rust
pub use context::Context;
pub use error::{KernelError, KernelResult};
pub use reduction::normalize;
pub use term::{Literal, Term, Universe};
pub use type_checker::{infer_type, is_subtype};
```

## Core Types

### Term

Unified representation for all expressions:

| Variant | Description |
|---------|-------------|
| `Sort(Universe)` | Type 0, Type 1, Prop |
| `Var(String)` | Local variable (bound by λ or Π) |
| `Global(String)` | Global definition or constructor |
| `Pi { param, param_type, body_type }` | Dependent function type: Π(x:A). B |
| `Lambda { param, param_type, body }` | Function: λ(x:A). t |
| `App(Box<Term>, Box<Term>)` | Application: f x |
| `Match { discriminant, motive, cases }` | Pattern matching on inductives |
| `Fix { name, body }` | Recursive fixpoint |
| `Lit(Literal)` | Primitive value |
| `Hole` | Implicit argument to be inferred |

### Universe

Type hierarchy with cumulativity:

```
Prop : Type 1 : Type 2 : Type 3 : ...
```

- `Prop` - universe of propositions
- `Type(n)` - universe of types at level n
- Subtyping: `Prop ≤ Type(i)` and `Type(i) ≤ Type(j)` when `i ≤ j`

### Literal

Hardware-native primitive values:

- `Int(i64)` - 64-bit signed integer
- `Float(f64)` - 64-bit floating point
- `Text(String)` - UTF-8 string

## Modules

| Module | Purpose |
|--------|---------|
| `term` | Unified term representation |
| `context` | Typing context with inductives, constructors, definitions |
| `type_checker` | Bidirectional type checking with universe cumulativity |
| `reduction` | Beta, iota, fix normalization |
| `termination` | Syntactic guard condition for recursive definitions |
| `positivity` | Strict positivity checking for inductive types |
| `prelude` | Standard library (Entity, Nat, Bool, Eq, And, Or, Ex...) |
| `interface` | Vernacular commands, parsers, REPL |
| `cc` | Congruence Closure (E-graphs + Union-Find) |
| `omega` | Omega Test for integer arithmetic |
| `lia` | Linear Integer Arithmetic via Fourier-Motzkin |
| `ring` | Ring tactic for polynomial equality |
| `simp` | Simplifier tactic (bottom-up rewriting) |

## Standard Library

Registered via `StandardLibrary::register(ctx)`:

### Types

| Name | Type | Description |
|------|------|-------------|
| `Entity` | `Type 0` | Domain of individuals (FOL) |
| `Nat` | `Type 0` | Natural numbers (Zero, Succ) |
| `Bool` | `Type 0` | Booleans (true, false) |
| `TList` | `Type 0 → Type 0` | Polymorphic lists (TNil, TCons) |
| `Int` | `Type 0` | 64-bit integers (opaque) |
| `Float` | `Type 0` | 64-bit floats (opaque) |
| `Text` | `Type 0` | UTF-8 strings (opaque) |

### Propositions

| Name | Type | Description |
|------|------|-------------|
| `True` | `Prop` | Unit type with constructor `I` |
| `False` | `Prop` | Empty type (no constructors) |
| `Not` | `Prop → Prop` | Negation: `Not P := P → False` |
| `Eq` | `Π(A:Type). A → A → Prop` | Equality with `refl` |
| `And` | `Prop → Prop → Prop` | Conjunction with `conj` |
| `Or` | `Prop → Prop → Prop` | Disjunction with `left`, `right` |
| `Ex` | `Π(A:Type). (A → Prop) → Prop` | Existential with `witness` |

### Equality Combinators

- `Eq_rec` - Leibniz's Law (substitution)
- `Eq_sym` - Symmetry
- `Eq_trans` - Transitivity

### Deep Embedding

| Name | Type | Description |
|------|------|-------------|
| `Univ` | `Type 0` | Representation of universes (UProp, UType) |
| `Syntax` | `Type 0` | De Bruijn term syntax (SVar, SGlobal, SApp, SLam, SPi...) |
| `Derivation` | `Type 0` | Proof trees (DAxiom, DModusPonens, DRefl...) |

## Decision Procedures

Tactics for automated proof:

| Tactic | Domain |
|--------|--------|
| `try_refl` | Reflexivity: `a = a` |
| `try_compute` | Computational equality |
| `try_ring` | Polynomial equality |
| `try_lia` | Linear integer arithmetic (Fourier-Motzkin) |
| `try_omega` | Integer arithmetic with floor/ceil rounding |
| `try_cc` | Congruence closure over uninterpreted functions |
| `try_simp` | Term simplification and rewriting |
| `try_auto` | Tries all tactics in sequence |
| `try_induction` | Generic induction over inductives |
| `try_inversion` | Derives False when no constructor applies |
| `try_rewrite` | Equality substitution in goals |
| `try_destruct` | Case analysis without induction hypotheses |
| `try_apply` | Manual backward chaining |

### Tactic Combinators

- `tact_orelse` - Try first, fallback to second
- `tact_then` - Sequence two tactics
- `tact_try` - Try but never fail
- `tact_repeat` - Apply until no progress
- `tact_first` - First success from a list
- `tact_solve` - Must completely solve goal
- `tact_fail` - Always fails

## Safety Guarantees

1. **Termination checking** - Syntactic guard condition prevents infinite loops in recursive definitions
2. **Positivity checking** - Strict positivity prevents logical paradoxes in inductive types
3. **Type checking** - Bidirectional algorithm with universe cumulativity ensures well-typed terms
4. **Normalized proofs** - Full beta/iota/fix reduction produces canonical forms

## License

BUSL-1.1
