# logicaffeine-proof

A backward-chaining proof engine over an owned, arena-independent IR (`ProofExpr` /
`ProofTerm`): it searches for derivations, certifies them into kernel terms, and offers
Socratic, leading-question hints when a proof gets stuck. It embodies the Curry-Howard
correspondence — propositions are types, proofs are programs, verification is type checking.

Part of the [Logicaffeine](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/README.md) workspace. Tier 2 — depends on
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
→ kernel-certified (`verify`). See [proof-and-verification.md](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/docs/proof-and-verification.md).

## Public API

Re-exported at the crate root: `BackwardChainer`, `ProofError`, `suggest_hint`,
`SocraticHint`, `SuggestedTactic`, `Substitution`; plus `ProofTerm`, `ProofExpr`, `MatchArm`,
`InferenceRule`, `DerivationTree`, `ProofGoal` defined directly in `lib.rs`.

**Search** — `engine::BackwardChainer`:

```text
let mut prover = BackwardChainer::new();
prover.set_max_depth(depth: usize);
prover.add_axiom(expr: ProofExpr);
prover.knowledge_base() -> &[ProofExpr];
prover.prove(goal: ProofExpr) -> ProofResult<DerivationTree>;
prover.prove_with_goal(goal: ProofGoal) -> ProofResult<DerivationTree>;
```

**The single door** — `verify`:

```text
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

```text
hints::suggest_hint(goal: &ProofExpr, kb: &[ProofExpr], failed: &[SuggestedTactic]) -> SocraticHint;
```

`SocraticHint { text, suggested_tactic, priority }` proposes a `SuggestedTactic` rather than
giving the answer outright.

**SAT / model checking** (Z3-free, browser-ready) — `sat` and `bmc`:

```text
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

Other trust-core modules: `certifier` (Curry-Howard `certify`: `DerivationTree` → kernel `Term`),
`unify` (Robinson unification + occurs check, capture-avoiding `beta_reduce`, Miller
patterns), `grounding` (expand bounded quantifiers over a finite domain), `grid_solver`
(certified logic-grid solver: watched-literal unit propagation + DPLL), `cdcl` (CDCL(T) core:
2-watched lits, 1-UIP, VSIDS, Luby, `Theory` trait, DRAT/LRAT log), `cnf` (Tseitin
clausification), `rup` (RUP checker), `arith` (proof-producing integer-equality oracle),
`error` (`ProofError` / `ProofResult`).

### Module atlas

Beyond the trust core, the crate is a broad library of certified reasoners and proof systems. Grouped by capability — every entry is a `pub mod`:

- **SAT/SMT engines & interchange** — `dimacs` (DIMACS CNF I/O), `satcli` (the SAT command-line driver shared verbatim by the `logos-sat` binary and `largo sat`: competition output, certificate export, injected streams), `twosat` (2-SAT via implication-graph SCC), `hornsat` (Horn-SAT unit propagation), `sdcl` (Satisfaction-Driven Clause Learning), `inprocess` (certified inprocessing simplifications), `discrimination` (the first-order discrimination-tree index under `simp`).
- **Certified proof output & trust tiers** — `proof` (shared proof-step vocabulary), `proof_emit` (DRAT/LRAT/DPR trace emission), `proof_rewrite` (proof-rewrite 2-cells), `pr` (propagation-redundancy checker), `res_width` (resolution-width lower bounds), `complexity` (self-sizing refutation bounds).
- **Algebraic proof systems** — `gf2` (GL(n,2)), `xorsat` / `xor_engine` / `xor_drat` (GF(2) parity: Gaussian XOR-SAT, DPLL(XOR), and the CNF→DRAT bridge), `modp` / `polycalc_gfp` (GF(p) linear algebra + Nullstellensatz), `modm` (ℤ/m by CRT), `polycalc` (Polynomial Calculus / Nullstellensatz over GF(2)), `pseudo_boolean` (cutting planes), `affine` / `affine_gfp` (the AGL(n,2) / AGL(n,p) affine symmetry a permutation break cannot see), `sos` (exact Sum-of-Squares / Positivstellensatz), `lll` (Lovász Local Lemma certificate).
- **Symmetry** — `symmetry` / `symmetry_detect` (detection), `sym_break` (lex-leader breaking), `sym_certify` (certified breaking), `sym_dynamic` (Symmetric Explanation Learning), `permgroup` (Schreier–Sims BSGS — the non-abelian coset decision), `orbit_stability` (symmetric Nullstellensatz at every scale), `families` / `census` (parametric hard-instance generators + the small-`n` SAT-space census).
- **Combinatorial reasoners** — `pigeonhole` / `matching` (bipartite-matching infeasibility), `cardinality` (cardinality constraints over boolean atoms), `counting_principle` (the modular counting principle `Count_q(n)`: `O(clauses)` recognition, `q ∤ n` certificate), `parity_cardinality` (the coupled exactly-one + parity obstruction, decided by GF(2) augmentation), `interval_sched` (sweep-line scheduling), `register_alloc` (linear-scan allocation as a Hall reasoner), `hypercube` (Boolean-hypercube subcube cover), `ordering` (the GT(n) linear-ordering contradiction: polynomial-time recognizer + certified refuter for the no-maximum total order the general cascade only decides by super-polynomial search), `lyapunov` (Lyapunov-measure synthesis).
- **The ∞-tower (homotopy of SAT)** — `cubical` (d-dimensional cubical homology), `kan_complex` / `two_type` / `two_group` (∞-groupoids had as objects), `category_collapse` / `groupoid` / `coalgebra` (the categorical meaning of symmetry breaking), `eilenberg_maclane` / `postnikov` / `steenrod` (K(A,n), k-invariants, the Steenrod algebra), `progress_complex` / `trace_determinism` (higher homotopy from real concurrency: determinism = contractibility).
- **Tactics, developments & simplification** — `tactic` / `tactic_script` (interactive goal-state proving), `formula` (formal-FOL surface-text parser), `development` (`## Theory` block bodies), `simp` (oriented rewrite-rule sets).
- **Arithmetic, optimization & dispatch** — `linarith_solve` (Fourier–Motzkin LIA core), `optimize` (certified SAT-based minimization), `solve` (the structure-detecting auto-dispatcher that fronts the whole arsenal), `ait` (certified algorithmic-information / description-length objects), `isogeny` (certified SIDH/SIKE torsion-image witnesses).
- **Number-theory / cryptanalysis substrate** — `factor` (structural factoring: trial / Fermat / Pollard `p−1` / rho + the RSA-ceiling thesis), `elliptic` (Montgomery x-only ECM), `period` (order-finding — the classical shell of Shor's algorithm), `lattice` (exact LLL / Coppersmith over `Rational`), `fp2` (𝔽_{p²} arithmetic + the supersingular 2-isogeny graph), `hyperelliptic` (genus-2 Richelot (2,2)-isogeny — the Castryck–Decru mechanism), `cyclotomic` (the power-of-two Module-LWE ring `ℤ[X]/(Xⁿ+1)`). Pure number theory over `logicaffeine_base::numeric` — the hardness lens `isogeny` / `ait` / `solve` ride on. (Relocated from `logicaffeine_base` in 0.10: the prover is their only consumer.)

## Feature flags

| Flag | Effect |
|------|--------|
| *(default)* | Kernel-certified search + SAT/RUP/BMC. No Z3, no external runtime dependency. |
| `verification` | Pulls in `logicaffeine-verify`, enabling `oracle` (Z3 SMT fallback) and the private `modal_translation` (modal/temporal → world-indexed FOL). Z3 verdicts are **never** kernel-certified. |

## Tactics and decision procedures

Beyond the certified SAT/BMC core, the crate ships a tactic layer and algebraic solvers:

- **`engine`** — the backward-chaining proof engine.
- **`rule_search`** — `aesop`-style rule-set search, turning `auto`'s fixed cascade into a searchable rule database.
- **`crush`** — the grind-style closer: E-matches quantified equality lemmas into the goal.
- **`decide`** — proof by evaluation for closed decidable goals.
- **`omega_solve`** — `omega`: linear integer (Presburger) arithmetic.
- **`lemma_index`** — `exact?` / `apply?` premise selection over a named, certified lemma index.
- **`counterexample`** — when a goal is false, exhibits a model instead of just failing.
- **`gf`** — Galois-field (GF(2)/GF(p)) arithmetic backing the algebraic refutations.
- **`polycalc_zm`** — Nullstellensatz over the rings `ℤ/m` (composite moduli, zero divisors and all).
- **`cofactor`** — the cofactor-DAG lens: symmetry above the instance.

## Dependencies

Internal: `logicaffeine-base`, `logicaffeine-kernel`; `logicaffeine-verify` (optional, gated
behind `verification`). **No** dependency on the language crate (Liskov invariant), and **no**
external (non-workspace) crates — the default build is pure Rust with no Z3 and no runtime
dependency, which is what keeps the SAT/BMC stack browser-ready.

## License

Business Source License 1.1 — see [LICENSE.md](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/LICENSE.md).

---
[Docs index](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/docs/README.md) · [Root README](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/README.md) · [Changelog](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/CHANGELOG.md)
