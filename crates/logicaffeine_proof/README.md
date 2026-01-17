# logicaffeine-proof

Backward-chaining proof engine with Socratic hint generation.

## The Liskov Invariant

This crate has **NO dependency on the language crate**. The conversion from `LogicExpr` to `ProofExpr` happens in the language layer. This separation ensures the proof engine is reusable across different frontends and prevents tight coupling between parsing and proving.

## Overview

A backward-chaining proof engine implementing the Curry-Howard correspondence:

- **A Proposition is a Type**
- **A Proof is a Program**
- **Verification is Type Checking**

The engine searches backward from a goal to axioms, building derivation trees that can be certified by the kernel.

### Key Capabilities

- Robinson's unification algorithm with occurs check
- Alpha-equivalence for quantifiers and lambda expressions
- Beta-reduction for lambda calculus normalization
- Higher-order pattern unification (Miller patterns)
- Support for modal logic, temporal logic, and inductive types
- Pedagogical hints via Socratic method
- Optional Z3 oracle fallback

## Quick Start

```rust
use logicaffeine_proof::{BackwardChainer, ProofExpr, ProofTerm};

// Create the proof engine
let mut prover = BackwardChainer::new();

// Add axioms to the knowledge base
let human = ProofExpr::Predicate {
    name: "Human".into(),
    args: vec![ProofTerm::Constant("Socrates".into())],
    world: None,
};
prover.add_axiom(human);

// Add a rule: ∀x (Human(x) → Mortal(x))
let rule = ProofExpr::ForAll {
    variable: "x".into(),
    body: Box::new(ProofExpr::Implies(
        Box::new(ProofExpr::Predicate {
            name: "Human".into(),
            args: vec![ProofTerm::Variable("x".into())],
            world: None,
        }),
        Box::new(ProofExpr::Predicate {
            name: "Mortal".into(),
            args: vec![ProofTerm::Variable("x".into())],
            world: None,
        }),
    )),
};
prover.add_axiom(rule);

// Prove Mortal(Socrates)
let goal = ProofExpr::Predicate {
    name: "Mortal".into(),
    args: vec![ProofTerm::Constant("Socrates".into())],
    world: None,
};

match prover.prove(goal) {
    Ok(derivation) => println!("{}", derivation.display_tree()),
    Err(e) => eprintln!("Proof failed: {}", e),
}
```

## Core Types

### ProofTerm

Owned representation of logical terms:

| Variant | Description |
|---------|-------------|
| `Constant(String)` | Named constant (e.g., "Socrates", "42") |
| `Variable(String)` | Unification variable (e.g., "x", "y") |
| `Function(String, Vec<ProofTerm>)` | Function application (e.g., "father(x)") |
| `Group(Vec<ProofTerm>)` | Tuple of terms |
| `BoundVarRef(String)` | Reference to a bound variable (prevents capture) |

### ProofExpr

Comprehensive logical expression language:

| Variant | Description |
|---------|-------------|
| `Predicate { name, args, world }` | Atomic predicate: P(t₁, t₂, ...) |
| `Identity(ProofTerm, ProofTerm)` | Equality: t₁ = t₂ |
| `Atom(String)` | Propositional atom |
| `And`, `Or`, `Implies`, `Iff`, `Not` | Logical connectives |
| `ForAll { variable, body }` | Universal quantification: ∀x P(x) |
| `Exists { variable, body }` | Existential quantification: ∃x P(x) |
| `Modal { domain, force, flavor, body }` | Modal operator: □P or ◇P |
| `Temporal { operator, body }` | Temporal operator: Past(P), Future(P) |
| `Lambda { variable, body }` | Lambda abstraction: λx.P |
| `App(Box<ProofExpr>, Box<ProofExpr>)` | Application: (f x) |
| `NeoEvent { event_var, verb, roles }` | Neo-Davidsonian event semantics |
| `Ctor { name, args }` | Data constructor: Zero, Succ(n), etc. |
| `Match { scrutinee, arms }` | Pattern matching |
| `Fixpoint { name, body }` | Recursive function (fix f. ...) |
| `TypedVar { name, typename }` | Typed variable: n : Nat |
| `Hole(String)` | Meta-variable for higher-order unification |

### InferenceRule

The logical moves available to the prover:

| Rule | Logic |
|------|-------|
| `PremiseMatch` | Γ, P ⊢ P |
| `ModusPonens` | P → Q, P ⊢ Q |
| `ModusTollens` | ¬Q, P → Q ⊢ ¬P |
| `ConjunctionIntro` | P, Q ⊢ P ∧ Q |
| `ConjunctionElim` | P ∧ Q ⊢ P (or Q) |
| `DisjunctionIntro` | P ⊢ P ∨ Q |
| `DisjunctionElim` | P ∨ Q, P → R, Q → R ⊢ R |
| `DoubleNegation` | ¬¬P ⊢ P |
| `UniversalInst(String)` | ∀x P(x) ⊢ P(c) |
| `UniversalIntro { variable, var_type }` | Γ, x:T ⊢ P(x) ⟹ Γ ⊢ ∀x:T. P(x) |
| `ExistentialIntro { witness, witness_type }` | P(w) ⊢ ∃x.P(x) |
| `ExistentialElim { witness }` | ∃x.P(x), P(c) ⊢ Goal ⟹ ∃x.P(x) ⊢ Goal |
| `StructuralInduction { variable, ind_type, step_var }` | P(0), ∀k(P(k) → P(S(k))) ⊢ ∀n P(n) |
| `Rewrite { from, to }` | a = b, P(a) ⊢ P(b) |
| `Reflexivity` | a = a |
| `EqualitySymmetry` | a = b ⊢ b = a |
| `EqualityTransitivity` | a = b, b = c ⊢ a = c |
| `ReductioAdAbsurdum` | Assume ¬C, derive ⊥, conclude C |
| `Contradiction` | P, ¬P ⊢ ⊥ |
| `ModalAccess` | □P, Accessible(w₀, w₁) ⊢ P (in w₁) |
| `ModalGeneralization` | P true in all accessible worlds ⊢ □P |
| `Axiom` | Top-level axiom |
| `OracleVerification(String)` | Z3 oracle result |

### DerivationTree

The recursive proof structure:

```rust
pub struct DerivationTree {
    pub conclusion: ProofExpr,       // What was proved
    pub rule: InferenceRule,         // How it was proved
    pub premises: Vec<DerivationTree>, // Sub-proofs
    pub depth: usize,                // Tree depth
    pub substitution: Substitution,  // Unification bindings
}
```

### ProofGoal

Target for backward chaining:

```rust
pub struct ProofGoal {
    pub target: ProofExpr,      // What we're proving
    pub context: Vec<ProofExpr>, // Local assumptions
}
```

## Modules

| Module | Purpose |
|--------|---------|
| `lib.rs` | Core proof structures (ProofTerm, ProofExpr, InferenceRule, etc.) |
| `engine.rs` | Backward chaining engine (BackwardChainer) |
| `unify.rs` | Robinson's unification + beta-reduction + alpha-equivalence |
| `hints.rs` | Socratic hint generation for pedagogical guidance |
| `certifier.rs` | DerivationTree → Kernel Term conversion |
| `error.rs` | Error types (ProofError, ProofResult) |
| `oracle.rs` | Z3 oracle integration (optional, requires `verification` feature) |

## Public API

### BackwardChainer

The main proof engine:

| Method | Description |
|--------|-------------|
| `new()` | Create engine with empty knowledge base |
| `add_axiom(expr)` | Add a fact/rule to the knowledge base |
| `prove(goal)` | Attempt to prove a goal, returns `ProofResult<DerivationTree>` |
| `set_max_depth(depth)` | Limit search depth (default: 100) |
| `knowledge_base()` | Get reference to current KB |

### Unification Functions

| Function | Description |
|----------|-------------|
| `unify_terms(t1, t2)` | Unify two ProofTerms, returns MGU |
| `unify_exprs(e1, e2)` | Unify two ProofExprs with alpha-equivalence |
| `unify_pattern(lhs, rhs)` | Higher-order pattern unification (Miller patterns) |
| `beta_reduce(expr)` | Reduce to Weak Head Normal Form |
| `apply_subst_to_term(term, subst)` | Apply substitution to a term |
| `apply_subst_to_expr(expr, subst)` | Apply substitution to an expression |
| `compose_substitutions(s1, s2)` | Compose two substitutions |

### Socratic Hints

| Function/Type | Description |
|---------------|-------------|
| `suggest_hint(goal, kb, failed)` | Generate a pedagogical hint |
| `SocraticHint` | Hint with text, suggested tactic, and priority |
| `SuggestedTactic` | Tactic suggestions (ModusPonens, Induction, etc.) |

### Certifier

| Function | Description |
|----------|-------------|
| `certify(tree, ctx)` | Convert DerivationTree to Kernel Term |

## Proof Strategies

The backward chainer attempts strategies in priority order:

1. **Structural induction** - For goals with TypedVar over inductive types
2. **Reflexivity** - When both sides of an identity reduce to the same form
3. **Direct fact matching** - Goal matches a fact in the KB
4. **Introduction rules** - Conjunction, disjunction, implication intro
5. **Backward chaining** - Modus ponens with implications from KB
6. **Modus tollens** - ¬Q and P → Q to derive ¬P
7. **Universal instantiation** - ∀x P(x) to P(c)
8. **Existential introduction** - Find a witness for ∃x P(x)
9. **Disjunction elimination** - Case analysis on P ∨ Q
10. **Proof by contradiction** - Reductio ad absurdum
11. **Existential elimination** - Skolemize ∃x P(x)
12. **Equality rewriting** - Use equations to transform terms
13. **Oracle fallback** - Z3 SMT solver (if enabled)

## Feature Flags

| Feature | Description |
|---------|-------------|
| `verification` | Enables Z3-based oracle fallback (requires Z3 installed) |

### Enabling Z3 Oracle

```toml
[dependencies]
logicaffeine-proof = { path = "../logicaffeine_proof", features = ["verification"] }
```

When enabled, if the structural prover fails, the engine consults Z3 as a fallback oracle. Z3 can verify goals that are valid but difficult for the structural prover (e.g., complex arithmetic). However, Z3 cannot handle inductive constructs (Ctor, TypedVar, Match, Fixpoint) without explicit axiomatization.

## Dependencies

| Crate | Purpose |
|-------|---------|
| `logicaffeine-base` | Arena allocation, spans, symbols |
| `logicaffeine-kernel` | Type theory kernel for certification |
| `logicaffeine-verify` | Z3 integration (optional) |

## Error Types

| Error | Description |
|-------|-------------|
| `NoProofFound` | Goal could not be derived from KB |
| `DepthExceeded` | Search exceeded max depth |
| `OccursCheck` | Variable appears in its own binding (infinite type) |
| `UnificationFailed` | Terms cannot be unified |
| `ExprUnificationFailed` | Expressions cannot be unified |
| `SymbolMismatch` | Different predicate/function names |
| `ArityMismatch` | Different argument counts |
| `PatternNotDistinct` | Miller pattern has duplicate variables |
| `NotAPattern` | Expression is not a valid Miller pattern |
| `ScopeViolation` | RHS uses variables not in pattern scope |

## License

BUSL-1.1
