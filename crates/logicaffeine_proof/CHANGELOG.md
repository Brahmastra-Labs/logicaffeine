# Changelog

All notable changes to this crate will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

## [0.11.0] - 2026-11-02

### Added

- **Combinational and sequential equivalence, certified.** `aig`: and-inverter graphs with structural hashing,
  AIGER ASCII and binary read and write, 64-way simulation, cones, `compact`/`compact_keeping` (only the nodes
  an output, a property, a latch or a kept literal reads), and `sequential_cone` (the outputs asked for with only the
  latches that reach them through any number of clocks). `sweep`: two circuits joined by `miter` (inputs shared and
  outputs paired by name, kept to the outputs' cones), swept by simulation classes into lemmas each a refuted
  difference whose LRAT proof `check` replays, a lemma first put on a depth-6 cut, a depth-16 cut and the cut both
  cones meet at before the whole cone; `equivalent` decides every output pair with its certificate, `separating_input`
  finds one input that tells two circuits apart. `distinguish` tells a family of circuits apart: the members that
  compute one function, each equality certified, a greedy cover of input assignments on which every two classes
  differ, and each class's outputs there (`Distinction::check` replays all of it). A decision's `Stats` count every
  query by the cut it was put at (`CutCost`: queries, verdicts, variables, clauses, conflicts) and how much of the cone
  the sweep has covered; `Options::progress` hands them to a report every so many candidates. `seq_equiv`: `unroll`,
  `at_latency` (a pipeline's outputs at clock L over the clock-0 inputs, every initial state free), `aligned` and
  `latency_equivalent`, and `always_equal` by register correspondence with an `InductionCertificate` and bounded
  model checking for a difference from reset. `lrat`: watched-literal LRAT hints, backward-trimmed, and a structured
  checker; `derivation` trims a log back from any target clauses, and `check_derivation` replays one and finds every
  claimed clause derived. `aig_equivalence_lock`, `rtl_equivalence_lock`.
- **`fraig`: equivalence by incremental SAT sweeping, the scheme of ABC's `&cec`, certified.** One solver answers every
  query under assumptions within a conflict budget, holding only the cones asked about, defined over their fan-ins'
  representatives, and numbered as it meets them; only the nodes the pairs reach are swept and simulated, so a miter
  decided a few pairs at a time costs their cones, not the whole; nodes merge by structure with no solver call, and a fresh solver
  takes over once one holds `Options::recycle` variables. Every clause made carries its LRAT hints as it is made: a
  learned clause its conflict analysis's chain, a solved merge its refutation's, a structural merge or definition the
  chain propagation finds. What nothing a later clause can name rests on is let go as the log grows (`Options::collect`)
  and with each replaced solver, and the derivation is what the claims reach back to (`Report::check` replays it
  against the miter's own formula). `seq_equiv::{aligned_by_fraig, latency_equivalent_by_fraig}` decide a pipeline by
  it. On the exp_p re-cut (141,443-node miter) it proves 32 of 32 outputs in 38 s, its derivation checked in 0.3 s.
- **`units`: equivalence over units proven apart.** Two circuits with their units cut out (results free inputs,
  operands outputs), each unrolled as `Unrolled` says (every named input bound per clock: shared with the other side
  by a name, a constant, or fresh; output bits claimed at the clocks each side makes them), `decide` the case of a
  pipeline read at its latency. They are decided in rounds: a unit's function is its name on its side and the
  function `functions` pairs it with on the other, and the applications of one function whose operands fraig proves
  equal are identified, on one side or across (functional consistency, a pipelined unit's latency matched), a pair
  refuted leaving its group's other applications to be tried; the outputs are then decided over the units as
  functions nothing is known of — a difference there is unknown, never a verdict. An input with no name (an undefined
  bit) is a fresh one of its own side at every clock, and a latch with no name starts from a fresh value of its own.
  Every round's miter, derivation and identification is re-checked (`Report::check`, `check_unrolled`); an
  application made one twice, or one with itself through others, does not check.
- **`streams`: stream equivalence over units.** Two valid-ready circuits at their own widths and paces
  (`StreamSide`, `Ports`), each under an arrival pattern (`Environment`: the clocks a token is offered and taken,
  every other input per clock). Each side's handshakes are proven constants with the data and the units' results
  free (`schedule`: a `Schedule` whose derivation `check` replays, its clocks in token order), so a side whose ready
  or valid moves with the data is refused by name (`StreamError::DataDependent`); then each side's j-th input token
  is read where that side takes it (`Ports::token` names the symbols a token's bits are, so a side twice as wide reads
  words 2j and 2j+1), each k-th output token is claimed where it is given, and `decide` proves the tokens equal over
  the units (`Report::check` replays both schedules and every round). A schedule unrolls only the handshakes'
  sequential cone, so a datapath they never read costs it nothing.
- **`cdcl::Solver`, incremental and proof-producing**: `solve_under_assumptions_budgeted` (a conflict budget, Luby
  restarts, learned clauses reduced without touching the originals), `add_vars`, `add_decision_vars`, clauses added
  between solves placed against the level-0 facts, and online LRAT (`set_lrat`, `add_clause_with_id`,
  `set_next_clause_id`): every learned clause's hints recorded as it is learned, a level-0 fact named by its unit
  clause (derived once), `unsat_hints` for the clause of an unsatisfiable solve's negated assumptions, and
  `take_learned` / `live_ids` for a caller that keeps its own log.
- **`cycle_ratio`**: the maximum cycle ratio of a graph whose edges carry a delay and a register count, by Howard's
  policy iteration per strongly connected component, exact as a ratio in lowest terms, with the potential that
  certifies it (`check`); a loop of no registers is returned as the loop.
- **`min_cost_flow`**: network simplex (block-search pricing, a strongly feasible tree), the optimum certified by
  complementary slackness, infeasibility by a cut and unboundedness by a negative cycle (`check`).
- `tactic_script::canonical` is public: it maps a tactic word, synonyms included (`Suppose`/`assume`/`let` → `intro`), to the engine's own name, so an editor highlights exactly the words the engine accepts.

- **A Lean-shaped tactic vocabulary, in the API and in `Proof:` scripts.** Every step still ends in one kernel-certified derivation.
  - **`ProofState`:** `exact_term`, `refine_term` (one subgoal per `?_` hole; a bare hole is refused), `have_term`, `apply_named`, `rfl`, `by_contra` (certified through `ClassicalReductio`, so the axiom footprint records `dne`), `ring`, `linarith`, `calc` (named steps; the relation may change along the chain), `by_rule`, `cases_with` (component names chosen by the proof; a name already in scope is refused), `induction_on` (an inner variable is generalized first), `rewrite_at`, `rewrite_hyp`/`rewrite_hyp_at`, `named_simp_set`, `value_term`, and `intro` on a `¬P` goal. `tactic::PTerm` is the script term.
  - **Combinators:** `tactic::combinators` gains a value for each, plus `any_goals`, `focus` (an error when the block leaves its goal open), `simp_only`, `simp_with`, `exists_term`, `refine` and `have`.
  - **Script grammar:** `tactic_script::parse_script`, which a `Proof:` line runs, takes a term (names, application by juxtaposition, `+`/`-`/`*`, `(a, b)` tuples, `?_` holes) as the argument of `exact`/`exists`/`have`/`refine`, and reads `apply h`, `rfl`, `ring`, `by_contra h`, `intro a b c`, `cases h with a b`, `rewrite h at <k>` and `rewrite h at <hyp> [<k>]`, `simp only [h₁, h₂]` and `simp [h]` (names checked), `calc h₁ h₂ …`, `by_rule <Rule>`, `have h from <term>`, `induction n`, and `all_goals`/`any_goals`/`focus`. A connective filler word is skipped only when another word follows it, so `exists A` and `cases it` parse.
  - **Nothing unchecked reports success:** `by_rule` certifies its node through `verify::ScopedChecker` before it succeeds, a Leibniz step abstracts the motive position the kernel will check, and the eigenconstant-renaming walkers are exhaustive over rule payloads (`InferenceRule::map_payload`).
  - Tests: `crates/logicaffeine_tests/tests/T_tactics_extra.rs`, `crates/logicaffeine_tests/tests/tactic_surface_extra.rs`, `crates/logicaffeine_tests/tests/tactic_script.rs`.
- **Inference rules for positional rewriting, order totality, term witnesses and ground refutation.** `InferenceRule::RewriteAt { from, to, occurrence }` abstracts one occurrence in the `Eq_rec` motive, so `P(a, a)` can become `P(b, a)`. `LeTotal` certifies to `le_total`. `ExistentialIntroTerm` carries a `ProofTerm` witness (`use (n + 1)`). `GroundRefutation` proves `¬P` for a closed `P` that computes false through the `Bool` discriminator, with no new axiom. `LeSuccLt` is declared and certifiable against `le_succ_lt`, which the prelude does not register, so no producer builds it.
- **Rule introspection for proof producers.** `InferenceRule::{uncertifiable_reason, is_certifiable, NAMEABLE, from_name, canonical_name, map_payload}` and `DerivationTree::uncertifiable_rules`. The seven modal and temporal rules with no certifier arm report the live-rule translation, and `certifier::certify` refuses them with that reason instead of a generic "not implemented". `crates/logicaffeine_tests/tests/automation_extra.rs`.
- **`#print axioms` for a certified proof.** `VerifiedProof::axiom_footprint()` returns `verify::AxiomFootprint { trusted, introduced }`, read off the kernel term transitively through definitions and checked against a freshly registered `StandardLibrary`: prelude declarations the proof uses are `trusted`, and a premise, a symbol or a smuggled `sorryAx` lands in `introduced`. `crates/logicaffeine_tests/tests/axiom_footprint_complete.rs`; `crates/logicaffeine_tests/tests/axiom_footprint_lean_parity.rs` holds it to Lean 4.14's rule.
- **Scoped checking and premise-variable modes.** `verify::ScopedChecker::{new, certifies}` prepares a scope once and checks many candidate derivations against it; `verify::close_over_context_locals` closes a proof term over the context locals it mentions. `engine::PremiseVariables` (re-exported at the crate root) with `BackwardChainer::set_premise_variables`: a standalone chainer reads a knowledge-base equation as a rewrite schema (`Schematic`, the default), while the trust door and the tactic layer run `Local`, where a theorem's premise is a hypothesis about fixed values, so the search never builds an instance the certificate cannot cite.
- **`@[simp]` lemmas reach the proof-layer simplifier.** `simp::SimpSet::register_context_lemmas(&Context)` compiles every rule registered in a kernel context, and `ProofState::context_simp_set` combines them with the scope; a rewrite enters a certificate only where its lemma is in scope.
- **Gate-level Tseitin and a CNF-in certified refutation.** `cnf::Cnf::{add_clause, atom_lit, encode_and, encode_or, encode_implies, encode_iff}` and `sat::prove_unsat_certified_cnf(num_vars, clauses)` let a producer holding a shared gate DAG clausify it without building the exponentially larger `ProofExpr` tree and still get the same certified refutation; the SVA bit-blaster in `crates/logicaffeine_compile/src/codegen_sva/bitblast.rs` is the first consumer.
- **The algebraic refutation layer is reusable.** `polycalc::{poly_mul_mono, close_perm_group, gf2_solve, apply_perm_to_mono}` are public, and `polycalc::NsCertificate::coeffs()` exposes the Nullstellensatz witness polynomials.
- **`satcli::main()`: the whole `logos-sat` process, which the binary now is.** `crates/logicaffeine_proof/src/bin/logos-sat.rs` is that one call, and so is the test crate's `logos-sat-under-test`.
- **Exact linear algebra over ℚ, and kernel-certified solves.** `linalg::{solve, det, lu, inverse, rank, certify_solve, residual_is_zero, …}` decides `A·x = b` as an exact identity with an independently re-multiplied residual. `linalg_certify::{certify_system, prove_solve, prove_solution, prove_lu, prove_inverse, prove_nonzero}` emits a kernel-checked `Eq Int` term per row after clearing denominators, within the i64 cleared-integer width. `lapack_provable_lock` proves the stdlib `gesv` solve exactly; `crates/logicaffeine_tests/tests/linalg_induction_kernel.rs` proves trace additivity for every `n` by kernel induction.
- **XL linearization and a Rainbow reference.** `macaulay` enumerates and multiplies monomials. `xl::xl_solve` over `xl::{Gf2, Gf256}` returns a solution only after checking it against the original system. `rainbow::{keygen, sign, verify, forge_system}` is a two-layer Oil-and-Vinegar signature over GF(2⁸). `crates/logicaffeine_tests/tests/rainbow_logos.rs` verifies its signatures with the LOGOS `rainbowVerify` on the tree-walker, the VM and native code.
- **Independent replay checkers for physical claims.** `interval_replay::{check_range_claim, check_reduction_claim, check_integrator_claim}` replays claimed analog node bounds with outward-rounded `f64` intervals (no clipping, no noise-floor starvation; any NaN rejects). `relax_replay::{check_relax_claim, check_minimize_claim, check_ising_claim}` judges a settled readout by its exact rational residual. `thermal_replay::check_thermal_claim` reassembles the Laplacian from primitive edges and demands `G·ΔT = P` exactly. All three are dependency-free and fail closed. `analog_range_certificate_lock`, `analog_relax_solve_lock`, `thermal_map_lock`.
- **`revgate`: reversible-gate certificates.** `certify_gate` accepts an `n`-bit unit iff its truth table is a permutation, and `certify_linear` accepts a CNOT/XOR block iff its GF(2) matrix is invertible; `cnot`, `toffoli` and `fredkin` are the reference tables. `crates/logicaffeine_tests/unit/logicaffeine_proof/revgate/tests.rs`.
- **`tactic_cert` re-exports the kernel's tactic certificates.** `logicaffeine_kernel::tactic_cert` holds the LIA, ring and congruence certificate emitters and the checkers the second kernel replays. `crates/logicaffeine_tests/tests/tactic_cert_integration.rs`.

### Breaking changes
- **Seven public modules are gone.** They are listed under Removed. Migration: pin `logicaffeine-proof = "=0.10.1"` for them; code that rebuilt on their GF(2) core can use `polycalc`'s now-public `poly_mul_mono`, `close_perm_group`, `gf2_solve` and `apply_perm_to_mono`.
- **`InferenceRule` gained five variants.** `RewriteAt`, `LeTotal`, `ExistentialIntroTerm`, `GroundRefutation` and `LeSuccLt`; the enum is not `#[non_exhaustive]`, so an exhaustive `match` stops compiling. Migration: add the arms, or route through `InferenceRule::map_payload` and `uncertifiable_reason`, which already cover every variant.
- **`linarith` no longer means `omega`.** In 0.10.1 `linarith` was an alias of `omega`, which tightens `a < b` to `a + 1 ≤ b` — sound over ℤ, unsound over an ordered field. `linarith` (alias `rationals`) now runs rational Fourier–Motzkin with a Farkas certificate and declines a goal that needs integrality, so an integer goal such as `x < y, y < x + 1 ⊢ ⊥` fails under it. Migration: write `omega` (or `arithmetic`/`integers`). `crates/logicaffeine_tests/tests/omega_tactic.rs`.
- **`arith::prove_int_eq` returns `None` in a context whose `add`/`mul`/`sub` are not the checked integer operations.** Migration: build the context with `StandardLibrary::register`, as `linalg_certify::ring_context` does.

### Changed

- Licensed under the MIT License, which replaces the Business Source License 1.1, and the package ships it as `LICENSE.md`.
- **The RUP checker reads a clause without allocating.** One clause rule, `rup::clause_state` (a conflict, the unit a
  clause forces, or quiet), serves `rup::propagate` and the LRAT emitter's hint search and step check, where each
  collected a clause's distinct unset literals into a fresh vector on every pass; a clause is read only to its second
  distinct unset literal. The verdicts are the collected rule's, and wide bit-vector equivalence certifies 15x faster:
  `phase_hw_native_vs_z3`'s sixteen- and thirty-two-bit identities in 32 s, from 500 s.

- **Certification asks the standard library's one registration.** Every certifier entry (`prepare_ctx_with_defs`, behind `prove_development`, `prove_library_with_axioms` and the bounded certifiers), `VerifiedProof::axiom_footprint`, the tactic and search engines' prelude questions and `linalg_certify::ring_context` borrow or clone `StandardLibrary::context()` instead of registering the prelude; the opening of Tarski's geometry, eight theorems, registered it eight times, and its debug run now takes 23.1 G instructions instead of 133.8 G.
- **`omega` proves goals, not only contradictions.** It decides a ground `≤`/`<` by computation, a `≤` by chaining in-scope facts, and an `Int` equality through the ring oracle (`⊢ n + 0 = n` with no hypotheses). Any other `≤` goal is split on `le_total a b` with the right branch refuted, so no new axiom is needed. A strict symbolic conclusion is declined with a message naming the missing `le_succ_lt`. `crates/logicaffeine_tests/tests/proof_order_axioms.rs`.
- **The ring oracle computes with exact integers.** `arith::prove_int_eq` keeps coefficients as `BigInt`, decides closed goals by checked kernel normalization at any literal width, sorts monomial products constructively at any degree and reaches subtraction through `sub_elim`. The ring laws it cites have proof bodies in the prelude, so its proofs have empty transitive axiom dependencies. `crates/logicaffeine_tests/tests/symbolic_bigint_closed_proofs.rs`, `crates/logicaffeine_tests/tests/arith_identity_certified.rs`.
- **Order chains mix `=` and `≤`.** An equality is a two-way edge in the `≤` graph through positional Leibniz, so `a = b, b ≤ c, c ≤ d ⊢ a ≤ d` composes, and `calc` E-matches its named `∀`-equality steps at the terms the chain mentions.
- **`formula` reads nested function terms.** `Eq(Mul(a, b), Mul(b, a))` parses its arguments, and an equality with an arithmetic side is a ring identity rather than an uninterpreted `Eq`.
- **The published crate carries only its source.** An anchored `include` allowlist ships `src/`, the README and the CHANGELOG, so neither `tests/` nor `examples/` is in the tarball (`cargo run --example solver_bench` needs a checkout); the unit-test bodies live in `crates/logicaffeine_tests/unit/logicaffeine_proof/` behind one-line `#[cfg(test)] #[path]` stubs and the integration suites in `crates/logicaffeine_tests/tests/`. Without its tests, examples and research modules, the tarball is 0.79 MB (1.99 MB in 0.10.1). The declared `rust-version` is 1.88 (0.10.1 declared 1.75, below what its own dependency graph needed).

### Removed
- **Seven research modules, reachable from no product code, left the crate.** `category_collapse`, `census`, `cofactor`, `res_width`, `orbit_stability`, `polycalc_gfp` and `polycalc_zm` were public in 0.10.1.

### Fixed

- `unify::instantiate_expr`: a binder instantiated once and capture-avoiding; every quantifier instantiation in
  `tactic`, `engine`, `crush`, `simp` and `certifier` goes through it. `apply_subst_to_term` refuses a cyclic
  substitution by name instead of recursing until the stack is gone.
- A goal may be any conjunct of a rule's conclusion (`unify_with_conclusion`, `project_goal`); the ⊥ search remembers the fact sets it failed on (`FalsumMemo`) and skips case splits over premises the all-true interpretation satisfies or a Horn set propagation left open, so a refusal that took 194 s is immediate; an existential goal may be witnessed from its context (`collect_witnesses`).
- The engine no longer drops self-named guards (`dog(Dog)`) from rule antecedents and goals: the certifier kept them, so the engine proved rules stronger than the premises and the certifier rejected the proofs. Engine and certifier read every premise alike.
- A definite description presupposes its noun. Every lowercase constant that is also a noun (`butler`, "the butler"), meaning a predicate the problem never applies to more than one argument, yields `butler(butler)`. The engine takes it as a premise (`BackwardChainer::prove`/`prove_with_goal`, from the premises, the goal and its context, for that proof only), and the certifier registers it as a hypothesis (`verify`). Both read it from one symbol walk, `symbols::SymbolCollector`, moved out of `verify`. Proper names and nouns also used as relations presuppose nothing.
- The kernel files a predicate that shares its name with an individual under a name of its own (`symbols::KernelNames`, `is_<name>`). This includes a definiendum, whether or not the problem applies it. The renaming is applied to definitions, hypotheses, the goal type and the derivation at the verify door, so `butler(butler)` no longer applies the predicate to itself, and "a butler is one who serves" certifies beside "the butler". The engine and the returned derivation keep their names.
- `exprs_structurally_equal` matches every `ProofExpr` variant exhaustively, and each is equal to itself. A tensed proposition (`Past(see(butler))`) had been unequal to itself, so the ⊥ search never met it against its negation, and a rule concluding a tensed fact could not be refuted through a conjunct (`phase60_proof_engine::test_butler_did_it_theorem`). `Unsupported` stays equal to nothing.

- The certified solver validates a complete root-propagation model against every original clause before symmetry discovery. Duplicate literals retain unit-clause behavior, implication chains propagate to a fixed point, and inconclusive or conflicting inputs keep the SDCL/CDCL refutation path.
- **The certifier gives a term one sort across a statement.** `SymbolCollector` typed each occurrence on its own, so `f(3) = 3` declared `f : Entity → Entity` and then applied it to and equated it with an `Int`, a hypothesis whose type is not a type, which the kernel's assumption door now refuses. Sorts settle to a fixpoint (`settle_sorts`): the two sides of an equation share a sort, an argument position that takes an `Int` takes it at every application, and `register_function` declares each position's domain.
- **`decide` divides the way the kernels do.** `div`/`mod` are Euclidean, total and arbitrary-precision: `div(-7, 2)` is `-4`, and a zero divisor gives quotient `0` with the dividend as remainder. 0.10.1 divided with truncating `i64` arithmetic, answered `-3`, and got stuck at a zero divisor, which called true goals false and passed false ones on to the trust door. A goal that evaluates false is reported false, not undecided, and its negation is proved by `GroundRefutation`. `T_tactics_extra::decide_folds_division_the_way_all_four_kernel_engines_do` in `crates/logicaffeine_tests/tests/T_tactics_extra.rs`.
- **Integer literals past `i64` are numbers in every proof layer.** `decide`, the certifier, the domain classifier and the symbol collector share one numeral reading, so `9223372036854775808 + 1 = 9223372036854775809` is provable; the literal used to become an undeclared `Entity` global. `crates/logicaffeine_tests/unit/logicaffeine_proof/numeral/tests.rs`.
- **A theorem may mention a free variable.** A statement's free variables are registered as kernel locals typed by use: `Int` in arithmetic, the inductive a premise gives its unary predicate, and `Entity` otherwise. An `∃` binder takes the domain its body uses, so `∃m. m = n + 1` states and certifies. The independent re-checker is handed a closed term, so `double_check` no longer reports a disagreement over a valid proof by induction on a free local.
- **A development's definitions certify whatever order their atoms are collected in.** Definitions are registered after every atom their bodies mention, and a definiendum is never also declared opaque; the order of a hash map used to decide whether δ could unfold a definition. `definition_registration_lock`.

## [0.10.0] - 2026-07-08

### Added
- `satcli` module: the SAT command-line driver (DIMACS in, `s SATISFIABLE`/`s UNSATISFIABLE` + model out, DRAT/DPR/SR certificate export, competition exit codes) extracted from the `logos-sat` binary with injected output streams — shared verbatim by `logos-sat` and `largo sat`, and directly testable.
- Modal translation and independent verification.
- CDCL core with an incremental grid solver, grounding, and trust-tiers; label/PP convergence.

### Changed
- **`prove_unsat` — the expensive certified cuts are escalation-only.** The Lyapunov collapse, Nullstellensatz, Polynomial Calculus, and recursive certified-symmetry rungs ran on every call, taxing microsecond equivalence/BMC obligations ~50 ms each. The cheap structural recognizers stay upfront; the complete CDCL search then runs under a conflict budget (`EASY_SEARCH_CONFLICTS`), and only an instance that exhausts it escalates to the certified cuts, finishing on a fresh solver so the refutation certificate stays self-contained. Verdicts unchanged; the native-vs-Z3 speed locks hold again and the hard symmetric families still refute through the escalation path.

### Fixed
- **`cert_farkas` kernel reconstruction on doubled constants** — the proof-producing normalizer had no proof path for like monomials recombining to coefficient 1, and a merge that cancelled an entire prefix concluded with an unproven `add 0 x` residue the kernel rejected. Both close (locked by `arith::tests`), and `probe_double_constant_le_via_auto` runs un-ignored.

See the root CHANGELOG for the cross-crate history.

## [0.8.12] - 2026-02-14

Synced to workspace version 0.8.12. See root CHANGELOG for full history.

## [0.6.0] - 2026-01-17

Initial crates.io release.

### Added

- Backward-chaining proof engine implementing Curry-Howard correspondence
- Robinson's unification algorithm with occurs check
- Higher-order pattern unification (Miller patterns)
- Alpha-equivalence for quantifiers and lambda expressions
- Beta-reduction to weak head normal form
- Comprehensive inference rules (modus ponens, induction, etc.)
- Structural induction over inductive types
- Modal and temporal logic support
- Socratic hint generation for pedagogical guidance
- DerivationTree to Kernel Term certification
- Optional `verification` feature for Z3 oracle fallback
- Liskov Invariant: no dependency on language crate
