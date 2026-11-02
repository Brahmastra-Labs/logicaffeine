# logicaffeine-proof

Proof search for Logicaffeine: a backward-chaining prover over an owned IR (`ProofExpr` / `ProofTerm`), a
tactic language, certified SAT/CDCL solvers that emit DRAT/LRAT refutations, and a number-theory and
cryptanalysis substrate. Finding a proof is hard and checking one is easy, so this crate does the finding and
something smaller does the checking: a derivation becomes a term `logicaffeine_kernel` type-checks, and an
UNSAT verdict carries a refutation the RUP checker replays. The search itself is never trusted: a wrong one
yields `verified == false`, never a false claim.

Part of the [Logicaffeine](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/README.md) workspace, tier 2.
It depends on `logicaffeine_base` and `logicaffeine_kernel` and never on the language crate (the Liskov
invariant), so any front-end can reuse it. The default build has no Z3 and no external crates, which keeps the
SAT and model-checking stack browser-ready.

## Role in the workspace

This crate owns proof representation, search and certification; `logicaffeine_language` and
`logicaffeine_compile` both use it without a dependency cycle. The `LogicExpr → ProofExpr` lowering lives in
the language crate, not here, so the engine stays independent of any one front-end.

A proof is `verified` **iff** the chainer found a derivation, `certifier::certify` turned it into a kernel
`Term`, *and* the kernel type-checked that term against the goal. This is Curry-Howard made concrete: each
inference rule maps to a kernel term (`ModusPonens` is application), so the proof is a program and checking it
is type checking. An externally built `DerivationTree` (e.g. from the grid solver) is re-checked the same way,
so untrusted search sits outside the trusted base. Trust levels run fast → strong: untrusted CDCL/SMT
(`cnf::cdcl_entails`, `oracle`), then RUP-certified (`rup::entails_certified`, `sat::prove_unsat`), then
kernel-certified (`verify`). See [proof-and-verification.md](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/docs/proof-and-verification.md).

## Example

Socrates is a man and every man is mortal, so Socrates is mortal, and the kernel agrees:

```rust
use logicaffeine_kernel::infer_type;
use logicaffeine_proof::verify::prove_certify_check;
use logicaffeine_proof::{ProofExpr, ProofTerm};

let pred = |name: &str, arg: ProofTerm| ProofExpr::Predicate {
    name: name.to_string(),
    args: vec![arg],
    world: None,
};
let socrates = ProofTerm::Constant("Socrates".to_string());
let x = ProofTerm::Variable("x".to_string());

let man = pred("man", socrates.clone());
let every_man_is_mortal = ProofExpr::ForAll {
    variable: "x".to_string(),
    body: Box::new(ProofExpr::Implies(
        Box::new(pred("man", x.clone())),
        Box::new(pred("mortal", x)),
    )),
};
let result = prove_certify_check(&[man, every_man_is_mortal], &pred("mortal", socrates));

assert!(result.verified);
let term = result.proof_term.expect("a verified proof carries its term");
assert!(infer_type(&result.kernel_ctx, &term).is_ok());
```

## Public API

Re-exported at the crate root: `BackwardChainer`, `PremiseVariables`, `ProofError`,
`suggest_hint`, `SocraticHint`, `SuggestedTactic`, `Substitution`; plus `ProofTerm`, `ProofExpr`, `MatchArm`,
`InferenceRule`, `DerivationTree`, `ProofGoal` defined directly in `lib.rs`.

**Search** (`engine::BackwardChainer`):

```text
let mut prover = BackwardChainer::new();
prover.set_max_depth(depth: usize);
prover.set_premise_variables(reading: PremiseVariables);  // Schematic (default) | Local
prover.add_axiom(expr: ProofExpr);
prover.knowledge_base() -> &[ProofExpr];
prover.prove(goal: ProofExpr) -> ProofResult<DerivationTree>;
prover.prove_with_goal(goal: ProofGoal) -> ProofResult<DerivationTree>;
```

`PremiseVariables` says how a knowledge-base equation's free variables read: `Schematic`
(a rewrite schema standing for all its instances, the default for a standalone chainer) or
`Local` (a hypothesis about fixed values, so only the equation as stated is available). The
`verify` entry points and the tactic layer run `Local`, so the search never builds an instance the
certificate cannot cite.

**Verification** (`verify`):

```text
verify::prove_certify_check(premises: &[ProofExpr], goal: &ProofExpr) -> VerifiedProof;
verify::prove_certify_check_bounded(premises, goal, max_depth: usize) -> VerifiedProof;
verify::prove_certify_check_with_defs(..) / prove_certify_check_bounded_with_defs(..);
verify::check_derivation(premises, goal, tree: DerivationTree) -> VerifiedProof;
verify::detect_conflict(premises: &[ProofExpr]) -> ConflictReport;
verify::ScopedChecker::new(premises, goal).certifies(tree: DerivationTree) -> bool;
verify::close_over_context_locals(ctx: &Context, term: &Term) -> Term;
verify::prove_library(theorems: &[LibraryTheorem]) -> Vec<LibraryResult>;
VerifiedProof::axiom_footprint(&self) -> AxiomFootprint;  // { trusted, introduced }
```

`VerifiedProof { derivation, proof_term, kernel_ctx, verified, verification_error }` carries
the re-checkable kernel term; `detect_conflict` returns a kernel proof of `False` plus the
indices of the clashing premises. A theorem's free variables become kernel locals typed by use
(`Int` in arithmetic, the inductive a premise's unary predicate names, otherwise `Entity`),
and the finished term is closed over them. The `_with_defs` variants register `Definition`s
as δ-unfoldable kernel definitions after every atom their bodies mention. `ScopedChecker`
prepares a scope once and certifies many candidate derivations against it.
`axiom_footprint` is `#print axioms` for the certified term, read transitively through
definitions: `trusted` holds the prelude declarations the proof cites (checked against a
freshly registered `StandardLibrary`), `introduced` everything else: premises, symbols, and
any smuggled `sorryAx`.

**IR**: `ProofTerm` (`Constant`, `Variable`, `Function`, `Group`, `BoundVarRef`) and
`ProofExpr` cover full FOL plus extensions: connectives, quantifiers, `Modal` /
`Counterfactual` / `Temporal` / `TemporalBinary`, `Lambda` / `App`, Neo-Davidsonian
`NeoEvent`, inductive `Ctor` / `Match` / `Fixpoint` / `TypedVar`, and unification `Hole`s.
`InferenceRule` names the move at each step: `ModusPonens` / `ModusTollens`, ∧/∨ intro and
elim, ∀/∃ intro and elim (`ExistentialIntroTerm` carries a `ProofTerm` witness),
`ModalAccess`, `StructuralInduction`, Leibniz `Rewrite` and positional `RewriteAt { from,
to, occurrence }`, the order rules (`LeTrans`, `LeRefl`, `LeTotal`, `LtSuccLe`, …),
`ArithDecision`, `GroundRefutation` (¬P for a closed P that computes false),
`ReductioAdAbsurdum`, `CaseAnalysis`, `DisjunctionCases`, and more. Rule introspection:
`InferenceRule::{NAMEABLE, from_name, canonical_name, is_certifiable, uncertifiable_reason,
map_payload}` and `DerivationTree::uncertifiable_rules`; `certifier::certify` refuses a rule
with no certifier arm with that reason. A
`DerivationTree { conclusion, rule, premises, depth, substitution }` is the prover's result;
`ProofGoal { target, context }` is consumed.

**Hints** (`hints`):

```text
hints::suggest_hint(goal: &ProofExpr, kb: &[ProofExpr], failed: &[SuggestedTactic]) -> SocraticHint;
```

`SocraticHint { text, suggested_tactic, priority }` proposes a `SuggestedTactic` as a leading question rather
than giving the answer outright.

**SAT and model checking** (Z3-free, browser-ready; `sat` and `bmc`):

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

**Core modules**:

- `certifier`: `certify`, `DerivationTree` → kernel `Term`.
- `unify`: Robinson unification with occurs check, capture-avoiding `beta_reduce`, Miller patterns.
- `grounding`: expands bounded quantifiers over a finite domain.
- `grid_solver`: the certified logic-grid solver (watched-literal unit propagation + DPLL).
- `cdcl`: the CDCL(T) core: two watched literals, 1-UIP, VSIDS, Luby restarts, a `Theory` trait, and a
  DRAT/LRAT log. Incremental: solves under assumptions within a conflict budget, variables and decision variables
  added between solves, and LRAT hints recorded as each clause is learned (a level-0 fact named by its unit clause,
  derived once), so a caller keeps one derivation across every solve.
- `cnf`: Tseitin clausification of a `ProofExpr`, plus a gate-level builder
  (`Cnf::{atom_lit, add_clause, encode_and, encode_or, encode_implies, encode_iff}`) for a producer holding a
  shared gate DAG; `sat::prove_unsat_certified_cnf` returns the same checked refutation steps
  `sat::prove_unsat_certified` gives for a `ProofExpr`.
- `rup`: the RUP checker.
- `arith`: the proof-producing `Int` ring oracle. `BigInt` coefficients, closed goals by checked kernel
  normalization at any literal width, ring proofs whose laws are proved in the prelude, and `None` in a context
  whose `add`/`mul`/`sub` are not the checked integer operations.
- `interval_replay`: the independent range-safety checker for analog netlist claims. It replays claimed
  voltage bounds by directed-outward-rounded interval arithmetic with no dependencies: the RUP-checker trust
  pattern applied to clipping and noise-floor safety.
- `relax_replay`: the residual judge for physics-solved linear systems. It re-multiplies `A·x̂ − b` in exact
  ℚ and checks the declared ∞-norm bound, so the physical solver is untrusted by construction; it also judges
  settled minimization and Ising readouts.
- `thermal_replay`: the same judge for thermal maps. It reassembles the thermal Laplacian from a neutral
  edge/ground/injection list and demands `G·ΔT = P` as an exact equality, plus connectivity and positivity.
- `error`: `ProofError` / `ProofResult`.

## Module map

Beyond the core, the crate is a library of certified reasoners and proof systems. Every entry is a `pub mod`:

- **SAT engines and interchange**: `dimacs` (DIMACS CNF I/O), `satcli` (the SAT command-line driver shared by
  the `logos-sat` binary and `largo sat`: competition output, certificate export, injected streams), `twosat`
  (2-SAT via implication-graph SCC), `hornsat` (Horn-SAT unit propagation), `sdcl` (Satisfaction-Driven
  Clause Learning), `inprocess` (certified inprocessing simplifications), `discrimination` (the first-order
  discrimination-tree index under `simp`).
- **Certified proof output**: `proof` (shared proof-step vocabulary), `proof_emit` (DRAT/LRAT/DPR trace
  emission, replayable by the SAT community's independent checkers), `lrat` (LRAT hint chains found by
  watched-literal propagation over a solver's learned clauses, trimmed backward to the clauses the refutation
  uses, or to any target clauses a log derives, and a structured checker that replays them), `proof_rewrite` (the
  commutation of independent refutation steps, as proof-rewrite 2-cells), `pr` (propagation-redundancy checker),
  `complexity` (a refutation that carries a checkable bound on its own size).
- **Circuit equivalence**: `aig` (and-inverter graphs: structural hashing, the AIGER ASCII and binary formats,
  bit-parallel simulation), `sweep` (two circuits paired by port name and decided by SAT sweeping, every equality
  a lemma with an LRAT proof and every difference an input assignment that replays), `fraig` (the same decision on one
  incremental solver over the reduced graph: nodes merged by structure without a call, the rest by budgeted solves
  under assumptions on the cones they ask about, every clause logged with its hints as it is made, and every
  equality carried by one LRAT derivation from the miter's formula), `units` (two circuits decided over units proven
  apart, each cut out as a function nothing is known of: in rounds, applications of one function whose operands are
  proven equal identified, on either side or across, a unit's latency matched, each side unrolled with its inputs
  bound per clock and its outputs claimed at their own clocks, and every round's derivation and identification
  re-checked), `streams` (two valid-ready stream circuits at their own widths and paces decided token for token under
  an arrival pattern: each side's handshakes proven constants with the data free, so a schedule that depends on the
  data is refused by name, then the k-th tokens claimed equal over the units), `seq_equiv` (a pipeline unrolled to
  its latency against its combinational reference, decided by either engine, and two sequential circuits equal for all time by
  an inductive correspondence of their registers, with the certificate a checker replays), `cycle_ratio` (the
  maximum delay-per-register over a graph's cycles, the clock no retiming can beat: Howard's policy iteration on
  each strongly connected component, the bound certified by its cycle and an integer potential no edge violates),
  `min_cost_flow` (the network simplex on a strongly feasible tree, every optimum certified by complementary
  slackness, every infeasibility by a cut, every unbounded problem by a negative cycle: the dual of a least-area
  register cut at a target clock).
- **Algebraic proof systems**: `gf2` (GL(n,2)), `xorsat` / `xor_engine` / `xor_drat` (GF(2) parity:
  Gaussian XOR-SAT, DPLL(XOR), and the CNF→DRAT bridge), `modp` (GF(p) linear algebra), `modm` (ℤ/m by CRT),
  `polycalc` (Polynomial Calculus / Nullstellensatz over GF(2)), `pseudo_boolean` (cutting planes), `affine` /
  `affine_gfp` (the AGL(n,2) / AGL(n,p) affine symmetry a permutation break cannot see), `sos` (exact
  Sum-of-Squares / Positivstellensatz), `lll` (Lovász Local Lemma certificate), `macaulay` / `xl` (the
  Macaulay matrix and XL linearization that solve multivariate-quadratic systems over GF(2)/GF(2⁸), the
  algebraic cryptanalysis Ding formalized via the degree of regularity), `rainbow` (a Ding–Schmidt
  Oil-and-Vinegar signature reference, the multivariate scheme `xl` attacks).
- **Symmetry**: `symmetry` / `symmetry_detect` (detection), `sym_break` (lex-leader breaking), `sym_certify`
  (certified breaking), `sym_dynamic` (Symmetric Explanation Learning), `permgroup` (Schreier–Sims BSGS, the
  non-abelian coset decision), `families` (parametric generators for symmetry-rich hard instances).
- **Combinatorial reasoners**: `pigeonhole` / `matching` (bipartite-matching infeasibility), `cardinality`
  (cardinality constraints over boolean atoms), `counting_principle` (the modular counting principle
  `Count_q(n)`: `O(clauses)` recognition, `q ∤ n` certificate), `parity_cardinality` (the coupled
  exactly-one + parity obstruction, decided by GF(2) augmentation), `interval_sched` (sweep-line scheduling),
  `register_alloc` (linear-scan allocation as a Hall reasoner), `hypercube` (Boolean-hypercube subcube cover),
  `ordering` (a polynomial-time recognizer and certified refuter for the GT(n) no-maximum ordering principle,
  which the general cascade decides only by super-polynomial search), `lyapunov` (Lyapunov-measure synthesis).
- **Homotopy of symmetry and concurrency**: `groupoid` (symmetry breaking as keeping one assignment per
  orbit of the action groupoid), `coalgebra` (a certified refutation as a coalgebra, and a Lyapunov measure as a morphism to the
  countdown), `cubical` (cubical homology in any dimension over GF(2)), `kan_complex` (the nerve of a group,
  with the Kan condition checked in degrees 2 and 3), `two_group` / `two_type` / `eilenberg_maclane` / `postnikov`
  (crossed modules, `K(A, n)` spaces and the Postnikov `k`-invariant), `steenrod` (Steenrod squares on
  `Z/2[x]`), `progress_complex` / `trace_determinism` (the space of schedules of concurrent operations: mutual
  exclusion leaves a hole in it, and pairwise-independent operations, which reach the same state on every
  schedule, leave it contractible).
- **Tactics, developments and simplification**: `tactic` / `tactic_script` (interactive goal-state proving),
  `tactic_cert` (the certificate producers and independent checkers shared with the kernel's second checker:
  a tactic hands back a certificate that checker replays, so it trusts the certificate rather than the tactic
  that found it), `formula` (formal-FOL surface-text parser), `development` (`## Theory` block bodies),
  `simp` (oriented rewrite-rule sets).
- **Silicon reversibility**: `revgate` certifies a reversible logic gate as a bijection. An `n`-bit unit is
  reversible iff its truth table is a permutation of `{0..2ⁿ}`, and a linear CNOT/XOR block iff its GF(2)
  transfer matrix is invertible.
- **Arithmetic, optimization and dispatch**: `linarith_solve` (Fourier–Motzkin LIA core), `optimize`
  (certified SAT-based minimization), `solve` (the structure-detecting dispatcher: cheap recognizers first,
  each claiming a verdict only with a re-checked certificate, then CDCL for the rest), `ait` (certified
  algorithmic-information / description-length objects), `isogeny` (certified SIDH/SIKE torsion-image
  witnesses), `linalg` (exact linear algebra over ℚ: `A·x = b` decided as an exact identity with an
  independently re-multiplied `residual_is_zero` certificate, `det A ≠ 0` certifying nonsingularity outright),
  `linalg_certify` (each row identity cleared to an integer identity, proven by the `arith` ring oracle, and
  re-checked by the kernel's own `infer_type`).
- **Number theory and cryptanalysis**: `factor` (structural factoring and RSA attacks: trial division,
  Fermat, Pollard `p−1` and rho, Wiener, Coppersmith, each returning factors `verify_factorization`
  re-checks), `elliptic` (Montgomery x-only ECM), `period` (order-finding, the classical shell of Shor's
  algorithm), `lattice` (exact LLL / Coppersmith over `Rational`), `fp2` (𝔽_{p²} arithmetic and the
  supersingular 2-isogeny graph), `hyperelliptic` (genus-2 Richelot (2,2)-isogeny, the Castryck–Decru
  mechanism), `cyclotomic` (the power-of-two Module-LWE ring `ℤ[X]/(Xⁿ+1)`). This is pure number theory over
  `logicaffeine_base::numeric`, and the hardness lens `isogeny`, `ait` and `solve` build on.

## Tactics and decision procedures

Beyond the certified SAT/BMC core, the crate ships a tactic layer and algebraic solvers:

- **`engine`**: the backward-chaining proof engine. Order chains mix `=` and `≤` (an equality is a two-way edge through positional Leibniz).
- **`rule_search`**: `aesop`-style rule-set search, turning `auto`'s fixed cascade into a searchable rule database.
- **`crush`**: the grind-style closer. It E-matches quantified equality lemmas into the goal.
- **`decide`**: proof by evaluation for closed decidable goals. `div`/`mod` are Euclidean, total and arbitrary-precision (a zero divisor gives quotient `0`, remainder the dividend); integer literals past `i64` are integers; a goal that evaluates false is reported false, not undecided, and a negated ground goal is proved by `GroundRefutation`.
- **`omega_solve`**: `omega`, linear integer arithmetic. It refutes the hypotheses (a strict `a < b` becomes `a + 1 ≤ b` through `lt_succ_le`, then a Farkas refutation), proves the goal directly (a ground comparison by computation, a `≤` by chaining in-scope facts, an `Int` equality through the ring oracle), or splits on `le_total` and refutes the other side. It reaches every goal whose negation-by-totality is refutable, not all of Presburger arithmetic, and adds no axiom.
- **`linarith_solve`**: the rational Fourier–Motzkin core with Farkas certificates behind `linarith`, which declines any goal that needs integrality.
- **`lemma_index`**: `exact?` / `apply?` premise selection over a named, certified lemma index.
- **`counterexample`**: when a goal is false, exhibits a model instead of just failing.
- **`simp`**: oriented rewrite-rule sets; `SimpSet::register_context_lemmas(&Context)` compiles the `@[simp]` rules a kernel context registers.

### The tactic language

`tactic::ProofState` holds the open goals; a tactic replaces the focused goal with zero or more subgoals and records the rule it used, and `ProofState::qed` assembles the steps into one `DerivationTree` and sends it through `verify::check_derivation`. A tactic builds the derivation; the kernel checks it. `tactic::Tactic` is `Box<dyn Fn(&mut ProofState) -> Result<(), TacticError>>`, and `tactic::combinators` builds one per primitive plus `seq`, `first`, `try_`, `repeat`, `then_all`, `all_goals`, `any_goals` and `focus` (an error when its block leaves the goal open). `tactic::PTerm` is a script term: one syntax read as a proof or as a value by its position.

`tactic_script::parse_script` compiles proof text to a `Tactic`; `ProofState::run_script` runs it, and `logicaffeine_language::compile::run_proof_sketch` runs a theorem's `Proof:` line through it, discharging what the sketch leaves open with `auto`. Sentence punctuation (`.`, `,`) separates steps like `;`.

```text
script  := seq
seq     := chain (';' chain)*                  run in order
chain   := atom ('<;>' atom)*                  t2 on every goal t1 spawned
atom    := '(' script ')' | 'first' '[' script ('|' script)* ']'
         | 'try' atom | 'repeat' atom | 'all_goals' atom | 'any_goals' atom
         | 'focus' atom | NAME ARG*
term    := factor (('+'|'-') factor)*          the argument of exact/exists/have/refine
factor  := app ('*' app)*
app     := primary primary*                    application by juxtaposition
primary := NAME | '?' NAME? | '(' term (',' term)* ')'
```

Primitive steps: `intro a b`, `exact <term>`, `apply h`, `rfl`, `cases h [with a b]`, `rewrite h [at <k> | at <hyp> [<k>]]` (position, location, or both), `exists <term>`, `assumption`, `simp`, `simp only [h₁, h₂]`, `simp [h]`, `decide`, `omega`, `linarith`, `ring`, `crush`, `calc h₁ h₂ …`, `by_rule <Rule>` (certified before it reports success), `split`, `left`, `right`, `auto`, `induction [n]`, `by_contra h`, `have h from <term>`, `refine <term>` (one subgoal per `?_` hole). Each has English synonyms (`use`/`witness` for `exists`, `assume`/`suppose` for `intro`, `arithmetic`/`integers` for `omega`, `rationals` for `linarith`, `grind` for `crush`, …). `tactic_script::TacticEnv` names user-defined tactics as scripts (expansion is depth-bounded, so a recursive definition is refused), and `prove_scripted_library` proves a dependency-ordered library of `ScriptedTheorem`s, each proved conclusion citable by later ones.

## Feature flags

| Feature | Default | Enables |
|---------|---------|---------|
| `verification` | off | `logicaffeine-verify`, and with it `oracle` (the Z3 SMT fallback) and the private `modal_translation` (modal/temporal → world-indexed FOL). Z3 verdicts are **never** kernel-certified. |

Without it the crate is kernel-certified search plus SAT/RUP/BMC, with no Z3 and no external runtime dependency.

## Dependencies

Internal: `logicaffeine-base`, `logicaffeine-kernel`; `logicaffeine-verify` (optional, gated
behind `verification`). **No** dependency on the language crate (Liskov invariant), and **no**
external (non-workspace) crates: the default build is pure Rust with no Z3 and no runtime
dependency, which is what keeps the SAT/BMC stack browser-ready.

## License

MIT License — see [LICENSE.md](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/LICENSE.md).

---
[Docs index](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/docs/README.md) · [Root README](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/README.md) · [Changelog](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/crates/logicaffeine_proof/CHANGELOG.md)
