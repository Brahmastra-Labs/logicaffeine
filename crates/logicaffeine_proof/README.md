# logicaffeine-proof

A backward-chaining proof engine over an owned, arena-independent IR (`ProofExpr` /
`ProofTerm`): it searches for derivations, certifies them into kernel terms, and offers
Socratic, leading-question hints when a proof gets stuck. It embodies the Curry-Howard
correspondence — propositions are types, proofs are programs, verification is type checking.

Part of the [Logicaffeine](../../NEW_README.md) workspace. Tier 2 — depends on
logicaffeine_base and logicaffeine_kernel. **Liskov invariant**: no dependency on the
language crate, so the engine is reusable across front-ends.

## Role in the workspace

This crate owns proof representation, *search*, and *certification* — the trust core that
both `logicaffeine_language` and `logicaffeine_compile` reach without a dependency cycle.
The `LogicExpr → ProofExpr` lowering lives in the **language** crate, not here, so the proof
engine stays pure and the Liskov boundary holds.

The single trust door: a proof is `verified` **iff** the chainer found a derivation, the
certifier turned it into a kernel `Term`, *and* the kernel type-checked that term against the
goal type. An externally built `DerivationTree` (e.g. from the grid solver) is re-checked the
same way, so untrusted search sits *outside* the trusted base — a wrong tree yields
`verified == false`, never a false claim. Trust tiers run fast → strong: untrusted CDCL/SMT
(`cnf::cdcl_entails`, `oracle`) → RUP-certified (`rup::entails_certified`, `sat::prove_unsat`)
→ kernel-certified (`verify`). See [proof-and-verification.md](../../new_docs/proof-and-verification.md).

## Public API

Re-exported at the crate root: `BackwardChainer`, `ProofError`, `suggest_hint`,
`SocraticHint`, `SuggestedTactic`, `Substitution`; plus `ProofTerm`, `ProofExpr`, `MatchArm`,
`InferenceRule`, `DerivationTree`, `ProofGoal` defined directly in `lib.rs`.

**Search** — `engine::BackwardChainer`:

```rust
let mut prover = BackwardChainer::new();
prover.set_max_depth(depth: usize);
prover.add_axiom(expr: ProofExpr);
prover.knowledge_base() -> &[ProofExpr];
prover.prove(goal: ProofExpr) -> ProofResult<DerivationTree>;
prover.prove_with_goal(goal: ProofGoal) -> ProofResult<DerivationTree>;
```

**The single door** — `verify`:

```rust
verify::prove_certify_check(premises: &[ProofExpr], goal: &ProofExpr) -> VerifiedProof;
verify::prove_certify_check_bounded(premises, goal, max_depth: usize) -> VerifiedProof;
verify::check_derivation(premises, goal, tree: DerivationTree) -> VerifiedProof;
verify::detect_conflict(premises: &[ProofExpr]) -> ConflictReport;
```

`VerifiedProof { derivation, proof_term, kernel_ctx, verified, verification_error }` carries
the re-checkable kernel term; `detect_conflict` returns a kernel proof of `False` plus the
indices of the clashing premises.

**IR** — `ProofTerm` (`Constant`, `Variable`, `Function`, `Group`, `BoundVarRef`) and
`ProofExpr` cover full FOL plus extensions: connectives, quantifiers, `Modal` /
`Counterfactual` / `Temporal` / `TemporalBinary`, `Lambda` / `App`, Neo-Davidsonian
`NeoEvent`, inductive `Ctor` / `Match` / `Fixpoint` / `TypedVar`, and unification `Hole`s.
`InferenceRule` names the move at each step — `ModusPonens` / `ModusTollens`, ∧/∨ intro &
elim, ∀/∃ intro & elim, `ModalAccess`, `StructuralInduction`, Leibniz `Rewrite`,
`ArithDecision`, `ReductioAdAbsurdum`, `CaseAnalysis`, `DisjunctionCases`, … A
`DerivationTree { conclusion, rule, premises, depth, substitution }` is the prover's result;
`ProofGoal { target, context }` is consumed.

**Hints** — `hints`:

```rust
hints::suggest_hint(goal: &ProofExpr, kb: &[ProofExpr], failed: &[SuggestedTactic]) -> SocraticHint;
```

`SocraticHint { text, suggested_tactic, priority }` proposes a `SuggestedTactic` rather than
giving the answer outright.

**SAT / model checking** (Z3-free, browser-ready) — `sat` and `bmc`:

```rust
sat::find_model(e: &ProofExpr) -> ModelOutcome;            // Sat(model) | Unsat | Unsupported
sat::prove_equivalence(a, b: &ProofExpr) -> EquivOutcome;  // Equivalent | Differ(cex) | Unsupported
sat::prove_unsat(e: &ProofExpr) -> UnsatOutcome;           // Refuted (RUP) | Sat(model) | Unsupported
bmc::find_counterexample(init, trans, property, max_k) -> BmcOutcome;
bmc::prove_invariant(init, trans, property, k) -> InductionOutcome;  // k-induction, unbounded
bmc::check_vacuity(antecedent: &ProofExpr) -> VacuityOutcome;
```

UNSAT verdicts from `sat` are independently RUP-certified (a refutation the trusted checker
cannot replay yields `Unsupported`, never a false `Refuted`); `bmc` reduces BMC, k-induction,
and vacuity to `prove_unsat`.

Other public modules: `certifier` (Curry-Howard `certify`: `DerivationTree` → kernel `Term`),
`unify` (Robinson unification + occurs check, capture-avoiding `beta_reduce`, Miller
patterns), `grounding` (expand bounded quantifiers over a finite domain), `grid_solver`
(certified logic-grid solver: watched-literal unit propagation + DPLL), `cdcl` (CDCL(T) core:
2-watched lits, 1-UIP, VSIDS, Luby, `Theory` trait, DRAT/LRAT log), `cnf` (Tseitin
clausification), `rup` (RUP checker), `arith` (proof-producing integer-equality oracle),
`error` (`ProofError` / `ProofResult`).

## Feature flags

| Flag | Effect |
|------|--------|
| *(default)* | Kernel-certified search + SAT/RUP/BMC. No Z3, no external runtime dependency. |
| `verification` | Pulls in `logicaffeine-verify`, enabling `oracle` (Z3 SMT fallback) and the private `modal_translation` (modal/temporal → world-indexed FOL). Z3 verdicts are **never** kernel-certified. |

## Dependencies

Internal: `logicaffeine-base`, `logicaffeine-kernel`; `logicaffeine-verify` (optional, gated
behind `verification`). **No** dependency on the language crate (Liskov invariant), and **no**
external (non-workspace) crates — the default build is pure Rust with no Z3 and no runtime
dependency, which is what keeps the SAT/BMC stack browser-ready.

## License

Business Source License 1.1 — see [LICENSE.md](../../LICENSE.md).

---
[Docs index](../../new_docs/README.md) · [Root README](../../NEW_README.md) · [Changelog](../../CHANGELOG.md)
