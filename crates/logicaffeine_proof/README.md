# logicaffeine-proof

Backward-chaining proof engine with Socratic hints for the Logicaffeine project.

Part of the [Logicaffeine](https://logicaffeine.com) project.

## Overview

This crate implements a proof search engine that works backwards from goals to axioms, constructing derivation trees that can be certified by the kernel. It embodies the Curry-Howard correspondence: **proofs are programs**, and verification is type checking.

### Architecture Invariant

This crate has **no dependency** on the language crate (Liskov boundary). The conversion from `LogicExpr` to `ProofExpr` lives in the language crate, ensuring the proof engine remains pure and reusable.

## Core Concepts

### Backward Chaining

The engine searches for proofs by working backwards:

1. Start with the goal to prove
2. Find rules whose conclusions unify with the goal
3. Recursively prove the premises of those rules
4. Build the derivation tree as proofs succeed

```text
Goal: Mortal(Socrates)

Knowledge Base:
  - Human(Socrates)
  - ∀x(Human(x) → Mortal(x))

Search:
  1. Goal matches conclusion of ∀x(Human(x) → Mortal(x)) with x=Socrates
  2. New subgoal: Human(Socrates)
  3. Human(Socrates) matches knowledge base fact
  4. Build derivation tree: ModusPonens(UniversalInst, PremiseMatch)
```

### Unification

Implements Robinson's algorithm with occurs check. Unification finds a substitution that makes two terms identical:

| Pattern | Target | Substitution |
|---------|--------|--------------|
| `Mortal(x)` | `Mortal(Socrates)` | `{x ↦ Socrates}` |
| `Add(Succ(n), 0)` | `Add(Succ(Zero), 0)` | `{n ↦ Zero}` |
| `f(x, x)` | `f(a, b)` | Fails (x can't be both a and b) |

The occurs check prevents infinite terms: `x = f(x)` has no finite solution.

### Curry-Howard Correspondence

The proof engine implements propositions-as-types:

- **A proposition is a type** — logical formulas correspond to types
- **A proof is a program** — derivation trees are proof terms
- **Verification is type checking** — the kernel validates proof terms

## Module Structure

| Module | Purpose |
|--------|---------|
| `engine.rs` | `BackwardChainer` proof search implementation |
| `unify.rs` | Robinson's unification with occurs check and beta-reduction |
| `certifier.rs` | Curry-Howard conversion to kernel terms |
| `hints.rs` | Socratic pedagogical guidance for stuck proofs |
| `error.rs` | Error types (`ProofError`) |

## Key Types

### ProofTerm

Owned term representation decoupled from arena allocation:

```rust
pub enum ProofTerm {
    Constant(String),           // e.g., "Socrates", "42"
    Variable(String),           // e.g., "x", "y"
    Function(String, Vec<ProofTerm>), // e.g., "father(x)"
    Group(Vec<ProofTerm>),      // e.g., "(x, y)"
    BoundVarRef(String),        // Reference to bound variable
}
```

### ProofExpr

Owned expression/proposition representation supporting full FOL and extensions:

- **Core FOL**: `Predicate`, `Identity`, `Atom`
- **Connectives**: `And`, `Or`, `Implies`, `Iff`, `Not`
- **Quantifiers**: `ForAll`, `Exists`
- **Lambda calculus**: `Lambda`, `App`
- **Inductive types**: `Ctor`, `Match`, `Fixpoint`
- **Modal/Temporal**: `Modal`, `Temporal`
- **Event semantics**: `NeoEvent`

### InferenceRule

The logical moves available to the prover:

```rust
pub enum InferenceRule {
    PremiseMatch,           // Direct match with known fact
    ModusPonens,            // P → Q, P ⊢ Q
    ModusTollens,           // ¬Q, P → Q ⊢ ¬P
    ConjunctionIntro,       // P, Q ⊢ P ∧ Q
    ConjunctionElim,        // P ∧ Q ⊢ P
    DisjunctionIntro,       // P ⊢ P ∨ Q
    DisjunctionElim,        // P ∨ Q, P → R, Q → R ⊢ R
    UniversalInst(String),  // ∀x P(x) ⊢ P(c)
    UniversalIntro { variable, var_type },  // Γ, x:T ⊢ P(x) ⊢ ∀x P(x)
    ExistentialIntro { witness, witness_type }, // P(w) ⊢ ∃x P(x)
    StructuralInduction { variable, ind_type, step_var },
    Rewrite { from, to },   // Leibniz's law
    Reflexivity,            // a = a
    // ... and more
}
```

### DerivationTree

The recursive proof structure returned by the prover:

```rust
pub struct DerivationTree {
    pub conclusion: ProofExpr,      // What was proved
    pub rule: InferenceRule,        // How it was proved
    pub premises: Vec<DerivationTree>, // Sub-proofs
    pub depth: usize,
    pub substitution: Substitution,
}
```

### ProofGoal

The target state for backward chaining:

```rust
pub struct ProofGoal {
    pub target: ProofExpr,      // What to prove
    pub context: Vec<ProofExpr>, // Local assumptions
}
```

## Public API

### Proof Search

```rust
use logicaffeine_proof::{BackwardChainer, ProofExpr, ProofTerm, ProofGoal};

// Create a prover
let mut prover = BackwardChainer::new();

// Add axioms
prover.add_axiom(human_socrates);
prover.add_axiom(all_humans_mortal);

// Prove a goal
let result = prover.prove(mortal_socrates);

// Or prove with context
let goal = ProofGoal::with_context(target, assumptions);
let result = prover.prove_with_goal(goal);
```

### Unification

```rust
use logicaffeine_proof::{ProofTerm, unify::{unify_terms, unify_exprs, apply_subst_to_term}};

// Unify terms
let pattern = ProofTerm::Function("Mortal".into(), vec![ProofTerm::Variable("x".into())]);
let target = ProofTerm::Function("Mortal".into(), vec![ProofTerm::Constant("Socrates".into())]);
let subst = unify_terms(&pattern, &target)?;
// subst = { "x" ↦ Constant("Socrates") }

// Apply substitution
let result = apply_subst_to_term(&pattern, &subst);
// result = Mortal(Socrates)

// Expression-level unification with alpha-equivalence
let subst = unify_exprs(&expr1, &expr2)?;
```

### Beta-Reduction

```rust
use logicaffeine_proof::unify::beta_reduce;

// (λx. P(x))(Socrates) → P(Socrates)
let reduced = beta_reduce(&lambda_application);
```

### Certification

```rust
use logicaffeine_proof::certifier::{certify, CertificationContext};
use logicaffeine_kernel::Context;

let kernel_ctx = Context::new();
let cert_ctx = CertificationContext::new(&kernel_ctx);
let kernel_term = certify(&derivation_tree, &cert_ctx)?;
```

### Socratic Hints

```rust
use logicaffeine_proof::{suggest_hint, SocraticHint, SuggestedTactic};

let hint = suggest_hint(&goal, &knowledge_base, &failed_tactics);
println!("{}", hint.text);
// e.g., "You have an implication that concludes your goal. Can you prove its antecedent?"
```

## Usage Example

```rust
use logicaffeine_proof::{BackwardChainer, ProofExpr, ProofTerm};

fn main() {
    let mut prover = BackwardChainer::new();

    // Fact: Human(Socrates)
    let human_socrates = ProofExpr::Predicate {
        name: "Human".to_string(),
        args: vec![ProofTerm::Constant("Socrates".to_string())],
        world: None,
    };

    // Rule: ∀x(Human(x) → Mortal(x))
    let all_humans_mortal = ProofExpr::ForAll {
        variable: "x".to_string(),
        body: Box::new(ProofExpr::Implies(
            Box::new(ProofExpr::Predicate {
                name: "Human".to_string(),
                args: vec![ProofTerm::Variable("x".to_string())],
                world: None,
            }),
            Box::new(ProofExpr::Predicate {
                name: "Mortal".to_string(),
                args: vec![ProofTerm::Variable("x".to_string())],
                world: None,
            }),
        )),
    };

    prover.add_axiom(human_socrates);
    prover.add_axiom(all_humans_mortal);

    // Goal: Mortal(Socrates)
    let goal = ProofExpr::Predicate {
        name: "Mortal".to_string(),
        args: vec![ProofTerm::Constant("Socrates".to_string())],
        world: None,
    };

    match prover.prove(goal) {
        Ok(tree) => println!("Proof found:\n{}", tree.display_tree()),
        Err(e) => println!("Could not prove: {:?}", e),
    }
}
```

## Dependencies

### Internal

- `logicaffeine-base` — Shared utilities
- `logicaffeine-kernel` — Type checking and term verification

### External

None (zero external dependencies by design).

## License

Business Source License 1.1 (BUSL-1.1)

- **Free** for individuals and organizations with <25 employees
- **Commercial license** required for organizations with 25+ employees offering Logic Services
- **Converts to MIT** on December 24, 2029

See [LICENSE](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/LICENSE.md) for full terms.
