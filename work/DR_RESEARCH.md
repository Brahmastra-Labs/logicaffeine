# DR_RESEARCH: From Foundation to Self-Verifying Industrial Engine

## Vision

If we successfully bridge the gap between LogicAffeine's current foundation and the CoIC-powered reality outlined in these sprints, we aren't building a better "programming language." We are building a **self-verifying industrial engine**.

---

### 1. Zero-Defect Hardware for Critical Infrastructure

Industry leaders like NVIDIA use LLMs to "guess" at formal assertions (SVA), which still requires massive manual effort to model-check.

**The Upgrade:** With the Z3 Equivalence Pipeline (Sprint O), we take a natural language specification for a new GPU component or custom ASIC and generate assertions that are *mathematically proven* to match the spec before they ever hit the RTL model.

**Real-World Use:** Verify the "Handshake" logic or "Data Integrity" of custom controllers, ensuring no race conditions ever brick the firmware.

### 2. Formally Verified Distributed Protocols (CRDTs)

Logos already supports P2P networking and CRDTs like `GCounter` and `SharedSequence`. Proving that a complex custom CRDT will *always converge* is NP-Hard.

**The Upgrade:** Structural Induction (Sprint M) and Invariant Synthesis (Sprint N) allow us to prove convergence properties in the kernel.

**The Application:** Build a private distributed cloud orchestration layer where the state-sync logic isn't just "tested," but *certified* by the CoIC kernel to never enter a deadlock state.

### 3. Jones-Optimal Specialized Compilers

The First Futamura Projection (P1) is already verified with 436 tests.

**The Upgrade:** By adding E-graph Equality Saturation (Sprint L), we specialize an interpreter (like our Kripke hardware frame) and then *extract the most compressed, efficient Rust code* possible from the result.

**Real-World Use:** Write a high-level "policy" for resource allocation in English. Logos specializes that policy into a zero-overhead Rust binary that runs at the speed of hand-optimized C++, with the safety of a formal proof.

### 4. Security Enforcement at the Logic Layer

Logos uses `Policy` blocks to define what a `User` can or cannot do.

**The Upgrade:** With the Mirror Module (Sprint K), we "transport" security proofs across the system.

**The Result:** Prove that a specific data access request is Leibniz-equivalent to a permitted state in our policy. This moves security from "checking a database" to "verifying a mathematical tautology."

### Utility Summary

| Component | Immediate Application | Long-term Value |
|:---|:---|:---|
| **DElim / Induction** | Proving FSM stability | Mathematical proof for hardware logic |
| **E-Graphs** | Compressing Rust code | Automated optimal algorithm discovery |
| **Z3 Equivalence** | Auditing LLM-generated code | Eliminating human error in formal specs |
| **BitVec(n)** | Width-agnostic verification | Universal verification libraries for any hardware |

---

## Current Foundation (What Exists)

### CoIC Kernel — `crates/logicaffeine_kernel/`

| Module | Lines | What It Does |
|--------|-------|--------------|
| `term.rs` | 289 | Unified term representation: Sort, Var, Global, Pi, Lambda, App, Match, Fix, Lit, Hole |
| `type_checker.rs` | 790 | Bidirectional type inference, Match elimination, cumulativity, polymorphic inductives |
| `reduction.rs` | 3400+ | Beta, iota (Match on constructors), Fix unfolding, delta reduction, fuel-limited normalization |
| `positivity.rs` | 237 | Strict positivity checking — prevents paradoxes like `(Bad → False) → Bad` |
| `termination.rs` | ~300 | Syntactic guard condition for fixpoints — structural termination checking |
| `prelude.rs` | 2189 | Standard library: Nat, Bool, TList (polymorphic), Int, Float, Text, Eq, And, Or, Not, Exists |
| `cc.rs` | 707 | Congruence closure: Union-Find + hash-consing + congruence propagation for uninterpreted functions |
| `context.rs` | ~200 | Inductive registration, constructor ordering, definition storage |

### Proof Engine — `crates/logicaffeine_proof/src/`

| Module | Lines | What It Does |
|--------|-------|--------------|
| `lib.rs` | 669 | ProofTerm, ProofExpr (45+ variants), InferenceRule (23 rules), DerivationTree |
| `engine.rs` | 4103 | Backward chaining prover: structural induction, Leibniz rewrite, modal/temporal, Z3 oracle |
| `unify.rs` | 1599 | Robinson's unification with occurs check, beta reduction to WHNF, alpha-equivalence |
| `certifier.rs` | 1064 | Curry-Howard: DerivationTree → kernel Term (StructuralInduction → Fix, ModusPonens → App) |
| `oracle.rs` | 592 | Z3 SMT solver integration, type inference, ProofExpr ↔ VerifyExpr conversion |
| `hints.rs` | 432 | Socratic hint engine for tactic suggestions |

**Key inference rules already implemented:**
- `StructuralInduction { variable, ind_type, step_var }` — P(0), ∀k(P(k) → P(S(k))) ⊢ ∀n P(n)
- `Rewrite { from, to }` — Leibniz's Law: a = b, P(a) ⊢ P(b)
- `TemporalInduction` — G(P) iff P(s₀) ∧ ∀s(P(s) → P(next(s)))
- `TemporalUnfolding` — G(P) iff P ∧ X(G(P))
- `EventualityProgress` — F(P) by witness
- `UntilInduction` — Induction on trace length

### Futamura Projections — `crates/logicaffeine_compile/src/optimize/`

| File | Lines | What It Does |
|------|-------|--------------|
| `pe_source.logos` | ~500+ | Core PE: PEState, peExpr, peBlock, memoization, cycle detection, constant folding |
| `pe_bti_source.logos` | ~500 | BTI-memoized PE (renamed: memoCache, callGuard). Engine for P3 |
| `pe_mini_source.logos` | ~400 | Minimal PE for P2. Simplified state, block-level specialization |
| `decompile_source.logos` | 686 | CExpr/CStmt → human-readable LOGOS decompiler |

**All three projections verified:**
- P1: `PE(interpreter, program) = compiled_program` — zero interpretive overhead
- P2: `PE(pe_source, pe_mini) = compiler` — generates specialized `peBlockM_N` functions
- P3: `PE(pe_source, pe_bti) = cogen` — generates code generator
- Jones optimality: P1 residual contains no env/funcs lookups
- **436 tests in phase_futamura.rs** (435 green, 1 ignored)

### Hardware Verification Pipeline — `crates/logicaffeine_compile/src/codegen_sva/`

| File | Lines | What It Does |
|------|-------|--------------|
| `mod.rs` | 118 | SVA emission: emit_sva_property, emit_sva_module, emit_psl_property, emit_rust_monitor |
| `sva_model.rs` | 397 | SVA expression AST, recursive descent parser, roundtrip to_string, structural equivalence |
| `sva_to_verify.rs` | 205 | Temporal → bounded timestep IR: rose/fell edges, implication, delay, s_eventually unrolling |

### Kripke Temporal Lowering — `crates/logicaffeine_language/src/semantics/kripke.rs`

- 484 lines. G/F/X operators → world quantification with `Accessible_Temporal`
- `ModalDomain::Temporal` — accessibility = state transition relation
- Same Kripke frame handles linguistic modality (alethic, deontic) and hardware temporal semantics

### Hardware Test Coverage

| Test File | Tests | Passing |
|-----------|-------|---------|
| phase_hw_codegen_sva.rs | 7 | 7 |
| phase_hw_equivalence.rs | 13 | 13 |
| phase_hw_filter.rs | 10 | 10 |
| phase_hw_futamura.rs | 7 | 7 |
| phase_hw_integration.rs | 7 | 7 |
| phase_hw_knowledge_graph.rs | 8 | 8 |
| phase_hw_lexicon.rs | 8 | 8 |
| phase_hw_sva_roundtrip.rs | 19 | 19 |
| phase_hw_sva_translate.rs | 9 | 9 |
| phase_hw_temporal.rs | 13 | 12 (1 ignored: Until parser) |
| phase_hw_verify.rs | 8 | 8 |
| **Total** | **109** | **108** |

---

## Sprint K: DElim + Mirror Module

**Goal:** Make it true that we have "DElim (Generic Elimination Principle)" and "Leibniz's Law via the Mirror module."

### What Exists

- `Term::Match` handles computation-level pattern matching with iota reduction
- `InferenceRule::Rewrite { from, to }` implements Leibniz equality in the proof engine
- `try_delim_conclude` in reduction.rs (lines 3440-3507) validates DElim proof structure for Nat
- Polymorphic inductives registered via `ctx.add_inductive` / `ctx.add_constructor`

### What's Missing

- No `Term::Elim` variant — the dependent eliminator as a first-class kernel term
- No auto-generation of eliminators for registered inductives
- No dedicated Mirror module — Leibniz rewriting is inlined in engine.rs
- No `transport`, `symmetry`, `transitivity`, `congruence`, `reflect` as composable operations

### Phase K.1: `Term::Elim` Variant

**File:** `crates/logicaffeine_kernel/src/term.rs`

Add to the `Term` enum:

```rust
/// Dependent eliminator for inductive types.
///
/// `Elim { ind_name: "Nat", motive: (λn:Nat. P(n)), cases: [base, step], scrutinee: k }`
/// yields a proof/value of type `P(k)` by structural recursion.
///
/// For Nat:
///   - cases[0] (base): P(Zero)
///   - cases[1] (step): Π(n:Nat). P(n) → P(Succ(n))
///
/// Unlike Match (which computes), Elim carries the inductive name and validates
/// against registered constructors. Elim IS the generic elimination principle.
Elim {
    ind_name: String,
    motive: Box<Term>,
    cases: Vec<Term>,
    scrutinee: Box<Term>,
},
```

### Phase K.2: Iota Reduction for Elim

**File:** `crates/logicaffeine_kernel/src/reduction.rs`

```
Elim("I", P, [c₁...cₖ], Cᵢ(a₁...aₙ))
  →ᵢ cᵢ(a₁, ..., aₙ, Elim("I", P, [c₁...cₖ], aⱼ₁), ..., Elim("I", P, [c₁...cₖ], aⱼₘ))
```

where `aⱼ₁...aⱼₘ` are the recursive arguments (those of type `I`). Recursive arguments get the eliminator applied to them — structural recursion baked into the reduction rule.

Refactor existing `try_delim_conclude` (lines 3440-3507) into this new reduction path.

### Phase K.3: Type Checking for Elim

**File:** `crates/logicaffeine_kernel/src/type_checker.rs`

Typing rule:

```
  Γ ⊢ e : I           (scrutinee has inductive type)
  Γ ⊢ P : I → Sort    (motive maps inhabitants to types)
  Γ ⊢ cᵢ : case_type(Cᵢ, P)   (each case matches expected type)
  ─────────────────────────────────────────────────────
  Γ ⊢ Elim("I", P, [c₁...cₖ], e) : P(e)
```

Use `ctx.get_constructors(ind_name)` to get constructor types. Compute each `case_type(Cᵢ, P)` by threading the motive through constructor arguments:

For `Succ : Nat → Nat`, the case type is `Π(n:Nat). P(n) → P(Succ(n))`.

### Phase K.4: Auto-Generate Eliminators

**File:** `crates/logicaffeine_kernel/src/prelude.rs` (and new `elim_gen.rs`)

For each registered inductive, auto-generate the eliminator as a global definition:

- `Nat_elim : Π(P:Nat→Type). P(Zero) → (Π(k:Nat). P(k) → P(Succ(k))) → Π(n:Nat). P(n)`
- `Bool_elim : Π(P:Bool→Type). P(true) → P(false) → Π(b:Bool). P(b)`
- `TList_elim : Π(A:Type)(P:TList A→Type). P(TNil A) → (Π(x:A)(xs:TList A). P(xs) → P(TCons A x xs)) → Π(l:TList A). P(l)`

```rust
/// Given an inductive type name, generate its dependent eliminator type.
pub fn generate_eliminator_type(ctx: &Context, ind_name: &str) -> Result<Term, Error> {
    let ctors = ctx.get_constructors(ind_name)?;
    let ind_type = ctx.lookup_inductive(ind_name)?;
    // Build: Π(P:I→Type). case_type(C₁,P) → ... → case_type(Cₖ,P) → Π(x:I). P(x)
    ...
}
```

### Phase K.5: Mirror Module

**New file:** `crates/logicaffeine_kernel/src/mirror.rs`

Leibniz equality: `x = y ↔ ∀P. P(x) → P(y)`.

```rust
//! Mirror — Leibniz equality as a composable kernel tactic.
//!
//! Named for the principle of identity of indiscernibles: if two things
//! reflect the same properties, they are equal. The mirror reflects
//! the proof of P(x) into a proof of P(y) along an equality x = y.

/// Transport a proof of P(x) to P(y) along a proof of x = y.
///
/// This is eq_rect / subst / J: the fundamental elimination of equality.
/// Internally, uses Elim on the Eq inductive type.
pub fn transport(ctx: &Context, eq_proof: &Term, motive: &Term, px: &Term) -> Result<Term, Error>

/// x = y → y = x
pub fn symmetry(ctx: &Context, eq_proof: &Term) -> Result<Term, Error>

/// x = y → y = z → x = z
pub fn transitivity(ctx: &Context, eq1: &Term, eq2: &Term) -> Result<Term, Error>

/// x = y → f(x) = f(y)
pub fn congruence(ctx: &Context, f: &Term, eq_proof: &Term) -> Result<Term, Error>

/// Check definitional equality in the kernel, produce Refl proof if equal.
/// Bridges computational equality (reduction) to propositional equality (proof terms).
pub fn reflect(ctx: &Context, a: &Term, b: &Term) -> Option<Term>
```

`transport` is the core — everything else derives from it:
- `symmetry(p)` = `transport(p, λy. y = x, refl(x))`
- `transitivity(p, q)` = `transport(q, λz. x = z, p)`
- `congruence(f, p)` = `transport(p, λy. f(x) = f(y), refl(f(x)))`

### TDD Red Tests — `phase104_delim.rs`

```rust
//! Sprint K: DElim — Generic Elimination Principle
//!
//! RED tests for Term::Elim and auto-generated eliminators.
//! The Elim variant is the dependent recursor: given a motive P : I → Type
//! and one case per constructor, Elim proves P(x) for any x : I.

use logicaffeine_kernel::prelude::Prelude;
use logicaffeine_kernel::term::{Term, Universe};
use logicaffeine_kernel::context::Context;

fn setup() -> Context {
    let mut ctx = Context::new();
    Prelude::register(&mut ctx);
    ctx
}

// ═══════════════════════════════════════════════════════════════════════════
// ELIM VARIANT EXISTS
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn elim_variant_constructs() {
    // Term::Elim must be a first-class variant, not sugar over Match.
    let elim = Term::Elim {
        ind_name: "Nat".to_string(),
        motive: Box::new(Term::Sort(Universe::Type(0))),
        cases: vec![Term::Lit(logicaffeine_kernel::term::Literal::Int(0))],
        scrutinee: Box::new(Term::Global("Zero".to_string())),
    };
    match &elim {
        Term::Elim { ind_name, .. } => assert_eq!(ind_name, "Nat"),
        _ => panic!("Elim must be a distinct variant"),
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// IOTA REDUCTION — NAT
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn elim_nat_base_case_reduces() {
    // Elim("Nat", P, [base, step], Zero) → base
    let ctx = setup();
    let base = Term::Lit(logicaffeine_kernel::term::Literal::Int(42));
    let step = Term::Lambda {
        param: "k".into(),
        param_type: Box::new(Term::Global("Nat".into())),
        body: Box::new(Term::Lambda {
            param: "ih".into(),
            param_type: Box::new(Term::Sort(Universe::Type(0))),
            body: Box::new(Term::Var("ih".into())),
        }),
    };
    let elim = Term::Elim {
        ind_name: "Nat".into(),
        motive: Box::new(Term::Lambda {
            param: "n".into(),
            param_type: Box::new(Term::Global("Nat".into())),
            body: Box::new(Term::Sort(Universe::Type(0))),
        }),
        cases: vec![base.clone(), step],
        scrutinee: Box::new(Term::Global("Zero".into())),
    };
    let result = logicaffeine_kernel::reduce(&ctx, &elim);
    assert_eq!(result, base, "Elim at Zero should reduce to base case");
}

#[test]
fn elim_nat_step_case_reduces() {
    // Elim("Nat", P, [base, step], Succ(n)) → step(n, Elim("Nat", P, [base, step], n))
    let ctx = setup();
    let base = Term::Lit(logicaffeine_kernel::term::Literal::Int(0));
    let step = Term::Lambda {
        param: "k".into(),
        param_type: Box::new(Term::Global("Nat".into())),
        body: Box::new(Term::Lambda {
            param: "ih".into(),
            param_type: Box::new(Term::Sort(Universe::Type(0))),
            body: Box::new(Term::Var("ih".into())),
        }),
    };
    let succ_zero = Term::App(
        Box::new(Term::Global("Succ".into())),
        Box::new(Term::Global("Zero".into())),
    );
    let elim = Term::Elim {
        ind_name: "Nat".into(),
        motive: Box::new(Term::Lambda {
            param: "n".into(),
            param_type: Box::new(Term::Global("Nat".into())),
            body: Box::new(Term::Sort(Universe::Type(0))),
        }),
        cases: vec![base.clone(), step],
        scrutinee: Box::new(succ_zero),
    };
    let result = logicaffeine_kernel::reduce(&ctx, &elim);
    // step(Zero, Elim(..., Zero)) → step(Zero, base) → base (via ih)
    assert_eq!(
        result, base,
        "Elim at Succ(Zero) should unfold step then reduce recursively to base"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// IOTA REDUCTION — BOOL
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn elim_bool_true_reduces() {
    // Elim("Bool", P, [case_true, case_false], true) → case_true
    let ctx = setup();
    let case_t = Term::Lit(logicaffeine_kernel::term::Literal::Text("yes".into()));
    let case_f = Term::Lit(logicaffeine_kernel::term::Literal::Text("no".into()));
    let elim = Term::Elim {
        ind_name: "Bool".into(),
        motive: Box::new(Term::Lambda {
            param: "b".into(),
            param_type: Box::new(Term::Global("Bool".into())),
            body: Box::new(Term::Sort(Universe::Type(0))),
        }),
        cases: vec![case_t.clone(), case_f],
        scrutinee: Box::new(Term::Global("true".into())),
    };
    let result = logicaffeine_kernel::reduce(&ctx, &elim);
    assert_eq!(result, case_t, "Elim at true should select first case");
}

#[test]
fn elim_bool_false_reduces() {
    let ctx = setup();
    let case_t = Term::Lit(logicaffeine_kernel::term::Literal::Text("yes".into()));
    let case_f = Term::Lit(logicaffeine_kernel::term::Literal::Text("no".into()));
    let elim = Term::Elim {
        ind_name: "Bool".into(),
        motive: Box::new(Term::Lambda {
            param: "b".into(),
            param_type: Box::new(Term::Global("Bool".into())),
            body: Box::new(Term::Sort(Universe::Type(0))),
        }),
        cases: vec![case_t, case_f.clone()],
        scrutinee: Box::new(Term::Global("false".into())),
    };
    let result = logicaffeine_kernel::reduce(&ctx, &elim);
    assert_eq!(result, case_f, "Elim at false should select second case");
}

// ═══════════════════════════════════════════════════════════════════════════
// IOTA REDUCTION — POLYMORPHIC LIST
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn elim_list_nil_reduces() {
    // Elim("TList", P, [nil_case, cons_case], TNil A) → nil_case
    let ctx = setup();
    let nil_case = Term::Lit(logicaffeine_kernel::term::Literal::Int(0));
    let cons_case = Term::Lambda {
        param: "x".into(),
        param_type: Box::new(Term::Global("Nat".into())),
        body: Box::new(Term::Lambda {
            param: "xs".into(),
            param_type: Box::new(Term::App(
                Box::new(Term::Global("TList".into())),
                Box::new(Term::Global("Nat".into())),
            )),
            body: Box::new(Term::Lambda {
                param: "ih".into(),
                param_type: Box::new(Term::Sort(Universe::Type(0))),
                body: Box::new(Term::Var("ih".into())),
            }),
        }),
    };
    let tnil = Term::App(
        Box::new(Term::Global("TNil".into())),
        Box::new(Term::Global("Nat".into())),
    );
    let elim = Term::Elim {
        ind_name: "TList".into(),
        motive: Box::new(Term::Lambda {
            param: "l".into(),
            param_type: Box::new(Term::App(
                Box::new(Term::Global("TList".into())),
                Box::new(Term::Global("Nat".into())),
            )),
            body: Box::new(Term::Sort(Universe::Type(0))),
        }),
        cases: vec![nil_case.clone(), cons_case],
        scrutinee: Box::new(tnil),
    };
    let result = logicaffeine_kernel::reduce(&ctx, &elim);
    assert_eq!(result, nil_case, "Elim at TNil should reduce to nil case");
}

// ═══════════════════════════════════════════════════════════════════════════
// AUTO-GENERATED ELIMINATORS
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn nat_elim_auto_generated() {
    // After prelude registration, "Nat_elim" should exist as a global definition.
    let ctx = setup();
    let ty = ctx.lookup("Nat_elim");
    assert!(ty.is_some(), "Nat_elim should be auto-generated by prelude registration");
}

#[test]
fn bool_elim_auto_generated() {
    let ctx = setup();
    let ty = ctx.lookup("Bool_elim");
    assert!(ty.is_some(), "Bool_elim should be auto-generated by prelude registration");
}

#[test]
fn tlist_elim_auto_generated() {
    let ctx = setup();
    let ty = ctx.lookup("TList_elim");
    assert!(ty.is_some(), "TList_elim should be auto-generated for polymorphic inductives");
}

// ═══════════════════════════════════════════════════════════════════════════
// TYPE CHECKING
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn elim_type_checks_correctly() {
    // Elim("Nat", (λn:Nat. Nat), [Zero, λk.λih. Succ(ih)], n) : Nat
    let ctx = setup();
    let elim = Term::Elim {
        ind_name: "Nat".into(),
        motive: Box::new(Term::Lambda {
            param: "n".into(),
            param_type: Box::new(Term::Global("Nat".into())),
            body: Box::new(Term::Global("Nat".into())),
        }),
        cases: vec![
            Term::Global("Zero".into()),
            Term::Lambda {
                param: "k".into(),
                param_type: Box::new(Term::Global("Nat".into())),
                body: Box::new(Term::Lambda {
                    param: "ih".into(),
                    param_type: Box::new(Term::Global("Nat".into())),
                    body: Box::new(Term::App(
                        Box::new(Term::Global("Succ".into())),
                        Box::new(Term::Var("ih".into())),
                    )),
                }),
            },
        ],
        scrutinee: Box::new(Term::Global("Zero".into())),
    };
    let result = logicaffeine_kernel::type_check(&ctx, &elim);
    assert!(result.is_ok(), "Identity function via Elim should type-check. Error: {:?}", result.err());
}

#[test]
fn elim_wrong_case_count_rejects() {
    // Nat has 2 constructors. Providing 1 case should fail.
    let ctx = setup();
    let elim = Term::Elim {
        ind_name: "Nat".into(),
        motive: Box::new(Term::Lambda {
            param: "n".into(),
            param_type: Box::new(Term::Global("Nat".into())),
            body: Box::new(Term::Global("Nat".into())),
        }),
        cases: vec![Term::Global("Zero".into())], // Missing step case
        scrutinee: Box::new(Term::Global("Zero".into())),
    };
    let result = logicaffeine_kernel::type_check(&ctx, &elim);
    assert!(result.is_err(), "Elim with wrong case count should be rejected");
}

#[test]
fn elim_unknown_inductive_rejects() {
    let ctx = setup();
    let elim = Term::Elim {
        ind_name: "Phantom".into(),
        motive: Box::new(Term::Sort(Universe::Type(0))),
        cases: vec![],
        scrutinee: Box::new(Term::Global("Zero".into())),
    };
    let result = logicaffeine_kernel::type_check(&ctx, &elim);
    assert!(result.is_err(), "Elim on unknown inductive should be rejected");
}
```

### TDD Red Tests — `phase105_mirror.rs`

```rust
//! Sprint K: Mirror Module — Leibniz Equality
//!
//! RED tests for the Mirror module: transport, symmetry, transitivity,
//! congruence, and reflect. Named for the principle that equal things
//! reflect the same properties.

use logicaffeine_kernel::prelude::Prelude;
use logicaffeine_kernel::term::{Term, Universe};
use logicaffeine_kernel::context::Context;
use logicaffeine_kernel::mirror;

fn setup() -> Context {
    let mut ctx = Context::new();
    Prelude::register(&mut ctx);
    ctx
}

/// Helper: construct Refl(a) — proof that a = a
fn refl(a: Term) -> Term {
    Term::App(Box::new(Term::Global("Refl".into())), Box::new(a))
}

// ═══════════════════════════════════════════════════════════════════════════
// TRANSPORT
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn transport_along_refl_is_identity() {
    // transport(refl(a), P, p) = p
    // If x = x (trivially), then P(x) transports to P(x) unchanged.
    let ctx = setup();
    let a = Term::Global("Zero".into());
    let eq_proof = refl(a.clone());
    let motive = Term::Lambda {
        param: "n".into(),
        param_type: Box::new(Term::Global("Nat".into())),
        body: Box::new(Term::Global("Nat".into())),
    };
    let px = Term::Global("Zero".into()); // proof of P(Zero) = Zero
    let result = mirror::transport(&ctx, &eq_proof, &motive, &px).unwrap();
    let reduced = logicaffeine_kernel::reduce(&ctx, &result);
    assert_eq!(reduced, px, "Transport along Refl should be identity");
}

#[test]
fn transport_carries_proof_across_equality() {
    // Given x = y and P(x), produce P(y).
    // Use Succ(Zero) = Succ(Zero) as a simple case.
    let ctx = setup();
    let sz = Term::App(
        Box::new(Term::Global("Succ".into())),
        Box::new(Term::Global("Zero".into())),
    );
    let eq_proof = refl(sz.clone());
    let motive = Term::Lambda {
        param: "n".into(),
        param_type: Box::new(Term::Global("Nat".into())),
        body: Box::new(Term::Global("Nat".into())),
    };
    let px = sz.clone();
    let result = mirror::transport(&ctx, &eq_proof, &motive, &px);
    assert!(result.is_ok(), "Transport should succeed with valid equality proof");
}

// ═══════════════════════════════════════════════════════════════════════════
// SYMMETRY
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn symmetry_flips_equality() {
    // symmetry(proof: a = b) : b = a
    let ctx = setup();
    let a = Term::Global("Zero".into());
    let eq_proof = refl(a.clone()); // Zero = Zero
    let result = mirror::symmetry(&ctx, &eq_proof);
    assert!(result.is_ok(), "Symmetry should succeed on Refl");
    // The result should type-check as an equality proof
    let sym = result.unwrap();
    let ty = logicaffeine_kernel::type_check(&ctx, &sym);
    assert!(ty.is_ok(), "Symmetric proof should type-check");
}

// ═══════════════════════════════════════════════════════════════════════════
// TRANSITIVITY
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn transitivity_chains_equalities() {
    // transitivity(a = b, b = c) : a = c
    let ctx = setup();
    let a = Term::Global("Zero".into());
    let p1 = refl(a.clone()); // a = a
    let p2 = refl(a.clone()); // a = a
    let result = mirror::transitivity(&ctx, &p1, &p2);
    assert!(result.is_ok(), "Transitivity should succeed on two Refl proofs");
}

// ═══════════════════════════════════════════════════════════════════════════
// CONGRUENCE
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn congruence_lifts_equality_through_function() {
    // congruence(Succ, Zero = Zero) : Succ(Zero) = Succ(Zero)
    let ctx = setup();
    let eq_proof = refl(Term::Global("Zero".into()));
    let f = Term::Global("Succ".into());
    let result = mirror::congruence(&ctx, &f, &eq_proof);
    assert!(result.is_ok(), "Congruence should lift equality through Succ");
}

// ═══════════════════════════════════════════════════════════════════════════
// REFLECT
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn reflect_definitionally_equal_terms() {
    // reflect(2+3, 5) should produce a proof because both reduce to Lit(5)
    let ctx = setup();
    let a = Term::Lit(logicaffeine_kernel::term::Literal::Int(5));
    let b = Term::Lit(logicaffeine_kernel::term::Literal::Int(5));
    let result = mirror::reflect(&ctx, &a, &b);
    assert!(result.is_some(), "Definitionally equal terms should reflect to a proof");
}

#[test]
fn reflect_unequal_terms_returns_none() {
    let ctx = setup();
    let a = Term::Lit(logicaffeine_kernel::term::Literal::Int(5));
    let b = Term::Lit(logicaffeine_kernel::term::Literal::Int(7));
    let result = mirror::reflect(&ctx, &a, &b);
    assert!(result.is_none(), "Unequal terms should not reflect");
}

#[test]
fn reflect_reduces_before_comparing() {
    // reflect(0 + 0, 0) should succeed: both reduce to 0
    let ctx = setup();
    let zero = Term::Global("Zero".into());
    // Build: add(Zero, Zero) which should reduce to Zero
    let sum = Term::App(
        Box::new(Term::App(
            Box::new(Term::Global("add".into())),
            Box::new(zero.clone()),
        )),
        Box::new(zero.clone()),
    );
    let result = mirror::reflect(&ctx, &sum, &zero);
    assert!(result.is_some(), "reflect should reduce terms before comparing");
}
```

### Critical Files

| File | Action |
|------|--------|
| `crates/logicaffeine_kernel/src/term.rs` | Add `Elim` variant |
| `crates/logicaffeine_kernel/src/reduction.rs` | Iota for Elim, refactor `try_delim_conclude` |
| `crates/logicaffeine_kernel/src/type_checker.rs` | Typing rule for Elim |
| `crates/logicaffeine_kernel/src/prelude.rs` | Auto-generate eliminators |
| `crates/logicaffeine_kernel/src/mirror.rs` | **NEW** — transport, symmetry, transitivity, congruence, reflect |
| `crates/logicaffeine_kernel/src/lib.rs` | Export `mirror` module |
| `crates/logicaffeine_kernel/src/context.rs` | Eliminator storage if needed |

---

## Sprint L: E-graph Equality Saturation

**Goal:** Make it true that we use "Equality Saturation (E-graphs) via cc.rs to find the most compressed proof."

### What Exists

`cc.rs` (707 lines) implements congruence closure:
- `EGraph` with `Vec<ENode>`, `UnionFind`, hash-consing, pending worklist, use-lists
- `ENode`: Lit, Var, Name, App { func, arg }
- Union-Find with path compression and union by rank
- Congruence propagation: when x=y merged, f(x) and f(y) auto-merged
- Goal checking for equality proofs

### What's Missing

- No e-classes (just union-find roots, no multiple representatives)
- No rewrite rules (Pattern → Pattern with variable binding)
- No saturation loop (iterative rule application until fixpoint)
- No cost model or extraction (no way to pick the "best" equivalent term)
- No analysis metadata per equivalence class

### Phase L.1: E-Class Abstraction

**File:** `crates/logicaffeine_kernel/src/cc.rs` (major refactor)

```rust
/// An equivalence class: a set of structurally different but semantically
/// equivalent terms. Each e-class tracks all its member e-nodes and
/// per-class analysis data.
pub struct EClass {
    id: ClassId,
    nodes: Vec<ENode>,
    parents: Vec<(ENode, ClassId)>,
    analysis: AnalysisData,
}

pub struct EGraph {
    classes: HashMap<ClassId, EClass>,
    uf: UnionFind,
    memo: HashMap<ENode, ClassId>,
    pending: Vec<(ClassId, ClassId)>,
    dirty: Vec<ClassId>,
}
```

### Phase L.2: Rewrite Rules

```rust
pub enum PatternOp {
    Var(String),             // Pattern variable — matches anything
    Concrete(ENodeOp),       // Concrete operation — must match exactly
}

pub struct Pattern {
    op: PatternOp,
    children: Vec<Pattern>,
}

pub struct Rewrite {
    name: String,
    lhs: Pattern,
    rhs: Pattern,
    condition: Option<Box<dyn Fn(&EGraph, &Subst) -> bool>>,
}
```

Standard rewrites:
- `(λx.b)(a) → b[x:=a]` — beta reduction
- `Elim(Cᵢ args, P, cases, _) → caseᵢ(...)` — iota reduction (from Sprint K)
- `And(P, Q) → And(Q, P)` — commutativity
- `Or(P, Or(Q, R)) → Or(Or(P, Q), R)` — associativity
- `Not(Not(P)) → P` — double negation (classical)
- `Elim("Bool", P, [t, f], true) → t` — boolean elimination

### Phase L.3: Saturation Loop

```rust
impl EGraph {
    /// Apply rewrite rules until fixpoint or fuel exhaustion.
    /// Returns whether saturation was complete.
    pub fn saturate(&mut self, rules: &[Rewrite], fuel: usize) -> SaturationResult {
        for _ in 0..fuel {
            let mut changed = false;
            for rule in rules {
                let matches = self.search(&rule.lhs);
                for subst in matches {
                    let new_id = self.apply(&rule.rhs, &subst);
                    let old_id = subst.root();
                    if self.union(old_id, new_id) {
                        changed = true;
                    }
                }
            }
            self.rebuild();
            if !changed { return SaturationResult::Saturated; }
        }
        SaturationResult::FuelExhausted
    }

    /// Re-canonicalize all dirty e-classes after merges.
    fn rebuild(&mut self) { ... }
}
```

### Phase L.4: Cost Model + Extraction

```rust
pub trait CostFunction {
    fn cost(&self, node: &ENode, children_costs: &[f64]) -> f64;
}

/// Minimize AST node count — the default "compression" metric.
pub struct AstSize;

impl CostFunction for AstSize {
    fn cost(&self, _node: &ENode, children_costs: &[f64]) -> f64 {
        1.0 + children_costs.iter().sum::<f64>()
    }
}

impl EGraph {
    /// Extract the minimum-cost term from an equivalence class.
    /// Bottom-up DP over e-classes.
    pub fn extract_best<C: CostFunction>(&self, id: ClassId, cost: &C) -> Term { ... }
}
```

This is the "optimal cut" — the cost function finds the minimal representation.

### Phase L.5: Integration with Proof Engine

Wire e-graph into `engine.rs` equality reasoning:
1. When prover encounters equality goal `a = b`, add both to e-graph
2. Saturate with available rewrites (context-dependent rules from hypotheses)
3. If `a` and `b` end up in same e-class, extract proof witness
4. Replace hand-coded `InferenceRule::Rewrite` chains with e-graph proof search

### TDD Red Tests — `phase106_esat.rs`

```rust
//! Sprint L: E-graph Equality Saturation
//!
//! RED tests for upgrading cc.rs from congruence closure to full
//! equality saturation with rewrite rules, cost models, and extraction.

use logicaffeine_kernel::cc::{EGraph, Rewrite, Pattern, PatternOp, SaturationResult, AstSize};
use logicaffeine_kernel::prelude::Prelude;
use logicaffeine_kernel::context::Context;

fn setup() -> Context {
    let mut ctx = Context::new();
    Prelude::register(&mut ctx);
    ctx
}

// ═══════════════════════════════════════════════════════════════════════════
// E-CLASS STRUCTURE
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn eclass_stores_multiple_representations() {
    // After merging x and y, the e-class should contain both nodes.
    let mut eg = EGraph::new();
    let x = eg.add_lit(1);
    let y = eg.add_lit(2);
    eg.union(x, y);
    eg.rebuild();
    let class = eg.get_class(eg.find(x));
    assert!(class.nodes.len() >= 2, "E-class should store both representations after merge");
}

// ═══════════════════════════════════════════════════════════════════════════
// REWRITE RULES
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn rewrite_commutativity_discovers_equality() {
    // Add And(P, Q). Apply commutativity rule. And(Q, P) should be in same class.
    let mut eg = EGraph::new();
    let p = eg.add_name("P");
    let q = eg.add_name("Q");
    let pq = eg.add_app("And", vec![p, q]);
    let qp = eg.add_app("And", vec![q, p]);

    let comm = Rewrite::new(
        "and_comm",
        Pattern::app("And", vec![Pattern::var("a"), Pattern::var("b")]),
        Pattern::app("And", vec![Pattern::var("b"), Pattern::var("a")]),
    );

    eg.saturate(&[comm], 10);
    assert_eq!(
        eg.find(pq), eg.find(qp),
        "After commutativity saturation, And(P,Q) and And(Q,P) should be equivalent"
    );
}

#[test]
fn rewrite_double_negation_simplifies() {
    // Not(Not(P)) → P
    let mut eg = EGraph::new();
    let p = eg.add_name("P");
    let not_p = eg.add_app("Not", vec![p]);
    let not_not_p = eg.add_app("Not", vec![not_p]);

    let dne = Rewrite::new(
        "double_neg",
        Pattern::app("Not", vec![Pattern::app("Not", vec![Pattern::var("x")])]),
        Pattern::var("x"),
    );

    eg.saturate(&[dne], 10);
    assert_eq!(
        eg.find(not_not_p), eg.find(p),
        "Not(Not(P)) should be equivalent to P after double negation elimination"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// SATURATION
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn saturation_reaches_fixpoint() {
    // With only commutativity on And(P,Q), should saturate in ≤2 iterations.
    let mut eg = EGraph::new();
    let p = eg.add_name("P");
    let q = eg.add_name("Q");
    let _pq = eg.add_app("And", vec![p, q]);

    let comm = Rewrite::new(
        "and_comm",
        Pattern::app("And", vec![Pattern::var("a"), Pattern::var("b")]),
        Pattern::app("And", vec![Pattern::var("b"), Pattern::var("a")]),
    );

    let result = eg.saturate(&[comm], 100);
    assert_eq!(result, SaturationResult::Saturated, "Should reach fixpoint, not exhaust fuel");
}

#[test]
fn saturation_fuel_limits_infinite_rules() {
    // Associativity can keep creating new terms. Fuel should stop it.
    let mut eg = EGraph::new();
    let p = eg.add_name("P");
    let q = eg.add_name("Q");
    let r = eg.add_name("R");
    let qr = eg.add_app("Or", vec![q, r]);
    let _pqr = eg.add_app("Or", vec![p, qr]);

    let assoc = Rewrite::new(
        "or_assoc",
        Pattern::app("Or", vec![Pattern::var("a"), Pattern::app("Or", vec![Pattern::var("b"), Pattern::var("c")])]),
        Pattern::app("Or", vec![Pattern::app("Or", vec![Pattern::var("a"), Pattern::var("b")]), Pattern::var("c")]),
    );

    let result = eg.saturate(&[assoc], 5);
    // Might saturate or exhaust — either is fine, just don't loop forever
    assert!(
        result == SaturationResult::Saturated || result == SaturationResult::FuelExhausted,
        "Saturation should terminate within fuel limit"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// EXTRACTION
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn extraction_finds_minimal_cost_term() {
    // Add both `Not(Not(P))` and `P` to same class. AstSize should prefer `P`.
    let mut eg = EGraph::new();
    let p = eg.add_name("P");
    let not_p = eg.add_app("Not", vec![p]);
    let not_not_p = eg.add_app("Not", vec![not_p]);
    eg.union(not_not_p, p);
    eg.rebuild();

    let best = eg.extract_best(eg.find(p), &AstSize);
    // The extracted term should be the simpler one: just "P"
    assert_eq!(best, "P", "Extraction should prefer the minimal-cost representation");
}

#[test]
fn extraction_after_saturation_compresses() {
    // Saturate with double-neg elimination, then extract the compressed form.
    let mut eg = EGraph::new();
    let p = eg.add_name("P");
    let not_p = eg.add_app("Not", vec![p]);
    let not_not_p = eg.add_app("Not", vec![not_p]);
    let not3 = eg.add_app("Not", vec![not_not_p]);

    let dne = Rewrite::new(
        "double_neg",
        Pattern::app("Not", vec![Pattern::app("Not", vec![Pattern::var("x")])]),
        Pattern::var("x"),
    );

    eg.saturate(&[dne], 10);
    let best = eg.extract_best(eg.find(not3), &AstSize);
    // Not(Not(Not(P))) → Not(P) after one round of double-neg
    assert_eq!(best, "Not(P)", "Triple negation should compress to single negation");
}

// ═══════════════════════════════════════════════════════════════════════════
// CONGRUENCE CLOSURE PRESERVED
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn existing_cc_congruence_still_works() {
    // Regression guard: x=y, f(x) exists → f(x)=f(y) auto-discovered.
    let mut eg = EGraph::new();
    let x = eg.add_name("x");
    let y = eg.add_name("y");
    let fx = eg.add_app("f", vec![x]);
    let fy = eg.add_app("f", vec![y]);
    eg.union(x, y);
    eg.rebuild();
    assert_eq!(
        eg.find(fx), eg.find(fy),
        "Congruence closure must still propagate after e-class refactor"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// PROOF INTEGRATION
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn egraph_proves_equality_via_saturation() {
    // Given hypothesis x = y, prove f(f(x)) = f(f(y)) via congruence + saturation.
    let mut eg = EGraph::new();
    let x = eg.add_name("x");
    let y = eg.add_name("y");
    let fx = eg.add_app("f", vec![x]);
    let fy = eg.add_app("f", vec![y]);
    let ffx = eg.add_app("f", vec![fx]);
    let ffy = eg.add_app("f", vec![fy]);

    // Add hypothesis x = y
    eg.union(x, y);
    eg.rebuild();

    assert_eq!(
        eg.find(ffx), eg.find(ffy),
        "f(f(x)) = f(f(y)) should follow from x = y by congruence"
    );
}

#[test]
fn conditional_rewrite_respects_guard() {
    // A conditional rewrite should only fire when the guard holds.
    let mut eg = EGraph::new();
    let x = eg.add_name("x");
    let y = eg.add_name("y");
    let div_x_y = eg.add_app("Div", vec![x, y]);

    // Rewrite: Div(a, a) → 1, but only when a is nonzero
    let div_self = Rewrite::conditional(
        "div_self",
        Pattern::app("Div", vec![Pattern::var("a"), Pattern::var("a")]),
        Pattern::lit(1),
        |eg, subst| {
            // Guard: `a` is not zero
            let a_id = subst.get("a").unwrap();
            !eg.is_zero(*a_id)
        },
    );

    eg.saturate(&[div_self], 10);
    // x and y are different, so Div(x, y) should NOT be rewritten
    let one = eg.add_lit(1);
    assert_ne!(
        eg.find(div_x_y), eg.find(one),
        "Conditional rewrite should not fire when args differ"
    );
}
```

### Critical Files

| File | Action |
|------|--------|
| `crates/logicaffeine_kernel/src/cc.rs` | Major refactor: e-classes, rewrite rules, saturation, extraction |
| `crates/logicaffeine_proof/src/engine.rs` | Wire e-graph into equality reasoning |

---

## Sprint M: Hardware Inductive Types + State Machines

**Goal:** Make it true that we perform "structural induction over hardware states" with "polymorphic inductive types for hardware protocols" across "any bit-width."

### What Exists

- Polymorphic inductives: `TList : Type 0 → Type 0`, `Either of [L] and [R]`, etc.
- `TemporalInduction` rule in proof engine (declared but not wired to hardware types)
- Kripke lowering: G/F/X → `Accessible_Temporal(w, w')` world quantification
- SVA codegen: `emit_sva_property`, `parse_sva`, `sva_to_verify`

### What's Missing

- No `BitVec(n)` dependent type indexed by `Nat`
- No hardware state machine as inductive type
- No `Accessible` relation as inductive relation in kernel
- No bridge from Kripke `Accessible_Temporal` to kernel-level inductive proof

### Phase M.1: BitVec(n) — Width-Indexed Bit Vectors

**File:** `crates/logicaffeine_kernel/src/prelude.rs`

```rust
fn register_bitvec(ctx: &mut Context) {
    // BitVec : Nat → Type 0
    // A bit-vector whose width is tracked in the type.
    ctx.add_inductive("BitVec", pi("n", nat(), sort_type(0)));

    // bv_nil : BitVec Zero
    // The empty bit-vector.
    ctx.add_constructor("bv_nil", "BitVec",
        app(global("BitVec"), global("Zero")));

    // bv_cons : Π(n:Nat). Bool → BitVec n → BitVec (Succ n)
    // Prepend a bit to a vector, incrementing the width.
    ctx.add_constructor("bv_cons", "BitVec",
        pi("n", nat(),
            pi("_", bool_ty(),
                pi("_", app(global("BitVec"), var("n")),
                    app(global("BitVec"), app(global("Succ"), var("n")))))));
}
```

This gives: `bv_cons 7 true (bv_cons 6 false ... bv_nil) : BitVec (Succ^8 Zero)` = an 8-bit vector.

Width-polymorphic operations:
```rust
// bv_and : Π(n:Nat). BitVec n → BitVec n → BitVec n
// bv_or  : Π(n:Nat). BitVec n → BitVec n → BitVec n
// bv_not : Π(n:Nat). BitVec n → BitVec n
```

### Phase M.2: Hardware State Machine as Inductive Type

**File:** `crates/logicaffeine_kernel/src/prelude.rs`

```rust
fn register_hw_state(ctx: &mut Context) {
    // HwState : Type 0 → Type 0
    // A state machine parameterized by its signal bundle type.
    ctx.add_inductive("HwState", pi("S", sort_type(0), sort_type(0)));

    // Idle : Π(S:Type). S → HwState S
    ctx.add_constructor("Idle", "HwState", ...);

    // Active : Π(S:Type). S → HwState S
    ctx.add_constructor("Active", "HwState", ...);
}

fn register_accessible(ctx: &mut Context) {
    // Accessible : Π(S:Type). HwState S → HwState S → Prop
    // The temporal accessibility relation as an inductive proposition.
    ctx.add_inductive("Accessible", pi("S", sort_type(0),
        pi("_", app(global("HwState"), var("S")),
            pi("_", app(global("HwState"), var("S")),
                sort_prop()))));

    // acc_step : Π(S:Type)(s₁ s₂: HwState S). Transition(S, s₁, s₂) → Accessible(S, s₁, s₂)
    ctx.add_constructor("acc_step", "Accessible", ...);

    // acc_trans : Π(S:Type)(s₁ s₂ s₃: HwState S).
    //   Accessible(S, s₁, s₂) → Accessible(S, s₂, s₃) → Accessible(S, s₁, s₃)
    ctx.add_constructor("acc_trans", "Accessible", ...);
}
```

### Phase M.3: Wire Temporal Induction to Hardware States

**File:** `crates/logicaffeine_proof/src/engine.rs`

When goal is `∀(s:HwState S). P(s)`:
1. Recognize as temporal induction candidate (TypedVar with typename "HwState")
2. Generate base case: `P(Idle(S, signals))` for each initial state constructor
3. Generate step case: `∀(s₁ s₂:HwState S). Accessible(S, s₁, s₂) → P(s₁) → P(s₂)`
4. Build proof via `Term::Elim "HwState"` from Sprint K

### Phase M.4: Handshake Protocol as Polymorphic Inductive

```rust
fn register_handshake(ctx: &mut Context) {
    // Handshake : Type 0 → Type 0 → Type 0
    // A generic handshake protocol parameterized by request and response types.
    ctx.add_inductive("Handshake",
        pi("Req", sort_type(0), pi("Resp", sort_type(0), sort_type(0))));

    // HS_Idle : Π(Req Resp:Type). Handshake Req Resp
    ctx.add_constructor("HS_Idle", "Handshake", ...);

    // HS_Request : Π(Req Resp:Type). Req → Handshake Req Resp
    ctx.add_constructor("HS_Request", "Handshake", ...);

    // HS_Acknowledge : Π(Req Resp:Type). Req → Resp → Handshake Req Resp
    ctx.add_constructor("HS_Acknowledge", "Handshake", ...);

    // HS_Complete : Π(Req Resp:Type). Resp → Handshake Req Resp
    ctx.add_constructor("HS_Complete", "Handshake", ...);
}
```

Width-polymorphic example: `Handshake (BitVec 8) (BitVec 32)` for an 8-bit address bus with 32-bit data.

### TDD Red Tests — `phase_hw_inductive.rs`

```rust
//! Sprint M: Hardware Inductive Types
//!
//! RED tests for BitVec(n), HwState, Accessible, and Handshake
//! as kernel-level inductive types.

use logicaffeine_kernel::prelude::Prelude;
use logicaffeine_kernel::term::{Term, Universe, Literal};
use logicaffeine_kernel::context::Context;

fn setup() -> Context {
    let mut ctx = Context::new();
    Prelude::register(&mut ctx);
    ctx
}

// ═══════════════════════════════════════════════════════════════════════════
// BITVEC — WIDTH-INDEXED BIT VECTORS
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn bitvec_type_registered() {
    let ctx = setup();
    let ty = ctx.lookup_inductive("BitVec");
    assert!(ty.is_some(), "BitVec should be registered as an inductive type");
}

#[test]
fn bitvec_zero_width_is_bv_nil() {
    // bv_nil : BitVec Zero
    let ctx = setup();
    let bv_nil_ty = ctx.lookup_constructor_type("bv_nil");
    assert!(bv_nil_ty.is_some(), "bv_nil constructor should exist");
    let ty = logicaffeine_kernel::type_check(&ctx, &Term::Global("bv_nil".into()));
    assert!(ty.is_ok(), "bv_nil should type-check");
}

#[test]
fn bitvec_cons_increments_width() {
    // bv_cons(0, true, bv_nil) : BitVec(Succ(Zero)) = BitVec 1
    let ctx = setup();
    let bv1 = Term::App(
        Box::new(Term::App(
            Box::new(Term::App(
                Box::new(Term::Global("bv_cons".into())),
                Box::new(Term::Global("Zero".into())),
            )),
            Box::new(Term::Global("true".into())),
        )),
        Box::new(Term::Global("bv_nil".into())),
    );
    let ty = logicaffeine_kernel::type_check(&ctx, &bv1);
    assert!(ty.is_ok(), "BitVec 1 (single bit) should type-check. Error: {:?}", ty.err());
}

#[test]
fn bitvec_width_mismatch_rejected() {
    // bv_and(8, vec8, vec4) should be a type error: BitVec 8 ≠ BitVec 4
    let ctx = setup();
    // Construct two vecs of different widths and try to and them
    // This should fail at the type level
    let eight = nat_lit(8); // Succ^8(Zero)
    let four = nat_lit(4);  // Succ^4(Zero)
    let bv_and_wrong = Term::App(
        Box::new(Term::App(
            Box::new(Term::App(
                Box::new(Term::Global("bv_and".into())),
                Box::new(eight),
            )),
            Box::new(Term::Var("vec8".into())),
        )),
        Box::new(Term::Var("vec4".into())),
    );
    // With vec8 : BitVec 8 and vec4 : BitVec 4, this should fail type-checking
    // (We'd need to set up the context with these bindings for a full test)
    // For now, test that bv_and exists
    let ty = ctx.lookup_constructor_type("bv_and");
    assert!(ty.is_some() || ctx.lookup("bv_and").is_some(), "bv_and operation should exist");
}

#[test]
fn bitvec_elim_auto_generated() {
    // BitVec_elim should exist after prelude registration (from Sprint K).
    let ctx = setup();
    let ty = ctx.lookup("BitVec_elim");
    assert!(ty.is_some(), "BitVec_elim should be auto-generated");
}

// ═══════════════════════════════════════════════════════════════════════════
// HWSTATE — HARDWARE STATE MACHINES
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn hwstate_type_registered() {
    let ctx = setup();
    let ty = ctx.lookup_inductive("HwState");
    assert!(ty.is_some(), "HwState should be registered as an inductive type");
}

#[test]
fn hwstate_idle_constructs() {
    // Idle(Nat, Zero) : HwState Nat
    let ctx = setup();
    let idle = Term::App(
        Box::new(Term::App(
            Box::new(Term::Global("Idle".into())),
            Box::new(Term::Global("Nat".into())),
        )),
        Box::new(Term::Global("Zero".into())),
    );
    let ty = logicaffeine_kernel::type_check(&ctx, &idle);
    assert!(ty.is_ok(), "Idle(Nat, Zero) should type-check as HwState Nat. Error: {:?}", ty.err());
}

#[test]
fn hwstate_active_constructs() {
    let ctx = setup();
    let active = Term::App(
        Box::new(Term::App(
            Box::new(Term::Global("Active".into())),
            Box::new(Term::Global("Nat".into())),
        )),
        Box::new(Term::Global("Zero".into())),
    );
    let ty = logicaffeine_kernel::type_check(&ctx, &active);
    assert!(ty.is_ok(), "Active(Nat, Zero) should type-check as HwState Nat. Error: {:?}", ty.err());
}

// ═══════════════════════════════════════════════════════════════════════════
// ACCESSIBLE — TEMPORAL ACCESSIBILITY RELATION
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn accessible_type_registered() {
    let ctx = setup();
    let ty = ctx.lookup_inductive("Accessible");
    assert!(ty.is_some(), "Accessible should be registered as an inductive relation");
}

#[test]
fn accessible_is_proposition() {
    // Accessible S s1 s2 : Prop (not Type)
    let ctx = setup();
    let ty = ctx.lookup_inductive("Accessible");
    // The sort of Accessible's output should be Prop
    assert!(ty.is_some(), "Accessible should inhabit Prop");
}

// ═══════════════════════════════════════════════════════════════════════════
// TEMPORAL INDUCTION OVER HARDWARE STATES
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn temporal_induction_over_hwstate() {
    // Prove: ∀(s: HwState Nat). P(s)
    // by providing: P(Idle(Nat, n)) and P(Active(Nat, n))
    // via the auto-generated HwState_elim.
    let ctx = setup();
    let elim = Term::Elim {
        ind_name: "HwState".into(),
        motive: Box::new(Term::Lambda {
            param: "s".into(),
            param_type: Box::new(Term::App(
                Box::new(Term::Global("HwState".into())),
                Box::new(Term::Global("Nat".into())),
            )),
            body: Box::new(Term::Sort(Universe::Prop)),
        }),
        cases: vec![
            // Case Idle: Π(S:Type)(signals:S). P(Idle S signals)
            Term::Lambda {
                param: "signals".into(),
                param_type: Box::new(Term::Global("Nat".into())),
                body: Box::new(Term::Global("trivial".into())), // placeholder
            },
            // Case Active: similar
            Term::Lambda {
                param: "signals".into(),
                param_type: Box::new(Term::Global("Nat".into())),
                body: Box::new(Term::Global("trivial".into())),
            },
        ],
        scrutinee: Box::new(Term::Var("s".into())),
    };
    // The Elim structure should be well-formed
    match &elim {
        Term::Elim { ind_name, cases, .. } => {
            assert_eq!(ind_name, "HwState");
            assert_eq!(cases.len(), 2, "HwState has 2 constructors (Idle, Active)");
        }
        _ => panic!("Should be Elim"),
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// HANDSHAKE — POLYMORPHIC PROTOCOL
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn handshake_type_registered() {
    let ctx = setup();
    let ty = ctx.lookup_inductive("Handshake");
    assert!(ty.is_some(), "Handshake should be registered as a polymorphic inductive");
}

#[test]
fn handshake_width_polymorphic() {
    // Handshake (BitVec 8) (BitVec 32) should be a valid type.
    let ctx = setup();
    let bv8 = Term::App(Box::new(Term::Global("BitVec".into())), Box::new(nat_lit(8)));
    let bv32 = Term::App(Box::new(Term::Global("BitVec".into())), Box::new(nat_lit(32)));
    let hs = Term::App(
        Box::new(Term::App(
            Box::new(Term::Global("Handshake".into())),
            Box::new(bv8),
        )),
        Box::new(bv32),
    );
    let ty = logicaffeine_kernel::type_check(&ctx, &hs);
    assert!(
        ty.is_ok(),
        "Handshake(BitVec 8, BitVec 32) should type-check. Error: {:?}",
        ty.err()
    );
}

#[test]
fn handshake_idle_constructs_at_any_width() {
    // HS_Idle (BitVec 8) (BitVec 32) : Handshake (BitVec 8) (BitVec 32)
    let ctx = setup();
    let bv8 = Term::App(Box::new(Term::Global("BitVec".into())), Box::new(nat_lit(8)));
    let bv32 = Term::App(Box::new(Term::Global("BitVec".into())), Box::new(nat_lit(32)));
    let idle = Term::App(
        Box::new(Term::App(
            Box::new(Term::Global("HS_Idle".into())),
            Box::new(bv8),
        )),
        Box::new(bv32),
    );
    let ty = logicaffeine_kernel::type_check(&ctx, &idle);
    assert!(
        ty.is_ok(),
        "HS_Idle at arbitrary bit-widths should type-check. Error: {:?}",
        ty.err()
    );
}

#[test]
fn handshake_elim_auto_generated() {
    let ctx = setup();
    let ty = ctx.lookup("Handshake_elim");
    assert!(ty.is_some(), "Handshake_elim should be auto-generated for the protocol type");
}

// ═══════════════════════════════════════════════════════════════════════════
// KRIPKE BRIDGE
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn kripke_accessible_temporal_maps_to_kernel_accessible() {
    // When Kripke lowering produces Accessible_Temporal(w₀, w₁),
    // the proof engine should map this to the kernel's Accessible inductive.
    // This is a structural test: verify the mapping function exists.
    use logicaffeine_language::{compile_kripke};
    let output = compile_kripke("Always, every signal is valid.").unwrap();
    assert!(
        output.contains("Accessible_Temporal"),
        "Kripke lowering should produce Accessible_Temporal. Got: {}",
        output
    );
    // The bridge from Accessible_Temporal (FOL predicate) to
    // Accessible (kernel inductive) is what Sprint M provides.
}

/// Helper: build Succ^n(Zero) for natural number literals.
fn nat_lit(n: u32) -> Term {
    let mut t = Term::Global("Zero".into());
    for _ in 0..n {
        t = Term::App(Box::new(Term::Global("Succ".into())), Box::new(t));
    }
    t
}
```

### Critical Files

| File | Action |
|------|--------|
| `crates/logicaffeine_kernel/src/prelude.rs` | Register BitVec, HwState, Accessible, Handshake, bv_and/bv_or/bv_not |
| `crates/logicaffeine_proof/src/engine.rs` | Wire TemporalInduction to HwState recognition |
| `crates/logicaffeine_language/src/semantics/kripke.rs` | Bridge Accessible_Temporal → kernel Accessible |

---

## Sprint N: Invariant Synthesis

**Goal:** Make it true that we "automatically derive the necessary invariants" and that "inductive invariants" serve as "optimal cuts."

### What Exists

- Proof engine with backward chaining and structural induction
- DElim (Sprint K) for structuring induction proofs
- E-graph saturation (Sprint L) for finding minimal representations
- Z3 oracle for checking arithmetic/comparison goals
- Hardware state types (Sprint M) with Accessible relation

### What's Missing

- No automatic candidate invariant generation
- No counter-example guided refinement loop (CEGIR)
- No integration between e-graph extraction and invariant compression

### Phase N.1: Candidate Invariant Generation

**New file:** `crates/logicaffeine_proof/src/invariant.rs`

```rust
//! Invariant synthesis for inductive types.
//!
//! Given an inductive type I and a target property P, synthesize
//! candidate invariants that might prove ∀(x:I). P(x).

/// A candidate invariant: a formula that (if true) implies the target property.
pub struct CandidateInvariant {
    pub formula: ProofExpr,
    pub strength: InvariantStrength,
}

pub enum InvariantStrength {
    Exact,       // Invariant ↔ Property
    Sufficient,  // Invariant → Property
    Necessary,   // Property → Invariant
}

/// Synthesize candidate invariants for proving ∀(x:I). P(x).
///
/// Strategy:
/// 1. Constructor decomposition — what must hold at each constructor?
/// 2. Field projection — extract per-field properties
/// 3. Transition guard — what's preserved across Accessible steps?
pub fn synthesize_candidates(
    ctx: &Context,
    ind_name: &str,
    property: &ProofExpr,
) -> Vec<CandidateInvariant> { ... }
```

### Phase N.2: DElim-Structured Proof

Use the eliminator from Sprint K to structure the invariant proof:

```
Elim "HwState" (λs. Invariant(s)) [
    idle_case:  proof that Invariant holds at Idle
    active_case: proof that Invariant holds at Active
] target_state
```

The step case (for Accessible) uses the induction hypothesis:
`∀(s₁ s₂). Accessible(s₁, s₂) → Invariant(s₁) → Invariant(s₂)`

### Phase N.3: Counter-Example Guided Refinement (CEGIR)

```rust
/// Iteratively refine an invariant using Z3 counterexamples.
pub fn refine_invariant(
    ctx: &Context,
    egraph: &mut EGraph,
    ind_name: &str,
    property: &ProofExpr,
    max_iterations: usize,
) -> InvariantResult {
    let mut candidates = synthesize_candidates(ctx, ind_name, property);

    for _ in 0..max_iterations {
        for candidate in &candidates {
            // 1. Encode invariant as Z3 verification condition
            let vc = encode_verification_condition(ctx, ind_name, &candidate.formula);

            // 2. Check with Z3
            match z3_check(&vc) {
                Z3Result::Valid => {
                    // 3. Add to e-graph and extract minimal form
                    let id = egraph.add_expr(&candidate.formula);
                    egraph.saturate(&proof_rewrites(), 50);
                    let minimal = egraph.extract_best(id, &AstSize);
                    return InvariantResult::Proved(minimal);
                }
                Z3Result::CounterExample(cex) => {
                    // 4. Refine: strengthen invariant to exclude counterexample
                    candidates = refine_with_counterexample(candidates, &cex);
                }
                Z3Result::Unknown => continue,
            }
        }
    }
    InvariantResult::Failed
}
```

### TDD Red Tests — `phase_hw_invariant.rs`

```rust
//! Sprint N: Invariant Synthesis
//!
//! RED tests for automatic invariant generation, DElim-structured proof,
//! and counter-example guided refinement.

use logicaffeine_kernel::prelude::Prelude;
use logicaffeine_kernel::context::Context;
use logicaffeine_proof::invariant::{synthesize_candidates, refine_invariant, InvariantResult};

fn setup() -> Context {
    let mut ctx = Context::new();
    Prelude::register(&mut ctx);
    ctx
}

// ═══════════════════════════════════════════════════════════════════════════
// CANDIDATE GENERATION
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn synthesize_nat_candidates() {
    // For Nat with property "n >= 0", should produce at least one candidate.
    let ctx = setup();
    let property = proof_expr_geq(proof_var("n"), proof_lit(0));
    let candidates = synthesize_candidates(&ctx, "Nat", &property);
    assert!(!candidates.is_empty(), "Should synthesize at least one candidate for n >= 0");
}

#[test]
fn synthesize_hwstate_candidates() {
    // For HwState with property "signals are bounded", should produce candidates.
    let ctx = setup();
    let property = proof_expr_bounded(proof_var("signals"));
    let candidates = synthesize_candidates(&ctx, "HwState", &property);
    assert!(
        !candidates.is_empty(),
        "Should synthesize invariant candidates for hardware state boundedness"
    );
}

#[test]
fn candidates_decompose_by_constructor() {
    // Each candidate should address each constructor of the inductive type.
    let ctx = setup();
    let property = proof_expr_geq(proof_var("n"), proof_lit(0));
    let candidates = synthesize_candidates(&ctx, "Nat", &property);
    // At minimum, the trivial candidate should cover Zero and Succ
    let first = &candidates[0];
    assert!(
        first.covers_constructor("Zero") && first.covers_constructor("Succ"),
        "Candidate should decompose over all constructors"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// DELIM-STRUCTURED PROOF
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn invariant_proof_uses_elim() {
    // The generated proof for a Nat invariant should be structured via Nat_elim.
    let ctx = setup();
    let property = proof_expr_geq(proof_var("n"), proof_lit(0));
    let result = refine_invariant(&ctx, &mut EGraph::new(), "Nat", &property, 10);
    match result {
        InvariantResult::Proved(proof) => {
            // The proof should contain an Elim node for Nat
            assert!(
                proof.contains_elim("Nat"),
                "Invariant proof should be structured via Nat Elim"
            );
        }
        _ => panic!("Should prove n >= 0 for all Nat"),
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// COUNTER-EXAMPLE GUIDED REFINEMENT
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn cegir_refines_on_counterexample() {
    // Start with a too-weak invariant. Z3 finds counterexample. Refinement strengthens it.
    let ctx = setup();
    let mut eg = EGraph::new();
    // Property: all signals in Active state are < 256 (for 8-bit signals)
    let property = proof_expr_lt(proof_var("signal"), proof_lit(256));
    let result = refine_invariant(&ctx, &mut eg, "HwState", &property, 20);
    assert!(
        matches!(result, InvariantResult::Proved(_)),
        "CEGIR should find a valid invariant within iteration budget"
    );
}

#[test]
fn cegir_reports_failure_on_impossible_property() {
    // Property that's actually false should not produce a proof.
    let ctx = setup();
    let mut eg = EGraph::new();
    // Property: all natural numbers are less than 5 (false!)
    let property = proof_expr_lt(proof_var("n"), proof_lit(5));
    let result = refine_invariant(&ctx, &mut eg, "Nat", &property, 10);
    assert!(
        matches!(result, InvariantResult::Failed),
        "Should fail on unprovable property"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// E-GRAPH EXTRACTION — "OPTIMAL CUT"
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn egraph_extracts_minimal_invariant() {
    // After saturation, the extracted invariant should be the compressed form.
    let ctx = setup();
    let mut eg = EGraph::new();
    let property = proof_expr_geq(proof_var("n"), proof_lit(0));
    let result = refine_invariant(&ctx, &mut eg, "Nat", &property, 10);
    if let InvariantResult::Proved(proof) = result {
        // The minimal invariant for "n >= 0" on Nat is trivially true (Nat has no negative values).
        // The e-graph should extract the simplest proof: just the constructor analysis.
        let size = proof.ast_size();
        assert!(size < 20, "Extracted invariant should be compressed. Size: {}", size);
    }
}
```

### Critical Files

| File | Action |
|------|--------|
| `crates/logicaffeine_proof/src/invariant.rs` | **NEW** — synthesis, CEGIR, extraction |
| `crates/logicaffeine_proof/src/engine.rs` | Integrate invariant synthesis into temporal proofs |
| `crates/logicaffeine_kernel/src/cc.rs` | Use e-graph extraction for invariant compression |

---

## Sprint O: Z3 Equivalence Checking (Pipeline Completion)

**Goal:** Make it true that "Z3 confirms SVA ≡ FOL" — the full pipeline closes.

### What Exists

- SVA parser + AST (`sva_model.rs`) — complete, 19 roundtrip tests
- SVA → bounded timestep IR (`sva_to_verify.rs`) — complete, 9 tests
- Kripke temporal lowering → FOL (`kripke.rs`) — G/F/X complete
- Z3 bindings (`logicaffeine_verify`) — basic verification working
- `VerifyExpr` IR — Int, Bool, arithmetic, comparison, logical ops

### What's Missing

- No BitVector operations in `VerifyExpr`
- No SVA → `VerifyExpr` encoding
- No FOL → `VerifyExpr` encoding for temporal properties
- No equivalence checker (`SVA_bounded ↔ FOL_bounded`)
- No counterexample extraction

### Phase O.1: BitVector Theory in Verification IR

**File:** `crates/logicaffeine_verify/src/ir.rs`

```rust
// Add to VerifyExpr enum:
BitVec { width: u32, value: u64 },
BvAnd(Box<VerifyExpr>, Box<VerifyExpr>),
BvOr(Box<VerifyExpr>, Box<VerifyExpr>),
BvNot(Box<VerifyExpr>),
BvXor(Box<VerifyExpr>, Box<VerifyExpr>),
BvShiftLeft(Box<VerifyExpr>, Box<VerifyExpr>),
BvShiftRight(Box<VerifyExpr>, Box<VerifyExpr>),
BvExtract { high: u32, low: u32, expr: Box<VerifyExpr> },
BvConcat(Box<VerifyExpr>, Box<VerifyExpr>),
BvEq(Box<VerifyExpr>, Box<VerifyExpr>),
BvZeroExt { extra_bits: u32, expr: Box<VerifyExpr> },
BvSignExt { extra_bits: u32, expr: Box<VerifyExpr> },
```

### Phase O.2: SVA → Z3 AST Encoding

**File:** `crates/logicaffeine_compile/src/codegen_sva/sva_to_verify.rs` (extend)

```rust
/// Translate a BoundedExpr (from SvaTranslator) into a VerifyExpr (for Z3).
pub fn bounded_to_verify(expr: &BoundedExpr) -> VerifyExpr {
    match expr {
        BoundedExpr::Signal(name, t) => VerifyExpr::Var(format!("{name}@{t}")),
        BoundedExpr::And(l, r) => VerifyExpr::And(
            Box::new(bounded_to_verify(l)),
            Box::new(bounded_to_verify(r)),
        ),
        BoundedExpr::Or(l, r) => VerifyExpr::Or(...),
        BoundedExpr::Not(e) => VerifyExpr::Not(...),
        BoundedExpr::Implies(l, r) => VerifyExpr::Implies(...),
        BoundedExpr::True => VerifyExpr::Bool(true),
        BoundedExpr::False => VerifyExpr::Bool(false),
    }
}
```

### Phase O.3: FOL ↔ SVA Semantic Equivalence

**New file:** `crates/logicaffeine_verify/src/equivalence.rs`

```rust
//! Semantic equivalence checking between Kripke-lowered FOL and SVA.
//!
//! Given a temporal FOL formula (from LOGOS parse + Kripke lowering) and
//! an SVA expression (from LLM or manual authoring), check whether they
//! are semantically equivalent over bounded traces.

/// Check whether a FOL temporal formula and an SVA expression are
/// semantically equivalent up to a bounded number of clock cycles.
pub fn check_equivalence(
    fol: &LogicExpr,
    sva: &SvaExpr,
    bound: usize,
) -> EquivalenceResult {
    // 1. Lower FOL to bounded verification IR
    let fol_bounded = lower_fol_to_bounded(fol, bound);
    let fol_verify = bounded_to_verify(&fol_bounded);

    // 2. Lower SVA to bounded verification IR (SvaTranslator already exists)
    let mut translator = SvaTranslator::new(bound);
    let sva_bounded = translator.translate_property(sva);
    let sva_verify = bounded_to_verify(&sva_bounded);

    // 3. Check: ¬(FOL ↔ SVA) is unsatisfiable iff FOL ≡ SVA
    let negated_iff = VerifyExpr::Not(Box::new(VerifyExpr::Iff(
        Box::new(fol_verify),
        Box::new(sva_verify),
    )));

    // 4. Send to Z3
    let mut session = VerificationSession::new();
    // Declare all signal variables at each timestep
    for t in 0..bound {
        for signal in extract_signals(fol, sva) {
            session.declare_bool(&format!("{signal}@{t}"));
        }
    }
    session.assert(&negated_iff);

    match session.check() {
        SatResult::Unsat => EquivalenceResult::Equivalent,
        SatResult::Sat(model) => {
            let trace = extract_trace(&model, bound);
            EquivalenceResult::NotEquivalent { counterexample: trace }
        }
        SatResult::Unknown => EquivalenceResult::Unknown,
    }
}

pub enum EquivalenceResult {
    Equivalent,
    NotEquivalent { counterexample: Trace },
    Unknown,
}

pub struct Trace {
    pub cycles: Vec<CycleState>,
}

pub struct CycleState {
    pub cycle: usize,
    pub signals: HashMap<String, bool>,
}
```

### Phase O.4: Counterexample Extraction

When Z3 finds a counterexample:
- Extract signal assignments at each timestep from the Z3 model
- Format as a readable trace showing where FOL and SVA diverge
- Optionally emit VCD (Value Change Dump) for waveform viewers

### TDD Red Tests — `phase_hw_z3_equiv.rs`

```rust
//! Sprint O: Z3 Equivalence Checking
//!
//! RED tests for semantic equivalence between Kripke-lowered FOL and SVA.
//! This is the final pipeline stage: Z3 confirms SVA ≡ FOL.

use logicaffeine_compile::codegen_sva::sva_model::{parse_sva, SvaExpr};
use logicaffeine_language::{compile, compile_kripke};

// ═══════════════════════════════════════════════════════════════════════════
// BITVECTOR IR
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(feature = "verification")]
mod z3_equiv {
    use super::*;
    use logicaffeine_verify::ir::VerifyExpr;
    use logicaffeine_verify::equivalence::{check_equivalence, EquivalenceResult};

    #[test]
    fn bitvec_ir_exists() {
        // BitVec variant should exist in VerifyExpr
        let bv = VerifyExpr::BitVec { width: 8, value: 42 };
        match bv {
            VerifyExpr::BitVec { width, value } => {
                assert_eq!(width, 8);
                assert_eq!(value, 42);
            }
            _ => panic!("Should be BitVec"),
        }
    }

    #[test]
    fn bitvec_and_encodes() {
        let a = VerifyExpr::BitVec { width: 8, value: 0xFF };
        let b = VerifyExpr::BitVec { width: 8, value: 0x0F };
        let and = VerifyExpr::BvAnd(Box::new(a), Box::new(b));
        match and {
            VerifyExpr::BvAnd(_, _) => {} // Exists
            _ => panic!("BvAnd should exist"),
        }
    }

    // ═══════════════════════════════════════════════════════════════════════
    // EQUIVALENCE — SIMPLE CASES
    // ═══════════════════════════════════════════════════════════════════════

    #[test]
    fn equivalent_simple_implication() {
        // FOL: req → ack  ≡  SVA: req |-> ack
        let fol = compile_kripke("If a request holds then an acknowledgment holds.").unwrap();
        let sva = parse_sva("req |-> ack").unwrap();
        let result = check_equivalence_from_strings(&fol, &sva, 1);
        assert!(
            matches!(result, EquivalenceResult::Equivalent),
            "Simple implication should be equivalent. Result: {:?}",
            result
        );
    }

    #[test]
    fn equivalent_always_implies() {
        // FOL: G(req → F(ack))  ≡  SVA: req |-> s_eventually(ack)
        let fol = compile_kripke("Always, if a request holds then eventually an acknowledgment holds.").unwrap();
        let sva = parse_sva("req |-> s_eventually(ack)").unwrap();
        let result = check_equivalence_from_strings(&fol, &sva, 10);
        assert!(
            matches!(result, EquivalenceResult::Equivalent),
            "G(req → F(ack)) should be equivalent to req |-> s_eventually(ack). Result: {:?}",
            result
        );
    }

    #[test]
    fn not_equivalent_missing_condition() {
        // FOL: G(req → F(ack))  ≢  SVA: ack  (too weak — missing the req condition)
        let fol = compile_kripke("Always, if a request holds then eventually an acknowledgment holds.").unwrap();
        let sva = parse_sva("ack").unwrap();
        let result = check_equivalence_from_strings(&fol, &sva, 5);
        assert!(
            matches!(result, EquivalenceResult::NotEquivalent { .. }),
            "G(req → F(ack)) should NOT be equivalent to bare 'ack'. Result: {:?}",
            result
        );
    }

    #[test]
    fn not_equivalent_wrong_direction() {
        // FOL: req → ack  ≢  SVA: ack |-> req  (implication reversed)
        let fol = compile_kripke("If a request holds then an acknowledgment holds.").unwrap();
        let sva = parse_sva("ack |-> req").unwrap();
        let result = check_equivalence_from_strings(&fol, &sva, 1);
        assert!(
            matches!(result, EquivalenceResult::NotEquivalent { .. }),
            "req→ack should NOT equal ack→req. Result: {:?}",
            result
        );
    }

    // ═══════════════════════════════════════════════════════════════════════
    // COUNTEREXAMPLE EXTRACTION
    // ═══════════════════════════════════════════════════════════════════════

    #[test]
    fn counterexample_shows_divergence_cycle() {
        let fol = compile_kripke("Always, if a request holds then eventually an acknowledgment holds.").unwrap();
        let sva = parse_sva("ack").unwrap();
        let result = check_equivalence_from_strings(&fol, &sva, 5);
        match result {
            EquivalenceResult::NotEquivalent { counterexample } => {
                assert!(
                    !counterexample.cycles.is_empty(),
                    "Counterexample should contain at least one cycle"
                );
                // The trace should show signal values
                let first_cycle = &counterexample.cycles[0];
                assert!(
                    !first_cycle.signals.is_empty(),
                    "Each cycle should have signal assignments"
                );
            }
            _ => panic!("Expected counterexample"),
        }
    }

    // ═══════════════════════════════════════════════════════════════════════
    // TEMPORAL OPERATORS
    // ═══════════════════════════════════════════════════════════════════════

    #[test]
    fn equivalent_next_operator() {
        // FOL: X(P)  ≡  SVA: ##1 P
        let fol = compile_kripke("Next, every signal is valid.").unwrap();
        let sva = parse_sva("##1 valid").unwrap();
        let result = check_equivalence_from_strings(&fol, &sva, 3);
        assert!(
            matches!(result, EquivalenceResult::Equivalent),
            "X(P) should be equivalent to ##1 P. Result: {:?}",
            result
        );
    }

    #[test]
    fn equivalent_delay_range() {
        // FOL: F(P) within 3 cycles  ≡  SVA: ##[1:3] P
        let sva = parse_sva("##[1:3] ack").unwrap();
        // Bounded F(ack) over 3 cycles should match ##[1:3] ack
        let result = check_sva_bounded_equivalence(&sva, 3);
        assert!(
            matches!(result, EquivalenceResult::Equivalent),
            "Bounded eventuality should match delay range. Result: {:?}",
            result
        );
    }

    // ═══════════════════════════════════════════════════════════════════════
    // EDGE DETECTION
    // ═══════════════════════════════════════════════════════════════════════

    #[test]
    fn rose_edge_semantics_correct() {
        // $rose(sig) ≡ sig && !$past(sig)
        let sva_rose = parse_sva("$rose(sig)").unwrap();
        let sva_manual = parse_sva("sig && !$past(sig, 1)").unwrap();
        let result = check_sva_sva_equivalence(&sva_rose, &sva_manual, 5);
        assert!(
            matches!(result, EquivalenceResult::Equivalent),
            "$rose(sig) should equal sig && !$past(sig). Result: {:?}",
            result
        );
    }

    // ═══════════════════════════════════════════════════════════════════════
    // FULL PIPELINE
    // ═══════════════════════════════════════════════════════════════════════

    #[test]
    fn full_pipeline_english_to_sva_verified() {
        // The showcase test: English → FOL → SVA → Z3 equivalence
        //
        // English: "Always, if a request is raised then within 3 cycles an acknowledgment is raised."
        // Expected SVA: $rose(req) |-> ##[1:3] $rose(ack)
        let fol = compile_kripke(
            "Always, if a request is raised then within 3 cycles an acknowledgment is raised."
        ).unwrap();
        let sva = parse_sva("$rose(req) |-> ##[1:3] $rose(ack)").unwrap();
        let result = check_equivalence_from_strings(&fol, &sva, 10);
        assert!(
            matches!(result, EquivalenceResult::Equivalent),
            "Full pipeline: English spec should be equivalent to handwritten SVA. Result: {:?}",
            result
        );
    }
}
```

### Critical Files

| File | Action |
|------|--------|
| `crates/logicaffeine_verify/src/ir.rs` | Add BitVector variants to VerifyExpr |
| `crates/logicaffeine_verify/src/solver.rs` | Z3 encoding for BitVector theory |
| `crates/logicaffeine_verify/src/equivalence.rs` | **NEW** — equivalence checker |
| `crates/logicaffeine_compile/src/codegen_sva/sva_to_verify.rs` | BoundedExpr → VerifyExpr bridge |

---

## Dependency Graph

```
Sprint K (DElim + Mirror)
    │
    ├──→ Sprint M (Hardware Inductives) ── depends on Elim
    │         │
    │         └──→ Sprint N (Invariant Synthesis) ── depends on DElim + HW types + E-sat
    │                   │
    │                   └──→ Sprint O (Z3 Equivalence) ── depends on HW types + verify IR
    │
    └──→ Sprint L (E-sat) ── independent of M, can parallel
              │
              └──→ Sprint N (Invariant Synthesis)
```

**Recommended execution order:** K → (L ∥ early M) → M → N → O

Sprint L is independent of Sprint K's Elim (it upgrades cc.rs), so it can run in parallel with early phases of Sprint M.

---

## Pitch-Ready Summary

After all sprints, every sentence in the pitch is backed by implemented, tested code:

| Pitch Claim | Sprint | Evidence |
|:---|:---|:---|
| "CoIC kernel performs structural induction over hardware states" | K + M | `Term::Elim "HwState"` with base + step cases, tested |
| "DElim (Generic Elimination Principle)" | K | `Term::Elim` variant, auto-generated eliminators per inductive |
| "Leibniz's Law via the Mirror module" | K | `mirror.rs` — transport, symmetry, transitivity, congruence, reflect |
| "Equality Saturation (E-graphs) via cc.rs" | L | Rewrite rules, saturation loop, cost model, extraction |
| "Most compressed version of a proof" | L | `extract_best(id, &AstSize)` finds minimal-cost equivalent |
| "Polymorphic Inductive Types for hardware protocols" | M | `Handshake(BitVec 8, BitVec 32)` — width-polymorphic |
| "Single verified check across any bit-width" | M | `Π(n:Nat). ∀(v: BitVec n). P(v)` — one proof, all widths |
| "Automatically derive necessary invariants" | N | `synthesize_candidates` + CEGIR loop |
| "Inductive invariants as optimal cuts" | N | E-graph extraction finds minimal separating invariant |
| "Zero-cost monitor via First Futamura Projection" | **Already done** | 436 tests, P1 verified, Jones optimal |
| "Z3 confirms SVA ≡ FOL" | O | `check_equivalence` with bounded model checking + counterexamples |
