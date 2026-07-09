//! Structure-detecting solve front-end — the auto-dispatcher that puts our whole arsenal behind a
//! single entry point.
//!
//! An opaque CNF is offered, in cost order, to a battery of CHEAP (O(clauses)) structure
//! recognizers — each *polynomial* on structure that costs plain CDCL/resolution exponentially, and
//! each FAIL-CLOSED: it claims a verdict only with a re-checked certificate (a Hall witness, a
//! cutting-plane derivation, a GF(2) refutation, a covering-measure collapse, or — the complete
//! deciders — a 2-SAT/Horn model). Anything no recognizer fires on is decided by the authoritative
//! CDCL core. Every recognizer is cheap to apply AND to reject, so the dispatcher is never slower
//! than the plain solver — only faster when structure is present.
//!
//! Routes, in order:
//! 1. **2-SAT** (`twosat`) — every clause ≤ 2 literals: linear, decides SAT (with model) or UNSAT.
//! 2. **Horn** (`hornsat`) — every clause ≤ 1 positive literal: linear least-model / refutation.
//! 3. **LLL** (`lll`) — *satisfiability from sparsity*: when every clause shares variables with few
//!    others (`e·2^{-w}·(d+1) ≤ 1`), a model is guaranteed and constructed by Moser–Tardos. The
//!    SAT-side specialist, dual to the UNSAT recognizers below.
//! 4. **Pigeonhole / Hall** (`pigeonhole::decide_pigeonhole_unsat`) — bipartite-matching infeasibility:
//!    crushes pigeonhole (PHP) instances of any size in microseconds.
//! 5. **Cutting planes** (`pseudo_boolean::refute_clausal`) — cardinality refutation.
//! 6. **Parity / GF(2)** (`xorsat::refute_via_parity`) — XOR/Tseitin linear systems.
//! 7. **Covering collapse** (`lyapunov::auto_collapse`) — auto-discovers the covering symmetry
//!    (pigeonhole, clique-colouring, …) or parity collapse with a Lyapunov-certified artifact.
//! 8. **CDCL** — the authoritative fallback (model on SAT, RUP proof on UNSAT).

use std::collections::HashMap;
use std::collections::HashSet;

use crate::cdcl::{BudgetedResult, Lit, SolveResult, Solver};
use crate::lyapunov::{auto_collapse, extract_xor, AutoCollapse};
use crate::proof::ProofStep;
use crate::xor_engine::IncXor;
use crate::xorsat::XorOutcome;
use crate::ProofExpr;

// NOTE: generic graph-automorphism detection (`symmetry_detect::find_generators`) and its SEL search
// are deliberately NOT on this hot path — `find_generators` does not scale (measured ~57s on a
// 1359-variable circuit instance CDCL solves in 4ms), and SEL spins on easy SAT instances. They are
// *discovery* tools, not competition moves: run offline to learn a family's symmetry, then distill
// it into a cheap O(clauses) fingerprint here. Everything below is O(clauses) to apply and reject,
// so the dispatcher is never slower than the plain CDCL solver.

/// Which engine decided the instance.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Route {
    TwoSat,
    Horn,
    Lll,
    Pigeonhole,
    CuttingPlanes,
    Parity,
    /// Exactly-one groups harvested from the raw clauses (all-positive clause + full pairwise
    /// at-most-one) yield `Σ_{v∈g} x_v = 1` over every modulus; Gaussian elimination over small
    /// fields finds the counting obstruction — the parity sum, the signed bipartite combination —
    /// with the refutation re-checked fail-closed. The covering-encoded counting crusher.
    ExactCover,
    ModP,
    ModM,
    Collapse,
    HybridXor,
    Sos,
    Nullstellensatz,
    SymmetryBreak,
    NestedSymmetry,
    Sel,
    LocalSymmetry,
    OrbitalBranch,
    SymmetricProbe,
    SymmetricBinary,
    OrbitWeightQuotient,
    SymmetryPropagate,
    SymmetricComponent,
    SymmetrySimplify,
    SemanticSymmetry,
    AlmostSymmetry,
    DeclaredSymmetry,
    RecursiveBreak,
    /// The formula is certified to carry no linear/parity symmetry shortcut and is provably rigid, so
    /// the symmetry arsenal is useless — decided by CDCL with that honest "no shortcut" verdict on record.
    Incompressible,
    /// The formula split into independent components with no symmetry relating them; each was solved apart
    /// through the full arsenal and the verdicts combined (the plain-decomposition analogue of separability).
    Component,
    /// Certified bounded variable elimination (Davis–Putnam, non-growing) reduced the formula to `⊥`.
    /// The missing preprocessing crusher: it refutes bounded-treewidth cores the symmetry/algebra chain
    /// declared `Incompressible`. Its RUP resolvents + clause deletions are the independently checkable proof.
    BoundedVarElim,
    /// The binary-implication graph's SCCs forced `x ≡ ¬x` — the 2-clauses alone refute the formula. Catches
    /// general (non-pure-2SAT) formulas whose binary sub-part is contradictory, which the pure-TwoSat route
    /// misses. Certified by a short RUP chain (`(x)`, `(¬x)`, `⊥`), re-checkable by propagation.
    EquivLit,
    /// Davis–Putnam bucket elimination refuted the formula with every resolvent width ≤ a cap — a bounded-
    /// treewidth resolution certificate (`2^w·n`). Covers the medium-treewidth families that BVE's non-growing
    /// rule misses; declines (leaving `Incompressible`) on the high-treewidth residue where width would blow up.
    TreeWidth,
    Cdcl,
}

/// The verdict; a model on SAT, with any UNSAT certificate carried in [`Solved::proof`].
#[derive(Clone, Debug)]
pub enum Answer {
    Sat(Vec<bool>),
    Unsat,
}

/// A decided instance.
#[derive(Clone, Debug)]
pub struct Solved {
    pub answer: Answer,
    pub via: Route,
    /// UNSAT certificate as a proof stream where one exists (RUP for the CDCL route); empty for the
    /// polynomial specialists, which certify internally.
    pub proof: Vec<ProofStep>,
    /// CDCL conflicts spent (0 whenever a specialist collapsed the instance without search).
    pub conflicts: u64,
}

impl Solved {
    fn unsat(via: Route) -> Self {
        Solved { answer: Answer::Unsat, via, proof: Vec::new(), conflicts: 0 }
    }
}

/// Decide `clauses` over `num_vars` variables, routing through every cheap specialist before CDCL — the
/// fast default front-end. For the full arsenal (heavy algebraic + complete symmetry breaking before the
/// fallback) use [`solve_comprehensive`].
pub fn solve_structured(num_vars: usize, clauses: &[Vec<Lit>]) -> Solved {
    structured_prefix(num_vars, clauses).unwrap_or_else(|| cdcl_fallback(num_vars, clauses))
}

/// The cheap-specialist chain — every route O(clauses) to apply and reject, so the front-end is never
/// slower than plain CDCL. Returns the decision if a specialist fires, or `None` (the CDCL fallback is
/// needed). Public so callers that only want the O(clauses) specialist verdict — e.g. a per-node
/// cofactor-DAG leaf check — can skip the expensive CDCL search entirely.
pub fn structured_prefix(num_vars: usize, clauses: &[Vec<Lit>]) -> Option<Solved> {
    // 1. 2-SAT: a complete polynomial decision procedure with a model.
    if let Some(binary) = as_two_sat(clauses) {
        return Some(match crate::twosat::solve(&binary, num_vars) {
            crate::twosat::TwoSatOutcome::Sat(model) => {
                Solved { answer: Answer::Sat(model), via: Route::TwoSat, proof: Vec::new(), conflicts: 0 }
            }
            crate::twosat::TwoSatOutcome::Unsat(_) => Solved::unsat(Route::TwoSat),
        });
    }

    // 2. Horn: linear least-model / forward-chaining refutation.
    if let Some(horn) = as_horn(clauses) {
        return Some(match crate::hornsat::solve(&horn, num_vars) {
            crate::hornsat::HornOutcome::Sat(model) => {
                Solved { answer: Answer::Sat(model), via: Route::Horn, proof: Vec::new(), conflicts: 0 }
            }
            crate::hornsat::HornOutcome::Unsat(_) => Solved::unsat(Route::Horn),
        });
    }

    // 3. LLL: satisfiability from sparsity. If the local-lemma condition holds a model is guaranteed;
    //    construct it with Moser–Tardos. The check is O(clauses) and an empty clause fails it, so this
    //    only fires on genuinely sparse (SAT) instances and never claims SAT without a re-checked model.
    if !clauses.is_empty() && crate::lll::lll_certifies_sat(clauses).is_some() {
        let budget = 1000 + 64 * clauses.len();
        if let Some(model) = crate::lll::moser_tardos_witness(num_vars, clauses, 0x10C0_5EED_C0DE_F00D, budget)
        {
            // Fail-closed: only accept a witness that genuinely satisfies every clause.
            if clauses
                .iter()
                .all(|c| c.iter().any(|l| model.get(l.var() as usize).copied().unwrap_or(false) == l.is_positive()))
            {
                return Some(Solved { answer: Answer::Sat(model), via: Route::Lll, proof: Vec::new(), conflicts: 0 });
            }
        }
    }

    // 4-6. Polynomial UNSAT recognizers over the formula's ProofExpr view.
    if let Some(expr) = cnf_to_expr(clauses) {
        if crate::pigeonhole::decide_pigeonhole_unsat(&expr) {
            return Some(Solved::unsat(Route::Pigeonhole));
        }
        if crate::pseudo_boolean::refute_clausal(&expr) {
            return Some(Solved::unsat(Route::CuttingPlanes));
        }
        if crate::xorsat::refute_via_parity(&expr) {
            return Some(Solved::unsat(Route::Parity));
        }
    }

    // 6¼. Exact-cover lift: harvest exactly-one groups (covering encodings — modular counting,
    //     domino tilings) and Gaussian-eliminate their `Σ x = 1` equations over GF(2), GF(3), GF(5).
    //     The equations are consequences over EVERY modulus, so any inconsistency — re-checked
    //     fail-closed — is a certified UNSAT with zero search. Never claims SAT (declines onward).
    if let Some(solved) = exact_cover_route(num_vars, clauses) {
        return Some(solved);
    }

    // 6½. GF(p)/ℤ/m lift: recover a mod-m one-hot system from the raw clauses and decide it by Gaussian
    //     elimination over the right field/ring — the parity cut carried to every modulus. Crushes the
    //     mod-m counting obstruction that GF(2) is blind to and resolution (CDCL, Z3, Kissat) needs
    //     2^Ω(n) for. Sound: UNSAT carries the certified refutation; a SAT model is re-checked.
    if let Some(solved) = modp_route(num_vars, clauses) {
        return Some(solved);
    }

    // 6. Covering / algebraic collapse: auto-discover the symmetry or parity that flattens it.
    match auto_collapse(num_vars, clauses) {
        AutoCollapse::None => {}
        // Any recognized collapse (covering symmetry, parity, cardinality, …) is a certified UNSAT.
        _ => return Some(Solved::unsat(Route::Collapse)),
    }

    // 7. Hybrid XOR: for XOR-heavy formulas (e.g. parity-learning) that are neither pure-XOR nor
    //    caught above, solve the recovered GF(2) subsystem and seed CDCL with that assignment so it
    //    starts on the linear-system's solution manifold and only repairs the residual clauses.
    if let Some(solved) = hybrid_xor(num_vars, clauses) {
        return Some(solved);
    }

    // 7½. Sum-of-Squares / Positivstellensatz: a degree-2 algebraic refutation over ℚ for a small core
    //     no cheaper recognizer caught — the ordered-field cut beyond the linear/parity engines (it
    //     closes integrality gaps GF(2) cannot see). Sound and bounded (num_vars ≤ 6). A certified-
    //     specialist backstop; the standalone engine (`crate::sos`) is where its degree-2 power lives.
    if num_vars <= 6 && crate::sos::sos_refutes(num_vars, clauses) {
        return Some(Solved::unsat(Route::Sos));
    }

    // 7¾. Bounded-degree **Nullstellensatz (degree ≤ 3)** — the algebraic obstruction *beyond* SoS's
    //      degree-2. The residue-map census showed the symmetric-but-uncrushed families sit at
    //      Nullstellensatz degree 3: their symmetry is *already fully broken* (0 underbroken), so no
    //      symmetry route can touch them — the only remaining lens is the deeper algebraic one. Small-core
    //      gated (num_vars ≤ 12) so the fast router stays bounded; sound (a genuine degree-d NS refutation).
    if num_vars <= 12 {
        for d in 2..=num_vars.min(3) {
            if crate::polycalc::nullstellensatz_refutes(num_vars, clauses, d) {
                return Some(Solved::unsat(Route::Nullstellensatz));
            }
        }
    }

    None
}

/// **The exact-cover lift.** Harvest every exactly-one group — an all-positive clause of size ≥ 2
/// whose variables are pairwise at-most-one-forbidden — and read off the equation `Σ_{v∈g} x_v = 1`.
/// Exactly-one semantics makes the equation a valid consequence over *every* modulus, so the group
/// system is checked for inconsistency by Gaussian elimination over `GF(2)` (via `xorsat`) and
/// `GF(3)`, `GF(5)` (via `modp`); every refutation is re-checked fail-closed before the route
/// reports. This is the covering-encoded face of the counting cut: `Count_2`'s parity sum, the
/// mutilated chessboard's black-minus-white combination (a `GF(3)` linear dependency the Gaussian
/// finds unaided), the mod-3/5 sums of `Count_{3,5}`. UNSAT-only: a consistent system proves
/// nothing about the rest of the formula, so the route declines rather than guess.
fn exact_cover_route(num_vars: usize, clauses: &[Vec<Lit>]) -> Option<Solved> {
    use std::collections::HashSet;
    let mut amo: HashSet<(u32, u32)> = HashSet::new();
    for c in clauses {
        if c.len() == 2 && c.iter().all(|l| !l.is_positive()) {
            let (a, b) = (c[0].var(), c[1].var());
            amo.insert((a.min(b), a.max(b)));
        }
    }
    if amo.is_empty() {
        return None;
    }
    let mut groups: Vec<Vec<u32>> = Vec::new();
    for c in clauses {
        if c.len() < 2 || !c.iter().all(|l| l.is_positive()) {
            continue;
        }
        let mut g: Vec<u32> = c.iter().map(|l| l.var()).collect();
        g.sort_unstable();
        g.dedup();
        if g.len() != c.len() {
            continue;
        }
        let full_amo = (0..g.len())
            .all(|i| (i + 1..g.len()).all(|j| amo.contains(&(g[i], g[j]))));
        if full_amo {
            groups.push(g);
        }
    }
    if groups.len() < 2 {
        return None; // one equation is never inconsistent — nothing to combine
    }
    // GF(2): the parity rung, certificate re-checked by the XOR checker.
    let eqs: Vec<crate::xorsat::XorEquation> = groups
        .iter()
        .map(|g| crate::xorsat::XorEquation::new(g.iter().map(|&v| v as usize).collect::<Vec<_>>(), true))
        .collect();
    if let crate::xorsat::XorOutcome::Unsat(refutation) = crate::xorsat::solve(&eqs, num_vars) {
        if crate::xorsat::is_refutation(&eqs, num_vars, &refutation) {
            return Some(Solved::unsat(Route::ExactCover));
        }
    }
    // GF(3), GF(5): the higher rungs of the same harvest, re-checked by the mod-p checker.
    for p in [3u64, 5] {
        let meqs: Vec<crate::modp::ModpEquation> = groups
            .iter()
            .map(|g| {
                crate::modp::ModpEquation::new(
                    g.iter().map(|&v| (v as usize, 1u64)).collect::<Vec<_>>(),
                    1,
                )
            })
            .collect();
        if let crate::modp::ModpOutcome::Unsat(combo) = crate::modp::solve(&meqs, num_vars, p) {
            if crate::modp::is_refutation(&meqs, num_vars, p, &combo) {
                return Some(Solved::unsat(Route::ExactCover));
            }
        }
    }
    None
}

/// The authoritative CDCL fallback — enriched with the mined clause bundle: every structure-mining
/// contributor's implied no-goods, unioned into the formula so CDCL inherits the discovered structure.
/// Sound (only implied clauses) and never-worse.
fn cdcl_fallback(num_vars: usize, clauses: &[Vec<Lit>]) -> Solved {
    let mut solver = Solver::new(num_vars);
    for c in clauses {
        solver.add_clause(c.clone());
    }
    for c in mine_clauses(num_vars, clauses) {
        solver.add_clause(c);
    }
    match solver.solve() {
        SolveResult::Sat(model) => {
            Solved { answer: Answer::Sat(model), via: Route::Cdcl, proof: Vec::new(), conflicts: solver.conflicts() }
        }
        SolveResult::Unsat => {
            let proof = solver.learned().iter().map(|lc| ProofStep::Rup(lc.lits.clone())).collect();
            Solved { answer: Answer::Unsat, via: Route::Cdcl, proof, conflicts: solver.conflicts() }
        }
    }
}

/// **The full arsenal in one call** — the opt-in power-mode solver. Runs the cheap specialists
/// ([`structured_prefix`]); then, *before* the exponential CDCL fallback, the heavier engines the fast
/// path skips: bounded-degree **Nullstellensatz / Polynomial Calculus** (the nonlinear algebraic
/// refutation, [`crate::polycalc`]) and **complete lex-leader symmetry breaking** ([`crate::sym_break`]:
/// the whole automorphism group from the Schreier–Sims backend, feeding CDCL a one-model-per-orbit
/// formula). Slower than [`solve_structured`] (it may run Nullstellensatz and automorphism detection),
/// so it is the maximum-power entry, not the default. Always correct — the CDCL fallback is complete.
pub fn solve_comprehensive(num_vars: usize, clauses: &[Vec<Lit>]) -> Solved {
    if let Some(s) = structured_prefix(num_vars, clauses) {
        return s;
    }
    // Certified bounded variable elimination: eliminate every non-growing variable (Davis–Putnam) to a
    // fixpoint. If the residue contains `⊥`, that IS a certified (RUP) refutation — the bounded-treewidth
    // crusher the fast chain lacks. Runs before the incompressibility verdict so bve-easy cores are decided
    // honestly (they were never truly rigid-hard) rather than mislabeled `Incompressible`.
    // Certified equivalent-literal detection: the binary-implication graph's SCCs. If some `x ≡ ¬x` the
    // 2-clauses alone refute the formula (a contradictory binary sub-part the pure-TwoSat route, which needs a
    // fully-binary formula, does not see). Cheap (Tarjan on the 2-clauses) and certified by a short RUP chain.
    if let crate::inprocess::EquivResult::Unsat(steps) = crate::inprocess::equivalent_literal_scc(num_vars, clauses) {
        return Solved { answer: Answer::Unsat, via: Route::EquivLit, proof: steps, conflicts: 0 };
    }
    let (reduced, steps) = crate::inprocess::bve(num_vars, clauses);
    if reduced.iter().any(|c| c.is_empty()) {
        return Solved { answer: Answer::Unsat, via: Route::BoundedVarElim, proof: steps, conflicts: 0 };
    }
    // Certified bucket elimination (tree-width): full Davis–Putnam in min-degree order, capping resolvent width.
    // A refutation within the cap is a `2^cap·n` resolution certificate — crushes bounded-treewidth families
    // BVE's non-growing rule misses. Declines on the high-treewidth residue (width exceeds the cap).
    if let Some(steps) = crate::inprocess::bucket_elimination_refute(num_vars, clauses, 12) {
        return Solved { answer: Answer::Unsat, via: Route::TreeWidth, proof: steps, conflicts: 0 };
    }
    // Certified incompressibility: if the formula's parity structure is fully exposed AND it is provably
    // rigid (|Aut| = 1, no symmetry to exploit — the exact check is size-gated so it stays cheap), then
    // the entire symmetry arsenal below is provably useless on it. Decide it with CDCL and record the
    // honest "no shortcut of this class" route. Fail-closed: any doubt falls through unchanged.
    if crate::ait::incompressibility_gate(num_vars, clauses).is_some() {
        let mut solved = cdcl_fallback(num_vars, clauses);
        solved.via = Route::Incompressible;
        return solved;
    }
    // Symmetric COMPONENT decomposition: when the formula splits into independent components and the
    // automorphism group maps some onto each other (isomorphic copies), solve one representative per
    // component-orbit and replicate its model through the symmetry — `k` identical sub-problems for the
    // price of one. Each representative goes back through the full arsenal recursively.
    if let Some(s) = symmetric_component_solve(num_vars, clauses) {
        return s;
    }
    // Symmetry hidden by units: propagate the formula's unit clauses, then detect symmetry on the
    // SIMPLIFIED residual. Units can mask automorphisms the raw-formula routes structurally cannot see;
    // once revealed, the residual is solved with the full arsenal and the forced assignment re-applied.
    if let Some(s) = symmetry_via_simplification_solve(num_vars, clauses) {
        return s;
    }
    // Orbit-weight QUOTIENT: when the symmetry group is the full product of symmetric groups on its
    // orbits, satisfiability depends only on the per-orbit weights — so the whole 2ⁿ space collapses to
    // Π(|Oᵢ|+1) weight-tuple representatives, decided exactly by evaluation. Complete, not just sound.
    if let Some(s) = orbit_weight_quotient_solve(num_vars, clauses) {
        return s;
    }
    // Nonlinear algebraic: a degree-d Nullstellensatz refutation (subsumes parity at d=1; d ≥ 2 reaches
    // counting-style obstructions). Bounded to the explicit-monomial regime.
    if num_vars <= 16 {
        for d in 2..=num_vars.min(3) {
            if crate::polycalc::nullstellensatz_refutes(num_vars, clauses, d) {
                return Solved::unsat(Route::Nullstellensatz);
            }
        }
    }
    // Dynamic in-CDCL symmetry breaking — Symmetric Explanation Learning. Amplify each learned clause by
    // the symmetry group during a budgeted search, so symmetric conflicts are never re-derived (the
    // conflict count collapses on symmetry-rich instances). For symmetric UNSAT it usually wins outright;
    // a SAT model or an honest Unknown falls through to the static / complete routes.
    if let Some(s) = dynamic_sel(num_vars, clauses) {
        return s;
    }
    // Symmetric inference (not breaking): probe one representative per literal-orbit for a failed literal
    // (`F ∧ ℓ` UNSAT ⟹ `F ⊨ ¬ℓ`); symmetry then forces the *whole orbit* false from that single probe.
    // The strengthened formula is solved directly. Sound — it only adds implied units.
    if let Some(s) = symmetric_probe_solve(num_vars, clauses) {
        return s;
    }
    // Symmetric hyper-binary inference: where the probe derives a UNIT (a failed literal), this derives an
    // IMPLICATION — BCP under `ℓ` forcing `m` means `F ⊨ ℓ→m` — and symmetry adds the whole orbit of that
    // binary clause from one probe. Sound (only implied clauses); strengthens the formula, then solves.
    if let Some(s) = symmetric_binary_inference_solve(num_vars, clauses) {
        return s;
    }
    // Nested (multi-dimensional) symmetry: a grid of three or more axes has a TOWER of block systems
    // (cells ⊂ lines ⊂ planes ⊂ …), not just one. Break structured swaps at every level of the tower,
    // each verified to lie in the group — the d-dimensional generalisation of the 2-D double-lex break.
    if let Some(s) = nested_symmetry_solve(num_vars, clauses) {
        return s;
    }
    // Orbital branching — break symmetry in the decision tree, not by clauses. When a large variable orbit
    // exists, fixing one representative collapses all its symmetric "some-member-true" branches into one,
    // so the search explores a single branch where a complete lex-leader would still enumerate the orbit.
    if let Some(s) = orbital_branch_solve(num_vars, clauses) {
        return s;
    }
    // Symmetry breaking DURING search (SBDS): a lex-leader propagator enforces `a ≤ₗₑₓ a∘g` for each
    // generator dynamically through the DPLL(T) interface — no static SBP clauses, no aux variables.
    if let Some(s) = symmetry_propagate_solve(num_vars, clauses) {
        return s;
    }
    // Static complete/partial symmetry breaking (handles SAT, and is complete): add the lex-leader
    // predicate and let CDCL decide the broken formula.
    if let Some(s) = symmetry_break_solve(num_vars, clauses) {
        return s;
    }
    // Local symmetry breaking: split on a variable whose residuals carry the most conditional symmetry,
    // break each residual's symmetry, and solve the branches — exploiting symmetry that emerges only down
    // a branch, invisible to the global routes above.
    if let Some(s) = local_symmetry_solve(num_vars, clauses) {
        return s;
    }
    // Semantic symmetry: a permutation that preserves the MODEL SET (`F ≡ σ(F)`) without preserving the
    // clause set — invisible to the syntactic detector. Detected by logical implication, broken by the
    // (sound-for-any-model-preserving-permutation) lex-leader. The last symmetry route before search.
    if let Some(s) = semantic_symmetry_solve(num_vars, clauses) {
        return s;
    }
    // Almost-symmetry: a swap that preserves all but a few clauses is an automorphism of the rest, so it
    // maps a model to a model only when the broken clauses' images also hold — break it CONDITIONALLY,
    // guarded by those images. The weakest, most general symmetry; the last route before search.
    if let Some(s) = almost_symmetry_solve(num_vars, clauses) {
        return s;
    }
    // The hard ones: nothing above decided it. Try the RECURSIVE breaker — iterate detect-and-break to a
    // fixpoint (composing symmetry that earlier single passes leave behind), then decide the reduced
    // formula by search. Fires only when it actually breaks symmetry; otherwise the plain CDCL fallback.
    if let Some(s) = recursive_break_solve(num_vars, clauses) {
        return s;
    }
    // Fused parity + cardinality — applied BEFORE the affine reduction so a *mixed* instance (one carrying
    // BOTH a recovered GF(2) parity substructure AND an at-most-one cardinality substructure) is decided by
    // the two live theories reasoning together on one trail, rather than letting the affine rung reduce only
    // its linear half. Gated on both substructures, so pure-parity / pure-cardinality instances fall through
    // untouched to the affine and covering routes below.
    if let Some(s) = fused_modular_solve(num_vars, clauses) {
        return s;
    }
    // Affine (GF(2)) reduction — the symmetry every breaker above is *structurally* blind to: a shear
    // `xᵢ↦xᵢ⊕xⱼ` maps a clause to a non-subcube, so no clause permutation can reach it. Recover the
    // formula's linear substructure and Gauss-eliminate it — an inconsistent linear core refutes outright
    // (a GF(2) obstruction, hence Route::Parity), and otherwise the derived forced units / equivalences
    // (linear consequences no single clause states) strengthen the formula for the CDCL fallback.
    match crate::affine::affine_canonicalize(num_vars, clauses) {
        crate::affine::AffineCanon::Refuted(drat) => {
            // Carry the xor_drat certificate: the GF(2) linear-dependency refutation compiled to RUP
            // resolvents, drat-trim-checkable against the original CNF (None only if the resolution
            // expansion overran its budget — the verdict still stands on the algebraic certificate).
            let proof = drat.map(|res| res.into_iter().map(ProofStep::Rup).collect()).unwrap_or_default();
            return Solved { answer: Answer::Unsat, via: Route::Parity, proof, conflicts: 0 };
        }
        crate::affine::AffineCanon::Canonical(canon) => {
            // The canonical RREF break: solve the formula reduced to its free generators, then lift the
            // model back through the affine quotient. UNSAT of the reduced ⇒ UNSAT of the original.
            let sub = cdcl_fallback(canon.num_vars, &canon.clauses);
            return match sub.answer {
                Answer::Unsat => Solved { answer: Answer::Unsat, via: sub.via, proof: Vec::new(), conflicts: sub.conflicts },
                Answer::Sat(model) => {
                    let lifted = canon.lift(&model);
                    // Fail-closed: the lifted model must satisfy the original formula.
                    if clauses.iter().all(|c| c.iter().any(|l| lifted[l.var() as usize] == l.is_positive())) {
                        Solved { answer: Answer::Sat(lifted), via: sub.via, proof: Vec::new(), conflicts: sub.conflicts }
                    } else {
                        cdcl_fallback(num_vars, clauses) // defensive: re-solve raw (unreachable for a sound lift)
                    }
                }
            };
        }
        crate::affine::AffineCanon::Unchanged => {}
    }
    // The GF(p) affine break — the one-hot mod-p analogue of the GF(2) RREF reduction above. An
    // inconsistent mod-p core refutes (Route::ModP); otherwise *eliminate* the determined one-hot groups
    // (forced bits → constants, value-permuted linked bits → aliases), solve the reduced formula, and lift
    // the model back through the affine quotient (fail-closed). UNSAT of the reduced ⇒ UNSAT of the original.
    match crate::affine_gfp::affine_p_canonicalize(num_vars, clauses) {
        crate::affine_gfp::AffinePCanon::Refuted(drat) => {
            let proof = drat.map(|res| res.into_iter().map(ProofStep::Rup).collect()).unwrap_or_default();
            return Solved { answer: Answer::Unsat, via: Route::ModP, proof, conflicts: 0 };
        }
        crate::affine_gfp::AffinePCanon::Canonical(canon) => {
            let sub = cdcl_fallback(canon.num_vars, &canon.clauses);
            return match sub.answer {
                Answer::Unsat => Solved { answer: Answer::Unsat, via: sub.via, proof: Vec::new(), conflicts: sub.conflicts },
                Answer::Sat(model) => {
                    let lifted = canon.lift(&model);
                    if clauses.iter().all(|c| c.iter().any(|l| lifted[l.var() as usize] == l.is_positive())) {
                        Solved { answer: Answer::Sat(lifted), via: sub.via, proof: Vec::new(), conflicts: sub.conflicts }
                    } else {
                        cdcl_fallback(num_vars, clauses) // defensive: re-solve raw (unreachable for a sound lift)
                    }
                }
            };
        }
        crate::affine_gfp::AffinePCanon::Unchanged => {}
    }
    // The composite ℤ/m affine break — the same eliminate-and-lift move over a one-hot encoding with a
    // composite modulus, decomposed by CRT into the prime-power components (Smith normal form per ring). An
    // inconsistent ring core refutes (Route::ModM); otherwise the forced / partially-forced / ring-linked
    // groups are eliminated, the reduced formula solved, and its model lifted (fail-closed).
    match crate::affine_gfp::affine_m_canonicalize(num_vars, clauses) {
        crate::affine_gfp::AffinePCanon::Refuted(drat) => {
            let proof = drat.map(|res| res.into_iter().map(ProofStep::Rup).collect()).unwrap_or_default();
            return Solved { answer: Answer::Unsat, via: Route::ModM, proof, conflicts: 0 };
        }
        crate::affine_gfp::AffinePCanon::Canonical(canon) => {
            let sub = cdcl_fallback(canon.num_vars, &canon.clauses);
            return match sub.answer {
                Answer::Unsat => Solved { answer: Answer::Unsat, via: sub.via, proof: Vec::new(), conflicts: sub.conflicts },
                Answer::Sat(model) => {
                    let lifted = canon.lift(&model);
                    if clauses.iter().all(|c| c.iter().any(|l| lifted[l.var() as usize] == l.is_positive())) {
                        Solved { answer: Answer::Sat(lifted), via: sub.via, proof: Vec::new(), conflicts: sub.conflicts }
                    } else {
                        cdcl_fallback(num_vars, clauses) // defensive: re-solve raw (unreachable for a sound lift)
                    }
                }
            };
        }
        crate::affine_gfp::AffinePCanon::Unchanged => {}
    }
    // Plain component decomposition: nothing above split it, but if the formula still separates into
    // independent components, solve each apart through the full arsenal — a component may match a route the
    // whole formula did not — and combine, fail-closed. The last structural move before raw search.
    if let Some(s) = component_solve(num_vars, clauses) {
        return s;
    }
    cdcl_fallback(num_vars, clauses)
}

/// Decide via the **recursive breaker** as a dispatcher route for hard instances: iterate symmetry
/// detection and breaking to a fixpoint ([`break_all_symmetry`], aux-free), then solve the reduced formula
/// with CDCL. Sound — the breaker is equisatisfiable, so a model satisfies the original (re-checked
/// fail-closed) and an UNSAT verdict carries over. `None` when no symmetry is broken (the plain fallback
/// then handles it).
fn recursive_break_solve(num_vars: usize, clauses: &[Vec<Lit>]) -> Option<Solved> {
    if num_vars == 0 || num_vars > 64 {
        return None;
    }
    let broken = break_all_symmetry(num_vars, clauses);
    if broken.len() == clauses.len() {
        return None; // nothing broken ⇒ defer to the plain CDCL fallback
    }
    let mut solver = Solver::new(num_vars);
    for c in &broken {
        solver.add_clause(c.clone());
    }
    match solver.solve() {
        SolveResult::Sat(model) => {
            let projected: Vec<bool> = model[..num_vars].to_vec();
            clauses
                .iter()
                .all(|c| c.iter().any(|l| projected[l.var() as usize] == l.is_positive()))
                .then_some(Solved {
                    answer: Answer::Sat(projected),
                    via: Route::RecursiveBreak,
                    proof: Vec::new(),
                    conflicts: solver.conflicts(),
                })
        }
        SolveResult::Unsat => {
            Some(Solved { answer: Answer::Unsat, via: Route::RecursiveBreak, proof: Vec::new(), conflicts: solver.conflicts() })
        }
    }
}

/// Decide by a one-level **local-symmetry branch**: pick the variable whose two residuals carry the most
/// conditional symmetry (fixing that variable), split on it, break each residual's local symmetry, and
/// solve the branches with CDCL. `F` is SAT iff either branch is. Sound: a residual symmetry `σ` is used
/// only if it FIXES the branched variable (so it permutes that branch's models), making the lex-leader
/// sound for the branch. `None` if no branch reveals usable local symmetry (CDCL handles it) or the
/// instance is too large for the per-variable residual scan.
fn local_symmetry_solve(num_vars: usize, clauses: &[Vec<Lit>]) -> Option<Solved> {
    if num_vars == 0 || num_vars > 24 {
        return None;
    }
    // Residual symmetry generators for the branch `lit`, keeping only those that fix the branched variable.
    let fixed_gens = |lit: Lit| -> Vec<Vec<Lit>> {
        let v = lit.var() as usize;
        crate::sym_break::conditional_symmetry_generators(num_vars, clauses, &[lit])
            .into_iter()
            .filter(|img| img[v] == Lit::pos(v as u32))
            .collect()
    };
    let mut best: Option<usize> = None;
    let mut best_score = 0usize;
    for v in 0..num_vars {
        let score = fixed_gens(Lit::pos(v as u32)).len() + fixed_gens(Lit::neg(v as u32)).len();
        if score > best_score {
            best_score = score;
            best = Some(v);
        }
    }
    let v = best?; // no branch reveals usable local symmetry

    let mut conflicts = 0u64;
    for polarity in [false, true] {
        let lit = Lit::new(v as u32, polarity);
        let (sbp, total) = crate::sym_break::lex_leader_sbp_lit(num_vars, &fixed_gens(lit));
        let mut solver = Solver::new(total.max(num_vars));
        for c in clauses {
            solver.add_clause(c.clone());
        }
        solver.add_clause(vec![lit]); // commit the branch
        for c in &sbp {
            solver.add_clause(c.clone());
        }
        match solver.solve() {
            SolveResult::Sat(model) => {
                return Some(Solved {
                    answer: Answer::Sat(model[..num_vars].to_vec()),
                    via: Route::LocalSymmetry,
                    proof: Vec::new(),
                    conflicts: conflicts + solver.conflicts(),
                });
            }
            SolveResult::Unsat => conflicts += solver.conflicts(),
        }
    }
    Some(Solved { answer: Answer::Unsat, via: Route::LocalSymmetry, proof: Vec::new(), conflicts })
}

/// Dynamic in-CDCL symmetry breaking — Symmetric Explanation Learning ([`crate::sym_dynamic::sel_refute`]).
/// During a budgeted CDCL search, each learned clause is multiplied by the formula's symmetry group, so
/// symmetric conflicts are learned for free and symmetric subtrees are never re-explored. Sound:
/// UNSAT carries a checked PR/RUP proof, SAT a model, else an honest Unknown. Gated by a quick symmetry
/// check (skip if the formula is asymmetric — SEL would just burn budget) and `num_vars ≤ 64`.
fn dynamic_sel(num_vars: usize, clauses: &[Vec<Lit>]) -> Option<Solved> {
    use crate::sym_dynamic::{sel_refute, SelOutcome};
    if num_vars > 64 || crate::sym_break::literal_automorphism_generators(num_vars, clauses).is_empty() {
        return None;
    }
    match sel_refute(num_vars, clauses) {
        SelOutcome::Unsat { steps, conflicts, .. } => {
            Some(Solved { answer: Answer::Unsat, via: Route::Sel, proof: steps, conflicts })
        }
        SelOutcome::Sat(model) => {
            Some(Solved { answer: Answer::Sat(model), via: Route::Sel, proof: Vec::new(), conflicts: 0 })
        }
        SelOutcome::Unknown { .. } => None,
    }
}

/// Decide via complete symmetry breaking: detect the variable-automorphism group (Schreier–Sims
/// backend), add the complete lex-leader SBP, and run CDCL on the broken formula. `None` when there is no
/// usable (phase-free, moderate, non-trivial) symmetry, the instance is too large for automorphism
/// detection, or the model fails the fail-closed re-check.
fn symmetry_break_solve(num_vars: usize, clauses: &[Vec<Lit>]) -> Option<Solved> {
    if num_vars > 64 {
        return None; // automorphism detection does not scale; leave it to CDCL
    }
    // Literal symmetries — variable AND value/phase — as image-literal vectors, on the 2·num_vars points.
    let gens = crate::sym_break::literal_automorphism_generators(num_vars, clauses);
    if gens.is_empty() {
        return None; // no non-trivial symmetry
    }
    let point_gens: Vec<_> =
        gens.iter().map(|s| crate::sym_break::litsym_to_points(s, num_vars)).collect();
    let bsgs = crate::permgroup::schreier_sims(2 * num_vars, &point_gens);
    if bsgs.order() <= 1 {
        return None;
    }
    // Choose the break by group size and structure:
    //   • small group       → COMPLETE break (exactly one model per orbit, optimal);
    //   • large grid         → HIERARCHICAL block-wise break (polynomial, structured — for the imprimitive
    //                          product symmetries the complete enumeration could never touch);
    //   • large non-grid     → stabilizer-chain break (generators ∪ transversal coset reps, polynomial).
    let (sbp, total) = match bsgs.elements(50_000) {
        Some(pts) => crate::sym_break::lex_leader_sbp_lit(
            num_vars,
            &pts.iter().map(|p| crate::sym_break::litsym_from_points(p, num_vars)).collect::<Vec<_>>(),
        ),
        None => crate::sym_break::hierarchical_break(num_vars, clauses).unwrap_or_else(|| {
            let mut s = gens;
            s.extend(
                bsgs.transversal_elements().iter().map(|p| crate::sym_break::litsym_from_points(p, num_vars)),
            );
            crate::sym_break::lex_leader_sbp_lit(num_vars, &s)
        }),
    };
    let mut solver = Solver::new(total);
    for c in clauses {
        solver.add_clause(c.clone());
    }
    for c in &sbp {
        solver.add_clause(c.clone());
    }
    match solver.solve() {
        SolveResult::Sat(model) => {
            let projected: Vec<bool> = model[..num_vars].to_vec();
            // Fail-closed: the projection must satisfy the original formula.
            clauses
                .iter()
                .all(|c| c.iter().any(|l| projected[l.var() as usize] == l.is_positive()))
                .then_some(Solved {
                    answer: Answer::Sat(projected),
                    via: Route::SymmetryBreak,
                    proof: Vec::new(),
                    conflicts: solver.conflicts(),
                })
        }
        SolveResult::Unsat => Some(Solved::unsat(Route::SymmetryBreak)),
    }
}

/// Decide via **recursive orbital branching** (Margot) — symmetry breaking in the *decision tree* rather
/// than by added clauses, applied down the whole tree. For a variable orbit `O` under the residual
/// automorphism group, "*some* variable in `O` is true" is symmetric to "*the representative* `O[0]` is
/// true" (the group is transitive on `O`, and an automorphism maps models to models). So
///
/// > `F` is SAT  ⟺  `F ∧ (rep = true)` is SAT  **or**  `F ∧ (all of O false)` is SAT,
///
/// collapsing the `|O|` symmetric "some-`O`-true" branches into a single representative branch. The
/// representative branch still carries the residual symmetry (the generators fixing `rep`), so we recurse
/// — at every node taking the largest free-variable orbit of the generators that fix the committed
/// variables — until the residual is asymmetric and plain CDCL finishes. Sound at each level by the
/// orbital argument; the filtered generators are a genuine subgroup of the residual's automorphisms (an
/// under-approximation: always correct, never enumerates more than the true group would). Every model is
/// re-checked fail-closed; UNSAT is returned only when both branches are genuinely UNSAT. `None` when
/// there is no phase-free variable symmetry with an orbit of size ≥ 3 (nothing to collapse) or the
/// instance is too large for automorphism detection.
fn orbital_branch_solve(num_vars: usize, clauses: &[Vec<Lit>]) -> Option<Solved> {
    if num_vars == 0 || num_vars > 64 {
        return None;
    }
    // Variable (phase-free) automorphisms — orbital branching reasons about variable orbits, so a phase
    // flip (which sends a literal to its negation) is not a usable generator here.
    let gens = crate::sym_break::variable_automorphism_generators(num_vars, clauses)?;
    if gens.is_empty() {
        return None;
    }
    // Gate: a ≥3 orbit somewhere is the reduction over the plain two-branch baseline that earns the route.
    if !crate::permgroup::orbits(num_vars, &gens).iter().any(|o| o.len() >= 3) {
        return None;
    }
    let mut conflicts = 0u64;
    let mut budget = 1024u64; // node cap; on exhaustion a node finishes its residual with plain CDCL
    let answer = orbital_node(num_vars, clauses, &gens, &mut Vec::new(), &mut budget, &mut conflicts)?;
    Some(Solved { answer, via: Route::OrbitalBranch, proof: Vec::new(), conflicts })
}

/// One node of the recursive orbital-branching tree over the residual `clauses ∧ committed`. Returns the
/// node's verdict, or `None` if a returned model fails its fail-closed re-check (the caller must then
/// decline rather than conclude UNSAT).
fn orbital_node(
    num_vars: usize,
    clauses: &[Vec<Lit>],
    gens: &[crate::permgroup::Perm],
    committed: &mut Vec<Lit>,
    budget: &mut u64,
    conflicts: &mut u64,
) -> Option<Answer> {
    let checked = |m: Vec<bool>| -> Option<Answer> {
        clauses
            .iter()
            .all(|c| c.iter().any(|l| m[l.var() as usize] == l.is_positive()))
            .then_some(Answer::Sat(m))
    };
    // Residual symmetry: the generators that fix every committed variable (so they preserve the unit
    // commitments) — a genuine subgroup of the residual formula's automorphisms.
    let committed_vars: HashSet<usize> = committed.iter().map(|l| l.var() as usize).collect();
    let live: Vec<crate::permgroup::Perm> = if committed_vars.is_empty() {
        gens.to_vec()
    } else {
        gens.iter().filter(|g| committed_vars.iter().all(|&v| g[v] == v)).cloned().collect()
    };
    // Largest free-variable orbit (committed variables are fixed points of `live`, hence singletons).
    let orbit = (*budget > 0)
        .then(|| {
            crate::permgroup::orbits(num_vars, &live)
                .into_iter()
                .filter(|o| o.len() >= 2 && o.iter().all(|v| !committed_vars.contains(v)))
                .max_by_key(|o| o.len())
        })
        .flatten();
    let orbit = match orbit {
        Some(o) => o,
        None => return solve_residual(num_vars, clauses, committed, conflicts), // asymmetric / budget out
    };
    *budget -= 1;
    let rep = orbit[0];

    // Branch A: rep = true — the representative of every model in which some orbit member is true.
    committed.push(Lit::pos(rep as u32));
    let a = orbital_node(num_vars, clauses, gens, committed, budget, conflicts);
    committed.pop();
    match a {
        Some(Answer::Sat(m)) => return checked(m),
        None => return None, // branch A undecidable ⟹ this node is undecidable (fail-closed)
        Some(Answer::Unsat) => {}
    }

    // Branch B: every orbit member false — the only models branch A does not cover.
    let n0 = committed.len();
    committed.extend(orbit.iter().map(|&v| Lit::neg(v as u32)));
    let b = orbital_node(num_vars, clauses, gens, committed, budget, conflicts);
    committed.truncate(n0);
    match b {
        Some(Answer::Sat(m)) => checked(m),
        Some(Answer::Unsat) => Some(Answer::Unsat), // both branches genuinely UNSAT ⟹ node UNSAT
        None => None,
    }
}

/// CDCL the residual `clauses ∧ committed` — the base case of [`orbital_node`].
fn solve_residual(
    num_vars: usize,
    clauses: &[Vec<Lit>],
    committed: &[Lit],
    conflicts: &mut u64,
) -> Option<Answer> {
    let mut s = Solver::new(num_vars);
    for c in clauses {
        s.add_clause(c.clone());
    }
    for &l in committed {
        s.add_clause(vec![l]);
    }
    let r = s.solve();
    *conflicts += s.conflicts();
    Some(match r {
        SolveResult::Sat(model) => Answer::Sat(model[..num_vars].to_vec()),
        SolveResult::Unsat => Answer::Unsat,
    })
}

/// Decide via **symmetric failed-literal inference** — symmetry used to *derive* consequences rather than
/// to *break* the search. A literal `ℓ` is a *failed literal* when `F ∧ ℓ` is UNSAT, which means `F ⊨ ¬ℓ`.
/// If `ℓ` is failed then *every* literal in its automorphism orbit is failed (an automorphism maps the
/// refutation of `F ∧ ℓ` onto a refutation of `F ∧ σ(ℓ)`), so a single probe forces the whole orbit. We
/// probe one representative per literal-orbit (budgeted, so a hard probe is simply skipped), accumulate the
/// implied units over the orbits, and solve the strengthened formula. Sound in both directions: only
/// *implied* units are added, so `F ∧ forced` is equisatisfiable with `F` (a SAT model is re-checked, and
/// UNSAT of the strengthened formula is UNSAT of `F`). `None` when there is no non-trivial symmetry or no
/// orbit yields a failed literal — the other routes then decide.
fn symmetric_probe_solve(num_vars: usize, clauses: &[Vec<Lit>]) -> Option<Solved> {
    if num_vars == 0 || num_vars > 64 {
        return None;
    }
    // Literal symmetries (variable AND phase) as image-literal vectors, lifted to the 2·num_vars points.
    let gens = crate::sym_break::literal_automorphism_generators(num_vars, clauses);
    if gens.is_empty() {
        return None;
    }
    let point_gens: Vec<_> =
        gens.iter().map(|s| crate::sym_break::litsym_to_points(s, num_vars)).collect();
    let lit_orbits = crate::permgroup::orbits(2 * num_vars, &point_gens);
    let point_to_lit = |p: usize| Lit::new((p / 2) as u32, p % 2 == 0);

    // A budgeted probe: `F ∧ ℓ` proven UNSAT within budget ⟹ `ℓ` is a failed literal (`F ⊨ ¬ℓ`).
    const PROBE_BUDGET: u64 = 200;
    let probe_fails = |lit: Lit| -> bool {
        let mut s = Solver::new(num_vars);
        for c in clauses {
            s.add_clause(c.clone());
        }
        s.add_clause(vec![lit]);
        matches!(s.solve_budgeted(PROBE_BUDGET), BudgetedResult::Unsat)
    };

    // One probe per multi-element literal-orbit (a singleton offers no symmetry amplification); a failed
    // representative forces ¬m for every m in its orbit.
    let mut forced: Vec<Lit> = Vec::new();
    for orbit in &lit_orbits {
        if orbit.len() < 2 {
            continue;
        }
        if probe_fails(point_to_lit(orbit[0])) {
            forced.extend(orbit.iter().map(|&p| point_to_lit(p).negated()));
        }
    }
    if forced.is_empty() {
        return None; // nothing inferred — defer to the breaking routes
    }

    let mut solver = Solver::new(num_vars);
    for c in clauses {
        solver.add_clause(c.clone());
    }
    for &l in &forced {
        solver.add_clause(vec![l]);
    }
    let r = solver.solve();
    let conflicts = solver.conflicts();
    match r {
        SolveResult::Sat(model) => {
            let projected: Vec<bool> = model[..num_vars].to_vec();
            clauses
                .iter()
                .all(|c| c.iter().any(|l| projected[l.var() as usize] == l.is_positive()))
                .then_some(Solved {
                    answer: Answer::Sat(projected),
                    via: Route::SymmetricProbe,
                    proof: Vec::new(),
                    conflicts,
                })
        }
        SolveResult::Unsat => {
            Some(Solved { answer: Answer::Unsat, via: Route::SymmetricProbe, proof: Vec::new(), conflicts })
        }
    }
}

/// Root-level Boolean constraint propagation from a single assumed literal. Returns the literals forced by
/// unit propagation (including the assumption), or `None` if it propagates to a conflict (`ℓ` is a failed
/// literal). Every forced `m ≠ ℓ` is a sound consequence: `F ∧ ℓ ⊨ m`, i.e. `F ⊨ ℓ → m`.
fn bcp_forced(num_vars: usize, clauses: &[Vec<Lit>], assume: Lit) -> Option<Vec<Lit>> {
    let mut val: Vec<Option<bool>> = vec![None; num_vars];
    val[assume.var() as usize] = Some(assume.is_positive());
    let mut forced = vec![assume];
    loop {
        let mut changed = false;
        for c in clauses {
            let mut sat = false;
            let mut unassigned: Option<Lit> = None;
            let mut count = 0;
            for &l in c {
                match val[l.var() as usize] {
                    Some(b) if b == l.is_positive() => {
                        sat = true;
                        break;
                    }
                    Some(_) => {}
                    None => {
                        count += 1;
                        unassigned = Some(l);
                    }
                }
            }
            if sat {
                continue;
            }
            if count == 0 {
                return None; // conflict
            }
            if count == 1 {
                let u = unassigned.unwrap();
                val[u.var() as usize] = Some(u.is_positive());
                forced.push(u);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    Some(forced)
}

/// Decide via **symmetric hyper-binary inference**. For one representative per variable-orbit, propagate
/// each polarity ([`bcp_forced`]); a non-conflicting probe of `ℓ` that forces `m` yields the implied
/// binary `¬ℓ ∨ m` (`F ⊨ ℓ → m`). Each such implication is then expanded over its **variable-symmetry
/// orbit** — `{¬σ(ℓ) ∨ σ(m)}` — so a single probe contributes the whole orbit of implied binaries, which
/// strengthen the formula before CDCL decides it. Sound: every added clause is a logical consequence of
/// `F` (a BCP implication, or an automorphic image of one), so `F` with them is equisatisfiable with `F`
/// (a SAT model is re-checked). `None` when there is no phase-free variable symmetry or no probe yields a
/// binary not already present (nothing new to learn — the other routes decide).
fn symmetric_binary_inference_solve(num_vars: usize, clauses: &[Vec<Lit>]) -> Option<Solved> {
    if num_vars == 0 || num_vars > 64 {
        return None;
    }
    let gens = crate::sym_break::variable_automorphism_generators(num_vars, clauses)?;
    if gens.is_empty() {
        return None;
    }
    let key2 = |a: Lit, b: Lit| -> [(u32, bool); 2] {
        let mut k = [(a.var(), a.is_positive()), (b.var(), b.is_positive())];
        k.sort_unstable();
        k
    };
    let lit_apply = |g: &[usize], l: Lit| Lit::new(g[l.var() as usize] as u32, l.is_positive());

    // Already-present binaries — only genuinely new implications are worth adding.
    let mut seen: HashSet<[(u32, bool); 2]> =
        clauses.iter().filter(|c| c.len() == 2).map(|c| key2(c[0], c[1])).collect();

    const CAP: usize = 4096;
    let mut new_bins: Vec<Vec<Lit>> = Vec::new();
    'outer: for orbit in crate::permgroup::orbits(num_vars, &gens) {
        let v = orbit[0];
        for pol in [false, true] {
            let lit = Lit::new(v as u32, pol);
            let Some(forced) = bcp_forced(num_vars, clauses, lit) else {
                continue; // a failed literal — symmetric_probe_solve handles units
            };
            for &m in &forced {
                if m.var() == lit.var() {
                    continue;
                }
                // ℓ → m, i.e. the binary ¬ℓ ∨ m; expand its variable-symmetry orbit.
                let mut local: HashSet<[(u32, bool); 2]> = HashSet::new();
                let mut stack = vec![(lit.negated(), m)];
                while let Some((a, b)) = stack.pop() {
                    if a.var() == b.var() {
                        continue;
                    }
                    let k = key2(a, b);
                    if !local.insert(k) {
                        continue;
                    }
                    if seen.insert(k) {
                        new_bins.push(vec![a, b]);
                        if new_bins.len() >= CAP {
                            break 'outer;
                        }
                    }
                    for g in &gens {
                        stack.push((lit_apply(g, a), lit_apply(g, b)));
                    }
                }
            }
        }
    }
    if new_bins.is_empty() {
        return None; // nothing new to learn
    }

    let mut solver = Solver::new(num_vars);
    for c in clauses {
        solver.add_clause(c.clone());
    }
    for c in &new_bins {
        solver.add_clause(c.clone());
    }
    let r = solver.solve();
    let conflicts = solver.conflicts();
    match r {
        SolveResult::Sat(model) => {
            let projected: Vec<bool> = model[..num_vars].to_vec();
            clauses
                .iter()
                .all(|c| c.iter().any(|l| projected[l.var() as usize] == l.is_positive()))
                .then_some(Solved {
                    answer: Answer::Sat(projected),
                    via: Route::SymmetricBinary,
                    proof: Vec::new(),
                    conflicts,
                })
        }
        SolveResult::Unsat => {
            Some(Solved { answer: Answer::Unsat, via: Route::SymmetricBinary, proof: Vec::new(), conflicts })
        }
    }
}

/// Decide by collapsing the formula onto its **orbit-weight quotient**. When the variable-automorphism
/// group is the *full* product of symmetric groups on its orbits — `G = S_{O₁} × … × S_{O_k}` — every
/// assignment is `G`-equivalent to one determined solely by its *weight* per orbit (how many variables in
/// each orbit are true): the sorting permutation lies in `G`, and an automorphism maps models to models.
/// So `F` is satisfiable iff some weight-tuple representative satisfies it, and the `2ⁿ` search space
/// collapses to `Π(|Oᵢ|+1)` representatives decided by direct evaluation. This is **complete and exact**
/// (not merely sound): a real decision, with an exponential collapse on fully-interchangeable instances.
///
/// The full-product gate is checked without factorials: for each orbit, every adjacent transposition
/// (swapping two members, fixing all else) must lie in the group ([`Bsgs::contains`]) — those generate
/// each `S_{Oᵢ}`, and the group is always a *subgroup* of the product, so membership of all of them gives
/// equality. `None` when the group is not the full product, there is no phase-free symmetry, or the
/// representative count is not a genuine (bounded) collapse over `2ⁿ`.
fn orbit_weight_quotient_solve(num_vars: usize, clauses: &[Vec<Lit>]) -> Option<Solved> {
    if num_vars == 0 || num_vars > 64 {
        return None;
    }
    let gens = crate::sym_break::variable_automorphism_generators(num_vars, clauses)?;
    if gens.is_empty() {
        return None;
    }
    let orbits = crate::permgroup::orbits(num_vars, &gens);
    let bsgs = crate::permgroup::schreier_sims(num_vars, &gens);

    // Full-product gate: every adjacent transposition within every orbit must be in the group.
    for orbit in &orbits {
        for w in orbit.windows(2) {
            let mut t: Vec<usize> = (0..num_vars).collect();
            t.swap(w[0], w[1]);
            if !bsgs.contains(&t) {
                return None; // an orbit on which G is not the full symmetric group
            }
        }
    }

    // Representative count Π(|Oᵢ|+1) — bounded, and a genuine collapse over 2ⁿ.
    let mut num_reps: u128 = 1;
    for o in &orbits {
        num_reps = num_reps.saturating_mul(o.len() as u128 + 1);
    }
    if num_reps > 200_000 || num_reps >= (1u128 << num_vars) {
        return None;
    }

    // Enumerate every weight-tuple (w₀,…,w_{k-1}), build its representative (the first wᵢ members of orbit
    // i set true), and evaluate. The first satisfying representative is a genuine model of F.
    let dims: Vec<usize> = orbits.iter().map(|o| o.len() + 1).collect();
    let total = num_reps as usize;
    let satisfies = |a: &[bool]| -> bool {
        clauses.iter().all(|c| c.iter().any(|l| a[l.var() as usize] == l.is_positive()))
    };
    for idx in 0..total {
        let mut rem = idx;
        let mut assign = vec![false; num_vars];
        for (oi, orbit) in orbits.iter().enumerate() {
            let w = rem % dims[oi];
            rem /= dims[oi];
            for &v in orbit.iter().take(w) {
                assign[v] = true;
            }
        }
        if satisfies(&assign) {
            return Some(Solved {
                answer: Answer::Sat(assign),
                via: Route::OrbitWeightQuotient,
                proof: Vec::new(),
                conflicts: 0,
            });
        }
    }
    // No weight-tuple representative satisfies F, and every assignment reduces to one ⟹ UNSAT.
    Some(Solved::unsat(Route::OrbitWeightQuotient))
}

/// A **lex-leader propagator** (Symmetry Breaking During Search) presented as a DPLL(T) theory. For each
/// generator `g` it enforces the symmetry-breaking constraint `a ≤_lex a∘g` (with `(a∘g)[j] = a[g[j]]`)
/// *dynamically* against the trail — no static SBP clauses, no auxiliary variables. Walking the variable
/// order, while the prefix is equal it records the equality-break literals; at the first position `j`
/// where `a[j]` and `a[g[j]]` are not (yet) equal it forms the lex clause
/// `(prefix equalities break) ∨ ¬a[j] ∨ a[g[j]]` — "if the prefix is equal then `a[j] ≤ a[g[j]]`" — and
/// emits it only when it is currently unit or falsified, so the core propagates or conflicts. Every
/// emitted clause is a sound consequence of `a ≤_lex a∘g`, which is satisfiability-preserving (the lex-min
/// of each orbit survives), so `F` with the theory is equisatisfiable with `F`.
struct LexLeaderTheory {
    num_vars: usize,
    generators: Vec<crate::permgroup::Perm>,
}

impl crate::cdcl::Theory for LexLeaderTheory {
    fn propagate(&mut self, trail: &[Lit]) -> Vec<Vec<Lit>> {
        let mut val: Vec<Option<bool>> = vec![None; self.num_vars];
        for &l in trail {
            let v = l.var() as usize;
            if v < self.num_vars {
                val[v] = Some(l.is_positive());
            }
        }
        // Keep a clause only if it is currently unit (one unassigned, rest false) or falsified (all false);
        // a satisfied or under-determined clause carries no immediate force and is dropped.
        let actionable = |c: &[Lit]| -> bool {
            let mut unassigned = 0;
            for &lit in c {
                match val[lit.var() as usize] {
                    Some(b) if b == lit.is_positive() => return false, // already satisfied
                    Some(_) => {}
                    None => unassigned += 1,
                }
            }
            unassigned <= 1
        };

        let mut out: Vec<Vec<Lit>> = Vec::new();
        for g in &self.generators {
            let mut prefix: Vec<Lit> = Vec::new();
            for j in 0..self.num_vars {
                let k = g[j];
                if k == j {
                    continue; // a[j] = a[g[j]] identically; the prefix is unaffected
                }
                match (val[j], val[k]) {
                    (Some(a), Some(b)) if a == b => {
                        // prefix still equal: the clause is satisfied if either side flips off this value
                        prefix.push(Lit::new(j as u32, !a));
                        prefix.push(Lit::new(k as u32, !b));
                    }
                    _ => {
                        // first position where the prefix is not (yet) equal: enforce a[j] ≤ a[g[j]].
                        let mut c = prefix.clone();
                        c.push(Lit::new(j as u32, false)); // ¬a[j]
                        c.push(Lit::new(k as u32, true)); //  a[g[j]]
                        if actionable(&c) {
                            out.push(c);
                        }
                        break;
                    }
                }
            }
        }
        out
    }
}

/// Decide via **symmetry breaking during search**: drive CDCL with the [`LexLeaderTheory`] propagator so
/// non-canonical (non-lex-leader) assignments are pruned on the fly through the DPLL(T) interface, rather
/// than by the static lex-leader clauses of [`symmetry_break_solve`]. Sound: the propagator only adds
/// consequences of `a ≤_lex a∘g`, which is satisfiability-preserving, so the verdict is correct (a SAT
/// model is re-checked fail-closed). `None` when there is no phase-free variable symmetry.
fn symmetry_propagate_solve(num_vars: usize, clauses: &[Vec<Lit>]) -> Option<Solved> {
    if num_vars == 0 || num_vars > 64 {
        return None;
    }
    let gens = crate::sym_break::variable_automorphism_generators(num_vars, clauses)?;
    if gens.is_empty() {
        return None;
    }
    let mut solver = Solver::new(num_vars);
    for c in clauses {
        solver.add_clause(c.clone());
    }
    let mut theories: Vec<Box<dyn crate::cdcl::Theory>> =
        vec![Box::new(LexLeaderTheory { num_vars, generators: gens })];
    match solver.solve_with(&mut theories) {
        SolveResult::Sat(model) => {
            let projected: Vec<bool> = model[..num_vars].to_vec();
            clauses
                .iter()
                .all(|c| c.iter().any(|l| projected[l.var() as usize] == l.is_positive()))
                .then_some(Solved {
                    answer: Answer::Sat(projected),
                    via: Route::SymmetryPropagate,
                    proof: Vec::new(),
                    conflicts: solver.conflicts(),
                })
        }
        SolveResult::Unsat => Some(Solved {
            answer: Answer::Unsat,
            via: Route::SymmetryPropagate,
            proof: Vec::new(),
            conflicts: solver.conflicts(),
        }),
    }
}

/// Union-find root with path halving.
fn uf_find(parent: &mut [usize], mut x: usize) -> usize {
    while parent[x] != x {
        parent[x] = parent[parent[x]];
        x = parent[x];
    }
    x
}

/// Decide by **plain component decomposition** — the payoff of separability in the solver. When the formula
/// splits into independent components (variables linked when they share a clause) with NO symmetry relating
/// them — exactly the case [`symmetric_component_solve`] declines — solve each component apart through the
/// full arsenal and combine: `F` is UNSAT iff any component is, and a model is the disjoint union of the
/// components' models (re-checked fail-closed). Solving apart avoids wrestling the whole formula at once and
/// lets a specialized route fire on a component that stumped it on the whole. Sound in both directions:
/// a component UNSAT ⟹ `F` UNSAT; and disjoint variable sets make the assembled model conflict-free.
/// `None` for a single component (nothing to split).
fn component_solve(num_vars: usize, clauses: &[Vec<Lit>]) -> Option<Solved> {
    if num_vars == 0 || num_vars > 64 || clauses.is_empty() {
        return None;
    }
    let mut parent: Vec<usize> = (0..num_vars).collect();
    for c in clauses {
        if c.is_empty() {
            return Some(Solved::unsat(Route::Component)); // an empty clause is UNSAT
        }
        let r0 = uf_find(&mut parent, c[0].var() as usize);
        for l in &c[1..] {
            let r = uf_find(&mut parent, l.var() as usize);
            parent[r] = r0;
        }
    }
    let mut comp_clauses: Vec<Vec<Vec<Lit>>> = vec![Vec::new(); num_vars];
    for c in clauses {
        let r = uf_find(&mut parent, c[0].var() as usize);
        comp_clauses[r].push(c.clone());
    }
    let roots: Vec<usize> = (0..num_vars).filter(|&r| !comp_clauses[r].is_empty()).collect();
    if roots.len() <= 1 {
        return None; // a single component — nothing to decompose
    }
    let mut model = vec![false; num_vars];
    let mut conflicts = 0u64;
    for &r in &roots {
        let sub = solve_comprehensive(num_vars, &comp_clauses[r]);
        conflicts += sub.conflicts;
        match sub.answer {
            Answer::Unsat => {
                return Some(Solved { answer: Answer::Unsat, via: Route::Component, proof: Vec::new(), conflicts });
            }
            Answer::Sat(m) => {
                let vars: std::collections::BTreeSet<usize> =
                    comp_clauses[r].iter().flatten().map(|l| l.var() as usize).collect();
                for v in vars {
                    model[v] = m[v];
                }
            }
        }
    }
    // Fail-closed: the assembled model must satisfy every clause, or defer.
    clauses
        .iter()
        .all(|c| c.iter().any(|l| model[l.var() as usize] == l.is_positive()))
        .then_some(Solved { answer: Answer::Sat(model), via: Route::Component, proof: Vec::new(), conflicts })
}

/// Decide via **plain component decomposition** as a public dispatcher route: split into independent
/// components and solve each apart through the full arsenal, combining the verdicts. A single component (or
/// a model that fails the fail-closed recheck) defers to the plain CDCL fallback.
pub fn solve_by_components(num_vars: usize, clauses: &[Vec<Lit>]) -> Solved {
    component_solve(num_vars, clauses).unwrap_or_else(|| cdcl_fallback(num_vars, clauses))
}

/// Decide by **symmetric component decomposition** — divide-and-conquer with symmetry. The formula splits
/// into independent components (variables linked when they share a clause); `F` is SAT iff every component
/// is. The automorphism group permutes the components, and components in the same orbit are *isomorphic
/// copies*, so we solve **one representative per component-orbit** (recursively, through the full arsenal)
/// and replicate its model through the symmetry — `k` identical sub-problems solved once. Sound: a
/// component is UNSAT ⟹ `F` is UNSAT; and for a copy `C = ρ(rep)` with `ρ` an automorphism, `ρ(rep-model)`
/// satisfies `C`'s clauses, so the assembled assignment (re-checked fail-closed) is a genuine model of
/// `F`. `None` for a single component, no phase-free symmetry, or no orbit with ≥ 2 components (no
/// symmetric copies to exploit — plain CDCL handles the rest).
fn symmetric_component_solve(num_vars: usize, clauses: &[Vec<Lit>]) -> Option<Solved> {
    if num_vars == 0 || num_vars > 64 || clauses.is_empty() {
        return None;
    }
    // Connected components of the variable-interaction graph.
    let mut parent: Vec<usize> = (0..num_vars).collect();
    for c in clauses {
        if c.is_empty() {
            return Some(Solved::unsat(Route::SymmetricComponent)); // an empty clause is UNSAT
        }
        let r0 = uf_find(&mut parent, c[0].var() as usize);
        for l in &c[1..] {
            let r = uf_find(&mut parent, l.var() as usize);
            parent[r] = r0;
        }
    }
    let mut appears = vec![false; num_vars];
    for c in clauses {
        for l in c {
            appears[l.var() as usize] = true;
        }
    }
    let mut comp_vars: Vec<Vec<usize>> = vec![Vec::new(); num_vars];
    for v in 0..num_vars {
        if appears[v] {
            let r = uf_find(&mut parent, v);
            comp_vars[r].push(v);
        }
    }
    let roots: Vec<usize> = (0..num_vars).filter(|&r| !comp_vars[r].is_empty()).collect();
    if roots.len() <= 1 {
        return None; // a single component — nothing to decompose
    }
    let mut comp_clauses: Vec<Vec<Vec<Lit>>> = vec![Vec::new(); num_vars];
    for c in clauses {
        let r = uf_find(&mut parent, c[0].var() as usize);
        comp_clauses[r].push(c.clone());
    }
    // Phase-free variable automorphisms identify isomorphic components.
    let gens = crate::sym_break::variable_automorphism_generators(num_vars, clauses)?;
    if gens.is_empty() {
        return None;
    }
    // Component orbits, each component tagged with the permutation mapping the orbit representative onto it.
    let identity: Vec<usize> = (0..num_vars).collect();
    let mut orbit_of: Vec<Option<usize>> = vec![None; num_vars];
    let mut orbits: Vec<Vec<(usize, Vec<usize>)>> = Vec::new();
    for &r in &roots {
        if orbit_of[r].is_some() {
            continue;
        }
        let oi = orbits.len();
        orbit_of[r] = Some(oi);
        let mut orbit: Vec<(usize, Vec<usize>)> = vec![(r, identity.clone())];
        let mut i = 0;
        while i < orbit.len() {
            let (cr, perm) = orbit[i].clone();
            i += 1;
            for g in &gens {
                let img_root = uf_find(&mut parent, g[comp_vars[cr][0]]);
                if orbit_of[img_root].is_none() {
                    orbit_of[img_root] = Some(oi);
                    let new_perm: Vec<usize> = (0..num_vars).map(|v| g[perm[v]]).collect();
                    orbit.push((img_root, new_perm));
                }
            }
        }
        orbits.push(orbit);
    }
    // A genuine symmetric copy is required — otherwise there is no symmetry redundancy to exploit here.
    if !orbits.iter().any(|o| o.len() >= 2) {
        return None;
    }
    // Solve one representative per orbit (recursively, through the full arsenal); assemble or refute.
    let mut model = vec![false; num_vars];
    let mut conflicts = 0u64;
    for orbit in &orbits {
        let rep_root = orbit[0].0;
        let solved = solve_comprehensive(num_vars, &comp_clauses[rep_root]);
        conflicts += solved.conflicts;
        match solved.answer {
            Answer::Unsat => {
                return Some(Solved {
                    answer: Answer::Unsat,
                    via: Route::SymmetricComponent,
                    proof: Vec::new(),
                    conflicts,
                });
            }
            Answer::Sat(rep_model) => {
                for (_, perm) in orbit {
                    for &v in &comp_vars[rep_root] {
                        model[perm[v]] = rep_model[v];
                    }
                }
            }
        }
    }
    clauses
        .iter()
        .all(|c| c.iter().any(|l| model[l.var() as usize] == l.is_positive()))
        .then_some(Solved { answer: Answer::Sat(model), via: Route::SymmetricComponent, proof: Vec::new(), conflicts })
}

/// Propagate the formula's unit clauses to a fixpoint. Returns the forced literals and the simplified
/// residual (satisfied clauses dropped, falsified literals removed — so forced variables appear in no
/// residual clause and the residual has no units), or `None` if propagation reaches a conflict.
fn root_propagate(num_vars: usize, clauses: &[Vec<Lit>]) -> Option<(Vec<Lit>, Vec<Vec<Lit>>)> {
    let mut val: Vec<Option<bool>> = vec![None; num_vars];
    loop {
        let mut changed = false;
        for c in clauses {
            let mut sat = false;
            let mut unit: Option<Lit> = None;
            let mut count = 0;
            for &l in c {
                match val[l.var() as usize] {
                    Some(b) if b == l.is_positive() => {
                        sat = true;
                        break;
                    }
                    Some(_) => {}
                    None => {
                        count += 1;
                        unit = Some(l);
                    }
                }
            }
            if sat {
                continue;
            }
            if count == 0 {
                return None; // conflict
            }
            if count == 1 {
                let u = unit.unwrap();
                val[u.var() as usize] = Some(u.is_positive());
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    let forced: Vec<Lit> =
        (0..num_vars).filter_map(|v| val[v].map(|b| Lit::new(v as u32, b))).collect();
    let mut residual: Vec<Vec<Lit>> = Vec::new();
    for c in clauses {
        let mut sat = false;
        let mut shrunk: Vec<Lit> = Vec::new();
        for &l in c {
            match val[l.var() as usize] {
                Some(b) if b == l.is_positive() => {
                    sat = true;
                    break;
                }
                Some(_) => {} // a falsified literal — drop it
                None => shrunk.push(l),
            }
        }
        if !sat {
            if shrunk.is_empty() {
                return None; // an emptied clause (cannot occur past the fixpoint, but stay safe)
            }
            residual.push(shrunk);
        }
    }
    Some((forced, residual))
}

/// Decide via **symmetry unlocked by simplification**. Unit clauses can mask automorphisms — a symmetry of
/// the residual `F|ρ` need not be a symmetry of the raw `F`, so the raw-formula detectors structurally miss
/// it. This route propagates the units ([`root_propagate`]), detects symmetry on the simplified residual,
/// and fires only when that symmetry is genuinely *new* (some residual generator is not a symmetry of the
/// raw clauses). It then solves the residual with the full arsenal — which now sees the revealed symmetry —
/// and re-applies the forced assignment. Sound: `ρ` is implied (BCP), `F` is equisatisfiable with `F|ρ`,
/// and the assembled model is re-checked fail-closed. `None` when nothing propagates, the residual is
/// asymmetric, or its symmetry was already visible on the raw formula.
fn symmetry_via_simplification_solve(num_vars: usize, clauses: &[Vec<Lit>]) -> Option<Solved> {
    if num_vars == 0 || num_vars > 64 {
        return None;
    }
    let (rho, residual) = match root_propagate(num_vars, clauses) {
        None => return Some(Solved::unsat(Route::SymmetrySimplify)), // refuted by propagation
        Some(x) => x,
    };
    if rho.is_empty() {
        return None; // nothing to simplify — no symmetry could be unlocked
    }
    // Detect on the residual with the forced literals re-pinned as units: this keeps the forced variables
    // constrained (an isolated variable carries a spurious phase symmetry that would mask the real ones)
    // while still exposing the symmetry that simplification revealed.
    let mut detect = residual.clone();
    for &l in &rho {
        detect.push(vec![l]);
    }
    let res_gens = crate::sym_break::variable_automorphism_generators(num_vars, &detect)
        .unwrap_or_default();
    if res_gens.is_empty() {
        return None; // the residual carries no usable symmetry
    }
    // Fire only if simplification revealed symmetry the raw formula did not already have.
    let raw_set: HashSet<Vec<(u32, bool)>> = clauses
        .iter()
        .map(|c| {
            let mut k: Vec<(u32, bool)> = c.iter().map(|l| (l.var(), l.is_positive())).collect();
            k.sort_unstable();
            k
        })
        .collect();
    let is_raw_symmetry = |g: &[usize]| -> bool {
        clauses.iter().all(|c| {
            let mut img: Vec<(u32, bool)> =
                c.iter().map(|l| (g[l.var() as usize] as u32, l.is_positive())).collect();
            img.sort_unstable();
            raw_set.contains(&img)
        })
    };
    if res_gens.iter().all(|g| is_raw_symmetry(g)) {
        return None; // the raw-formula routes already see this symmetry
    }

    // Solve the simplified residual with the full arsenal, then re-apply the forced assignment.
    let solved = solve_comprehensive(num_vars, &residual);
    match solved.answer {
        Answer::Unsat => Some(Solved {
            answer: Answer::Unsat,
            via: Route::SymmetrySimplify,
            proof: Vec::new(),
            conflicts: solved.conflicts,
        }),
        Answer::Sat(mut model) => {
            for &l in &rho {
                model[l.var() as usize] = l.is_positive();
            }
            clauses
                .iter()
                .all(|c| c.iter().any(|l| model[l.var() as usize] == l.is_positive()))
                .then_some(Solved {
                    answer: Answer::Sat(model),
                    via: Route::SymmetrySimplify,
                    proof: Vec::new(),
                    conflicts: solved.conflicts,
                })
        }
    }
}

/// Build a **lex-leader SBP from the whole tower of block systems** — multi-dimensional symmetry breaking.
/// `hierarchical_break` breaks a single (minimal) block system: the inter-block and intra-block swaps of a
/// 2-D row/column grid. A `d`-dimensional grid (`S_{n₁} × … × S_{n_d}`) has nested block systems
/// `cells ⊂ lines ⊂ planes ⊂ …`; this ascends the tower via the group's *induced action on blocks*,
/// emitting the structured swaps at every level. Each candidate swap is verified to lie in the group
/// ([`Bsgs::contains`]), so the SBP only ever uses genuine automorphisms and is sound (it keeps the
/// lex-leader of each orbit). `None` when there is no phase-free, transitive, imprimitive symmetry.
fn nested_block_tower_break(num_vars: usize, clauses: &[Vec<Lit>]) -> Option<(Vec<Vec<Lit>>, usize)> {
    let gens = crate::sym_break::variable_automorphism_generators(num_vars, clauses)?;
    if gens.is_empty() {
        return None;
    }
    let bsgs = crate::permgroup::schreier_sims(num_vars, &gens);
    let to_litsym = |p: &[usize]| -> Vec<Lit> { (0..num_vars).map(|v| Lit::pos(p[v] as u32)).collect() };

    let mut structured: Vec<Vec<Lit>> = Vec::new();
    let mut blocks = crate::permgroup::minimal_block_system(num_vars, &gens)?; // None if primitive/intransitive
    for _level in 0..num_vars {
        let k = blocks.len();
        let m = blocks[0].len();
        if blocks.iter().any(|b| b.len() != m) {
            break; // irregular tower — stop ascending
        }
        // Inter-block adjacent swaps: exchange two whole adjacent blocks position-wise.
        for i in 0..k.saturating_sub(1) {
            let mut p: Vec<usize> = (0..num_vars).collect();
            for j in 0..m {
                p[blocks[i][j]] = blocks[i + 1][j];
                p[blocks[i + 1][j]] = blocks[i][j];
            }
            if bsgs.contains(&p) {
                structured.push(to_litsym(&p));
            }
        }
        // Intra-block uniform adjacent swaps: exchange positions j, j+1 within every block at once.
        for j in 0..m.saturating_sub(1) {
            let mut p: Vec<usize> = (0..num_vars).collect();
            for b in &blocks {
                p[b[j]] = b[j + 1];
                p[b[j + 1]] = b[j];
            }
            if bsgs.contains(&p) {
                structured.push(to_litsym(&p));
            }
        }
        if k <= 1 {
            break;
        }
        // Ascend: the group's induced action on the k blocks, then that action's block system.
        let mut block_of = vec![usize::MAX; num_vars];
        for (bi, b) in blocks.iter().enumerate() {
            for &v in b {
                block_of[v] = bi;
            }
        }
        let induced: Vec<Vec<usize>> = gens
            .iter()
            .map(|g| (0..k).map(|bi| block_of[g[blocks[bi][0]]]).collect())
            .collect();
        let Some(super_blocks) = crate::permgroup::minimal_block_system(k, &induced) else {
            break; // the blocks are primitive — top of the tower
        };
        let mut next: Vec<Vec<usize>> = Vec::new();
        for sb in &super_blocks {
            let mut nb = Vec::new();
            for &bi in sb {
                nb.extend_from_slice(&blocks[bi]);
            }
            next.push(nb);
        }
        if next.len() >= blocks.len() {
            break; // no genuine coarsening
        }
        blocks = next;
    }
    if structured.is_empty() {
        return None;
    }
    Some(crate::sym_break::lex_leader_sbp_lit(num_vars, &structured))
}

/// Decide via **multi-dimensional (nested block-tower) symmetry breaking** — see [`nested_block_tower_break`].
/// Adds the tower's lex-leader SBP and lets CDCL decide the broken formula; sound (verified group elements
/// only), with a fail-closed re-check on the returned model. `None` when there is no usable nested grid
/// symmetry.
fn nested_symmetry_solve(num_vars: usize, clauses: &[Vec<Lit>]) -> Option<Solved> {
    if num_vars == 0 || num_vars > 64 {
        return None;
    }
    let (sbp, total) = nested_block_tower_break(num_vars, clauses)?;
    let mut solver = Solver::new(total);
    for c in clauses {
        solver.add_clause(c.clone());
    }
    for c in &sbp {
        solver.add_clause(c.clone());
    }
    match solver.solve() {
        SolveResult::Sat(model) => {
            let projected: Vec<bool> = model[..num_vars].to_vec();
            clauses
                .iter()
                .all(|c| c.iter().any(|l| projected[l.var() as usize] == l.is_positive()))
                .then_some(Solved {
                    answer: Answer::Sat(projected),
                    via: Route::NestedSymmetry,
                    proof: Vec::new(),
                    conflicts: solver.conflicts(),
                })
        }
        SolveResult::Unsat => {
            Some(Solved { answer: Answer::Unsat, via: Route::NestedSymmetry, proof: Vec::new(), conflicts: solver.conflicts() })
        }
    }
}

/// The sorted (var, polarity) signature of a clause, for set membership.
fn canon_clause(c: &[Lit]) -> Vec<(u32, bool)> {
    let mut k: Vec<(u32, bool)> = c.iter().map(|l| (l.var(), l.is_positive())).collect();
    k.sort_unstable();
    k
}

/// A clause with variables `a` and `b` interchanged (polarities preserved).
fn swap_clause_vars(c: &[Lit], a: usize, b: usize) -> Vec<Lit> {
    c.iter()
        .map(|l| {
            let v = l.var() as usize;
            let nv = if v == a {
                b
            } else if v == b {
                a
            } else {
                v
            };
            Lit::new(nv as u32, l.is_positive())
        })
        .collect()
}

/// Is the clause `c` a logical consequence of `clauses`? (`F ∧ ¬c` is UNSAT.)
fn clause_is_implied(num_vars: usize, clauses: &[Vec<Lit>], c: &[Lit]) -> bool {
    let mut s = Solver::new(num_vars);
    for cl in clauses {
        s.add_clause(cl.clone());
    }
    for &l in c {
        s.add_clause(vec![l.negated()]);
    }
    matches!(s.solve(), SolveResult::Unsat)
}

/// The **semantic** symmetries (variable transpositions) of a CNF: pairs `(a,b)` whose swap preserves the
/// MODEL SET, `F ≡ swap(F)`, even when it does NOT preserve the clause set. Checked by implication: `F ⊨
/// swap(F)` (and, by the involution `swap² = id`, that is full equivalence). Returns the semantic pairs
/// and whether any is *non-syntactic* (clause set changed) — i.e. genuinely beyond what the syntactic
/// detector ([`crate::sym_break::variable_automorphism_generators`]) can see.
pub fn semantic_symmetry_pairs(num_vars: usize, clauses: &[Vec<Lit>]) -> (Vec<(usize, usize)>, bool) {
    let clause_set: HashSet<Vec<(u32, bool)>> = clauses.iter().map(|c| canon_clause(c)).collect();
    let mut pairs = Vec::new();
    let mut any_non_syntactic = false;
    for a in 0..num_vars {
        for b in (a + 1)..num_vars {
            let syntactic =
                clauses.iter().all(|c| clause_set.contains(&canon_clause(&swap_clause_vars(c, a, b))));
            let semantic = syntactic
                || clauses.iter().all(|c| {
                    let sc = swap_clause_vars(c, a, b);
                    clause_set.contains(&canon_clause(&sc)) || clause_is_implied(num_vars, clauses, &sc)
                });
            if semantic {
                pairs.push((a, b));
                if !syntactic {
                    any_non_syntactic = true;
                }
            }
        }
    }
    (pairs, any_non_syntactic)
}

/// Decide via **semantic symmetry breaking**. Detects variable transpositions that preserve the model set
/// without preserving the clause set ([`semantic_symmetry_pairs`]) — symmetries the syntactic detector
/// structurally cannot find — and breaks them with the lex-leader, which is sound for any model-set-
/// preserving permutation. Fires only when there is a genuinely *non-syntactic* symmetry (otherwise
/// [`symmetry_break_solve`] already covers it). Detection is `O(n²)` implication checks, so it is gated to
/// small instances and runs last, just before search. SAT models are re-checked fail-closed.
fn semantic_symmetry_solve(num_vars: usize, clauses: &[Vec<Lit>]) -> Option<Solved> {
    if num_vars == 0 || num_vars > 20 || clauses.len() > 256 {
        return None; // O(n² · clauses) implication probing — bound it
    }
    let (pairs, any_non_syntactic) = semantic_symmetry_pairs(num_vars, clauses);
    if pairs.is_empty() || !any_non_syntactic {
        return None; // nothing, or nothing beyond the syntactic routes
    }
    let gens: Vec<Vec<Lit>> = pairs
        .iter()
        .map(|&(a, b)| {
            (0..num_vars)
                .map(|v| {
                    let img = if v == a {
                        b
                    } else if v == b {
                        a
                    } else {
                        v
                    };
                    Lit::pos(img as u32)
                })
                .collect()
        })
        .collect();
    let (sbp, total) = crate::sym_break::lex_leader_sbp_lit(num_vars, &gens);
    let mut solver = Solver::new(total);
    for c in clauses {
        solver.add_clause(c.clone());
    }
    for c in &sbp {
        solver.add_clause(c.clone());
    }
    match solver.solve() {
        SolveResult::Sat(model) => {
            let projected: Vec<bool> = model[..num_vars].to_vec();
            clauses
                .iter()
                .all(|c| c.iter().any(|l| projected[l.var() as usize] == l.is_positive()))
                .then_some(Solved {
                    answer: Answer::Sat(projected),
                    via: Route::SemanticSymmetry,
                    proof: Vec::new(),
                    conflicts: solver.conflicts(),
                })
        }
        SolveResult::Unsat => {
            Some(Solved { answer: Answer::Unsat, via: Route::SemanticSymmetry, proof: Vec::new(), conflicts: solver.conflicts() })
        }
    }
}

/// The **almost-symmetries** (variable transpositions) of a CNF: pairs `(a,b)` whose swap preserves all but
/// at most `max_broken` clauses. Returns each pair with the **broken images** `σ(B) = { swap(c) : swap(c) ∉
/// F }` — the clauses that must hold for the swap to map a model to a model, i.e. the guard. An empty image
/// set is a true (syntactic) symmetry and is excluded.
pub fn almost_symmetry_pairs(
    num_vars: usize,
    clauses: &[Vec<Lit>],
    max_broken: usize,
) -> Vec<(usize, usize, Vec<Vec<Lit>>)> {
    let clause_set: HashSet<Vec<(u32, bool)>> = clauses.iter().map(|c| canon_clause(c)).collect();
    let mut out = Vec::new();
    for a in 0..num_vars {
        for b in (a + 1)..num_vars {
            let mut images = Vec::new();
            for c in clauses {
                let sc = swap_clause_vars(c, a, b);
                if !clause_set.contains(&canon_clause(&sc)) {
                    images.push(sc);
                }
            }
            if !images.is_empty() && images.len() <= max_broken {
                out.push((a, b, images));
            }
        }
    }
    out
}

/// Decide via **almost-symmetry breaking** (conditional). A transposition `σ = (a,b)` that breaks only a
/// few clauses is an automorphism of `F` minus those clauses, so it maps a model `m` of `F` to a model of
/// `F` *exactly when* `σ(m)` also satisfies the broken clauses — equivalently when `m` satisfies their
/// images `σ(B)`. We therefore add the **guarded** break `(⋀ σ(B)) → (a@0 ≤ b@0)`, encoded with a
/// reification `z_c ↔ c` per image clause. Sound: where the guard holds, both `m` and `σ(m)` are models and
/// the ordering keeps one; where it does not, `m` is untouched. We break the single almost-symmetry with
/// the fewest broken clauses (composing partial breaks can be unsound), so a SAT model is re-checked and an
/// UNSAT verdict is faithful. `None` when no transposition breaks few enough clauses.
fn almost_symmetry_solve(num_vars: usize, clauses: &[Vec<Lit>]) -> Option<Solved> {
    if num_vars == 0 || num_vars > 20 || clauses.len() > 256 {
        return None;
    }
    let mut pairs = almost_symmetry_pairs(num_vars, clauses, 2);
    if pairs.is_empty() {
        return None;
    }
    // The fewest-broken almost-symmetry: most often active, and we break exactly one for soundness.
    pairs.sort_by_key(|(_, _, imgs)| imgs.len());
    let (a, b, images) = &pairs[0];

    let mut aux = num_vars as u32;
    let mut extra: Vec<Vec<Lit>> = Vec::new();
    let mut guard_neg: Vec<Lit> = Vec::new();
    for img in images {
        let z = aux;
        aux += 1;
        // z ↔ (img is satisfied): (each literal → z) and (z → the clause).
        for &l in img {
            extra.push(vec![l.negated(), Lit::pos(z)]);
        }
        let mut zc = vec![Lit::neg(z)];
        zc.extend(img.iter().copied());
        extra.push(zc);
        guard_neg.push(Lit::neg(z));
    }
    // (⋀ images) → (a ≤ b)   ≡   [⋁ ¬z_c] ∨ ¬a ∨ b
    let mut guarded = guard_neg;
    guarded.push(Lit::neg(*a as u32));
    guarded.push(Lit::pos(*b as u32));
    extra.push(guarded);

    let mut solver = Solver::new(aux as usize);
    for c in clauses {
        solver.add_clause(c.clone());
    }
    for c in &extra {
        solver.add_clause(c.clone());
    }
    match solver.solve() {
        SolveResult::Sat(model) => {
            let projected: Vec<bool> = model[..num_vars].to_vec();
            clauses
                .iter()
                .all(|c| c.iter().any(|l| projected[l.var() as usize] == l.is_positive()))
                .then_some(Solved {
                    answer: Answer::Sat(projected),
                    via: Route::AlmostSymmetry,
                    proof: Vec::new(),
                    conflicts: solver.conflicts(),
                })
        }
        SolveResult::Unsat => {
            Some(Solved { answer: Answer::Unsat, via: Route::AlmostSymmetry, proof: Vec::new(), conflicts: solver.conflicts() })
        }
    }
}

/// A clause with its variables permuted by `g`.
fn apply_perm_to_clause(c: &[Lit], g: &[usize]) -> Vec<Lit> {
    c.iter().map(|l| Lit::new(g[l.var() as usize] as u32, l.is_positive())).collect()
}

/// Whether the variable permutation `g` preserves the clause SET of `clauses` (`σ(F) = F`) — i.e. `g` is a
/// phase-free automorphism of the formula. `g` is a variable bijection, so it maps the (deduplicated)
/// clause set injectively into itself; if every image is present, it permutes the set.
fn perm_preserves_clause_set(clauses: &[Vec<Lit>], g: &[usize]) -> bool {
    let set: HashSet<Vec<(u32, bool)>> = clauses.iter().map(|c| canon_clause(c)).collect();
    clauses.iter().all(|c| set.contains(&canon_clause(&apply_perm_to_clause(c, g))))
}

/// The **common variable automorphisms** of two formulas — phase-free permutations `σ` with `σ(F)=F` AND
/// `σ(S)=S`, a generating set for a subgroup of `Aut(F) ∩ Aut(S)`. Detected from each formula's own
/// automorphisms and then VERIFIED against *both* clause sets, so every returned generator is a genuine
/// common automorphism (a wrong one can never slip through). This is the group under which a *disagreement*
/// between `F` and `S` is symmetric: if `σ` fixes both, then `F` and `S` disagree at `a` iff they disagree
/// at `σ(a)` — so equivalence checking can be reduced along its orbits.
pub fn common_automorphism_generators(
    num_vars: usize,
    f: &[Vec<Lit>],
    s: &[Vec<Lit>],
) -> Vec<crate::permgroup::Perm> {
    let mut candidates = crate::sym_break::variable_automorphism_generators(num_vars, f).unwrap_or_default();
    candidates.extend(crate::sym_break::variable_automorphism_generators(num_vars, s).unwrap_or_default());
    let mut out: Vec<Vec<usize>> = Vec::new();
    for g in candidates {
        let well_formed = g.len() == num_vars;
        let nontrivial = g.iter().enumerate().any(|(i, &x)| i != x);
        if well_formed
            && nontrivial
            && perm_preserves_clause_set(f, &g)
            && perm_preserves_clause_set(s, &g)
            && !out.contains(&g)
        {
            out.push(g);
        }
    }
    out
}

/// The orbits (as index sets) of `clauses` under the group generated by `gens`, acting by
/// `σ(C) = {σ(l) : l ∈ C}`. A clause and its symmetric images share an orbit.
fn clause_orbits(clauses: &[Vec<Lit>], gens: &[crate::permgroup::Perm]) -> Vec<Vec<usize>> {
    let index: HashMap<Vec<(u32, bool)>, usize> =
        clauses.iter().enumerate().map(|(i, c)| (canon_clause(c), i)).collect();
    let n = clauses.len();
    let mut parent: Vec<usize> = (0..n).collect();
    fn find(parent: &mut [usize], mut x: usize) -> usize {
        while parent[x] != x {
            parent[x] = parent[parent[x]];
            x = parent[x];
        }
        x
    }
    for (i, c) in clauses.iter().enumerate() {
        for g in gens {
            if let Some(&j) = index.get(&canon_clause(&apply_perm_to_clause(c, g))) {
                let (a, b) = (find(&mut parent, i), find(&mut parent, j));
                parent[a] = b;
            }
        }
    }
    let mut groups: std::collections::BTreeMap<usize, Vec<usize>> = std::collections::BTreeMap::new();
    for i in 0..n {
        let r = find(&mut parent, i);
        groups.entry(r).or_default().push(i);
    }
    groups.into_values().collect()
}

/// A model of `clauses ∧ ¬c` — a witness that `c` is NOT entailed by `clauses` — or `None` if `clauses ⊨ c`.
fn entailment_counterexample(num_vars: usize, clauses: &[Vec<Lit>], c: &[Lit]) -> Option<Vec<bool>> {
    let mut s = Solver::new(num_vars);
    for cl in clauses {
        s.add_clause(cl.clone());
    }
    for &l in c {
        s.add_clause(vec![l.negated()]);
    }
    match s.solve() {
        SolveResult::Sat(m) => Some(m),
        SolveResult::Unsat => None,
    }
}

/// The verdict of a symmetry-reduced equivalence check.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EquivVerdict {
    /// `F` and `S` denote the same Boolean function (same models).
    Equivalent,
    /// They differ; the assignment satisfies exactly one of the two formulas (a distinguishing witness).
    Differ(Vec<bool>),
}

/// **Logical equivalence `F ≡ S`, symmetry-reduced.** `F ≡ S` iff `F ⊨ S` and `S ⊨ F`, and `F ⊨ S` iff `F`
/// entails every clause of `S`. The key reduction: for a common automorphism `σ ∈ Aut(F) ∩ Aut(S)`,
/// `F ⊨ C ⟺ F ⊨ σ(C)` (a model of `F ∧ ¬C` maps under `σ` to a model of `F ∧ ¬σ(C)`), so it suffices to
/// check **one clause per orbit** of `S`'s clauses under the common symmetry (and dually for `F`). The
/// verdict is unchanged from the naive check — only the work shrinks — and a `Differ` witness is a concrete,
/// re-checkable disagreement (it satisfies one formula and violates a clause of the other). This is the
/// symmetry-aware companion to [`crate::sat::prove_equivalence`].
pub fn equivalent_modulo_symmetry(num_vars: usize, f: &[Vec<Lit>], s: &[Vec<Lit>]) -> EquivVerdict {
    let gens = common_automorphism_generators(num_vars, f, s);
    // F ⊨ S: one representative clause per orbit of S under the shared symmetry.
    for orbit in clause_orbits(s, &gens) {
        if let Some(m) = entailment_counterexample(num_vars, f, &s[orbit[0]]) {
            return EquivVerdict::Differ(m); // satisfies F, violates a clause of S ⇒ F true, S false
        }
    }
    // S ⊨ F: dually.
    for orbit in clause_orbits(f, &gens) {
        if let Some(m) = entailment_counterexample(num_vars, s, &f[orbit[0]]) {
            return EquivVerdict::Differ(m);
        }
    }
    EquivVerdict::Equivalent
}

/// `(checks_with_symmetry, naive_checks)` — the number of entailment checks the symmetry reduction performs
/// (one per clause-orbit, both directions) versus the naive per-clause count. Equal when there is no usable
/// common symmetry; strictly smaller when the shared automorphism group fuses clauses into orbits.
pub fn equivalence_check_counts(num_vars: usize, f: &[Vec<Lit>], s: &[Vec<Lit>]) -> (usize, usize) {
    let gens = common_automorphism_generators(num_vars, f, s);
    (clause_orbits(s, &gens).len() + clause_orbits(f, &gens).len(), s.len() + f.len())
}

/// The symmetries of an **optimization** problem `(F, weights)`: variable automorphisms of `F` that *also*
/// preserve the objective (`weights[g[v]] = weights[v]` for all `v`). Under such a `σ` the objective is
/// constant on orbits, so every optimal model's whole orbit is optimal — the group along which the optimum
/// may be symmetry-reduced. Verified against both `F` and the weights, so every generator is genuine.
pub fn optimization_symmetry_generators(
    num_vars: usize,
    clauses: &[Vec<Lit>],
    weights: &[i64],
) -> Vec<crate::permgroup::Perm> {
    crate::sym_break::variable_automorphism_generators(num_vars, clauses)
        .unwrap_or_default()
        .into_iter()
        .filter(|g| g.len() == num_vars && (0..num_vars).all(|v| weights[g[v]] == weights[v]))
        .collect()
}

/// `F` augmented with sound single-bit lex-leader clauses for the optimization symmetry. Each generator `g`
/// contributes `x_v ≤ x_{g[v]}` at its least moved point `v`; the lex-minimum of any orbit satisfies all of
/// them, so — since the objective is constant on orbits — an *optimal* model always survives. The optimum is
/// therefore unchanged while the model space shrinks.
fn optimization_break(num_vars: usize, clauses: &[Vec<Lit>], weights: &[i64]) -> Vec<Vec<Lit>> {
    let mut broken = clauses.to_vec();
    for g in optimization_symmetry_generators(num_vars, clauses, weights) {
        if let Some(v) = (0..num_vars).find(|&v| g[v] != v) {
            broken.push(vec![Lit::new(v as u32, false), Lit::new(g[v] as u32, true)]); // ¬x_v ∨ x_{g[v]}
        }
    }
    broken
}

/// Enumerate the models of `clauses` (CDCL + blocking) and return the minimum-weight one with the number of
/// models visited: `(optimum, witness, models_enumerated)`, or `None` if unsatisfiable. A FRESH solver is
/// built each round with the accumulated blocking clauses — our CDCL core is not incrementally re-solvable
/// (the same pattern the model-counting routines use), so reusing one solver across `solve()` calls would
/// loop.
fn min_weight_model(num_vars: usize, clauses: &[Vec<Lit>], weights: &[i64]) -> Option<(i64, Vec<bool>, usize)> {
    let mut working = clauses.to_vec();
    let mut best: Option<(i64, Vec<bool>)> = None;
    let mut enumerated = 0usize;
    loop {
        let mut solver = Solver::new(num_vars);
        for c in &working {
            solver.add_clause(c.clone());
        }
        match solver.solve() {
            SolveResult::Unsat => break,
            SolveResult::Sat(model) => {
                enumerated += 1;
                let w: i64 = (0..num_vars).filter(|&i| model[i]).map(|i| weights[i]).sum();
                if best.as_ref().map_or(true, |(bw, _)| w < *bw) {
                    best = Some((w, model[..num_vars].to_vec()));
                }
                working.push((0..num_vars).map(|i| Lit::new(i as u32, !model[i])).collect());
            }
        }
    }
    best.map(|(w, m)| (w, m, enumerated))
}

/// **Weighted minimization, symmetry-reduced.** Minimise `Σ weights[i]·x_i` over the models of `clauses`,
/// exploiting [`optimization_symmetry_generators`]: break the objective-preserving symmetry, then search the
/// reduced model space. SOUND — the optimum is identical to the unbroken problem (an optimal orbit's
/// lex-leader survives the break and has the same objective) and the witness is a genuine model of the
/// original `F`. `None` iff `F` is unsatisfiable. The symmetry-aware optimizer (a new problem class beyond
/// the decision/counting/equivalence faces).
pub fn optimize_modulo_symmetry(
    num_vars: usize,
    clauses: &[Vec<Lit>],
    weights: &[i64],
) -> Option<(i64, Vec<bool>)> {
    let broken = optimization_break(num_vars, clauses, weights);
    min_weight_model(num_vars, &broken, weights).map(|(w, m, _)| (w, m))
}

/// `(models_with_symmetry, models_without_symmetry)` — how many candidate models the optimizer enumerates
/// with the symmetry break versus the naive enumeration. Equal with no usable symmetry, smaller when the
/// objective-preserving group fuses optimal (and non-optimal) models into orbits.
pub fn optimize_enumeration_counts(num_vars: usize, clauses: &[Vec<Lit>], weights: &[i64]) -> (usize, usize) {
    let broken = optimization_break(num_vars, clauses, weights);
    let with = min_weight_model(num_vars, &broken, weights).map_or(0, |(_, _, c)| c);
    let without = min_weight_model(num_vars, clauses, weights).map_or(0, |(_, _, c)| c);
    (with, without)
}

/// The full orbit of a Boolean assignment `m` under the group generated by `gens` (action `σ(m)[g[v]] =
/// m[v]`), as a set of complete assignments — no projection, so it is exact for counting.
fn full_assignment_orbit(num_vars: usize, m: &[bool], gens: &[crate::permgroup::Perm]) -> Vec<Vec<bool>> {
    let mut seen: HashSet<Vec<bool>> = HashSet::from([m.to_vec()]);
    let mut out = vec![m.to_vec()];
    let mut i = 0;
    while i < out.len() {
        let cur = out[i].clone();
        i += 1;
        for g in gens {
            let mut pm = vec![false; num_vars];
            for v in 0..num_vars {
                pm[g[v]] = cur[v];
            }
            if seen.insert(pm.clone()) {
                out.push(pm);
            }
        }
    }
    out
}

/// **Symmetry-accelerated weighted model counting** — the partition function `Z = Σ_{m ⊨ F} W(m)` with
/// literal weights `W(m) = Π_v weight[v].{1 if m[v] else 0}`. The formula's variable symmetry partitions the
/// models into orbits, so each `solve()` recovers one model, its whole orbit is enumerated and its members'
/// weights summed, then the entire orbit is blocked — **one solve per orbit** instead of per model. Exact for
/// ARBITRARY weights (every model's true weight is summed; the symmetry only groups the search), the
/// symmetry-aware analogue of weighted #SAT / lifted inference. Returns `Z`.
pub fn weighted_model_count(num_vars: usize, clauses: &[Vec<Lit>], weight: &[(i64, i64)]) -> i128 {
    let gens = crate::sym_break::variable_automorphism_generators(num_vars, clauses).unwrap_or_default();
    let weight_of = |m: &[bool]| -> i128 {
        (0..num_vars).map(|v| if m[v] { weight[v].1 as i128 } else { weight[v].0 as i128 }).product()
    };
    let mut working = clauses.to_vec();
    let mut z = 0i128;
    loop {
        let mut solver = Solver::new(num_vars);
        for c in &working {
            solver.add_clause(c.clone());
        }
        match solver.solve() {
            SolveResult::Unsat => break,
            SolveResult::Sat(model) => {
                let orbit = full_assignment_orbit(num_vars, &model[..num_vars], &gens);
                z += orbit.iter().map(|m| weight_of(m)).sum::<i128>();
                for m in &orbit {
                    working.push((0..num_vars).map(|i| Lit::new(i as u32, !m[i])).collect());
                }
            }
        }
    }
    z
}

/// `(orbit_solves, total_models)` — how many `solve()` calls the symmetry-accelerated weighted count makes
/// (one per model-orbit) versus the number of models. Equal with no usable symmetry, smaller when symmetry
/// fuses models into orbits.
pub fn weighted_model_count_solve_counts(num_vars: usize, clauses: &[Vec<Lit>]) -> (usize, usize) {
    let gens = crate::sym_break::variable_automorphism_generators(num_vars, clauses).unwrap_or_default();
    let mut working = clauses.to_vec();
    let (mut solves, mut models) = (0usize, 0usize);
    loop {
        let mut solver = Solver::new(num_vars);
        for c in &working {
            solver.add_clause(c.clone());
        }
        match solver.solve() {
            SolveResult::Unsat => break,
            SolveResult::Sat(model) => {
                solves += 1;
                let orbit = full_assignment_orbit(num_vars, &model[..num_vars], &gens);
                models += orbit.len();
                for m in &orbit {
                    working.push((0..num_vars).map(|i| Lit::new(i as u32, !m[i])).collect());
                }
            }
        }
    }
    (solves, models)
}

/// Re-label a list of signatures to dense ranks `0..k` in sorted order — equal signatures get equal ranks.
/// The canonical relabelling that makes color refinement's fixpoint test stable.
fn rank_signatures<S: Ord + Clone>(sigs: &[S]) -> Vec<usize> {
    let mut distinct: Vec<S> = sigs.to_vec();
    distinct.sort();
    distinct.dedup();
    sigs.iter().map(|s| distinct.binary_search(s).expect("present")).collect()
}

/// **Color refinement (1-dimensional Weisfeiler–Leman)** of a formula's variable–clause incidence graph —
/// the coarsest *equitable* partition of the variables, computed in polynomial time. Variables and clauses
/// are coloured (clauses initialised by width); each round recolours a clause by the sorted multiset of its
/// incident `(variable colour, sign)` pairs and a variable by the sorted multiset of its incident
/// `(clause colour, sign)` pairs, to a fixpoint. The result `cell[v]` is the variable's colour.
///
/// This is the polynomial *foundation* of symmetry detection (the pre-filter saucy/nauty run before the
/// exponential search) and a SOUND over-approximation of the orbit partition: every phase-free variable
/// automorphism preserves the colouring, so `orbit(v) ⊆ cell(v)` — variables of different colours are
/// provably in different orbits. Returns dense cell indices `0..k`.
pub fn color_refinement(num_vars: usize, clauses: &[Vec<Lit>]) -> Vec<usize> {
    equitable_refine(num_vars, clauses, &vec![0usize; num_vars])
}

/// Color refinement to a stable (equitable) partition, starting from an arbitrary initial variable coloring
/// `var_init` (rather than the all-equal coloring [`color_refinement`] uses). This is the refinement step of
/// individualization–refinement: after individualizing a vertex, re-stabilize from the perturbed coloring.
fn equitable_refine(num_vars: usize, clauses: &[Vec<Lit>], var_init: &[usize]) -> Vec<usize> {
    let mut var_color = rank_signatures(var_init);
    let mut clause_color: Vec<usize> = rank_signatures(&clauses.iter().map(|c| c.len()).collect::<Vec<_>>());
    loop {
        // Recolour clauses by the multiset of incident (variable colour, sign).
        let clause_sig: Vec<(usize, Vec<(usize, bool)>)> = clauses
            .iter()
            .enumerate()
            .map(|(ci, c)| {
                let mut nbrs: Vec<(usize, bool)> =
                    c.iter().map(|l| (var_color[l.var() as usize], l.is_positive())).collect();
                nbrs.sort_unstable();
                (clause_color[ci], nbrs)
            })
            .collect();
        let new_clause_color = rank_signatures(&clause_sig);
        // Recolour variables by the multiset of incident (clause colour, sign).
        let mut var_nbrs: Vec<Vec<(usize, bool)>> = vec![Vec::new(); num_vars];
        for (ci, c) in clauses.iter().enumerate() {
            for l in c {
                var_nbrs[l.var() as usize].push((new_clause_color[ci], l.is_positive()));
            }
        }
        for nbrs in var_nbrs.iter_mut() {
            nbrs.sort_unstable();
        }
        let var_sig: Vec<(usize, Vec<(usize, bool)>)> =
            (0..num_vars).map(|v| (var_color[v], std::mem::take(&mut var_nbrs[v]))).collect();
        let new_var_color = rank_signatures(&var_sig);
        if new_var_color == var_color && new_clause_color == clause_color {
            break; // the partition is stable (equitable)
        }
        var_color = new_var_color;
        clause_color = new_clause_color;
    }
    var_color
}

/// The number of cells in the color-refinement (equitable) partition — a cheap polynomial symmetry
/// indicator. At most the number of variable orbits (`cells ≤ orbits`, since each cell is a union of orbits).
pub fn color_refinement_cells(num_vars: usize, clauses: &[Vec<Lit>]) -> usize {
    color_refinement(num_vars, clauses).iter().copied().max().map_or(0, |m| m + 1)
}

/// The variables that color refinement places in a **singleton cell** — provably fixed by every (phase-free)
/// automorphism, since `orbit(v) ⊆ cell(v)` and `|cell(v)| = 1` forces `|orbit(v)| = 1`. A polynomial
/// certificate of asymmetry: these variables can be skipped entirely when detecting or breaking symmetry.
pub fn provably_asymmetric_variables(num_vars: usize, clauses: &[Vec<Lit>]) -> Vec<usize> {
    let cells = color_refinement(num_vars, clauses);
    let mut size = vec![0usize; num_vars];
    for &c in &cells {
        size[c] += 1;
    }
    (0..num_vars).filter(|&v| size[cells[v]] == 1).collect()
}

/// The **variable co-occurrence matrix** `A[u][v]` = number of clauses containing both variables `u` and `v`
/// (`0` on the diagonal) — the natural weighted graph on the formula's variables.
fn cooccurrence_matrix(num_vars: usize, clauses: &[Vec<Lit>]) -> Vec<Vec<i64>> {
    let mut a = vec![vec![0i64; num_vars]; num_vars];
    for c in clauses {
        let vars: Vec<usize> = c.iter().map(|l| l.var() as usize).collect();
        for &u in &vars {
            for &v in &vars {
                if u != v {
                    a[u][v] += 1;
                }
            }
        }
    }
    a
}

/// The coarsest **equitable partition** of the variable co-occurrence graph `A`: 1-WL on the weighted graph,
/// recolouring each vertex by the sorted multiset of `(neighbour colour, edge weight)` to a fixpoint.
fn equitable_partition_of(num_vars: usize, a: &[Vec<i64>]) -> Vec<usize> {
    let mut color = vec![0usize; num_vars];
    loop {
        let sig: Vec<(usize, Vec<(usize, i64)>)> = (0..num_vars)
            .map(|u| {
                let mut nbrs: Vec<(usize, i64)> =
                    (0..num_vars).filter(|&v| a[u][v] != 0).map(|v| (color[v], a[u][v])).collect();
                nbrs.sort_unstable();
                (color[u], nbrs)
            })
            .collect();
        let next = rank_signatures(&sig);
        if next == color {
            return color;
        }
        color = next;
    }
}

/// A **fractional automorphism** of a formula: a *doubly-stochastic* matrix `B` commuting with the variable
/// co-occurrence matrix `A` (`BA = AB`) — the LP relaxation of an automorphism (a *permutation* matrix
/// commuting with `A`). The canonical one is block-averaging over the coarsest equitable partition
/// ([`equitable_partition_of`]): `B[u][v] = 1/|cell(u)|` if `u,v` share a cell, else `0`. By Tinhofer's
/// theorem it commutes with `A` iff the partition is equitable, and it is **non-trivial** (not a permutation)
/// exactly when the partition is non-discrete — i.e. exactly when the formula has non-trivial 1-WL symmetry.
///
/// Returns the partition `cell[v]`. Its non-discreteness certifies a fractional automorphism; the commutation
/// `BA = AB` is checked exactly (over integers, by clearing the `1/|cell|` denominators) by
/// [`is_fractional_automorphism`].
pub fn fractional_automorphism(num_vars: usize, clauses: &[Vec<Lit>]) -> Vec<usize> {
    equitable_partition_of(num_vars, &cooccurrence_matrix(num_vars, clauses))
}

/// Verify that block-averaging over `partition` gives a doubly-stochastic matrix `B` commuting with the
/// co-occurrence matrix `A`. `B` is doubly-stochastic by construction (each row/column sums to 1), so this
/// checks `BA = AB`, which holds iff `partition` is equitable — the exact (integer) certificate. The identity
/// `(1/|cell(u)|)·Σ_{x∈cell(u)} A[x][w] = (1/|cell(w)|)·Σ_{x∈cell(w)} A[u][x]` is verified cross-multiplied.
pub fn is_fractional_automorphism(num_vars: usize, clauses: &[Vec<Lit>], partition: &[usize]) -> bool {
    let a = cooccurrence_matrix(num_vars, clauses);
    let d = partition.iter().copied().max().map_or(0, |m| m + 1);
    let mut cell: Vec<Vec<usize>> = vec![Vec::new(); d];
    for (v, &c) in partition.iter().enumerate() {
        cell[c].push(v);
    }
    let sum_over = |rows: &[usize], w: usize, by_row: bool| -> i64 {
        rows.iter().map(|&x| if by_row { a[x][w] } else { a[w][x] }).sum()
    };
    for u in 0..num_vars {
        for w in 0..num_vars {
            let cu = &cell[partition[u]];
            let cw = &cell[partition[w]];
            // (1/|cu|)·Σ_{x∈cu} A[x][w] == (1/|cw|)·Σ_{x∈cw} A[u][x]  (cross-multiplied to integers).
            let lhs = cw.len() as i64 * sum_over(cu, w, true);
            let rhs = cu.len() as i64 * sum_over(cw, u, false);
            if lhs != rhs {
                return false;
            }
        }
    }
    true
}

/// **2-dimensional Weisfeiler–Leman (2-WL)** of a formula's variables — color refinement lifted from
/// vertices to *ordered pairs*. Each pair `(i,j)` is colored: the diagonal by its 1-WL vertex color, an
/// off-diagonal pair by `(1-WL(i), 1-WL(j), co-occurrence signature)` where the signature is the sorted
/// multiset over clauses containing both of `(clause width, sign of i, sign of j)`. Each round recolors
/// `(i,j)` by `(old color, sorted multiset over all k of (color(i,k), color(k,j)))`, to a fixpoint.
///
/// Strictly stronger than 1-WL (its diagonal refines the 1-WL coloring, and it sees pair structure 1-WL is
/// blind to) and a SOUND over-approximation of the **orbitals** (orbits on ordered pairs): every variable
/// automorphism preserves the pair coloring, so `orbital(i,j) ⊆ paircell(i,j)`. Returns the `n×n` color
/// matrix. `O(rounds · n³)`, so intended for moderate `n`.
pub fn two_wl_pair_colors(num_vars: usize, clauses: &[Vec<Lit>]) -> Vec<Vec<usize>> {
    let wl1 = color_refinement(num_vars, clauses);
    // Co-occurrence signature per ordered pair: how i and j appear together across clauses.
    let mut cooc: Vec<Vec<Vec<(usize, bool, bool)>>> = vec![vec![Vec::new(); num_vars]; num_vars];
    for c in clauses {
        let lits: Vec<(usize, bool)> = c.iter().map(|l| (l.var() as usize, l.is_positive())).collect();
        for &(vi, si) in &lits {
            for &(vj, sj) in &lits {
                if vi != vj {
                    cooc[vi][vj].push((c.len(), si, sj));
                }
            }
        }
    }
    for row in cooc.iter_mut() {
        for s in row.iter_mut() {
            s.sort_unstable();
        }
    }
    // Initial pair signature: (diagonal?, 1-WL(i), 1-WL(j), co-occurrence).
    let mut flat: Vec<(bool, usize, usize, Vec<(usize, bool, bool)>)> = Vec::with_capacity(num_vars * num_vars);
    for i in 0..num_vars {
        for j in 0..num_vars {
            flat.push((i == j, wl1[i], wl1[j], cooc[i][j].clone()));
        }
    }
    let mut color: Vec<usize> = rank_signatures(&flat); // indexed i*num_vars + j
    let at = |i: usize, j: usize| i * num_vars + j;
    loop {
        let mut sig: Vec<(usize, Vec<(usize, usize)>)> = Vec::with_capacity(num_vars * num_vars);
        for i in 0..num_vars {
            for j in 0..num_vars {
                let mut tri: Vec<(usize, usize)> =
                    (0..num_vars).map(|k| (color[at(i, k)], color[at(k, j)])).collect();
                tri.sort_unstable();
                sig.push((color[at(i, j)], tri));
            }
        }
        let next = rank_signatures(&sig);
        if next == color {
            break;
        }
        color = next;
    }
    (0..num_vars).map(|i| (0..num_vars).map(|j| color[at(i, j)]).collect()).collect()
}

/// The number of distinct 2-WL pair colors — at most the number of orbitals (`pair-cells ≤ orbitals`).
pub fn two_wl_pair_cells(num_vars: usize, clauses: &[Vec<Lit>]) -> usize {
    let c = two_wl_pair_colors(num_vars, clauses);
    c.iter().flatten().copied().max().map_or(0, |m| m + 1)
}

/// The label-independent **2-WL fingerprint**: the sorted multiset of pair-color class sizes. Two formulas
/// with different fingerprints are provably non-isomorphic (as colored variable structures) — the
/// distinguishing power that separates, e.g., a 6-cycle from two triangles where 1-WL cannot.
pub fn two_wl_fingerprint(num_vars: usize, clauses: &[Vec<Lit>]) -> Vec<usize> {
    let c = two_wl_pair_colors(num_vars, clauses);
    let k = c.iter().flatten().copied().max().map_or(0, |m| m + 1);
    let mut sizes = vec![0usize; k];
    for &col in c.iter().flatten() {
        sizes[col] += 1;
    }
    sizes.sort_unstable();
    sizes
}

/// The **coherent configuration** (association scheme) that 2-WL stabilizes into: its `d` basis relations
/// (the stable pair-color classes `R_0,…,R_{d-1}`) and their **intersection numbers**
/// `p[i][j][k] = |{z : (x,z) ∈ R_i and (z,y) ∈ R_j}|` for any `(x,y) ∈ R_k`. Returns `(d, p)`.
///
/// The intersection numbers are well-defined *because* the coloring is coherent (independent of which
/// `(x,y) ∈ R_k` is chosen) — and that well-definedness is checked over EVERY pair, FAIL-CLOSED: `None` if
/// the 2-WL coloring is somehow not coherent (it always is once stabilized, so this certifies it). These are
/// the structure constants of the coherent algebra — the combinatorial dual of the class-algebra structure
/// constants ([`crate::permgroup::class_multiplication_coefficients`]); for a group's orbital configuration
/// they are the orbital intersection numbers.
pub fn coherent_configuration_constants(num_vars: usize, clauses: &[Vec<Lit>]) -> Option<(usize, Vec<Vec<Vec<u128>>>)> {
    let n = num_vars;
    let pc = two_wl_pair_colors(n, clauses);
    let d = pc.iter().flatten().copied().max().map_or(0, |m| m + 1);
    if d == 0 {
        return None;
    }
    // The d × d count matrix [(x,z)-color][(z,y)-color] for a fixed pair (x,y).
    let count_matrix = |x: usize, y: usize| -> Vec<Vec<u128>> {
        let mut m = vec![vec![0u128; d]; d];
        for z in 0..n {
            m[pc[x][z]][pc[z][y]] += 1;
        }
        m
    };
    // p[i][j][k] from one representative pair per relation k.
    let mut rep: Vec<Option<(usize, usize)>> = vec![None; d];
    for i in 0..n {
        for j in 0..n {
            rep[pc[i][j]].get_or_insert((i, j));
        }
    }
    let mut p = vec![vec![vec![0u128; d]; d]; d];
    for k in 0..d {
        let (x, y) = rep[k]?;
        let m = count_matrix(x, y);
        for i in 0..d {
            for j in 0..d {
                p[i][j][k] = m[i][j];
            }
        }
    }
    // COHERENCE (fail-closed): every pair in R_k must yield the same intersection numbers.
    for x in 0..n {
        for y in 0..n {
            let k = pc[x][y];
            let m = count_matrix(x, y);
            for i in 0..d {
                for j in 0..d {
                    if m[i][j] != p[i][j][k] {
                        return None;
                    }
                }
            }
        }
    }
    Some((d, p))
}

/// The **rank** of the coherent configuration — the number of basis relations (`d`), the dimension of the
/// coherent algebra. Equal to [`two_wl_pair_cells`]; for a group's orbital configuration it is the orbital
/// rank. `None` when the configuration is out of range / not coherent.
pub fn coherent_rank(num_vars: usize, clauses: &[Vec<Lit>]) -> Option<usize> {
    coherent_configuration_constants(num_vars, clauses).map(|(d, _)| d)
}

/// Simultaneously diagonalize commuting `d×d` matrices over `GF(p)`: refine `GF(p)^d` into common 1-dim
/// eigenspaces by splitting on each matrix in turn, and return, per eigenspace, the vector of its eigenvalues
/// across the matrices. `None` if they do not fully diagonalize over this `GF(p)` (the caller retries with
/// another prime). The same construction the Burnside–Dixon character table uses, applied to the coherent
/// algebra's intersection matrices.
fn gf_simultaneous_eigenvalues(mmats: &[Vec<Vec<u64>>], d: usize, p: u64) -> Option<Vec<Vec<u64>>> {
    use crate::permgroup::{gf_mat_vec, gf_nullspace, mod_inv};
    let mut subspaces: Vec<Vec<Vec<u64>>> = vec![(0..d)
        .map(|i| {
            let mut e = vec![0u64; d];
            e[i] = 1;
            e
        })
        .collect()];
    for mi in mmats {
        if subspaces.iter().all(|s| s.len() == 1) {
            break;
        }
        let mut next: Vec<Vec<Vec<u64>>> = Vec::new();
        for s in &subspaces {
            if s.len() == 1 {
                next.push(s.clone());
                continue;
            }
            let bn = s.len();
            let mb: Vec<Vec<u64>> = s.iter().map(|b| gf_mat_vec(mi, b, p)).collect();
            let mut pieces: Vec<Vec<Vec<u64>>> = Vec::new();
            let mut covered = 0usize;
            for lam in 0..p {
                let mut rows = vec![vec![0u64; bn]; d];
                for k in 0..d {
                    for (jj, sj) in s.iter().enumerate() {
                        let shift = (lam as u128 * sj[k] as u128) % p as u128;
                        rows[k][jj] = ((mb[jj][k] as u128 + p as u128 - shift) % p as u128) as u64;
                    }
                }
                let ns = gf_nullspace(rows, bn, p);
                if ns.is_empty() {
                    continue;
                }
                let eig: Vec<Vec<u64>> = ns
                    .iter()
                    .map(|c| {
                        let mut x = vec![0u64; d];
                        for (jj, &cj) in c.iter().enumerate() {
                            if cj != 0 {
                                for k in 0..d {
                                    x[k] = ((x[k] as u128 + cj as u128 * s[jj][k] as u128) % p as u128) as u64;
                                }
                            }
                        }
                        x
                    })
                    .collect();
                covered += eig.len();
                pieces.push(eig);
                if covered == bn {
                    break;
                }
            }
            if covered == bn {
                next.extend(pieces);
            } else {
                next.push(s.clone());
            }
        }
        subspaces = next;
    }
    if subspaces.iter().any(|s| s.len() != 1) {
        return None; // not fully diagonalizable over this GF(p)
    }
    Some(
        subspaces
            .iter()
            .map(|s| {
                let v = &s[0];
                let t = v.iter().position(|&x| x != 0).unwrap();
                let inv = mod_inv(v[t], p);
                mmats
                    .iter()
                    .map(|mi| {
                        let mv = gf_mat_vec(mi, v, p);
                        (mv[t] as u128 * inv as u128 % p as u128) as u64
                    })
                    .collect()
            })
            .collect(),
    )
}

/// The **eigenmatrix (P-matrix) of the association scheme** of a formula's variable symmetry — the scheme's
/// "character table". The coherent configuration ([`coherent_configuration_constants`]) yields a commutative
/// algebra spanned by the relation matrices `A_i` (`A_i A_j = Σ_k a_{ijk} A_k`); its intersection matrices
/// `B_i` (`B_i[k][j] = a_{ijk}`) commute and are simultaneously diagonalizable, and `P[m][i]` is the
/// eigenvalue of `A_i` on the `m`-th common eigenspace. Computed exactly over a `GF(p)` chosen (by search)
/// so the algebra splits — Dixon's method applied to the scheme. Returns `(prime, P)`.
///
/// The rows of `P` are exactly the 1-dimensional representations of the coherent algebra, so
/// `P[m][i]·P[m][j] = Σ_k a_{ijk}·P[m][k]`; one row is the valencies `n_i` (the all-ones eigenvector). `None`
/// when the scheme is not commutative, is out of range, or no small prime splits it. Mirrors
/// [`crate::permgroup::character_table`] for the group ↔ scheme analogy.
pub fn association_scheme_eigenmatrix(num_vars: usize, clauses: &[Vec<Lit>]) -> Option<(u64, Vec<Vec<u64>>)> {
    let (d, a) = coherent_configuration_constants(num_vars, clauses)?;
    if d == 0 {
        return None;
    }
    // The algebra must be commutative for a P-matrix to exist.
    for i in 0..d {
        for j in 0..d {
            for k in 0..d {
                if a[i][j][k] != a[j][i][k] {
                    return None;
                }
            }
        }
    }
    // Valencies n_i = Σ_j a_{ij0} (independent of the third index — the all-ones eigenvalue).
    let valency: Vec<u128> = (0..d).map(|i| (0..d).map(|j| a[i][j][0]).sum()).collect();
    let mut tried = 0;
    let mut p = 2u64;
    while tried < 200 && p < 100_000 {
        if crate::permgroup::is_prime(p) {
            tried += 1;
            let mmats: Vec<Vec<Vec<u64>>> = (0..d)
                .map(|i| (0..d).map(|k| (0..d).map(|j| (a[i][j][k] % p as u128) as u64).collect()).collect())
                .collect();
            if let Some(rows) = gf_simultaneous_eigenvalues(&mmats, d, p) {
                // FAIL-CLOSED: each row is a 1-dim rep of the algebra, and some row is the valencies.
                let hom_ok = rows.iter().all(|row| {
                    (0..d).all(|i| {
                        (0..d).all(|j| {
                            let lhs = row[i] as u128 * row[j] as u128 % p as u128;
                            let rhs = (0..d)
                                .map(|k| (a[i][j][k] % p as u128) * row[k] as u128 % p as u128)
                                .sum::<u128>()
                                % p as u128;
                            lhs == rhs
                        })
                    })
                });
                let has_valency =
                    rows.iter().any(|row| (0..d).all(|i| row[i] as u128 == valency[i] % p as u128));
                if hom_ok && has_valency {
                    return Some((p, rows));
                }
            }
        }
        p += 1;
    }
    None
}

/// The **multiplicities of the association scheme** — the dimensions `m_j` of the common eigenspaces of the
/// relation matrices in the full `|X|`-dimensional space; the "degrees" of the scheme, dual to the valencies.
/// From the eigenmatrix `P` (#eigenmatrix) and the scheme orthogonality relation
/// `m_j = |X| / Σ_i P[j][i]·P[j][ī]/k_i` (`k_i` = valency, `ī` = transpose relation). Computed exactly over a
/// `GF(p)` chosen to split the algebra AND exceed `|X|` (so the small positive integer `m_j ≤ |X|` decodes
/// uniquely). Returns the multiplicities sorted ascending.
///
/// FAIL-CLOSED: `None` unless every `m_j` is a positive integer `≤ |X|`, `Σ_j m_j = |X|` (the eigenspaces
/// partition the space), and the trivial eigenspace (the valency row of `P`) has multiplicity 1. For a
/// multiplicity-free group action these are exactly the degrees of the constituent irreducibles.
pub fn association_scheme_multiplicities(num_vars: usize, clauses: &[Vec<Lit>]) -> Option<Vec<u128>> {
    let (d, a) = coherent_configuration_constants(num_vars, clauses)?;
    if d == 0 {
        return None;
    }
    for i in 0..d {
        for j in 0..d {
            for k in 0..d {
                if a[i][j][k] != a[j][i][k] {
                    return None; // commutative only
                }
            }
        }
    }
    let valency: Vec<u128> = (0..d).map(|i| (0..d).map(|j| a[i][j][0]).sum()).collect();
    // The transpose relation ī (the reverse of relation i) from the 2-WL pair coloring.
    let pc = two_wl_pair_colors(num_vars, clauses);
    let mut transpose = vec![usize::MAX; d];
    for x in 0..num_vars {
        for y in 0..num_vars {
            let i = pc[x][y];
            if transpose[i] == usize::MAX {
                transpose[i] = pc[y][x];
            }
        }
    }
    let n = num_vars as u128;
    let mut tried = 0;
    let mut p = 2u64;
    while tried < 300 && p < 1_000_000 {
        // The prime must split the algebra AND exceed |X| so multiplicities decode uniquely.
        if crate::permgroup::is_prime(p) && p as u128 > n {
            tried += 1;
            let mmats: Vec<Vec<Vec<u64>>> = (0..d)
                .map(|i| (0..d).map(|k| (0..d).map(|j| (a[i][j][k] % p as u128) as u64).collect()).collect())
                .collect();
            if let Some(rows) = gf_simultaneous_eigenvalues(&mmats, d, p) {
                let mut mult = Vec::with_capacity(d);
                let mut ok = true;
                for row in &rows {
                    let mut denom = 0u64;
                    for i in 0..d {
                        let ki = (valency[i] % p as u128) as u64;
                        let term = (row[i] as u128 * row[transpose[i]] as u128 % p as u128) as u64;
                        denom = ((denom as u128 + term as u128 * crate::permgroup::mod_inv(ki, p) as u128)
                            % p as u128) as u64;
                    }
                    if denom == 0 {
                        ok = false;
                        break;
                    }
                    let m = (n % p as u128) * crate::permgroup::mod_inv(denom, p) as u128 % p as u128;
                    if m == 0 || m > n {
                        ok = false;
                        break;
                    }
                    mult.push(m);
                }
                let trivial_ok = ok
                    && rows.iter().zip(&mult).any(|(row, &m)| {
                        m == 1 && (0..d).all(|i| row[i] as u128 == valency[i] % p as u128)
                    });
                if ok && trivial_ok && mult.iter().sum::<u128>() == n {
                    mult.sort_unstable();
                    return Some(mult);
                }
            }
        }
        p += 1;
    }
    None
}

/// **3-dimensional Weisfeiler–Leman (3-WL)** of a formula's variables — color refinement on *ordered
/// triples*. A triple `(i,j,k)` is initialized by the 2-WL colors of its three pairs
/// `(pc[i][j], pc[i][k], pc[j][k])`, then each round recolored by `(old color, sorted multiset over all w of
/// (color(w,j,k), color(i,w,k), color(i,j,w)))` to a fixpoint. Returns the `n×n×n` color tensor.
///
/// Strictly stronger than 2-WL (it distinguishes graphs with identical 2-WL colorings — e.g. the rook's
/// graph from the Shrikhande graph) and a SOUND over-approximation of the **3-orbits** (orbits on ordered
/// triples): every automorphism preserves the coloring, so `3-orbit(i,j,k) ⊆ triplecell(i,j,k)`.
/// `O(rounds · n⁴)`, so for small `n` only.
pub fn three_wl_colors(num_vars: usize, clauses: &[Vec<Lit>]) -> Vec<Vec<Vec<usize>>> {
    let n = num_vars;
    let pc = two_wl_pair_colors(n, clauses);
    let at = |i: usize, j: usize, k: usize| (i * n + j) * n + k;
    let init: Vec<(usize, usize, usize)> = (0..n)
        .flat_map(|i| (0..n).flat_map(move |j| (0..n).map(move |k| (i, j, k))))
        .map(|(i, j, k)| (pc[i][j], pc[i][k], pc[j][k]))
        .collect();
    let mut color = rank_signatures(&init);
    loop {
        let mut sig: Vec<(usize, Vec<(usize, usize, usize)>)> = Vec::with_capacity(n * n * n);
        for i in 0..n {
            for j in 0..n {
                for k in 0..n {
                    let mut nbr: Vec<(usize, usize, usize)> =
                        (0..n).map(|w| (color[at(w, j, k)], color[at(i, w, k)], color[at(i, j, w)])).collect();
                    nbr.sort_unstable();
                    sig.push((color[at(i, j, k)], nbr));
                }
            }
        }
        let next = rank_signatures(&sig);
        if next == color {
            break;
        }
        color = next;
    }
    (0..n)
        .map(|i| (0..n).map(|j| (0..n).map(|k| color[at(i, j, k)]).collect()).collect())
        .collect()
}

/// The label-independent **3-WL fingerprint**: the sorted multiset of triple-color class sizes. Separates
/// strongly-regular graphs that share every 2-WL invariant (the rook's vs Shrikhande graph).
pub fn three_wl_fingerprint(num_vars: usize, clauses: &[Vec<Lit>]) -> Vec<usize> {
    let c = three_wl_colors(num_vars, clauses);
    let max = c.iter().flatten().flatten().copied().max().map_or(0, |m| m + 1);
    let mut sizes = vec![0usize; max];
    for &col in c.iter().flatten().flatten() {
        sizes[col] += 1;
    }
    sizes.sort_unstable();
    sizes
}

/// The canonical certificate of a formula under a *discrete* variable coloring (a labeling): the clause set
/// rewritten with each variable replaced by its dense label, every clause sorted, and the clauses sorted —
/// an exact representation of the formula as seen through that labeling.
fn formula_certificate(clauses: &[Vec<Lit>], coloring: &[usize]) -> Vec<Vec<(usize, bool)>> {
    let label = rank_signatures(coloring); // discrete coloring ⇒ a bijection onto 0..n-1
    let mut cls: Vec<Vec<(usize, bool)>> = clauses
        .iter()
        .map(|c| {
            let mut lits: Vec<(usize, bool)> =
                c.iter().map(|l| (label[l.var() as usize], l.is_positive())).collect();
            lits.sort_unstable();
            lits
        })
        .collect();
    cls.sort_unstable();
    cls
}

/// The individualization–refinement search: refine the coloring; if discrete, return its certificate; else
/// individualize each vertex of the first non-singleton cell in turn, recurse, and keep the lexicographically
/// maximal leaf certificate. `None` if the node budget is exceeded (so the canonical form is not certified).
fn ir_canonical(
    num_vars: usize,
    clauses: &[Vec<Lit>],
    colors: &[usize],
    nodes: &mut usize,
    cap: usize,
) -> Option<Vec<Vec<(usize, bool)>>> {
    *nodes += 1;
    if *nodes > cap {
        return None;
    }
    let refined = equitable_refine(num_vars, clauses, colors);
    let d = refined.iter().copied().max().map_or(0, |m| m + 1);
    let mut members: Vec<Vec<usize>> = vec![Vec::new(); d];
    for (v, &c) in refined.iter().enumerate() {
        members[c].push(v);
    }
    match (0..d).find(|&c| members[c].len() > 1) {
        None => Some(formula_certificate(clauses, &refined)), // discrete ⇒ a leaf
        Some(c) => {
            let mut best: Option<Vec<Vec<(usize, bool)>>> = None;
            for &v in &members[c] {
                let mut nc = refined.clone();
                nc[v] = d; // individualize v: give it a fresh, unique color
                let leaf = ir_canonical(num_vars, clauses, &nc, nodes, cap)?;
                if best.as_ref().map_or(true, |b| leaf > *b) {
                    best = Some(leaf);
                }
            }
            best
        }
    }
}

/// The **canonical form** of a formula's variable structure, computed by individualization–refinement (the
/// algorithm behind nauty/saucy/bliss). It is an *isomorphism invariant*: two formulas equal up to a
/// variable permutation that preserves the clause structure have the **same** canonical form, and — since
/// I-R is complete — two that are not get **different** canonical forms (so it decides isomorphism exactly,
/// unlike Weisfeiler–Leman). Built on [`color_refinement`] as its refinement step. `None` if the search
/// exceeds its node budget (gate to moderate `num_vars`).
pub fn canonical_form(num_vars: usize, clauses: &[Vec<Lit>]) -> Option<Vec<Vec<(usize, bool)>>> {
    let mut nodes = 0;
    ir_canonical(num_vars, clauses, &vec![0usize; num_vars], &mut nodes, 200_000)
}

/// Whether two formulas over the same variables are **isomorphic** — equal up to a variable permutation
/// preserving the (signed) clause structure — by comparing their canonical forms. `None` if either canonical
/// form exceeds the search budget.
pub fn formulas_isomorphic(num_vars: usize, f: &[Vec<Lit>], g: &[Vec<Lit>]) -> Option<bool> {
    Some(canonical_form(num_vars, f)? == canonical_form(num_vars, g)?)
}

/// Is `g` a genuine symmetry of `clauses` that may be trusted for breaking? A well-formed permutation that
/// either preserves the clause set (syntactic) or whose image is logically entailed, `F ⊨ g(F)` — which for
/// a finite-order permutation means `F ≡ g(F)` (semantic). A malformed or non-symmetric declaration is
/// rejected, so a wrong declaration can never corrupt the result.
fn is_declared_symmetry(num_vars: usize, clauses: &[Vec<Lit>], g: &[usize]) -> bool {
    if g.len() != num_vars {
        return false;
    }
    let mut seen = vec![false; num_vars];
    for &x in g {
        if x >= num_vars || seen[x] {
            return false; // not a permutation
        }
        seen[x] = true;
    }
    let clause_set: HashSet<Vec<(u32, bool)>> = clauses.iter().map(|c| canon_clause(c)).collect();
    let syntactic = clauses.iter().all(|c| clause_set.contains(&canon_clause(&apply_perm_to_clause(c, g))));
    syntactic
        || clauses.iter().all(|c| {
            let gc = apply_perm_to_clause(c, g);
            clause_set.contains(&canon_clause(&gc)) || clause_is_implied(num_vars, clauses, &gc)
        })
}

/// **The full arsenal with caller-DECLARED symmetry.** The modeler often knows symmetries the automatic
/// detector cannot afford to find (geometric, semantic, or simply large). This entry point accepts declared
/// variable-permutation generators, **verifies** each is a genuine symmetry ([`is_declared_symmetry`] —
/// never trusting a declaration blindly, so an incorrect one is silently dropped), unions the survivors with
/// the auto-detected symmetry, and breaks the combined group with the lex-leader (complete by enumeration
/// when the group is small, partial over the generators otherwise). Sound: only verified, model-set-
/// preserving permutations enter the break; a SAT model is re-checked, and on any anomaly it falls back to
/// the authoritative [`solve_comprehensive`]. With no declared and no detected symmetry it simply *is*
/// [`solve_comprehensive`].
pub fn solve_with_declared_symmetry(
    num_vars: usize,
    clauses: &[Vec<Lit>],
    declared: &[crate::permgroup::Perm],
) -> Solved {
    let mut gens = crate::sym_break::variable_automorphism_generators(num_vars, clauses).unwrap_or_default();
    for g in declared {
        if is_declared_symmetry(num_vars, clauses, g) {
            gens.push(g.clone());
        }
    }
    if gens.is_empty() {
        return solve_comprehensive(num_vars, clauses);
    }
    let bsgs = crate::permgroup::schreier_sims(num_vars, &gens);
    let to_litsym = |p: &[usize]| -> Vec<Lit> { (0..num_vars).map(|v| Lit::pos(p[v] as u32)).collect() };
    let group: Vec<Vec<Lit>> = match bsgs.elements(50_000) {
        Some(elts) => elts.iter().map(|p| to_litsym(p)).collect(),
        None => gens.iter().map(|p| to_litsym(p)).collect(),
    };
    let (sbp, total) = crate::sym_break::lex_leader_sbp_lit(num_vars, &group);
    let mut solver = Solver::new(total);
    for c in clauses {
        solver.add_clause(c.clone());
    }
    for c in &sbp {
        solver.add_clause(c.clone());
    }
    match solver.solve() {
        SolveResult::Sat(model) => {
            let projected: Vec<bool> = model[..num_vars].to_vec();
            if clauses.iter().all(|c| c.iter().any(|l| projected[l.var() as usize] == l.is_positive())) {
                Solved { answer: Answer::Sat(projected), via: Route::DeclaredSymmetry, proof: Vec::new(), conflicts: solver.conflicts() }
            } else {
                solve_comprehensive(num_vars, clauses) // re-check failed ⟹ authoritative fallback
            }
        }
        SolveResult::Unsat => {
            Solved { answer: Answer::Unsat, via: Route::DeclaredSymmetry, proof: Vec::new(), conflicts: solver.conflicts() }
        }
    }
}

/// The solution set described **up to symmetry**: one representative per orbit, plus the exact total.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SymmetricCount {
    /// One satisfying assignment per orbit of the model set under the variable-symmetry group — the
    /// essentially-distinct solutions.
    pub representatives: Vec<Vec<bool>>,
    /// The EXACT number of models (over the variables that occur), recovered as the sum of orbit sizes —
    /// so a `2^Θ(n)` model count is obtained by enumerating only the orbits.
    pub total_models: u128,
    /// `false` if the representative cap was reached before the model set was exhausted.
    pub exhaustive: bool,
}

/// The orbit of an assignment under the variable-symmetry generators, as the distinct assignments reached
/// by the group (deduplicated on the occurring variables — free variables are immaterial to a model).
fn assignment_orbit(num_vars: usize, m: &[bool], gens: &[crate::permgroup::Perm], occurs: &[usize]) -> Vec<Vec<bool>> {
    let proj = |a: &[bool]| -> Vec<bool> { occurs.iter().map(|&v| a[v]).collect() };
    let mut seen: HashSet<Vec<bool>> = HashSet::from([proj(m)]);
    let mut out = vec![m.to_vec()];
    let mut i = 0;
    while i < out.len() {
        let cur = out[i].clone();
        i += 1;
        for g in gens {
            let mut pm = vec![false; num_vars];
            for v in 0..num_vars {
                pm[g[v]] = cur[v];
            }
            if seen.insert(proj(&pm)) {
                out.push(pm);
            }
        }
    }
    out
}

/// **Enumerate the solution set up to symmetry, and count it exactly.** Find a model (via the full
/// arsenal), record it as an orbit representative, then BLOCK its entire symmetry orbit and repeat until
/// unsatisfiable. Because every orbit is removed in one step, the representatives are the essentially-
/// distinct solutions and the exact model count is the sum of the orbit sizes — so an instance with
/// `2^Θ(n)` models is counted by enumerating only its (far fewer) orbits. Sound: each representative is a
/// re-checked model, each orbit a genuine set of models (the generators are automorphisms), so the orbits
/// partition the model set. The representative count agrees with Burnside
/// ([`crate::sym_break::count_models_modulo_symmetry`]) — two independent routes to the orbit count.
pub fn models_up_to_symmetry(num_vars: usize, clauses: &[Vec<Lit>], cap: usize) -> SymmetricCount {
    let mut appears = vec![false; num_vars];
    for c in clauses {
        for l in c {
            appears[l.var() as usize] = true;
        }
    }
    let occurs: Vec<usize> = (0..num_vars).filter(|&v| appears[v]).collect();
    let gens = crate::sym_break::variable_automorphism_generators(num_vars, clauses).unwrap_or_default();

    let mut working = clauses.to_vec();
    let mut representatives: Vec<Vec<bool>> = Vec::new();
    let mut total_models: u128 = 0;
    let mut exhaustive = true;
    loop {
        if representatives.len() >= cap {
            exhaustive = false;
            break;
        }
        match solve_comprehensive(num_vars, &working).answer {
            Answer::Unsat => break,
            Answer::Sat(m) => {
                let orbit = assignment_orbit(num_vars, &m, &gens, &occurs);
                total_models = total_models.saturating_add(orbit.len() as u128);
                representatives.push(m);
                for a in &orbit {
                    // Block this assignment on the occurring variables (free variables are immaterial).
                    working.push(occurs.iter().map(|&v| Lit::new(v as u32, !a[v])).collect());
                }
            }
        }
    }
    SymmetricCount { representatives, total_models, exhaustive }
}

/// The structural profile of a formula's **variable-symmetry group** — `|Aut|`, the orbit/rank/transitivity
/// ladder, primitivity, and block structure. The data a surfacing layer (e.g. a Studio panel) needs to
/// explain *why* an instance is symmetric.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SymmetryProfile {
    /// `|Aut|` of the phase-free variable-symmetry group (`1` when there is no symmetry).
    pub order: u128,
    /// Number of detected generators.
    pub generators: usize,
    /// Variable orbits (points moved together).
    pub num_orbits: usize,
    /// Rank — orbits on ordered pairs (`2` iff 2-transitive); `0` when not computed (group too large).
    pub rank: usize,
    /// Transitivity degree, capped (`1` = transitive, `2` = 2-transitive, …).
    pub transitivity: usize,
    /// Whether the (transitive) group is primitive.
    pub primitive: bool,
    /// The number of minimal blocks when the group is transitive and imprimitive, else `None`.
    pub blocks: Option<usize>,
    /// Whether the group is abelian.
    pub abelian: bool,
    /// Whether the group is solvable (its derived series reaches the trivial group); `None` when not
    /// computed (group too large for the derived-series walk).
    pub solvable: Option<bool>,
    /// Whether the group is nilpotent (its lower central series reaches the trivial group); strictly
    /// stronger than solvable. `None` when not computed.
    pub nilpotent: Option<bool>,
    /// Derived length (solvability class) — the depth of the derived series; `None` if unsolvable / not
    /// computed.
    pub derived_length: Option<usize>,
    /// Nilpotency class — the depth of the lower central series; `None` if not nilpotent / not computed.
    pub nilpotency_class: Option<usize>,
    /// Order of the derived (commutator) subgroup `[G, G]`; `0` when not computed.
    pub derived_order: u128,
    /// Number of conjugacy classes (= number of irreducible representations); `None` when the group is too
    /// large to enumerate.
    pub conjugacy_classes: Option<usize>,
    /// Order of the centre `Z(G)`; `None` when too large to enumerate.
    pub center_order: Option<u128>,
    /// The exponent — lcm of element orders (smallest `e` with `gᵉ = id` for all `g`); `None` when too
    /// large to enumerate.
    pub exponent: Option<u128>,
    /// The number of distinct `{0,1}` assignments to the variables up to symmetry (Pólya with 2 colours) —
    /// the symmetry-reduced size of the search space; `None` when too large to enumerate.
    pub assignment_orbits: Option<u128>,
    /// The abelianisation `G/[G,G]` as `(order, exponent)` — the largest abelian quotient; `None` when too
    /// large to enumerate.
    pub abelianization: Option<(u128, u128)>,
    /// The number of subgroups (size of the subgroup lattice); `None` for larger groups (the lattice walk
    /// is gated more tightly than the other invariants).
    pub subgroups: Option<usize>,
    /// Whether the group is simple (non-trivial, no normal subgroup but itself and `{id}`); `None` when too
    /// large to enumerate.
    pub simple: Option<bool>,
    /// The composition factors (Jordan–Hölder) as the sorted multiset of their orders; their product is
    /// `|G|`. `None` when the lattice walk is out of range.
    pub composition_factors: Option<Vec<u128>>,
    /// The Sylow structure as `(p, n_p)` pairs — the number of Sylow `p`-subgroups per prime; `None` when
    /// the lattice walk is out of range.
    pub sylow: Option<Vec<(u128, usize)>>,
    /// The number of real conjugacy classes (= number of real irreducible characters); `None` when too
    /// large to enumerate.
    pub real_classes: Option<usize>,
    /// The number of **rational** conjugacy classes (= number of rational-valued irreducible characters) —
    /// the singleton Galois orbits. Strictly refines [`Self::real_classes`] (`rational ≤ real`); they differ
    /// exactly when a character is real but irrational. `None` when too large to enumerate.
    pub rational_classes: Option<usize>,
    /// The irreducible-representation degrees `χ_s(1)` (sorted, `Σ dᵢ² = |G|`) from the Burnside–Dixon
    /// character table — the complete representation-theoretic fingerprint of the symmetry; `None` when the
    /// character table is out of range.
    pub irreducible_degrees: Option<Vec<u128>>,
    /// The Frobenius–Schur indicators (one per irreducible, aligned with the character table): `+1` real,
    /// `0` complex, `−1` quaternionic. Refines [`Self::real_classes`] and separates groups with identical
    /// character tables (`D₄` vs `Q₈`). `None` when the character table is out of range.
    pub frobenius_schur: Option<Vec<i8>>,
    /// The isotypic decomposition of the variable-permutation representation: the multiplicity `m_s` of each
    /// irreducible in `π = Σ_s m_s χ_s` (aligned with the character table). Bridges the action and the linear
    /// theory — `m_trivial = num_orbits`, `Σ m_s² = rank`, `Σ m_s·d_s = num_vars`. `None` when out of range.
    pub isotypic_multiplicities: Option<Vec<u128>>,
    /// The order of `Aut(G)` — the automorphism group of the symmetry group itself. The intrinsic symmetry of
    /// `G` as an abstract group. `None` when out of range. Separates groups indistinguishable by character
    /// table / Frobenius–Schur / rationality (e.g. `|Aut(D₄)|=8` vs `|Aut(Q₈)|=24`).
    pub automorphism_order: Option<u128>,
    /// The order of `Out(G) = Aut(G)/Inn(G)` — the outer automorphisms (those not realised by conjugation).
    /// `None` when out of range.
    pub outer_automorphism_order: Option<u128>,
    /// The **coherent (association-scheme) rank** — the number of relations in the coherent configuration
    /// that 2-WL stabilizes to (`= two_wl_pair_cells`), the combinatorial analog of the orbital `rank`. It is
    /// `≤ rank` (2-WL can only be coarser than the true orbitals), so a value below `rank` witnesses that the
    /// polynomial pre-filter cannot resolve the full pair structure. Clause-derived: `Some` only from
    /// [`symmetry_structure`] (`None` for the pseudo-Boolean coefficient profile, which has no clauses).
    pub coherent_rank: Option<usize>,
}

/// Compute the [`SymmetryProfile`] of `clauses` — detect the variable-symmetry generators and read off the
/// whole structural ladder (order via Schreier–Sims, orbits, orbitals/rank, transitivity, primitivity,
/// blocks). The rank and transitivity rungs are gated to moderate sizes (their tuple spaces grow with the
/// degree); `0` signals "not computed" there.
pub fn symmetry_structure(num_vars: usize, clauses: &[Vec<Lit>]) -> SymmetryProfile {
    let gens = crate::sym_break::variable_automorphism_generators(num_vars, clauses).unwrap_or_default();
    let mut profile = profile_of_generators(num_vars, &gens);
    // The coherent (association-scheme) rank is clause-derived — wire it in here (the group profile alone
    // cannot see it). It refines the orbital rank: coherent_rank ≤ rank.
    profile.coherent_rank = Some(coherent_rank(num_vars, clauses).unwrap_or_else(|| two_wl_pair_cells(num_vars, clauses)));
    profile
}

/// The structural profile of the group generated by `gens` over `num_vars` points — the shared core behind
/// [`symmetry_structure`] (CNF variable symmetry) and [`pb_symmetry_profile`] (pseudo-Boolean coefficient
/// symmetry).
fn profile_of_generators(num_vars: usize, gens: &[crate::permgroup::Perm]) -> SymmetryProfile {
    if gens.is_empty() {
        return SymmetryProfile {
            order: 1,
            generators: 0,
            num_orbits: num_vars,
            rank: 0,
            transitivity: 0,
            primitive: false,
            blocks: None,
            abelian: true, // the trivial group is abelian
            solvable: Some(true),
            nilpotent: Some(true),
            derived_length: Some(0),
            nilpotency_class: Some(0),
            derived_order: 1,
            conjugacy_classes: Some(1),
            center_order: Some(1),
            exponent: Some(1),
            assignment_orbits: Some(1u128 << num_vars.min(127)),
            abelianization: Some((1, 1)),
            subgroups: Some(1),
            simple: Some(false), // the trivial group is not simple
            composition_factors: Some(Vec::new()),
            sylow: Some(Vec::new()),
            irreducible_degrees: Some(vec![1]), // the trivial group has one trivial irreducible
            frobenius_schur: Some(vec![1]),     // its trivial character is real
            // The permutation rep of the trivial group is `num_vars` copies of the trivial irreducible.
            isotypic_multiplicities: Some(vec![num_vars as u128]),
            real_classes: Some(1),
            rational_classes: Some(1), // the trivial character is rational
            automorphism_order: Some(1), // the trivial group has only the identity automorphism
            outer_automorphism_order: Some(1),
            coherent_rank: None, // scheme rank is clause-derived; set by symmetry_structure
        };
    }
    let gens = gens.to_vec();
    let order = crate::permgroup::schreier_sims(num_vars, &gens).order();
    let num_orbits = crate::permgroup::orbits(num_vars, &gens).len();
    let rank = if num_vars <= 48 { crate::permgroup::rank(num_vars, &gens) } else { 0 };
    let transitivity =
        if num_vars <= 24 { crate::permgroup::transitivity_degree(num_vars, &gens, 3) } else { 0 };
    let primitive = crate::permgroup::is_primitive(num_vars, &gens);
    let blocks = crate::permgroup::minimal_block_system(num_vars, &gens).map(|b| b.len());
    let abelian = crate::permgroup::is_abelian(num_vars, &gens);
    // The series walks are heavier (repeated Schreier–Sims); gate them to moderate sizes. Compute the
    // depths once and read the booleans off them.
    let (solvable, nilpotent, derived_length, nilpotency_class, derived_order) = if num_vars <= 24 {
        let dl = crate::permgroup::derived_length(num_vars, &gens);
        let nc = crate::permgroup::nilpotency_class(num_vars, &gens);
        let d = crate::permgroup::derived_subgroup(num_vars, &gens);
        (Some(dl.is_some()), Some(nc.is_some()), dl, nc, crate::permgroup::schreier_sims(num_vars, &d).order())
    } else {
        (None, None, None, None, 0)
    };
    // Conjugacy classes (⇒ #irreps) and the centre need the group enumerated; cap the order.
    const ENUM_CAP: usize = 4096;
    let classes = crate::permgroup::conjugacy_classes(num_vars, &gens, ENUM_CAP);
    let conjugacy_classes = classes.as_ref().map(|c| c.len());
    let center_order = classes.map(|c| c.iter().filter(|cls| cls.len() == 1).count() as u128);
    let exponent = crate::permgroup::exponent(num_vars, &gens, ENUM_CAP);
    let assignment_orbits = crate::permgroup::polya_count(num_vars, &gens, 2, ENUM_CAP);
    let abelianization = crate::permgroup::abelianization(num_vars, &gens, ENUM_CAP);
    // The subgroup-lattice walk is heavier (exponential worst case) — gate it to small groups.
    let subgroups = crate::permgroup::subgroup_count(num_vars, &gens, 256);
    let simple = crate::permgroup::is_simple(num_vars, &gens, ENUM_CAP);
    // Composition factors share the subgroup-lattice walk — gate to small groups.
    let composition_factors = crate::permgroup::composition_factor_orders(num_vars, &gens, 256);
    let sylow = crate::permgroup::sylow_counts(num_vars, &gens, 256);
    let real_classes = crate::permgroup::real_class_count(num_vars, &gens, ENUM_CAP);
    let rational_classes = crate::permgroup::rational_class_count(num_vars, &gens, ENUM_CAP);
    // The Burnside–Dixon character table is computed once and both the degrees and the Frobenius–Schur
    // indicators are read off it.
    let ctable = crate::permgroup::character_table(num_vars, &gens, ENUM_CAP);
    let irreducible_degrees = ctable.as_ref().map(|t| t.degrees.clone());
    let frobenius_schur = ctable.as_ref().and_then(crate::permgroup::frobenius_schur_from_table);
    let isotypic_multiplicities =
        ctable.as_ref().and_then(|t| crate::permgroup::isotypic_from_table(num_vars, &gens, t));
    // The automorphism group of the symmetry group itself (gated tighter — the search is heavier).
    let automorphism_order = crate::permgroup::automorphism_group_order(num_vars, &gens, 256);
    let outer_automorphism_order = match (automorphism_order, center_order) {
        (Some(a), Some(c)) if c > 0 => Some(a / (order / c)), // |Out| = |Aut| / |Inn|, |Inn| = |G|/|Z|
        _ => None,
    };
    SymmetryProfile {
        order,
        generators: gens.len(),
        num_orbits,
        rank,
        transitivity,
        primitive,
        blocks,
        abelian,
        solvable,
        nilpotent,
        derived_length,
        nilpotency_class,
        derived_order,
        conjugacy_classes,
        center_order,
        exponent,
        assignment_orbits,
        abelianization,
        subgroups,
        simple,
        composition_factors,
        sylow,
        real_classes,
        rational_classes,
        irreducible_degrees,
        frobenius_schur,
        isotypic_multiplicities,
        automorphism_order,
        outer_automorphism_order,
        coherent_rank: None, // clause-derived; filled in by symmetry_structure
    }
}

/// The [`SymmetryProfile`] of a pseudo-Boolean system's **coefficient-symmetry** group — variables that
/// share a coefficient profile across all constraints ([`crate::pseudo_boolean::coeff_symmetry_generators`])
/// — read through the same structural ladder as [`symmetry_structure`]. The symmetry of *weighted*
/// constraints, surfaced exactly like the clause-structure symmetry.
pub fn pb_symmetry_profile(num_vars: usize, constraints: &[crate::pseudo_boolean::PbConstraint]) -> SymmetryProfile {
    let gens = crate::pseudo_boolean::coeff_symmetry_generators(num_vars, constraints);
    profile_of_generators(num_vars, &gens)
}

/// The **class-algebra structure constants** of a formula's variable-symmetry group — `a[i][j][k]`, the
/// multiplication coefficients of the conjugacy classes (`Cᵢ·Cⱼ = Σₖ a[i][j][k]·Cₖ`), the foundation of the
/// character table. `None` when the group is too large to enumerate.
pub fn class_algebra_constants(num_vars: usize, clauses: &[Vec<Lit>]) -> Option<Vec<Vec<Vec<u128>>>> {
    let gens = crate::sym_break::variable_automorphism_generators(num_vars, clauses).unwrap_or_default();
    crate::permgroup::class_multiplication_coefficients(num_vars, &gens, 4096)
}

/// The **character table** of a formula's variable-symmetry group, computed exactly over a finite field by
/// the Burnside–Dixon algorithm (degrees + `GF(p)`-valued irreducible characters). The deepest structural
/// view of the symmetry — `None` when the group is too large for the finite-field diagonalisation.
pub fn character_table(num_vars: usize, clauses: &[Vec<Lit>]) -> Option<crate::permgroup::CharacterTable> {
    let gens = crate::sym_break::variable_automorphism_generators(num_vars, clauses).unwrap_or_default();
    crate::permgroup::character_table(num_vars, &gens, 4096)
}

/// The **Frobenius–Schur indicators** of a formula's variable-symmetry group — `+1`/`0`/`−1` per
/// irreducible (real/complex/quaternionic), the finest representation-theoretic refinement of the symmetry
/// (it separates groups even when the character table cannot). `None` when out of range.
pub fn frobenius_schur_indicators(num_vars: usize, clauses: &[Vec<Lit>]) -> Option<Vec<i8>> {
    let gens = crate::sym_break::variable_automorphism_generators(num_vars, clauses).unwrap_or_default();
    crate::permgroup::frobenius_schur_indicators(num_vars, &gens, 4096)
}

/// The **permutation character** `π(g) = #fixed variables of g` of a formula's variable-symmetry action,
/// valued per conjugacy class. The character of the representation the symmetry group carries on the
/// variables themselves. `None` when the group is too large to enumerate.
pub fn permutation_character(num_vars: usize, clauses: &[Vec<Lit>]) -> Option<Vec<u128>> {
    let gens = crate::sym_break::variable_automorphism_generators(num_vars, clauses).unwrap_or_default();
    crate::permgroup::permutation_character(num_vars, &gens, 4096)
}

/// The **isotypic decomposition** of a formula's variable-permutation representation — the multiplicity of
/// each irreducible in `π = Σ_s m_s χ_s`. The representation-theoretic spectrum of the symmetry, tying the
/// character table back to the orbit/orbital structure of the action. `None` when out of range.
pub fn isotypic_multiplicities(num_vars: usize, clauses: &[Vec<Lit>]) -> Option<Vec<u128>> {
    let gens = crate::sym_break::variable_automorphism_generators(num_vars, clauses).unwrap_or_default();
    crate::permgroup::isotypic_multiplicities(num_vars, &gens, 4096)
}

/// The **tensor (fusion) decomposition** of a formula's variable-symmetry irreducibles — `N[i][j][k]`, the
/// multiplicity of `χ_k` in `χ_i ⊗ χ_j`, the multiplication table of the representation ring `R(G)`. `None`
/// when the character table is out of range.
pub fn tensor_decomposition(num_vars: usize, clauses: &[Vec<Lit>]) -> Option<Vec<Vec<Vec<u128>>>> {
    let gens = crate::sym_break::variable_automorphism_generators(num_vars, clauses).unwrap_or_default();
    crate::permgroup::tensor_decomposition(num_vars, &gens, 4096)
}

/// The **Galois orbits on conjugacy classes** of a formula's variable symmetry — classes fused by the
/// `(ℤ/e)*` action `C ↦ C^t` (algebraic conjugacy). The singletons are the rational classes. `None` when
/// out of range.
pub fn galois_class_orbits(num_vars: usize, clauses: &[Vec<Lit>]) -> Option<Vec<Vec<usize>>> {
    let gens = crate::sym_break::variable_automorphism_generators(num_vars, clauses).unwrap_or_default();
    crate::permgroup::galois_class_orbits(num_vars, &gens, 4096)
}

/// The **table of marks** of a formula's variable-symmetry group — the Burnside-ring classification of its
/// `G`-sets (the permutation-representation analogue of [`character_table`]). Returns
/// `(subgroup_class_orders, marks)`. `None` when the subgroup lattice is out of range.
pub fn table_of_marks(num_vars: usize, clauses: &[Vec<Lit>]) -> Option<(Vec<u128>, Vec<Vec<u128>>)> {
    let gens = crate::sym_break::variable_automorphism_generators(num_vars, clauses).unwrap_or_default();
    crate::permgroup::table_of_marks(num_vars, &gens, 256)
}

/// The **Burnside ring** multiplication of a formula's variable-symmetry group — `N[a][b][l]`, the
/// decomposition of the product G-set `(G/H_a)×(G/H_b)` into transitive G-sets (the G-set analogue of the
/// character table's fusion ring). `None` when the subgroup lattice is out of range.
pub fn burnside_ring_product(num_vars: usize, clauses: &[Vec<Lit>]) -> Option<Vec<Vec<Vec<i128>>>> {
    let gens = crate::sym_break::variable_automorphism_generators(num_vars, clauses).unwrap_or_default();
    crate::permgroup::burnside_ring_product(num_vars, &gens, 256)
}

/// The **Möbius number** `μ(1, G)` of the subgroup lattice of a formula's variable-symmetry group. `None`
/// when the subgroup lattice is out of range.
pub fn mobius_number(num_vars: usize, clauses: &[Vec<Lit>]) -> Option<i128> {
    let gens = crate::sym_break::variable_automorphism_generators(num_vars, clauses).unwrap_or_default();
    crate::permgroup::mobius_number(num_vars, &gens, 256)
}

/// The number of ordered `k`-tuples of symmetries that **generate** the whole variable-symmetry group `G`
/// (Hall's Eulerian function `e_k(G)`); `e_k(G)/|G|^k` is the probability `k` random symmetries generate it.
/// `None` when out of range.
pub fn generating_tuple_count(num_vars: usize, clauses: &[Vec<Lit>], k: u32) -> Option<i128> {
    let gens = crate::sym_break::variable_automorphism_generators(num_vars, clauses).unwrap_or_default();
    crate::permgroup::generating_tuple_count(num_vars, &gens, 256, k)
}

/// The **permutation-character decomposition** of a formula's variable-symmetry group — the bridge between
/// its table of marks and its character table. `M[i][s]` is the multiplicity of irreducible `χ_s` in the
/// permutation representation on the cosets `G/H_i` (Frobenius reciprocity). Returns
/// `(subgroup_orders, irreducible_degrees, M)`. `None` when out of range.
pub fn permutation_character_decomposition(
    num_vars: usize,
    clauses: &[Vec<Lit>],
) -> Option<(Vec<u128>, Vec<u128>, Vec<Vec<u128>>)> {
    let gens = crate::sym_break::variable_automorphism_generators(num_vars, clauses).unwrap_or_default();
    crate::permgroup::permutation_character_decomposition(num_vars, &gens, 256)
}

/// The order of `Aut(G)` for a formula's variable-symmetry group `G` — the automorphism group of the
/// symmetry itself. `None` when out of range.
pub fn automorphism_group_order(num_vars: usize, clauses: &[Vec<Lit>]) -> Option<u128> {
    let gens = crate::sym_break::variable_automorphism_generators(num_vars, clauses).unwrap_or_default();
    crate::permgroup::automorphism_group_order(num_vars, &gens, 256)
}

/// The **weight inventory** of a formula's variable symmetry: entry `w` is the number of essentially-
/// distinct (modulo symmetry) `{0,1}` assignments to the variables with exactly `w` ones — the weighted
/// Pólya refinement of [`SymmetryProfile::assignment_orbits`] (which is the sum). `None` when the group is
/// too large to enumerate.
pub fn assignment_weight_inventory(num_vars: usize, clauses: &[Vec<Lit>]) -> Option<Vec<u128>> {
    let gens = crate::sym_break::variable_automorphism_generators(num_vars, clauses).unwrap_or_default();
    crate::permgroup::pattern_inventory(num_vars, &gens, 4096)
}

/// **Iterated symmetry breaking to a fixpoint** — the automated breaker. Repeatedly detect the formula's
/// variable symmetry and break each generator with one sound lex constraint — `x_v ≤ x_{g[v]}` at `g`'s
/// least moved point `v` (`v < g[v]`, and the lex-minimum of `g`'s orbit satisfies it, so it keeps ≥ 1
/// model per orbit, no auxiliary variables) — then **re-detect on the strengthened formula**, so symmetry
/// that earlier breaks expose is broken in its turn. It runs until a round adds no new constraint: the
/// symmetry "broken to conclusion." Sound: every clause breaks a genuine symmetry of the *current* formula,
/// so each round is equisatisfiability-preserving and the whole is equisatisfiable with the input. Returns
/// the original clauses plus the accumulated breaks; the residual variable-symmetry group has shrunk (to
/// trivial when single-bit breaks suffice). Solve the result with [`solve_comprehensive`].
pub fn break_all_symmetry(num_vars: usize, clauses: &[Vec<Lit>]) -> Vec<Vec<Lit>> {
    let key = |a: Lit, b: Lit| -> (u32, bool, u32, bool) {
        let (x, y) = ((a.var(), a.is_positive()), (b.var(), b.is_positive()));
        if x <= y { (x.0, x.1, y.0, y.1) } else { (y.0, y.1, x.0, x.1) }
    };
    let mut combined = clauses.to_vec();
    let mut seen: HashSet<(u32, bool, u32, bool)> =
        combined.iter().filter(|c| c.len() == 2).map(|c| key(c[0], c[1])).collect();
    loop {
        let gens =
            crate::sym_break::variable_automorphism_generators(num_vars, &combined).unwrap_or_default();
        if gens.is_empty() {
            break; // fixpoint: no symmetry remains
        }
        let bsgs = crate::permgroup::schreier_sims(num_vars, &gens);
        let mut added = false;
        // Phase 1 — COMPLETE break of fully-symmetric, independent orbits via sortedness. An orbit whose
        // every pure adjacent transposition lies in the group carries an independent Sₙ action, for which
        // `x_{o₀} ≤ x_{o₁} ≤ …` is the complete (one-per-orbit), aux-free, sound symmetry break.
        for orbit in crate::permgroup::orbits(num_vars, &gens) {
            if orbit.len() < 2 {
                continue;
            }
            let full = orbit.windows(2).all(|w| {
                let mut t: Vec<usize> = (0..num_vars).collect();
                t.swap(w[0], w[1]);
                bsgs.contains(&t)
            });
            if full {
                for w in orbit.windows(2) {
                    let (a, b) = (Lit::neg(w[0] as u32), Lit::pos(w[1] as u32)); // x_{o_i} ≤ x_{o_{i+1}}
                    if seen.insert(key(a, b)) {
                        combined.push(vec![a, b]);
                        added = true;
                    }
                }
            }
        }
        // Phase 2 — fallback single-bit break per generator, only when no full orbit was sorted this round.
        if !added {
            for g in &gens {
                let Some(v) = (0..num_vars).find(|&i| g[i] != i) else { continue };
                let (a, b) = (Lit::neg(v as u32), Lit::pos(g[v] as u32)); // x_v ≤ x_{g[v]}
                if seen.insert(key(a, b)) {
                    combined.push(vec![a, b]);
                    added = true;
                }
            }
        }
        if !added {
            break; // no new ordering ⇒ fixpoint (any residual symmetry is single-bit-irreducible)
        }
    }
    combined
}

/// **Complete symmetry breaking** — the breaker driven all the way to a single representative per orbit.
/// Detect the full variable-symmetry group and add the COMPLETE lex-leader (one `a ≤_lex a∘g` per group
/// element, via the standard lex-chain auxiliaries), which admits **exactly one model per symmetry orbit**
/// when the group is enumerable. Returns the strengthened clauses and the new variable count (originals +
/// auxiliaries). Sound (equisatisfiable with the input); complete for groups up to the enumeration cap, and
/// a sound partial break (generators ∪ coset representatives) for larger ones. Unlike [`break_all_symmetry`]
/// (aux-free, complete only for fully-symmetric orbits), this leaves *no* residual orbit symmetry.
pub fn break_all_symmetry_complete(num_vars: usize, clauses: &[Vec<Lit>]) -> (Vec<Vec<Lit>>, usize) {
    let gens = crate::sym_break::variable_automorphism_generators(num_vars, clauses).unwrap_or_default();
    if gens.is_empty() {
        return (clauses.to_vec(), num_vars);
    }
    let bsgs = crate::permgroup::schreier_sims(num_vars, &gens);
    let to_litsym = |p: &[usize]| -> Vec<Lit> { (0..num_vars).map(|v| Lit::pos(p[v] as u32)).collect() };
    let group: Vec<Vec<Lit>> = match bsgs.elements(50_000) {
        Some(elts) => elts.iter().map(|p| to_litsym(p)).collect(), // complete: one constraint per element
        None => {
            // Too large to enumerate — sound partial break over generators ∪ coset representatives.
            let mut g: Vec<Vec<Lit>> = gens.iter().map(|p| to_litsym(p)).collect();
            g.extend(bsgs.transversal_elements().iter().map(|p| to_litsym(p)));
            g
        }
    };
    let (sbp, total) = crate::sym_break::lex_leader_sbp_lit(num_vars, &group);
    let mut combined = clauses.to_vec();
    combined.extend(sbp);
    (combined, total)
}

/// **Solve by breaking all symmetry first** — the breaker as a front-end. Run the aux-free recursive
/// breaker ([`break_all_symmetry`]) to collapse the symmetry to its fixpoint, then decide the reduced
/// formula with the full arsenal ([`solve_comprehensive`]). Equisatisfiable, so the verdict is the
/// original's, and any returned model satisfies the original clauses (a subset of the broken set) —
/// re-checked fail-closed. (Uses the aux-free reducer, not [`break_all_symmetry_complete`]: CDCL handles the
/// residual cheaply, and the complete lex-leader of a large group would be far more expensive than the
/// search it saves.)
pub fn solve_by_symmetry_breaking(num_vars: usize, clauses: &[Vec<Lit>]) -> Solved {
    let broken = break_all_symmetry(num_vars, clauses);
    let inner = solve_comprehensive(num_vars, &broken);
    match inner.answer {
        Answer::Sat(model) => {
            if clauses.iter().all(|c| c.iter().any(|l| model[l.var() as usize] == l.is_positive())) {
                Solved { answer: Answer::Sat(model), via: inner.via, proof: inner.proof, conflicts: inner.conflicts }
            } else {
                solve_comprehensive(num_vars, clauses) // re-check failed ⇒ authoritative fallback
            }
        }
        Answer::Unsat => Solved { answer: Answer::Unsat, via: inner.via, proof: inner.proof, conflicts: inner.conflicts },
    }
}

/// Decide an instance by lifting it onto `GF(p)`: recover the mod-`p` one-hot system from the clauses
/// ([`crate::modp::recover_from_cnf`]) and run the certified Gaussian engine. Returns `None` when the
/// formula is not a clean mod-`p` encoding, so the dispatcher falls through to its other routes. UNSAT is
/// sound by the equisatisfiable recovery (the linear refutation re-checks); a SAT model is translated
/// back to the Boolean variables and accepted only if it satisfies every original clause (fail-closed).
fn modp_route(num_vars: usize, clauses: &[Vec<Lit>]) -> Option<Solved> {
    use crate::modp::{self, ModpOutcome};
    let rec = modp::recover_from_cnf(num_vars, clauses)?;

    // Translate a recovered ℤ/modulus assignment (value per one-hot group) back to the Boolean variables,
    // accepting it only if it satisfies every original clause (fail-closed — never trust the lift blindly).
    let build_model = |assign: &[u64]| -> Option<Vec<bool>> {
        let mut model = vec![false; num_vars];
        for (g, group) in rec.groups.iter().enumerate() {
            if let Some(&bit) = group.get(*assign.get(g).unwrap_or(&0) as usize) {
                if (bit as usize) < model.len() {
                    model[bit as usize] = true;
                }
            }
        }
        clauses
            .iter()
            .all(|c| c.iter().any(|l| model.get(l.var() as usize).copied().unwrap_or(false) == l.is_positive()))
            .then_some(model)
    };

    if modp::is_prime(rec.modulus) {
        // Prime modulus: the proven field engine.
        match modp::solve(&rec.equations, rec.num_vars, rec.modulus) {
            ModpOutcome::Unsat(combo) => {
                debug_assert!(
                    modp::is_refutation(&rec.equations, rec.num_vars, rec.modulus, &combo),
                    "the recovered GF(p) refutation must re-check"
                );
                Some(Solved::unsat(Route::ModP))
            }
            ModpOutcome::Sat(assign) => Some(Solved {
                answer: Answer::Sat(build_model(&assign)?),
                via: Route::ModP,
                proof: Vec::new(),
                conflicts: 0,
            }),
        }
    } else {
        // Composite modulus: CRT over the prime-power components, the ℤ/m ring engine.
        use crate::modm::{self, ModmOutcome};
        match modm::solve(&rec.equations, rec.num_vars, rec.modulus)? {
            ModmOutcome::Unsat { modulus, combo } => {
                debug_assert!(
                    modm::is_refutation(&rec.equations, rec.num_vars, modulus, &combo),
                    "the recovered ℤ/m refutation must re-check"
                );
                Some(Solved::unsat(Route::ModM))
            }
            ModmOutcome::Sat(assign) => Some(Solved {
                answer: Answer::Sat(build_model(&assign)?),
                via: Route::ModM,
                proof: Vec::new(),
                conflicts: 0,
            }),
        }
    }
}

/// **The clause-bundle pass.** Run every structure-mining contributor and union the *implied* clauses
/// (no-goods) they discover. Each contributor's soundness contract: every returned clause is implied
/// by the formula, so the union preserves the solution set — a sound, never-worse enrichment that lets
/// one method's discovered structure accelerate the others (and CDCL). Contributors that find nothing
/// are cheap, so this is safe to run before the fallback on any instance.
pub fn mine_clauses(num_vars: usize, clauses: &[Vec<Lit>]) -> Vec<Vec<Lit>> {
    let mut seen: HashSet<Vec<i64>> = clauses.iter().map(|c| canon(c)).collect();
    let mut pool = Vec::new();
    let mut add = |bundle: Vec<Vec<Lit>>, pool: &mut Vec<Vec<Lit>>, seen: &mut HashSet<Vec<i64>>| {
        for c in bundle {
            if seen.insert(canon(&c)) {
                pool.push(c);
            }
        }
    };
    add(xor_gaussian_bundle(num_vars, clauses), &mut pool, &mut seen);
    add(failed_literal_bundle(num_vars, clauses), &mut pool, &mut seen);
    pool
}

/// Canonical clause key (sorted signed-var ids) for deduping against the formula and each other.
fn canon(c: &[Lit]) -> Vec<i64> {
    let mut k: Vec<i64> = c
        .iter()
        .map(|l| if l.is_positive() { l.var() as i64 + 1 } else { -(l.var() as i64 + 1) })
        .collect();
    k.sort_unstable();
    k.dedup();
    k
}

/// Max width of clauses the Gaussian bundle emits (tunable for experiments via `LOGOS_XOR_WIDTH`).
fn xor_width() -> usize {
    std::env::var("LOGOS_XOR_WIDTH").ok().and_then(|s| s.parse().ok()).unwrap_or(3)
}

/// Contributor: the short clauses the GF(2) Gaussian reduction implies (units, binaries, …).
fn xor_gaussian_bundle(num_vars: usize, clauses: &[Vec<Lit>]) -> Vec<Vec<Lit>> {
    let eqs = extract_xor(num_vars, clauses);
    if eqs.len() < 2 {
        return Vec::new();
    }
    IncXor::new(num_vars, &eqs).derived_clauses(xor_width())
}

/// Contributor: failed-literal probing — `v` whose assertion unit-propagates to a root conflict gives
/// the implied unit `¬v`. Catches RUP-implied units the gadget clauses hide. Budgeted: skipped on
/// very large instances where the O(vars·clauses) probe sweep would not pay off.
fn failed_literal_bundle(num_vars: usize, clauses: &[Vec<Lit>]) -> Vec<Vec<Lit>> {
    if num_vars > 5000 || clauses.len() > 50_000 {
        return Vec::new();
    }
    let mut out = Vec::new();
    for v in 0..num_vars {
        let neg = Lit::new(v as u32, false);
        let pos = Lit::new(v as u32, true);
        if crate::rup::is_rup(num_vars, clauses, std::slice::from_ref(&neg)) {
            out.push(vec![neg]);
        } else if crate::rup::is_rup(num_vars, clauses, std::slice::from_ref(&pos)) {
            out.push(vec![pos]);
        }
    }
    out
}

/// View the CNF as 2-SAT — `Some` iff every clause has 1 or 2 literals (a unit `l` becomes `l ∨ l`).
fn as_two_sat(clauses: &[Vec<Lit>]) -> Option<Vec<(crate::twosat::Lit, crate::twosat::Lit)>> {
    let cvt = |l: &Lit| {
        if l.is_positive() {
            crate::twosat::Lit::pos(l.var() as usize)
        } else {
            crate::twosat::Lit::neg(l.var() as usize)
        }
    };
    let mut out = Vec::with_capacity(clauses.len());
    for c in clauses {
        match c.as_slice() {
            [a] => out.push((cvt(a), cvt(a))),
            [a, b] => out.push((cvt(a), cvt(b))),
            _ => return None,
        }
    }
    Some(out)
}

/// View the CNF as Horn — `Some` iff every clause has at most one positive literal.
fn as_horn(clauses: &[Vec<Lit>]) -> Option<Vec<crate::hornsat::HornClause>> {
    let mut out = Vec::with_capacity(clauses.len());
    for c in clauses {
        if c.is_empty() {
            return None;
        }
        let pos: Vec<usize> = c.iter().filter(|l| l.is_positive()).map(|l| l.var() as usize).collect();
        let neg: Vec<usize> = c.iter().filter(|l| !l.is_positive()).map(|l| l.var() as usize).collect();
        match pos.len() {
            0 => out.push(crate::hornsat::HornClause::goal(neg)),
            1 if neg.is_empty() => out.push(crate::hornsat::HornClause::fact(pos[0])),
            1 => out.push(crate::hornsat::HornClause::rule(neg, pos[0])),
            _ => return None,
        }
    }
    Some(out)
}

/// Build the conjunction-of-disjunctions [`ProofExpr`] the pigeonhole/cutting-plane/parity detectors
/// read (atoms named `v{index}`). `None` if any clause is empty (handled by the CDCL fallback).
fn cnf_to_expr(clauses: &[Vec<Lit>]) -> Option<ProofExpr> {
    let atom = |l: &Lit| {
        let a = ProofExpr::Atom(format!("v{}", l.var()));
        if l.is_positive() { a } else { ProofExpr::Not(Box::new(a)) }
    };
    let mut clause_exprs = Vec::with_capacity(clauses.len());
    for c in clauses {
        if c.is_empty() {
            return None;
        }
        let atoms: Vec<ProofExpr> = c.iter().map(&atom).collect();
        clause_exprs.push(balanced(atoms, &|a, b| ProofExpr::Or(Box::new(a), Box::new(b)))?);
    }
    // Balanced trees keep the And/Or spine depth ~log(n), so the recursive detector traversals do
    // not overflow the stack on competition-scale formulas (tens of thousands of clauses).
    balanced(clause_exprs, &|a, b| ProofExpr::And(Box::new(a), Box::new(b)))
}

/// Fold `items` into one expression with a balanced (log-depth) tree under `combine`.
fn balanced(mut items: Vec<ProofExpr>, combine: &dyn Fn(ProofExpr, ProofExpr) -> ProofExpr) -> Option<ProofExpr> {
    if items.is_empty() {
        return None;
    }
    while items.len() > 1 {
        let mut next = Vec::with_capacity(items.len().div_ceil(2));
        let mut it = items.into_iter();
        while let Some(a) = it.next() {
            match it.next() {
                Some(b) => next.push(combine(a, b)),
                None => next.push(a),
            }
        }
        items = next;
    }
    items.into_iter().next()
}

/// Hybrid XOR-SAT: mine the GF(2) linear system for structure, hand it to CDCL.
///
/// Three contributions, all sound (the recovered equations are logical consequences of the formula):
/// (1) decide the linear system — an inconsistent subsystem refutes the whole formula; (2) inject the
/// short clauses Gaussian *implies* ([`IncXor::derived_clauses`] — units, binaries, …) that resolution
/// would not find, so CDCL inherits the strategy's discovered no-goods; (3) seed CDCL's phases with a
/// GF(2) witness so search starts on the solution manifold. CDCL remains the authoritative decider, so
/// adding implied clauses + a phase hint can only help. `None` when not XOR-heavy.
///
/// (The full incremental DPLL(XOR) engine — live Gaussian during search, [`crate::xor_engine::IncXor`]
/// — is built and proven correct against a brute-force + differential oracle, and is the decisive
/// lever for *UNSAT* parity. On *satisfiable* parity-learning it needs a watch-based bit-packed matrix
/// to beat this clause-mining path; that is the remaining perf work for par32-scale.)
fn hybrid_xor(num_vars: usize, clauses: &[Vec<Lit>]) -> Option<Solved> {
    if num_vars == 0 {
        return None;
    }
    let eqs = extract_xor(num_vars, clauses);
    let mut xor_vars = HashSet::new();
    for e in &eqs {
        xor_vars.extend(e.vars.iter().copied());
    }
    // Gate: a real linear system (≥2 equations) covering at least half the variables.
    if eqs.len() < 2 || xor_vars.len() * 2 < num_vars {
        return None;
    }
    let engine = IncXor::new(num_vars, &eqs);
    if !engine.is_active() {
        return None;
    }
    // Decide the linear system once: an inconsistent subsystem refutes the whole formula.
    let witness = match crate::xorsat::solve(&eqs, num_vars) {
        XorOutcome::Unsat(_) => return Some(Solved::unsat(Route::HybridXor)),
        XorOutcome::Sat(w) => w,
    };
    // Collect the strategy's discovered structure: the short clauses Gaussian implies (units,
    // binaries, …) that resolution misses. Inject them so CDCL inherits the ruled-out no-goods, and
    // seed phases with the GF(2) witness so search starts on the solution manifold. Both only help.
    let derived = engine.derived_clauses(xor_width());
    let mut solver = Solver::new(num_vars);
    for c in clauses {
        solver.add_clause(c.clone());
    }
    for c in derived {
        solver.add_clause(c);
    }
    if xor_seed() {
        solver.set_initial_phase(&witness);
    }
    let result = if xor_live() {
        // DPLL(XOR): run the live incremental Gaussian engine as a theory. Optionally restrict CDCL's
        // decisions to the kernel — the free variables the linear system leaves undetermined — so the
        // pivots are forced by Gaussian propagation rather than branched on (the search then ranges
        // only over the true degrees of freedom).
        let live = IncXor::new(num_vars, &eqs);
        if xor_kernel() {
            let decisions = live.decision_vars();
            solver.set_decision_vars(&decisions);
        }
        let mut theories: Vec<Box<dyn crate::cdcl::Theory>> = vec![Box::new(live)];
        solver.solve_with(&mut theories)
    } else {
        solver.solve()
    };
    match result {
        SolveResult::Sat(model) => Some(Solved {
            answer: Answer::Sat(model),
            via: Route::HybridXor,
            proof: Vec::new(),
            conflicts: solver.conflicts(),
        }),
        SolveResult::Unsat => {
            let proof = solver.learned().iter().map(|lc| ProofStep::Rup(lc.lits.clone())).collect();
            Some(Solved { answer: Answer::Unsat, via: Route::HybridXor, proof, conflicts: solver.conflicts() })
        }
    }
}

/// **Fused parity + cardinality.** When a formula carries BOTH a recovered GF(2) parity substructure
/// ([`extract_xor`]) AND an at-most-one cardinality substructure ([`crate::lyapunov::recover_at_most_one`]),
/// decide it with the two live theories reasoning TOGETHER on one trail under a single
/// [`Solver::solve_with`] — Gaussian elimination for the parity, cardinality propagation for the counting,
/// the structural attack on minimal-disagreement parity that neither alone cracks. `None` unless both
/// substructures are genuinely present, so pure-parity ([`hybrid_xor`]) and pure-cardinality (the covering
/// cut) instances keep their own routes. Sound: the original clauses are solved (Boolean-complete), both
/// theories return only formula-entailed clauses, and a SAT model is re-checked (fail-closed). Uses the
/// stateless [`crate::xor_engine::XorEngine`], whose trail-sync is always correct.
fn fused_modular_solve(num_vars: usize, clauses: &[Vec<Lit>]) -> Option<Solved> {
    if num_vars == 0 {
        return None;
    }
    let eqs = extract_xor(num_vars, clauses);
    let amo = crate::lyapunov::recover_cardinality_substructure(num_vars, clauses);
    if eqs.is_empty() || amo.is_empty() {
        return None;
    }
    // Break the FULL affine × cardinality symmetry — the wreath-tower permutation break (class + family) AND
    // the affine parity shears. Sound (equisatisfiable); the aux variables extend the var count (they appear
    // in no equation, so the theories ignore them), and the answer projects back onto the original variables.
    // Break the ENTIRE symmetry group — permutations AND affine maps AND cross-compositions — COMPLETELY and
    // DYNAMICALLY via the aux-free SymmetryTheory, fused on the shared trail with parity + cardinality. No
    // static clauses, no aux variables.
    let mut solver = Solver::new(num_vars);
    for c in clauses {
        solver.add_clause(c.clone());
    }
    let mut theories: Vec<Box<dyn crate::cdcl::Theory>> = vec![
        Box::new(crate::xor_engine::XorEngine::new(num_vars, &eqs)),
        Box::new(crate::pseudo_boolean::CardinalityTheory::new(num_vars, &amo)),
        Box::new(crate::lyapunov::SymmetryTheory::new(num_vars, crate::lyapunov::fused_symmetry_group(num_vars, clauses))),
    ];
    match solver.solve_with(&mut theories) {
        SolveResult::Sat(model) => {
            clauses
                .iter()
                .all(|c| c.iter().any(|l| model[l.var() as usize] == l.is_positive()))
                .then_some(Solved {
                    answer: Answer::Sat(model),
                    via: Route::HybridXor,
                    proof: Vec::new(),
                    conflicts: solver.conflicts(),
                })
        }
        SolveResult::Unsat => {
            Some(Solved { answer: Answer::Unsat, via: Route::HybridXor, proof: Vec::new(), conflicts: solver.conflicts() })
        }
    }
}

/// Whether to run the live incremental DPLL(XOR) engine during search (vs. clause-mining + phase
/// seeding only). Gated by `LOGOS_XOR_LIVE` while we A/B it against the mining path at par-scale.
fn xor_live() -> bool {
    std::env::var("LOGOS_XOR_LIVE").map(|s| s != "0" && !s.is_empty()).unwrap_or(false)
}

/// Whether to restrict CDCL's decisions to the kernel (non-pivot) variables under the live engine.
/// Default on; `LOGOS_XOR_KERNEL=0` lets VSIDS branch on every variable (Gaussian as pure propagator).
fn xor_kernel() -> bool {
    std::env::var("LOGOS_XOR_KERNEL").map(|s| s != "0" && !s.is_empty()).unwrap_or(true)
}

/// Whether to seed CDCL's phases with the GF(2) witness. Default on; `LOGOS_XOR_SEED=0` disables it.
fn xor_seed() -> bool {
    std::env::var("LOGOS_XOR_SEED").map(|s| s != "0" && !s.is_empty()).unwrap_or(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::families::{clique_coloring, php, tseitin_expander};
    use crate::rup::check_refutation;

    #[test]
    fn pigeonhole_is_crushed_by_a_specialist_not_cdcl() {
        // PHP(20): 20 pigeons into 19 holes — exponential for resolution, but a polynomial matching
        // refutation. It must be decided by a structural route, never the CDCL fallback.
        let (cnf, _) = php(20);
        let solved = solve_structured(cnf.num_vars, &cnf.clauses);
        assert!(matches!(solved.answer, Answer::Unsat));
        assert_ne!(solved.via, Route::Cdcl, "PHP must be crushed structurally, got CDCL");
    }

    #[test]
    fn massive_pigeonhole_does_not_fall_to_search() {
        // The headline: a large PHP a CDCL solver cannot touch is decided structurally and fast
        // (the test simply completing proves it did not enter exponential search).
        let (cnf, _) = php(50);
        let solved = solve_structured(cnf.num_vars, &cnf.clauses);
        assert!(matches!(solved.answer, Answer::Unsat));
        assert_ne!(solved.via, Route::Cdcl);
    }

    #[test]
    fn clique_colouring_is_crushed_structurally() {
        let (cnf, _) = clique_coloring(8, 7);
        let solved = solve_structured(cnf.num_vars, &cnf.clauses);
        assert!(matches!(solved.answer, Answer::Unsat));
        assert_ne!(solved.via, Route::Cdcl, "clique-colouring must be crushed structurally");
    }

    #[test]
    fn sparse_sat_is_certified_by_lll_not_search() {
        // Four width-4 clauses over DISJOINT variable sets: dependency degree 0, so the Lovász Local
        // Lemma guarantees a model. The dispatcher must certify SAT via the LLL / Moser–Tardos route
        // with a genuine, re-checked model — the SAT-side specialist, not the CDCL fallback.
        let cl = |vs: [u32; 4]| vs.iter().map(|&v| Lit::pos(v)).collect::<Vec<_>>();
        let clauses = vec![cl([0, 1, 2, 3]), cl([4, 5, 6, 7]), cl([8, 9, 10, 11]), cl([12, 13, 14, 15])];
        let solved = solve_structured(16, &clauses);
        assert_eq!(solved.via, Route::Lll, "a locally-sparse SAT formula must route to LLL");
        match &solved.answer {
            Answer::Sat(model) => assert!(
                clauses.iter().all(|c| c.iter().any(|l| model[l.var() as usize] == l.is_positive())),
                "the LLL/Moser–Tardos witness must satisfy every clause"
            ),
            Answer::Unsat => panic!("a satisfiable sparse formula must not be reported UNSAT"),
        }
    }

    #[test]
    fn tseitin_parity_is_crushed_structurally() {
        let (_, cnf, _) = tseitin_expander(40, 7);
        let solved = solve_structured(cnf.num_vars, &cnf.clauses);
        assert!(matches!(solved.answer, Answer::Unsat));
        assert!(
            matches!(solved.via, Route::Parity | Route::Collapse),
            "Tseitin parity must go through the GF(2) route, got {:?}",
            solved.via
        );
    }

    #[test]
    fn two_sat_is_decided_with_a_model() {
        // (x0 ∨ x1) ∧ (¬x0 ∨ x1) ∧ (x0 ∨ ¬x1) — 2-SAT, satisfiable (x0=x1=true).
        let clauses = vec![
            vec![Lit::new(0, true), Lit::new(1, true)],
            vec![Lit::new(0, false), Lit::new(1, true)],
            vec![Lit::new(0, true), Lit::new(1, false)],
        ];
        let solved = solve_structured(2, &clauses);
        assert_eq!(solved.via, Route::TwoSat);
        match solved.answer {
            Answer::Sat(m) => {
                for c in &clauses {
                    assert!(c.iter().any(|l| m[l.var() as usize] == l.is_positive()));
                }
            }
            Answer::Unsat => panic!("instance is SAT"),
        }
    }

    #[test]
    fn horn_is_decided_with_its_least_model() {
        // facts a, b; and a ∧ b → c (a 3-literal Horn clause, so it is NOT 2-SAT and reaches the
        // Horn route). Satisfiable, least model {a,b,c}.
        let clauses = vec![
            vec![Lit::new(0, true)],
            vec![Lit::new(1, true)],
            vec![Lit::new(0, false), Lit::new(1, false), Lit::new(2, true)],
        ];
        let solved = solve_structured(3, &clauses);
        assert_eq!(solved.via, Route::Horn);
        match solved.answer {
            Answer::Sat(m) => assert!(m[0] && m[1] && m[2]),
            Answer::Unsat => panic!("instance is SAT"),
        }
    }

    #[test]
    fn unstructured_sat_returns_a_model_via_cdcl() {
        // A 3-literal clause keeps it out of the 2-SAT / Horn classes; it is satisfiable.
        let clauses = vec![
            vec![Lit::new(0, true), Lit::new(1, true), Lit::new(2, true)],
            vec![Lit::new(0, false), Lit::new(1, true), Lit::new(2, false)],
        ];
        let solved = solve_structured(3, &clauses);
        match solved.answer {
            Answer::Sat(m) => {
                for c in &clauses {
                    assert!(c.iter().any(|l| m[l.var() as usize] == l.is_positive()));
                }
            }
            Answer::Unsat => panic!("instance is SAT"),
        }
    }

    #[test]
    fn cdcl_route_carries_a_valid_rup_certificate() {
        // All 8 clauses over 3 vars → UNSAT. Whichever route fires, the CDCL fallback's proof must
        // be a valid RUP refutation; a polynomial specialist certifies internally.
        let clauses = vec![
            vec![Lit::new(0, true), Lit::new(1, true), Lit::new(2, true)],
            vec![Lit::new(0, true), Lit::new(1, true), Lit::new(2, false)],
            vec![Lit::new(0, false), Lit::new(1, false), Lit::new(2, true)],
            vec![Lit::new(0, false), Lit::new(1, false), Lit::new(2, false)],
            vec![Lit::new(0, true), Lit::new(1, false), Lit::new(2, true)],
            vec![Lit::new(0, false), Lit::new(1, true), Lit::new(2, false)],
            vec![Lit::new(0, true), Lit::new(1, false), Lit::new(2, false)],
            vec![Lit::new(0, false), Lit::new(1, true), Lit::new(2, true)],
        ];
        let solved = solve_structured(3, &clauses);
        assert!(matches!(solved.answer, Answer::Unsat));
        if solved.via == Route::Cdcl {
            let learned: Vec<Vec<Lit>> = solved.proof.iter().map(|s| s.clause().to_vec()).collect();
            assert!(check_refutation(3, &clauses, &learned));
        }
    }

    /// The 2^(k-1) gadget clauses encoding XOR(vars)=rhs — each forbids one wrong-parity row.
    fn xor_gadget(vars: &[u32], rhs: bool) -> Vec<Vec<Lit>> {
        let k = vars.len();
        let mut clauses = Vec::new();
        for mask in 0u32..(1 << k) {
            if ((mask.count_ones() % 2) == 1) != rhs {
                clauses.push((0..k).map(|i| Lit::new(vars[i], (mask >> i) & 1 == 0)).collect());
            }
        }
        clauses
    }

    #[test]
    fn hybrid_xor_solves_an_xor_heavy_sat_instance_with_a_valid_model() {
        // XOR gadgets x0⊕x1⊕x2=0, x2⊕x3=1, unit x0; plus a genuine residual clause (x1∨x3) that is
        // NOT a complete gadget — so CDCL must repair the seeded GF(2) witness to satisfy it.
        let mut clauses = xor_gadget(&[0, 1, 2], false);
        clauses.extend(xor_gadget(&[2, 3], true));
        clauses.push(vec![Lit::new(0, true)]);
        clauses.push(vec![Lit::new(1, true), Lit::new(3, true)]);
        let solved = solve_structured(4, &clauses);
        assert_eq!(solved.via, Route::HybridXor, "XOR-heavy SAT must take the hybrid route");
        match solved.answer {
            Answer::Sat(m) => {
                for c in &clauses {
                    assert!(c.iter().any(|l| m[l.var() as usize] == l.is_positive()), "model fails {c:?}");
                }
            }
            Answer::Unsat => panic!("instance is SAT"),
        }
    }

    #[test]
    fn fused_route_decides_a_mixed_parity_cardinality_instance() {
        // Exactly-one of {0,1,2} linked by equalities to {3,4,5} under even parity: exactly-one forces an
        // ODD count, the parity forces EVEN — UNSAT, a MIX neither the parity route nor the covering cut
        // refutes alone. The fused rung (recovered XOR + recovered at-most-one, reasoned together) decides
        // it; verdict must be UNSAT and the fused route must actually fire (not fall through to plain CDCL).
        let mut clauses: Vec<Vec<Lit>> = vec![
            vec![Lit::new(0, true), Lit::new(1, true), Lit::new(2, true)],
            vec![Lit::new(0, false), Lit::new(1, false)],
            vec![Lit::new(0, false), Lit::new(2, false)],
            vec![Lit::new(1, false), Lit::new(2, false)],
        ];
        for i in 0..3u32 {
            clauses.extend(xor_gadget(&[i, i + 3], false)); // x_i = x_{i+3}
        }
        clauses.extend(xor_gadget(&[3, 4, 5], false)); // x3 ⊕ x4 ⊕ x5 = 0
        let solved = solve_comprehensive(6, &clauses);
        assert!(matches!(solved.answer, Answer::Unsat), "the mixed instance is UNSAT (via {:?})", solved.via);
        assert_eq!(solved.via, Route::HybridXor, "the fused parity+cardinality route must fire");
    }

    #[test]
    fn xor_inconsistent_system_is_refuted_structurally() {
        // x0⊕x1=0 ∧ x1⊕x2=0 ∧ x0⊕x2=1 sums to 0=1 — UNSAT by Gaussian alone, never CDCL search.
        let mut clauses = xor_gadget(&[0, 1], false);
        clauses.extend(xor_gadget(&[1, 2], false));
        clauses.extend(xor_gadget(&[0, 2], true));
        let solved = solve_structured(3, &clauses);
        assert!(matches!(solved.answer, Answer::Unsat));
        assert_ne!(solved.via, Route::Cdcl, "a contradictory linear system must collapse structurally");
    }

    #[test]
    fn mod_p_tseitin_cnf_is_lifted_to_gf_p_not_left_to_cdcl() {
        // The mod-3 Tseitin obstruction encoded as opaque Boolean CNF. It is invisible to the GF(2)
        // parity cut (the whole point of the family), so the dispatcher must RECOVER the one-hot GF(p)
        // system from the raw clauses and crush it by Gaussian elimination over the right field — never
        // fall to CDCL, which (like Z3 and Kissat) needs exponential resolution here.
        for &p in &[3u64, 5] {
            let (_, cnf, _) = crate::families::mod_p_tseitin_expander(6, p, 0xC0FFEE);
            let solved = solve_structured(cnf.num_vars, &cnf.clauses);
            assert!(matches!(solved.answer, Answer::Unsat), "mod-{p} Tseitin is UNSAT");
            assert_eq!(solved.via, Route::ModP, "mod-{p} CNF must be lifted to the GF(p) route");
            assert_eq!(solved.conflicts, 0, "the GF(p) collapse spends no search");
        }
    }

    #[test]
    fn the_gf_p_route_returns_a_verified_model_on_a_satisfiable_mod_p_cnf() {
        // A *consistent* mod-p divergence system (total charge 0) is satisfiable; the recovered GF(p)
        // route must report SAT with a Boolean model that genuinely satisfies every original clause.
        let p = 3u64;
        let (_, cnf, _) = crate::families::mod_p_consistent_onehot(6, p, 0xABCD);
        let solved = solve_structured(cnf.num_vars, &cnf.clauses);
        assert_eq!(solved.via, Route::ModP, "a consistent mod-p one-hot CNF must take the GF(p) route");
        match &solved.answer {
            Answer::Sat(m) => assert!(
                cnf.clauses.iter().all(|c| c.iter().any(|l| m[l.var() as usize] == l.is_positive())),
                "the GF(p) model must satisfy every clause"
            ),
            Answer::Unsat => panic!("a consistent mod-p system must be SAT"),
        }
    }

    #[test]
    fn the_gf_p_route_never_misfires_on_unstructured_or_non_linear_cnf() {
        // Soundness of the lift: it must decline (recover → None) on inputs that are NOT a clean mod-p
        // one-hot encoding, leaving them to the honest routes. Random 3-SAT and plain PHP must not be
        // misrouted to ModP.
        let rnd = crate::families::random_3sat(30, 120, 0x5EED);
        let rnd_via = solve_structured(rnd.num_vars, &rnd.clauses).via;
        assert_ne!(rnd_via, Route::ModP);
        assert_ne!(rnd_via, Route::ModM, "random must not be misrouted to the composite lift either");
        let (php_cnf, _) = php(6);
        let php_via = solve_structured(php_cnf.num_vars, &php_cnf.clauses).via;
        assert_ne!(php_via, Route::ModP);
        assert_ne!(php_via, Route::ModM);
    }

    #[test]
    fn the_gf_p_route_agrees_with_boolean_brute_force_on_tiny_instances() {
        // The ultimate oracle: on instances small enough to enumerate every Boolean assignment, the GF(p)
        // lift's verdict matches exhaustive search — on both the UNSAT (Tseitin) and SAT (consistent)
        // forms. K₄ at p=3 is 6 edges × 3 bits = 18 variables, a 2¹⁸ sweep.
        for (cnf, want_sat) in [
            (crate::families::mod_p_tseitin_expander(4, 3, 1).1, false),
            (crate::families::mod_p_consistent_onehot(4, 3, 1).1, true),
        ] {
            let solved = solve_structured(cnf.num_vars, &cnf.clauses);
            assert_eq!(solved.via, Route::ModP, "a tiny mod-3 instance must take the GF(p) route");
            let brute = (0u64..(1u64 << cnf.num_vars)).any(|code| {
                let asg: Vec<bool> = (0..cnf.num_vars).map(|i| (code >> i) & 1 == 1).collect();
                cnf.clauses.iter().all(|c| c.iter().any(|l| asg[l.var() as usize] == l.is_positive()))
            });
            assert_eq!(matches!(solved.answer, Answer::Sat(_)), brute, "GF(p) verdict must match brute force");
            assert_eq!(brute, want_sat, "family verdict sanity");
        }
    }

    #[test]
    fn composite_modulus_onehot_cnf_is_lifted_to_zmod_m_not_left_to_cdcl() {
        // A mod-6 one-hot Tseitin system: COMPOSITE modulus. It is invisible to GF(2) and the obstruction
        // lives in the GF(3) factor. The dispatcher must recover the ℤ/6 system from the raw clauses and
        // decide it by CRT over the prime-power components — the composite lift — never fall to CDCL.
        let (_, cnf, _) = crate::families::mod_p_tseitin_expander(6, 6, 0xC0FFEE);
        let solved = solve_structured(cnf.num_vars, &cnf.clauses);
        assert!(matches!(solved.answer, Answer::Unsat), "mod-6 Tseitin is UNSAT");
        assert_eq!(solved.via, Route::ModM, "a composite mod-6 CNF must be lifted to the ℤ/m route");
        assert_eq!(solved.conflicts, 0, "the ℤ/m collapse spends no search");
    }

    #[test]
    fn the_zmod_m_route_returns_a_verified_model_on_a_satisfiable_composite_cnf() {
        let (_, cnf, _) = crate::families::mod_p_consistent_onehot(6, 6, 0xABCD);
        let solved = solve_structured(cnf.num_vars, &cnf.clauses);
        assert_eq!(solved.via, Route::ModM, "a consistent composite one-hot CNF must take the ℤ/m route");
        match &solved.answer {
            Answer::Sat(m) => assert!(
                cnf.clauses.iter().all(|c| c.iter().any(|l| m[l.var() as usize] == l.is_positive())),
                "the ℤ/m model must satisfy every clause"
            ),
            Answer::Unsat => panic!("a consistent composite system must be SAT"),
        }
    }

    #[test]
    fn the_sos_route_is_sound_in_the_dispatcher() {
        // SoS is wired as the last specialist before CDCL (`Route::Sos`). This pins its CONTRACT in the
        // dispatcher: whatever it routes is genuinely UNSAT, and a satisfiable instance is never routed
        // to it. (Empirically it rarely *fires* — small instances are decided faster by CDCL and its
        // degree-2 niche is caught by the cheaper specialists — but the wiring is always sound.)
        fn sm(s: &mut u64) -> u64 {
            *s = s.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = *s;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z ^ (z >> 31)
        }
        fn brute_sat(nv: usize, cl: &[Vec<Lit>]) -> bool {
            (0u64..(1u64 << nv))
                .any(|x| cl.iter().all(|c| c.iter().any(|l| ((x >> l.var()) & 1 != 0) == l.is_positive())))
        }
        let mut state = 0x5005_7777u64;
        for _ in 0..120 {
            let nv = 4usize;
            let m = 6 + (sm(&mut state) % 8) as usize;
            let mut cl: Vec<Vec<Lit>> = Vec::new();
            for _ in 0..m {
                let mut vs: Vec<u32> = Vec::new();
                while vs.len() < 3 {
                    let v = (sm(&mut state) % nv as u64) as u32;
                    if !vs.contains(&v) {
                        vs.push(v);
                    }
                }
                cl.push(vs.iter().map(|&v| Lit::new(v, sm(&mut state) % 2 == 0)).collect());
            }
            let solved = solve_structured(nv, &cl);
            let sat = brute_sat(nv, &cl);
            if solved.via == Route::Sos {
                assert!(matches!(solved.answer, Answer::Unsat), "the SoS route only ever refutes");
                assert!(!sat, "the SoS route must never fire on a satisfiable instance: {cl:?}");
            }
            if sat {
                assert_ne!(solved.via, Route::Sos, "a satisfiable instance must not be routed to SoS");
            }
        }
    }

    #[test]
    fn solve_comprehensive_matches_brute_force() {
        // The full-arsenal power solver must decide correctly across a fuzz (the CDCL fallback is
        // complete; the heavy Nullstellensatz / symmetry-breaking routes must never change the verdict),
        // and every reported model must satisfy the formula.
        fn sm(s: &mut u64) -> u64 {
            *s = s.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = *s;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z ^ (z >> 31)
        }
        fn brute_sat(nv: usize, cl: &[Vec<Lit>]) -> bool {
            (0u64..(1u64 << nv))
                .any(|x| cl.iter().all(|c| c.iter().any(|l| ((x >> l.var()) & 1 != 0) == l.is_positive())))
        }
        let mut state = 0xC0DE_5A1Du64;
        for _ in 0..80 {
            let nv = 3 + (sm(&mut state) % 3) as usize; // 3..5
            let m = 2 + (sm(&mut state) % 10) as usize;
            let mut cl: Vec<Vec<Lit>> = Vec::new();
            for _ in 0..m {
                let mut c = Vec::new();
                for v in 0..nv {
                    if sm(&mut state) % 2 == 0 {
                        c.push(Lit::new(v as u32, sm(&mut state) % 2 == 0));
                    }
                }
                if !c.is_empty() {
                    cl.push(c);
                }
            }
            if cl.is_empty() {
                continue;
            }
            let solved = solve_comprehensive(nv, &cl);
            assert_eq!(
                matches!(solved.answer, Answer::Sat(_)),
                brute_sat(nv, &cl),
                "solve_comprehensive verdict must match brute force via {:?}: {cl:?}",
                solved.via
            );
            if let Answer::Sat(m) = &solved.answer {
                assert!(
                    cl.iter().all(|c| c.iter().any(|l| m[l.var() as usize] == l.is_positive())),
                    "a reported model must satisfy every clause: {cl:?}"
                );
            }
        }
    }

    #[test]
    fn the_symmetry_break_route_solves_a_symmetric_instance() {
        // clique_coloring(3,3) is SAT and richly symmetric (S₃ vertices × S₃ colours); it slips past the
        // cheap specialists, so the comprehensive solver's complete-symmetry-breaking route decides it
        // with a re-checked model.
        let (cnf, _) = crate::families::clique_coloring(3, 3);
        let s = symmetry_break_solve(cnf.num_vars, &cnf.clauses)
            .expect("clique colouring has a usable, phase-free symmetry group");
        assert_eq!(s.via, Route::SymmetryBreak);
        match s.answer {
            Answer::Sat(m) => assert!(
                cnf.clauses.iter().all(|c| c.iter().any(|l| m[l.var() as usize] == l.is_positive())),
                "the symmetry-break route returns a valid model"
            ),
            Answer::Unsat => panic!("clique_coloring(3,3) is SAT"),
        }
    }

    #[test]
    fn dynamic_sel_refutes_a_symmetric_instance_in_search() {
        // Dynamic in-CDCL symmetry breaking (SEL): refute PHP by amplifying learned clauses with the
        // symmetry group during the budgeted search — the dynamic complement to the static lex-leader,
        // with a proof checked internally before it is returned. (In the full dispatcher PHP is caught
        // earlier by the pigeonhole specialist; this exercises the SEL route directly.)
        let (cnf, _) = crate::families::php(5);
        let solved =
            dynamic_sel(cnf.num_vars, &cnf.clauses).expect("PHP is symmetric — dynamic SEL engages");
        assert_eq!(solved.via, Route::Sel);
        assert!(matches!(solved.answer, Answer::Unsat), "PHP(5) is UNSAT");
        assert!(!solved.proof.is_empty(), "SEL returns a refutation proof");
    }

    #[test]
    fn orbital_branch_collapses_symmetric_branches_and_is_correct() {
        // SAT, richly symmetric: clique_coloring(3,3). The whole variable grid is one orbit under
        // S₃(vertices)×S₃(colours), so orbital branching collapses every "some-cell-true" branch into the
        // single representative branch and decides it with a re-checked model.
        let (sat_cnf, _) = crate::families::clique_coloring(3, 3);
        let s = orbital_branch_solve(sat_cnf.num_vars, &sat_cnf.clauses)
            .expect("a large variable orbit drives orbital branching");
        assert_eq!(s.via, Route::OrbitalBranch);
        match &s.answer {
            Answer::Sat(m) => assert!(
                sat_cnf.clauses.iter().all(|c| c.iter().any(|l| m[l.var() as usize] == l.is_positive())),
                "orbital branching returns a valid model"
            ),
            Answer::Unsat => panic!("clique_coloring(3,3) is SAT"),
        }

        // UNSAT, richly symmetric: clique_coloring(4,3) — K₄ cannot be 3-coloured. Orbital concludes UNSAT
        // only because BOTH branches (rep-true and all-orbit-false) are genuinely UNSAT.
        let (unsat_cnf, _) = crate::families::clique_coloring(4, 3);
        let u = orbital_branch_solve(unsat_cnf.num_vars, &unsat_cnf.clauses)
            .expect("a large variable orbit drives orbital branching");
        assert_eq!(u.via, Route::OrbitalBranch);
        assert!(matches!(u.answer, Answer::Unsat), "clique_coloring(4,3) is UNSAT");

        // The RECURSION: clique_coloring(4,4) (16 vars) is SAT and far too symmetric to dispatch in one
        // split — the representative branch is itself symmetric, so orbital branching descends the
        // residual group level by level and still returns a re-checked model.
        let (big_cnf, _) = crate::families::clique_coloring(4, 4);
        let big = orbital_branch_solve(big_cnf.num_vars, &big_cnf.clauses)
            .expect("recursive orbital branching engages on the larger grid");
        assert_eq!(big.via, Route::OrbitalBranch);
        match &big.answer {
            Answer::Sat(m) => assert!(
                big_cnf.clauses.iter().all(|c| c.iter().any(|l| m[l.var() as usize] == l.is_positive())),
                "the recursive orbital model is valid"
            ),
            Answer::Unsat => panic!("clique_coloring(4,4) is SAT"),
        }

        // Soundness: the orbital verdict matches an independent brute-force decision across all instances.
        for (cnf, _) in [
            crate::families::clique_coloring(3, 3),
            crate::families::clique_coloring(4, 3),
            crate::families::clique_coloring(4, 4),
        ] {
            let nv = cnf.num_vars;
            let brute = (0u64..(1u64 << nv)).any(|x| {
                let a: Vec<bool> = (0..nv).map(|i| (x >> i) & 1 == 1).collect();
                cnf.clauses.iter().all(|c| c.iter().any(|l| a[l.var() as usize] == l.is_positive()))
            });
            let got = orbital_branch_solve(nv, &cnf.clauses).expect("orbital fires");
            assert_eq!(
                matches!(got.answer, Answer::Sat(_)),
                brute,
                "orbital-branch verdict matches brute force (nv={nv})"
            );
        }
    }

    #[test]
    fn solve_by_symmetry_breaking_matches_brute_force() {
        let instances = [
            crate::families::clique_coloring(3, 3), // SAT
            crate::families::clique_coloring(4, 3), // UNSAT (K₄ not 3-colourable)
            crate::families::clique_coloring(4, 4), // SAT
        ];
        for (cnf, _) in instances {
            let nv = cnf.num_vars;
            let brute = (0u64..(1u64 << nv)).any(|x| {
                let a: Vec<bool> = (0..nv).map(|i| (x >> i) & 1 == 1).collect();
                cnf.clauses.iter().all(|c| c.iter().any(|l| a[l.var() as usize] == l.is_positive()))
            });
            let solved = solve_by_symmetry_breaking(nv, &cnf.clauses);
            assert_eq!(matches!(solved.answer, Answer::Sat(_)), brute, "break-then-solve matches brute (nv={nv})");
            if let Answer::Sat(m) = &solved.answer {
                assert!(
                    cnf.clauses.iter().all(|c| c.iter().any(|l| m[l.var() as usize] == l.is_positive())),
                    "the projected model satisfies the original formula"
                );
            }
        }
    }

    #[test]
    fn break_all_symmetry_complete_leaves_one_model_per_orbit() {
        // Count the distinct ORIGINAL-variable models of a (possibly aux-laden) formula by blocking each
        // original projection in turn.
        let count_original = |total: usize, cl: &[Vec<Lit>], nv: usize| -> usize {
            let mut working = cl.to_vec();
            let mut count = 0;
            loop {
                match solve_comprehensive(total, &working).answer {
                    Answer::Unsat => break,
                    Answer::Sat(m) => {
                        count += 1;
                        working.push((0..nv).map(|v| Lit::new(v as u32, !m[v])).collect());
                    }
                }
            }
            count
        };
        let n = |v| Lit::new(v, false);

        // at-most-1-of-3 (S₃): models {000,100,010,001} → 2 orbits. Complete breaking ⇒ exactly 2 survive.
        let amo = vec![vec![n(0), n(1)], vec![n(0), n(2)], vec![n(1), n(2)]];
        let orbits_amo = models_up_to_symmetry(3, &amo, 1000).representatives.len();
        let (b1, t1) = break_all_symmetry_complete(3, &amo);
        assert_eq!(count_original(t1, &b1, 3), orbits_amo, "one model per orbit (at-most-1-of-3)");

        // clique_coloring(3,3) (S₃×S₃): the 6 proper colourings form 1 orbit ⇒ exactly 1 model survives.
        let (cnf, _) = crate::families::clique_coloring(3, 3);
        let orbits_clq = models_up_to_symmetry(cnf.num_vars, &cnf.clauses, 1000).representatives.len();
        let (b2, t2) = break_all_symmetry_complete(cnf.num_vars, &cnf.clauses);
        assert_eq!(orbits_clq, 1, "clique(3,3)'s colourings form a single orbit");
        assert_eq!(count_original(t2, &b2, cnf.num_vars), 1, "complete breaking leaves exactly one model (clique)");

        // No symmetry ⇒ returned unchanged.
        let asym = vec![vec![Lit::new(0, true)], vec![n(1), Lit::new(2, true)]];
        let (b3, t3) = break_all_symmetry_complete(3, &asym);
        assert_eq!((b3.len(), t3), (asym.len(), 3), "no symmetry ⇒ no breaks");
    }

    #[test]
    fn break_all_symmetry_runs_to_a_fixpoint_soundly() {
        // clique_coloring(3,3): rich S₃×S₃ symmetry. The automated breaker drives it down, stays
        // equisatisfiable, and reaches a fixpoint.
        let (cnf, _) = crate::families::clique_coloring(3, 3);
        let before = symmetry_structure(cnf.num_vars, &cnf.clauses).order;
        let broken = break_all_symmetry(cnf.num_vars, &cnf.clauses);
        let after = symmetry_structure(cnf.num_vars, &broken).order;
        assert!(after < before, "the breaker reduced the symmetry group: {before} → {after}");

        // Equisatisfiable: same verdict as the original, and the broken model satisfies the ORIGINAL.
        let orig = solve_comprehensive(cnf.num_vars, &cnf.clauses).answer;
        let brk = solve_comprehensive(cnf.num_vars, &broken).answer;
        assert_eq!(matches!(orig, Answer::Sat(_)), matches!(brk, Answer::Sat(_)), "breaking preserves the verdict");
        if let Answer::Sat(m) = &brk {
            assert!(
                cnf.clauses.iter().all(|c| c.iter().any(|l| m[l.var() as usize] == l.is_positive())),
                "the broken-formula model is a genuine model of the original"
            );
        }

        // Fixpoint: breaking the already-broken formula adds nothing.
        let twice = break_all_symmetry(cnf.num_vars, &broken);
        assert_eq!(twice.len(), broken.len(), "already at the fixpoint — re-breaking is a no-op");

        // An asymmetric formula is returned unchanged (nothing to break).
        let asym = vec![vec![Lit::new(0, true)], vec![Lit::new(1, false), Lit::new(2, true)]];
        assert_eq!(break_all_symmetry(3, &asym).len(), asym.len(), "no symmetry ⇒ no breaks");
    }

    #[test]
    fn class_algebra_constants_wrapper_has_the_right_shape() {
        let (cnf, _) = crate::families::clique_coloring(3, 3);
        let k = symmetry_structure(cnf.num_vars, &cnf.clauses).conjugacy_classes.unwrap(); // 9
        let a = class_algebra_constants(cnf.num_vars, &cnf.clauses).expect("enumerable group");
        assert_eq!(a.len(), k, "the class-algebra tensor is k × k × k");
        assert!(a.iter().all(|m| m.len() == k && m.iter().all(|r| r.len() == k)));
    }

    #[test]
    fn character_table_wrapper_is_the_grid_groups_table() {
        // The variable symmetry of clique_coloring(3,3) is S₃×S₃; its character table has one row per
        // conjugacy class with the product degrees, satisfies Σ dᵢ² = |G|, and re-checks orthogonality.
        let (cnf, _) = crate::families::clique_coloring(3, 3);
        let t = character_table(cnf.num_vars, &cnf.clauses).expect("enumerable group");
        assert_eq!(t.degrees, vec![1, 1, 1, 1, 2, 2, 2, 2, 4], "S₃×S₃ irreducible degrees");
        assert_eq!(t.degrees.iter().map(|d| d * d).sum::<u128>(), 36, "Σ dᵢ² = |S₃×S₃|");
        assert_eq!(t.values.len(), t.degrees.len(), "one character per irreducible");
        assert!(t.values.iter().all(|row| row.len() == t.class_sizes.len()), "a value per conjugacy class");
        assert!(t.values.iter().any(|row| row.iter().all(|&x| x == 1)), "the trivial character is present");
    }

    #[test]
    fn frobenius_schur_wrapper_reports_a_real_grid_group() {
        // S₃×S₃ is totally real, so every indicator is +1, one per irreducible.
        let (cnf, _) = crate::families::clique_coloring(3, 3);
        let fs = frobenius_schur_indicators(cnf.num_vars, &cnf.clauses).expect("enumerable group");
        assert_eq!(fs, vec![1; 9], "S₃×S₃: nine real irreducibles");
    }

    #[test]
    fn isotypic_decomposition_wrapper_bridges_action_and_representation() {
        // clique_coloring(3,3): S₃×S₃ on 9 cells. The permutation character decomposes with the trivial
        // multiplicity equal to the orbit count (1, transitive) and Σ m·d = 9.
        let (cnf, _) = crate::families::clique_coloring(3, 3);
        let iso = isotypic_multiplicities(cnf.num_vars, &cnf.clauses).expect("enumerable group");
        let degs = symmetry_structure(cnf.num_vars, &cnf.clauses).irreducible_degrees.unwrap();
        assert_eq!(iso.iter().zip(&degs).map(|(m, d)| m * d).sum::<u128>(), 9, "Σ m·d = 9 cells");
        // The permutation character: the identity fixes all 9 cells.
        let pi = permutation_character(cnf.num_vars, &cnf.clauses).expect("enumerable group");
        assert!(pi.contains(&9), "the identity class fixes all 9 variables");
    }

    #[test]
    fn automorphism_group_wrapper_measures_the_symmetry_of_the_symmetry() {
        // clique_coloring(3,3): the variable symmetry is S₃×S₃, whose automorphism group has order 72.
        let (cnf, _) = crate::families::clique_coloring(3, 3);
        assert_eq!(
            automorphism_group_order(cnf.num_vars, &cnf.clauses),
            Some(72),
            "|Aut(S₃×S₃)| = 72"
        );
    }

    #[test]
    fn table_of_marks_wrapper_classifies_a_grid_groups_g_sets() {
        // clique_coloring(3,3): the variable symmetry is S₃×S₃; its table of marks is triangular with the
        // trivial-subgroup row giving the coset indices [G:H] and the full-group column all ones.
        let (cnf, _) = crate::families::clique_coloring(3, 3);
        let (orders, marks) = table_of_marks(cnf.num_vars, &cnf.clauses).expect("subgroup lattice in range");
        let k = orders.len();
        assert_eq!(*orders.last().unwrap(), 36, "the whole group S₃×S₃ has order 36");
        assert_eq!(marks[0][0], 36, "m(1, 1) = [G:1] = |G| = 36 (the regular action)");
        for j in 0..k {
            assert_eq!(marks[0][j], 36 / orders[j], "m(1, H_j) = [G : H_j]");
            assert_eq!(marks[j][k - 1], 1, "every subgroup fixes the one coset of G");
        }
    }

    #[test]
    fn galois_class_orbits_wrapper_partitions_a_rational_grid_group() {
        // S₃×S₃ is rational, so every Galois orbit on its 9 classes is a singleton.
        let (cnf, _) = crate::families::clique_coloring(3, 3);
        let orbits = galois_class_orbits(cnf.num_vars, &cnf.clauses).expect("enumerable group");
        assert_eq!(orbits.len(), 9, "S₃×S₃ has 9 conjugacy classes");
        assert!(orbits.iter().all(|o| o.len() == 1), "all rational ⇒ every orbit is a singleton");
        let prof = symmetry_structure(cnf.num_vars, &cnf.clauses);
        assert_eq!(prof.rational_classes, Some(9));
    }

    #[test]
    fn tensor_decomposition_wrapper_is_a_valid_fusion_ring() {
        // clique_coloring(3,3): S₃×S₃, 9 irreducibles. The fusion coefficients form a valid representation
        // ring — every product has the right dimension and the trivial character is the unit.
        let (cnf, _) = crate::families::clique_coloring(3, 3);
        let degs = symmetry_structure(cnf.num_vars, &cnf.clauses).irreducible_degrees.unwrap();
        let n = tensor_decomposition(cnf.num_vars, &cnf.clauses).expect("enumerable group");
        let k = degs.len();
        assert_eq!(n.len(), k, "a k×k×k fusion tensor");
        for i in 0..k {
            for j in 0..k {
                assert_eq!(
                    (0..k).map(|c| n[i][j][c] * degs[c]).sum::<u128>(),
                    degs[i] * degs[j],
                    "dim(χ_i ⊗ χ_j) = d_i·d_j"
                );
            }
        }
    }

    #[test]
    fn equivalence_symmetry_matches_brute_force_and_reduces() {
        let p = |v: u32| Lit::new(v, true);
        let nl = |v: u32| Lit::new(v, false);
        let sat = |m: &[bool], cls: &[Vec<Lit>]| {
            cls.iter().all(|c| c.iter().any(|l| m[l.var() as usize] == l.is_positive()))
        };
        let brute_equiv = |nv: usize, f: &[Vec<Lit>], s: &[Vec<Lit>]| -> bool {
            (0u32..(1u32 << nv)).all(|x| {
                let a: Vec<bool> = (0..nv).map(|i| (x >> i) & 1 == 1).collect();
                sat(&a, f) == sat(&a, s)
            })
        };

        // 1. THE REDUCTION FIRES: at-most-one-of-4 is fully S₄-symmetric, so its 6 pair clauses fuse into
        //    ONE orbit — equivalence to itself is decided with far fewer entailment checks than the naive 12.
        let amo: Vec<Vec<Lit>> =
            (0..4u32).flat_map(|i| ((i + 1)..4).map(move |j| vec![nl(i), nl(j)])).collect();
        assert_eq!(amo.len(), 6);
        let gens = common_automorphism_generators(4, &amo, &amo);
        assert!(!gens.is_empty(), "at-most-one-of-4 has a nontrivial common symmetry group");
        let (reps, naive) = equivalence_check_counts(4, &amo, &amo);
        assert!(reps < naive, "the common symmetry must reduce the work: {reps} < {naive}");
        assert_eq!(equivalent_modulo_symmetry(4, &amo, &amo), EquivVerdict::Equivalent);

        // 2. EQUIVALENT but syntactically different: add a clause F already entails (resolvent x1). Still ≡.
        let f2 = vec![vec![p(0), p(1)], vec![nl(0), p(1)]];
        let mut s2 = f2.clone();
        s2.push(vec![p(1)]);
        assert!(brute_equiv(2, &f2, &s2), "sanity: the resolvent makes them equivalent");
        assert_eq!(equivalent_modulo_symmetry(2, &f2, &s2), EquivVerdict::Equivalent);

        // 3. INEQUIVALENT: a dropped constraint changes the function; the witness must truly distinguish.
        let f3 = vec![vec![p(0), p(1)], vec![nl(0), nl(1)]];
        let s3 = vec![vec![p(0), p(1)]];
        match equivalent_modulo_symmetry(2, &f3, &s3) {
            EquivVerdict::Differ(m) => assert_ne!(sat(&m, &f3), sat(&m, &s3), "witness must distinguish"),
            EquivVerdict::Equivalent => panic!("f3 and s3 are NOT equivalent"),
        }

        // 4. ASYMMETRIC pair: no usable common symmetry ⇒ no orbit fusion (and the verdict is still right).
        let fa = vec![vec![p(0), p(1)]];
        let sa = vec![vec![p(0), p(1)], vec![p(1), p(2)]];
        let (r, t) = equivalence_check_counts(3, &fa, &sa);
        assert_eq!(r, t, "no common symmetry ⇒ one check per clause");

        // 5. FUZZ: the symmetry-reduced verdict ALWAYS equals brute force, and every witness distinguishes.
        fn xs(s: &mut u64) -> u64 {
            *s ^= *s << 13;
            *s ^= *s >> 7;
            *s ^= *s << 17;
            *s
        }
        let nv = 4usize;
        let mk = |s: &mut u64| -> Vec<Vec<Lit>> {
            let m = (xs(s) % 5) as usize; // 0..=4 clauses
            (0..m)
                .map(|_| {
                    let w = 1 + (xs(s) % 2) as usize; // width 1 or 2
                    (0..w).map(|_| Lit::new((xs(s) % nv as u64) as u32, xs(s) & 1 == 0)).collect()
                })
                .collect()
        };
        let mut seed = 0x1234_5678_9abc_def0u64;
        for _ in 0..400 {
            let f = mk(&mut seed);
            let s = mk(&mut seed);
            let want = brute_equiv(nv, &f, &s);
            match equivalent_modulo_symmetry(nv, &f, &s) {
                EquivVerdict::Equivalent => {
                    assert!(want, "claimed equivalent but brute says differ: F={f:?} S={s:?}")
                }
                EquivVerdict::Differ(m) => {
                    assert!(!want, "claimed differ but brute says equivalent: F={f:?} S={s:?}");
                    assert_ne!(sat(&m, &f), sat(&m, &s), "witness must distinguish F and S");
                }
            }
        }
    }

    #[test]
    fn optimization_symmetry_matches_brute_force_and_reduces() {
        let p = |v: u32| Lit::new(v, true);
        let sat = |m: &[bool], cls: &[Vec<Lit>]| {
            cls.iter().all(|c| c.iter().any(|l| m[l.var() as usize] == l.is_positive()))
        };
        let brute_min = |nv: usize, f: &[Vec<Lit>], w: &[i64]| -> Option<i64> {
            (0u32..(1u32 << nv))
                .filter_map(|x| {
                    let a: Vec<bool> = (0..nv).map(|i| (x >> i) & 1 == 1).collect();
                    sat(&a, f).then(|| (0..nv).filter(|&i| a[i]).map(|i| w[i]).sum::<i64>())
                })
                .min()
        };

        // 1. THE REDUCTION FIRES: "at least one of 4", minimise the count (all weights 1). Fully S₄-symmetric
        //    (clauses AND weights), so the optimal "exactly one true" models form one orbit — far fewer
        //    candidate models to enumerate than the naive 15.
        let amo1 = vec![vec![p(0), p(1), p(2), p(3)]];
        let w1 = vec![1i64; 4];
        let (opt, m) = optimize_modulo_symmetry(4, &amo1, &w1).expect("F is satisfiable");
        assert_eq!(opt, 1, "minimum is one variable true");
        assert!(sat(&m, &amo1), "witness satisfies F");
        assert_eq!(m.iter().filter(|&&b| b).count(), 1, "witness has weight 1");
        let (with, without) = optimize_enumeration_counts(4, &amo1, &w1);
        assert!(with < without, "symmetry must shrink the enumeration: {with} < {without}");

        // 2. UNSAT hard clauses ⇒ no optimum.
        assert_eq!(optimize_modulo_symmetry(2, &[vec![p(0)], vec![Lit::new(0, false)]], &[1, 1]), None);

        // 3. ASYMMETRIC weights kill the symmetry ⇒ no reduction (distinct weights ⇒ no objective-preserving
        //    permutation), and the optimum is still correct.
        let w_distinct = vec![1i64, 2, 3, 4];
        assert!(optimization_symmetry_generators(4, &amo1, &w_distinct).is_empty(), "distinct weights ⇒ no sym");
        let (wa, wo) = optimize_enumeration_counts(4, &amo1, &w_distinct);
        assert_eq!(wa, wo, "no usable symmetry ⇒ no reduction");

        // 4. FUZZ: the symmetry-reduced optimum ALWAYS equals brute force, and the witness is a real model of
        //    F achieving exactly that weight.
        fn xs(s: &mut u64) -> u64 {
            *s ^= *s << 13;
            *s ^= *s >> 7;
            *s ^= *s << 17;
            *s
        }
        let nv = 4usize;
        let mut seed = 0xC0FFEE_1234_5678u64;
        for _ in 0..400 {
            let m = (xs(&mut seed) % 5) as usize;
            let f: Vec<Vec<Lit>> = (0..m)
                .map(|_| {
                    let wclause = 1 + (xs(&mut seed) % 2) as usize;
                    (0..wclause).map(|_| Lit::new((xs(&mut seed) % nv as u64) as u32, xs(&mut seed) & 1 == 0)).collect()
                })
                .collect();
            // Weights in -2..=2 (symmetric instances arise when repeats let an automorphism preserve them).
            let weights: Vec<i64> = (0..nv).map(|_| (xs(&mut seed) % 5) as i64 - 2).collect();
            let want = brute_min(nv, &f, &weights);
            match optimize_modulo_symmetry(nv, &f, &weights) {
                None => assert!(want.is_none(), "claimed UNSAT but a model exists: F={f:?}"),
                Some((opt, wit)) => {
                    assert_eq!(Some(opt), want, "optimum must match brute force: F={f:?} w={weights:?}");
                    assert!(sat(&wit, &f), "witness must satisfy F");
                    let ww: i64 = (0..nv).filter(|&i| wit[i]).map(|i| weights[i]).sum();
                    assert_eq!(ww, opt, "witness must achieve the optimum");
                }
            }
        }
    }

    #[test]
    fn fractional_automorphism_is_a_doubly_stochastic_commuting_relaxation() {
        let edge = |a: u32, b: u32| vec![Lit::new(a, true), Lit::new(b, true)];

        // The canonical (block-averaging over the coarsest equitable partition) matrix is ALWAYS a valid
        // fractional automorphism — a doubly-stochastic matrix commuting with the co-occurrence graph.
        let (php, _) = crate::families::php(4);
        let (clq, _) = crate::families::clique_coloring(3, 3);
        let c6: Vec<Vec<Lit>> = (0..6).map(|i| edge(i, (i + 1) % 6)).collect();
        for (nv, cl) in [(php.num_vars, php.clauses.clone()), (clq.num_vars, clq.clauses.clone()), (6, c6.clone())] {
            let part = fractional_automorphism(nv, &cl);
            assert!(is_fractional_automorphism(nv, &cl, &part), "the equitable partition commutes with A");
            // Non-trivial: symmetric formulas admit a fractional automorphism that is NOT a permutation.
            let cells = part.iter().copied().max().map_or(0, |m| m + 1);
            assert!(cells < nv, "a non-discrete partition ⇒ a genuine (non-permutation) fractional automorphism");
            // Fractional symmetry contains integer symmetry: every automorphism orbit lies within one cell.
            let gens = crate::sym_break::variable_automorphism_generators(nv, &cl).unwrap_or_default();
            for orbit in crate::permgroup::orbits(nv, &gens) {
                assert!(orbit.iter().all(|&v| part[v] == part[orbit[0]]), "orbit ⊆ fractional-automorphism cell");
            }
        }

        // The identity partition (each variable its own cell) is the trivial fractional automorphism (B = I).
        let (nv, _) = (clq.num_vars, ());
        assert!(is_fractional_automorphism(nv, &clq.clauses, &(0..nv).collect::<Vec<_>>()), "identity is a fractional automorphism");

        // REJECTION: on the path 0–1–2–3, {0,1} | {2,3} is NOT equitable (vertex 0 and vertex 1 see different
        // cell-distributions), so it is not a fractional automorphism — but the reflection {0,3} | {1,2} is.
        let p4 = vec![edge(0, 1), edge(1, 2), edge(2, 3)];
        assert!(!is_fractional_automorphism(4, &p4, &[0, 0, 1, 1]), "a non-equitable partition is rejected");
        assert!(is_fractional_automorphism(4, &p4, &[0, 1, 1, 0]), "the reflection partition is equitable");
        assert_eq!(fractional_automorphism(4, &p4), vec![0, 1, 1, 0], "P₄'s coarsest equitable partition is the reflection");

        // FUZZ: the canonical partition is always a valid fractional automorphism.
        fn xs(s: &mut u64) -> u64 {
            *s ^= *s << 13;
            *s ^= *s >> 7;
            *s ^= *s << 17;
            *s
        }
        let n = 5usize;
        let mut seed = 0x0FAC_0000_1234_9999u64;
        for _ in 0..200 {
            let m = (xs(&mut seed) % 6) as usize;
            let cl: Vec<Vec<Lit>> = (0..m)
                .map(|_| {
                    let w = 1 + (xs(&mut seed) % 3) as usize;
                    (0..w).map(|_| Lit::new((xs(&mut seed) % n as u64) as u32, xs(&mut seed) & 1 == 0)).collect()
                })
                .collect();
            let part = fractional_automorphism(n, &cl);
            assert!(is_fractional_automorphism(n, &cl, &part), "canonical partition must always commute: {cl:?}");
        }
    }

    #[test]
    fn color_refinement_over_approximates_the_orbit_partition() {
        let p = |v: u32| Lit::new(v, true);

        // 1. Vertex-transitive symmetric families collapse to ONE cell (one orbit ⇒ one colour).
        let (php, _) = crate::families::php(4);
        assert_eq!(color_refinement_cells(php.num_vars, &php.clauses), 1, "PHP(4) variables are all alike");
        let (clq, _) = crate::families::clique_coloring(3, 3);
        assert_eq!(color_refinement_cells(clq.num_vars, &clq.clauses), 1, "clique(3,3) variables are all alike");

        // 2. A structurally-distinguished variable gets its own cell — a polynomial asymmetry certificate.
        let f = vec![vec![p(0)], vec![p(1), p(2)]]; // x0 in a unit, x1/x2 symmetric in a binary
        let cells = color_refinement(3, &f);
        assert_ne!(cells[0], cells[1], "the unit variable is distinguished from the binary pair");
        assert_eq!(cells[1], cells[2], "x1 and x2 are interchangeable");
        assert_eq!(provably_asymmetric_variables(3, &f), vec![0], "x0 is provably fixed by every automorphism");
        // …and the detected symmetry indeed fixes the singleton-cell variable.
        for g in crate::sym_break::variable_automorphism_generators(3, &f).unwrap_or_default() {
            assert_eq!(g[0], 0, "an automorphism must fix the provably-asymmetric variable");
        }

        // 3. THE THEOREM, on every instance incl. a fuzz: orbit(v) ⊆ cell(v) (each orbit is monochromatic),
        //    and the equitable partition is never finer than the orbit partition (cells ≤ orbits).
        let check = |nv: usize, cl: &[Vec<Lit>]| {
            let gens = crate::sym_break::variable_automorphism_generators(nv, cl).unwrap_or_default();
            let orbits = crate::permgroup::orbits(nv, &gens);
            let cells = color_refinement(nv, cl);
            for orbit in &orbits {
                let c0 = cells[orbit[0]];
                assert!(orbit.iter().all(|&v| cells[v] == c0), "orbit {orbit:?} must be monochromatic: {cells:?}");
            }
            assert!(
                color_refinement_cells(nv, cl) <= orbits.len(),
                "the equitable partition is coarser than the orbit partition"
            );
        };
        check(php.num_vars, &php.clauses);
        check(clq.num_vars, &clq.clauses);
        check(3, &f);

        fn xs(s: &mut u64) -> u64 {
            *s ^= *s << 13;
            *s ^= *s >> 7;
            *s ^= *s << 17;
            *s
        }
        let nv = 5usize;
        let mut seed = 0xA5A5_1234_DEAD_BEEFu64;
        for _ in 0..200 {
            let m = (xs(&mut seed) % 6) as usize;
            let cl: Vec<Vec<Lit>> = (0..m)
                .map(|_| {
                    let w = 1 + (xs(&mut seed) % 3) as usize;
                    (0..w).map(|_| Lit::new((xs(&mut seed) % nv as u64) as u32, xs(&mut seed) & 1 == 0)).collect()
                })
                .collect();
            check(nv, &cl);
        }
    }

    #[test]
    fn two_wl_over_approximates_orbitals_and_beats_one_wl() {
        let edge = |a: u32, b: u32| vec![Lit::new(a, true), Lit::new(b, true)]; // an edge as a 2-clause

        // THE HEADLINE: a 6-cycle vs two triangles. Both are 2-regular, so 1-WL sees a single cell for each
        // and CANNOT tell them apart — but 2-WL distinguishes them by their pair structure.
        let c6 = vec![edge(0, 1), edge(1, 2), edge(2, 3), edge(3, 4), edge(4, 5), edge(5, 0)];
        let two_tri = vec![edge(0, 1), edge(1, 2), edge(0, 2), edge(3, 4), edge(4, 5), edge(3, 5)];
        assert_eq!(color_refinement_cells(6, &c6), 1, "1-WL: C₆ is one cell");
        assert_eq!(color_refinement_cells(6, &two_tri), 1, "1-WL: 2·C₃ is one cell — same as C₆");
        assert_ne!(
            two_wl_fingerprint(6, &c6),
            two_wl_fingerprint(6, &two_tri),
            "2-WL SEPARATES C₆ from 2·C₃ where 1-WL cannot"
        );

        // 2-WL refines 1-WL: the diagonal pair-coloring is at least as fine as the 1-WL vertex coloring.
        let diag_refines_1wl = |nv: usize, cl: &[Vec<Lit>]| {
            let wl1 = color_refinement(nv, cl);
            let pc = two_wl_pair_colors(nv, cl);
            for a in 0..nv {
                for b in 0..nv {
                    if pc[a][a] == pc[b][b] {
                        assert_eq!(wl1[a], wl1[b], "2-WL diagonal must refine 1-WL");
                    }
                }
            }
        };
        diag_refines_1wl(6, &c6);
        diag_refines_1wl(6, &two_tri);

        // THE THEOREM (the 2-orbit analogue of #42): orbital(i,j) ⊆ paircell(i,j) — every orbital is
        // monochromatic under 2-WL — and pair-cells are never finer than orbitals. Checked incl. a fuzz.
        let check = |nv: usize, cl: &[Vec<Lit>]| {
            let gens = crate::sym_break::variable_automorphism_generators(nv, cl).unwrap_or_default();
            let orbitals = crate::permgroup::orbitals(nv, &gens);
            let pc = two_wl_pair_colors(nv, cl);
            for orbital in &orbitals {
                let (i0, j0) = orbital[0];
                let c0 = pc[i0][j0];
                assert!(orbital.iter().all(|&(i, j)| pc[i][j] == c0), "orbital must be monochromatic");
            }
            assert!(two_wl_pair_cells(nv, cl) <= orbitals.len(), "pair-cells ≤ orbitals");
        };
        let (php, _) = crate::families::php(4);
        let (clq, _) = crate::families::clique_coloring(3, 3);
        check(php.num_vars, &php.clauses);
        check(clq.num_vars, &clq.clauses);
        check(6, &c6);
        check(6, &two_tri);
        // clique(3,3): 1-WL is one cell, but 2-WL recovers the rank-4 orbital structure 1-WL is blind to.
        assert_eq!(color_refinement_cells(clq.num_vars, &clq.clauses), 1);
        assert!(two_wl_pair_cells(clq.num_vars, &clq.clauses) >= 4, "2-WL sees the 4 orbitals of the grid");

        fn xs(s: &mut u64) -> u64 {
            *s ^= *s << 13;
            *s ^= *s >> 7;
            *s ^= *s << 17;
            *s
        }
        let nv = 5usize;
        let mut seed = 0x2B0C_1A7E_55AA_F00Du64;
        for _ in 0..120 {
            let m = (xs(&mut seed) % 6) as usize;
            let cl: Vec<Vec<Lit>> = (0..m)
                .map(|_| {
                    let w = 1 + (xs(&mut seed) % 3) as usize;
                    (0..w).map(|_| Lit::new((xs(&mut seed) % nv as u64) as u32, xs(&mut seed) & 1 == 0)).collect()
                })
                .collect();
            check(nv, &cl);
        }
    }

    #[test]
    fn association_scheme_multiplicities_are_the_eigenspace_dimensions() {
        let edge = |a: u32, b: u32| vec![Lit::new(a, true), Lit::new(b, true)];
        let (clq, _) = crate::families::clique_coloring(3, 3);
        let c6: Vec<Vec<Lit>> = (0..6).map(|i| edge(i, (i + 1) % 6)).collect();

        // clique(3,3) is the multiplicity-free S₃×S₃ action on 9 cells: its scheme multiplicities are the
        // degrees of the four constituent irreducibles — triv⊗triv=1, triv⊗std=2, std⊗triv=2, std⊗std=4.
        assert_eq!(
            association_scheme_multiplicities(clq.num_vars, &clq.clauses),
            Some(vec![1, 2, 2, 4]),
            "clique(3,3): eigenspace dimensions = S₃×S₃ constituent degrees"
        );

        // In general the multiplicities partition the space and include the trivial eigenspace (dim 1).
        for (nv, cl) in [(clq.num_vars, clq.clauses.clone()), (6, c6.clone())] {
            let m = association_scheme_multiplicities(nv, &cl).expect("commutative scheme has multiplicities");
            assert_eq!(m.iter().sum::<u128>(), nv as u128, "Σ multiplicities = number of vertices");
            assert!(m.iter().all(|&x| x >= 1), "each eigenspace is non-empty");
            assert_eq!(m[0], 1, "the smallest (trivial) eigenspace has dimension 1");
            // #multiplicities = #eigenspaces = scheme rank.
            assert_eq!(m.len(), coherent_rank(nv, &cl).unwrap(), "one multiplicity per eigenspace");
        }
    }

    #[test]
    fn association_scheme_eigenmatrix_is_the_scheme_character_table() {
        let edge = |a: u32, b: u32| vec![Lit::new(a, true), Lit::new(b, true)];
        let (clq, _) = crate::families::clique_coloring(3, 3);
        let c6: Vec<Vec<Lit>> = (0..6).map(|i| edge(i, (i + 1) % 6)).collect();

        // clique(3,3) is the rook's-graph scheme (rank 4); C₆ is the cyclic scheme (rank 4). Both are
        // commutative, so each has an eigenmatrix P — the scheme's character table.
        assert_eq!(coherent_rank(clq.num_vars, &clq.clauses), Some(4), "clique(3,3): 4 relations");

        for (nv, cl) in [(clq.num_vars, clq.clauses.clone()), (6, c6.clone())] {
            let d = coherent_rank(nv, &cl).unwrap();
            let (p, pm) = association_scheme_eigenmatrix(nv, &cl).expect("a commutative scheme has an eigenmatrix");
            assert_eq!(pm.len(), d, "P has one row per common eigenspace (d × d)");
            assert!(pm.iter().all(|r| r.len() == d));

            // Each row is a 1-dimensional representation of the coherent algebra:
            // P[m][i]·P[m][j] = Σ_k a_{ijk}·P[m][k]  (re-verified against the intersection numbers).
            let (_, a) = coherent_configuration_constants(nv, &cl).unwrap();
            for row in &pm {
                for i in 0..d {
                    for j in 0..d {
                        let lhs = row[i] as u128 * row[j] as u128 % p as u128;
                        let rhs = (0..d)
                            .map(|k| (a[i][j][k] % p as u128) * row[k] as u128 % p as u128)
                            .sum::<u128>()
                            % p as u128;
                        assert_eq!(lhs, rhs, "row must be an algebra homomorphism");
                    }
                }
            }

            // The valencies (all-ones eigenvalue) partition the vertices and include the diagonal (valency 1),
            // and that valency vector is one of P's rows.
            let valency: Vec<u128> = (0..d).map(|i| (0..d).map(|j| a[i][j][0]).sum()).collect();
            assert_eq!(valency.iter().sum::<u128>(), nv as u128, "Σ valencies = number of vertices");
            assert!(valency.contains(&1), "the diagonal relation has valency 1");
            assert!(
                pm.iter().any(|row| (0..d).all(|i| row[i] as u128 == valency[i] % p as u128)),
                "the valency vector is a row of P (the trivial eigenspace)"
            );
        }
    }

    #[test]
    fn coherent_configuration_is_a_genuine_association_scheme() {
        let edge = |a: u32, b: u32| vec![Lit::new(a, true), Lit::new(b, true)];

        // The stabilized 2-WL coloring is a coherent configuration: its intersection numbers are
        // well-defined (the function returns Some only after checking EVERY pair), and they satisfy the
        // basic algebra identities. Verified on several structures + a fuzz.
        let (php, _) = crate::families::php(4);
        let (clq, _) = crate::families::clique_coloring(3, 3);
        let c6 = vec![edge(0, 1), edge(1, 2), edge(2, 3), edge(3, 4), edge(4, 5), edge(5, 0)];

        let check = |nv: usize, cl: &[Vec<Lit>]| {
            let (d, p) = coherent_configuration_constants(nv, cl).expect("2-WL is coherent");
            assert_eq!(d, two_wl_pair_cells(nv, cl), "rank = number of basis relations");
            // Σ_{i,j} p[i][j][k] = n for every relation k (every intermediate z lands in exactly one cell).
            for k in 0..d {
                let total: u128 = (0..d).flat_map(|i| (0..d).map(move |j| (i, j))).map(|(i, j)| p[i][j][k]).sum();
                assert_eq!(total, nv as u128, "Σ_ij p[i][j][k] counts all n intermediate points");
            }
            // Transpose closure: the reverse of a basis relation is again a single basis relation.
            let pc = two_wl_pair_colors(nv, cl);
            let mut transpose = vec![usize::MAX; d];
            for i in 0..nv {
                for j in 0..nv {
                    let (r, rt) = (pc[i][j], pc[j][i]);
                    if transpose[r] == usize::MAX {
                        transpose[r] = rt;
                    } else {
                        assert_eq!(transpose[r], rt, "R_r^T must be a single relation");
                    }
                }
            }
            d
        };
        check(php.num_vars, &php.clauses);
        let dclq = check(clq.num_vars, &clq.clauses);
        check(6, &c6);

        // clique(3,3) is the 3×3 rook's graph: S₃×S₃ is transitive with rank 4, so the coherent
        // configuration has exactly 4 relations (diagonal, same-row, same-column, different-both) — and it
        // matches the group's orbital count (Schurian).
        assert_eq!(dclq, 4, "the rook's graph scheme has 4 relations");
        let clq_gens = crate::sym_break::variable_automorphism_generators(clq.num_vars, &clq.clauses).unwrap();
        assert_eq!(dclq, crate::permgroup::orbitals(clq.num_vars, &clq_gens).len(), "Schurian: relations = orbitals");

        // A concrete intersection number on C₆ (vertices 0..5 in a cycle): the adjacency relation is the
        // colour of an edge pair (0,1) and the distance-2 relation that of (0,2). Two vertices at distance 2
        // share exactly ONE common neighbour, so p[adjacency][adjacency][distance-2] = 1.
        let pc6 = two_wl_pair_colors(6, &c6);
        let (_d6, p6) = coherent_configuration_constants(6, &c6).unwrap();
        let adj = pc6[0][1];
        let dist2 = pc6[0][2];
        assert_ne!(adj, dist2, "2-WL separates adjacency from distance-2 in C₆");
        assert_eq!(p6[adj][adj][dist2], 1, "C₆: vertices at distance 2 share exactly one common neighbour");

        fn xs(s: &mut u64) -> u64 {
            *s ^= *s << 13;
            *s ^= *s >> 7;
            *s ^= *s << 17;
            *s
        }
        let nv = 5usize;
        let mut seed = 0x0C0E_4E17_C0FF_EE00u64;
        for _ in 0..100 {
            let m = (xs(&mut seed) % 6) as usize;
            let cl: Vec<Vec<Lit>> = (0..m)
                .map(|_| {
                    let w = 1 + (xs(&mut seed) % 3) as usize;
                    (0..w).map(|_| Lit::new((xs(&mut seed) % nv as u64) as u32, xs(&mut seed) & 1 == 0)).collect()
                })
                .collect();
            // The key invariant: 2-WL always yields a coherent configuration (Some, not None).
            assert!(coherent_configuration_constants(nv, &cl).is_some(), "2-WL must always be coherent");
        }
    }

    #[test]
    fn three_wl_over_approximates_3_orbits_and_beats_two_wl() {
        let edge = |a: usize, b: usize| vec![Lit::new(a as u32, true), Lit::new(b as u32, true)];

        // Build a 6-regular Cayley graph on ℤ₄×ℤ₄ (16 vertices v = 4*r + c) from a connection set.
        let cayley = |conn: &[(i32, i32)]| -> Vec<Vec<Lit>> {
            let idx = |r: i32, c: i32| (((r.rem_euclid(4)) * 4 + c.rem_euclid(4)) as usize);
            let mut edges = std::collections::BTreeSet::new();
            for r in 0..4 {
                for c in 0..4 {
                    for &(dr, dc) in conn {
                        let (a, b) = (idx(r, c), idx(r + dr, c + dc));
                        if a < b {
                            edges.insert((a, b));
                        }
                    }
                }
            }
            edges.into_iter().map(|(a, b)| edge(a, b)).collect()
        };
        // The 4×4 rook's graph (= K₄□K₄): same row or same column. SRG(16,6,2,2).
        let rook = cayley(&[(1, 0), (2, 0), (3, 0), (0, 1), (0, 2), (0, 3)]);
        // The Shrikhande graph: connection set ±(1,0), ±(0,1), ±(1,1). Also SRG(16,6,2,2).
        let shrikhande = cayley(&[(1, 0), (3, 0), (0, 1), (0, 3), (1, 1), (3, 3)]);

        // THE HEADLINE: both are strongly regular with the SAME parameters, so 2-WL produces identical
        // colorings (same fingerprint) and CANNOT tell them apart — but 3-WL distinguishes them.
        assert_eq!(
            two_wl_fingerprint(16, &rook),
            two_wl_fingerprint(16, &shrikhande),
            "2-WL cannot separate two SRG(16,6,2,2) graphs"
        );
        assert_ne!(
            three_wl_fingerprint(16, &rook),
            three_wl_fingerprint(16, &shrikhande),
            "3-WL SEPARATES the rook's graph from the Shrikhande graph"
        );

        // THE THEOREM (the 3-orbit analogue of #42/#43): every 3-orbit is monochromatic under 3-WL.
        let check = |nv: usize, cl: &[Vec<Lit>]| {
            let gens = crate::sym_break::variable_automorphism_generators(nv, cl).unwrap_or_default();
            let tw = three_wl_colors(nv, cl);
            for orbit in crate::permgroup::orbits_on_tuples(nv, &gens, 3) {
                let t0 = &orbit[0];
                let c0 = tw[t0[0]][t0[1]][t0[2]];
                assert!(orbit.iter().all(|t| tw[t[0]][t[1]][t[2]] == c0), "3-orbit must be monochromatic");
            }
        };
        let (clq, _) = crate::families::clique_coloring(3, 3);
        let c6: Vec<Vec<Lit>> = (0..6).map(|i| edge(i, (i + 1) % 6)).collect();
        check(clq.num_vars, &clq.clauses);
        check(6, &c6);

        fn xs(s: &mut u64) -> u64 {
            *s ^= *s << 13;
            *s ^= *s >> 7;
            *s ^= *s << 17;
            *s
        }
        let nv = 5usize;
        let mut seed = 0x33C0_FFEE_0033_0033u64;
        for _ in 0..60 {
            let m = (xs(&mut seed) % 6) as usize;
            let cl: Vec<Vec<Lit>> = (0..m)
                .map(|_| {
                    let w = 1 + (xs(&mut seed) % 3) as usize;
                    (0..w).map(|_| Lit::new((xs(&mut seed) % nv as u64) as u32, xs(&mut seed) & 1 == 0)).collect()
                })
                .collect();
            check(nv, &cl);
        }
    }

    #[test]
    fn canonical_form_decides_formula_isomorphism() {
        let edge = |a: usize, b: usize| vec![Lit::new(a as u32, true), Lit::new(b as u32, true)];
        let relabel = |cl: &[Vec<Lit>], perm: &[usize]| -> Vec<Vec<Lit>> {
            cl.iter()
                .map(|c| c.iter().map(|l| Lit::new(perm[l.var() as usize] as u32, l.is_positive())).collect())
                .collect()
        };

        // THE DEFINING PROPERTY: canonical form is an isomorphism invariant. Relabelling the variables by any
        // permutation leaves the canonical form unchanged.
        let c6: Vec<Vec<Lit>> = (0..6).map(|i| edge(i, (i + 1) % 6)).collect();
        let perm = [3usize, 5, 0, 2, 4, 1];
        assert_eq!(
            canonical_form(6, &c6),
            canonical_form(6, &relabel(&c6, &perm)),
            "isomorphic formulas share a canonical form"
        );
        assert_eq!(formulas_isomorphic(6, &c6, &relabel(&c6, &perm)), Some(true));

        // COMPLETENESS: canonical form separates C₆ from two triangles — where 1-WL is blind (one cell each).
        let two_tri = vec![edge(0, 1), edge(1, 2), edge(0, 2), edge(3, 4), edge(4, 5), edge(3, 5)];
        assert_eq!(color_refinement_cells(6, &c6), color_refinement_cells(6, &two_tri), "1-WL cannot tell them apart");
        assert_eq!(formulas_isomorphic(6, &c6, &two_tri), Some(false), "but canonical form CAN");
        assert_ne!(canonical_form(6, &c6), canonical_form(6, &two_tri));

        // A path and a star on 4 vertices are non-isomorphic trees; canonical form distinguishes them.
        let path4 = vec![edge(0, 1), edge(1, 2), edge(2, 3)];
        let star4 = vec![edge(0, 1), edge(0, 2), edge(0, 3)];
        assert_eq!(formulas_isomorphic(4, &path4, &star4), Some(false));

        // FUZZ: for random formulas and random permutations, F and π(F) ALWAYS share a canonical form, and a
        // structural change (dropping a clause) is detected as non-isomorphic when it truly changes the form.
        fn xs(s: &mut u64) -> u64 {
            *s ^= *s << 13;
            *s ^= *s >> 7;
            *s ^= *s << 17;
            *s
        }
        let nv = 6usize;
        let mut seed = 0xCA90_F00D_1234_5678u64;
        for _ in 0..120 {
            let m = (xs(&mut seed) % 7) as usize;
            let f: Vec<Vec<Lit>> = (0..m)
                .map(|_| {
                    let w = 1 + (xs(&mut seed) % 2) as usize;
                    (0..w).map(|_| Lit::new((xs(&mut seed) % nv as u64) as u32, xs(&mut seed) & 1 == 0)).collect()
                })
                .collect();
            // A random permutation via Fisher–Yates.
            let mut perm: Vec<usize> = (0..nv).collect();
            for i in (1..nv).rev() {
                let j = (xs(&mut seed) % (i as u64 + 1)) as usize;
                perm.swap(i, j);
            }
            let fp = relabel(&f, &perm);
            assert_eq!(canonical_form(nv, &f), canonical_form(nv, &fp), "F ≅ π(F): F={f:?} perm={perm:?}");
            assert_eq!(formulas_isomorphic(nv, &f, &fp), Some(true));
        }
    }

    #[test]
    fn weighted_model_count_is_exact_and_symmetry_accelerated() {
        let p = |v: u32| Lit::new(v, true);
        let nl = |v: u32| Lit::new(v, false);
        let sat = |m: &[bool], cls: &[Vec<Lit>]| {
            cls.iter().all(|c| c.iter().any(|l| m[l.var() as usize] == l.is_positive()))
        };
        let brute = |nv: usize, f: &[Vec<Lit>], w: &[(i64, i64)]| -> i128 {
            (0u32..(1u32 << nv))
                .filter_map(|x| {
                    let a: Vec<bool> = (0..nv).map(|i| (x >> i) & 1 == 1).collect();
                    sat(&a, f).then(|| (0..nv).map(|i| if a[i] { w[i].1 as i128 } else { w[i].0 as i128 }).product::<i128>())
                })
                .sum()
        };

        // Unit weights ⇒ the weighted count is just the model count.
        let amo = vec![vec![p(0), p(1), p(2)]]; // at-least-one-of-3: 7 models
        let ones = vec![(1i64, 1i64); 3];
        assert_eq!(weighted_model_count(3, &amo, &ones), 7, "unit weights ⇒ #models");
        assert_eq!(weighted_model_count(3, &amo, &ones), brute(3, &amo, &ones));

        // THE ACCELERATION: a fully S₃-symmetric instance fuses its models into few orbits, so the count
        // takes far fewer solves than there are models.
        let (solves, models) = weighted_model_count_solve_counts(3, &amo);
        assert_eq!(models, 7, "7 models of at-least-one-of-3");
        assert!(solves < models, "symmetry must reduce the solves: {solves} < {models}");

        // Weighted, symmetric weights: Z must match brute force exactly.
        let w_sym = vec![(2i64, 3i64); 3];
        assert_eq!(weighted_model_count(3, &amo, &w_sym), brute(3, &amo, &w_sym));

        // UNSAT formula ⇒ empty product sum = 0.
        assert_eq!(weighted_model_count(2, &[vec![p(0)], vec![nl(0)]], &[(1, 1), (1, 1)]), 0);

        // FUZZ: arbitrary formulas and arbitrary (asymmetric) weights — the symmetry-accelerated count is
        // ALWAYS the exact weighted model count (every model's true weight is summed; symmetry only groups
        // the search).
        fn xs(s: &mut u64) -> u64 {
            *s ^= *s << 13;
            *s ^= *s >> 7;
            *s ^= *s << 17;
            *s
        }
        let nv = 4usize;
        let mut seed = 0x5EED_4321_FACE_0001u64;
        for _ in 0..200 {
            let m = (xs(&mut seed) % 5) as usize;
            let f: Vec<Vec<Lit>> = (0..m)
                .map(|_| {
                    let wd = 1 + (xs(&mut seed) % 2) as usize;
                    (0..wd).map(|_| Lit::new((xs(&mut seed) % nv as u64) as u32, xs(&mut seed) & 1 == 0)).collect()
                })
                .collect();
            let w: Vec<(i64, i64)> = (0..nv).map(|_| ((xs(&mut seed) % 4) as i64, (xs(&mut seed) % 4) as i64)).collect();
            assert_eq!(weighted_model_count(nv, &f, &w), brute(nv, &f, &w), "F={f:?} w={w:?}");
        }
    }

    #[test]
    fn assignment_weight_inventory_splits_orbits_by_weight() {
        let (cnf, _) = crate::families::clique_coloring(3, 3);
        let inv = assignment_weight_inventory(cnf.num_vars, &cnf.clauses).expect("enumerable group");
        assert_eq!(inv.len(), 10, "weights 0..=9");
        assert_eq!(inv.iter().sum::<u128>(), 36, "sums to the assignment-orbit count (36 binary 3×3 matrices)");
        assert_eq!(inv[0], 1, "one all-false assignment");
        assert_eq!(inv[1], 1, "all single-one matrices are row/column equivalent");
        assert_eq!(inv[9], 1, "one all-true assignment");
        // The sum agrees with the profile's scalar assignment_orbits.
        assert_eq!(
            Some(inv.iter().sum::<u128>()),
            symmetry_structure(cnf.num_vars, &cnf.clauses).assignment_orbits,
            "the inventory sums to the profile's assignment_orbits"
        );

        // No symmetry ⇒ the binomial distribution (every assignment is its own orbit).
        let asym = vec![vec![Lit::new(0, true)], vec![Lit::new(1, false), Lit::new(2, true)]];
        assert_eq!(assignment_weight_inventory(3, &asym), Some(vec![1, 3, 3, 1]), "no symmetry ⇒ C(3,w)");
    }

    #[test]
    fn pb_coefficient_symmetry_profiles_through_the_same_ladder() {
        use crate::pseudo_boolean::PbConstraint;
        // 3·x0 + 3·x1 + 3·x2 + 5·x3 ≥ 6: {x0,x1,x2} share coefficient 3 (interchangeable, S₃), x3 (coeff 5)
        // is alone. The coefficient-symmetry group is S₃ on {0,1,2}.
        let c = PbConstraint::new_weighted(&[(0, 3, true), (1, 3, true), (2, 3, true), (3, 5, true)], 6);
        let prof = pb_symmetry_profile(4, &[c]);
        assert_eq!(prof.order, 6, "S₃ on the three weight-3 variables");
        assert_eq!(prof.num_orbits, 2, "two orbits: {{x0,x1,x2}} and the fixed point x3");
        assert!(!prof.abelian, "S₃ is non-abelian");
        assert_eq!(prof.solvable, Some(true), "S₃ is solvable");
        assert_eq!(prof.coherent_rank, None, "the coefficient profile has no clauses ⇒ no scheme rank");

        // All-distinct coefficients ⇒ no coefficient symmetry ⇒ the trivial profile.
        let distinct = PbConstraint::new_weighted(&[(0, 1, true), (1, 2, true), (2, 3, true)], 3);
        assert_eq!(pb_symmetry_profile(3, &[distinct]).order, 1, "distinct weights ⇒ trivial group");
    }

    #[test]
    fn symmetry_structure_profiles_the_variable_group() {
        let p = |v| Lit::new(v, true);
        let n = |v| Lit::new(v, false);

        // exactly-one-of-3: the variable group is S₃ on 3 points — sharply 3-transitive, primitive, rank 2.
        let exactly1 = vec![
            vec![p(0), p(1), p(2)],
            vec![n(0), n(1)], vec![n(0), n(2)], vec![n(1), n(2)],
        ];
        let prof = symmetry_structure(3, &exactly1);
        assert_eq!(prof.order, 6, "|S₃| = 6");
        assert_eq!(prof.num_orbits, 1, "transitive on the 3 cells");
        assert_eq!(prof.rank, 2, "S₃ is 2-transitive ⇒ rank 2");
        assert_eq!(prof.coherent_rank, Some(2), "coherent (scheme) rank matches the orbital rank here");
        assert!(prof.coherent_rank.unwrap() <= prof.rank, "coherent rank ≤ orbital rank");
        assert_eq!(prof.transitivity, 3, "S₃ is 3-transitive on 3 points");
        assert!(prof.primitive, "S₃ on 3 points is primitive");
        assert_eq!(prof.blocks, None, "a primitive group has no block system");
        assert!(!prof.abelian, "S₃ is non-abelian");
        assert_eq!(prof.solvable, Some(true), "S₃ is solvable");
        assert_eq!(prof.nilpotent, Some(false), "S₃ is solvable but NOT nilpotent");
        assert_eq!(prof.derived_length, Some(2), "S₃ has derived length 2");
        assert_eq!(prof.nilpotency_class, None, "S₃ is not nilpotent ⇒ no nilpotency class");
        assert_eq!(prof.derived_order, 3, "[S₃,S₃] = A₃ has order 3");
        assert_eq!(prof.conjugacy_classes, Some(3), "S₃ has 3 conjugacy classes (= 3 irreps)");
        assert_eq!(prof.center_order, Some(1), "S₃ has a trivial centre");
        assert_eq!(prof.exponent, Some(6), "S₃ has exponent 6 (lcm of orders 1,2,3)");
        assert_eq!(prof.subgroups, Some(6), "S₃ has 6 subgroups");
        assert_eq!(prof.simple, Some(false), "S₃ is not simple (A₃ is normal)");
        assert_eq!(prof.composition_factors, Some(vec![2, 3]), "S₃ = C₂, C₃");
        assert_eq!(prof.sylow, Some(vec![(2, 3), (3, 1)]), "S₃: 3 Sylow-2, 1 Sylow-3");
        assert_eq!(prof.real_classes, Some(3), "S₃: all 3 classes (= irreps) are real");
        assert_eq!(prof.rational_classes, Some(3), "S₃ is rational: all 3 classes rational");
        assert_eq!(prof.automorphism_order, Some(6), "|Aut(S₃)| = 6 (S₃ is complete)");
        assert_eq!(prof.outer_automorphism_order, Some(1), "Out(S₃) = 1");
        assert_eq!(prof.irreducible_degrees, Some(vec![1, 1, 2]), "S₃ irreps: trivial, sign, 2-dim standard");
        assert_eq!(prof.frobenius_schur, Some(vec![1, 1, 1]), "S₃ is totally real ⇒ all indicators +1");
        {
            // The permutation rep of S₃ on its 3 variables = trivial ⊕ standard: Σ m·d = 3, Σ m² = rank,
            // and the trivial multiplicity is the orbit count (Burnside).
            let iso = prof.isotypic_multiplicities.as_ref().expect("S₃ isotypic decomposition");
            let degs = prof.irreducible_degrees.as_ref().unwrap();
            assert_eq!(iso.iter().zip(degs).map(|(m, d)| m * d).sum::<u128>(), 3, "Σ m·d = 3 variables");
            assert_eq!(iso.iter().map(|m| m * m).sum::<u128>(), prof.rank as u128, "Σ m² = rank");
        }

        // clique_coloring(3,3): the variable group is S₃(vertices) × S₃(colours) on the 3×3 grid — order 36,
        // transitive but only 1-transitive, IMPRIMITIVE (rows/cols are blocks), rank 4 (diagonal + same-row
        // + same-col + different-both).
        let (clique, _) = crate::families::clique_coloring(3, 3);
        let cp = symmetry_structure(clique.num_vars, &clique.clauses);
        assert_eq!(cp.order, 36, "|S₃ × S₃| = 36");
        assert_eq!(cp.num_orbits, 1, "transitive on the 9 cells");
        assert_eq!(cp.rank, 4, "the grid action has rank 4");
        assert_eq!(cp.coherent_rank, Some(4), "clique(3,3): the coherent scheme has 4 relations");
        assert_eq!(cp.transitivity, 1, "transitive but not 2-transitive");
        assert!(!cp.primitive, "the grid action is imprimitive");
        assert!(cp.blocks.is_some(), "rows/columns form a block system");
        assert!(!cp.abelian, "S₃ × S₃ is non-abelian");
        assert_eq!(cp.solvable, Some(true), "S₃ × S₃ is solvable");
        assert_eq!(cp.nilpotent, Some(false), "S₃ × S₃ is not nilpotent (S₃ isn't)");
        assert_eq!(cp.derived_order, 9, "[S₃×S₃, S₃×S₃] = A₃ × A₃ has order 9");
        assert_eq!(cp.conjugacy_classes, Some(9), "S₃×S₃ has 3·3 = 9 conjugacy classes");
        assert_eq!(cp.center_order, Some(1), "S₃×S₃ has a trivial centre");
        assert_eq!(cp.rational_classes, Some(9), "S₃×S₃ is rational: all 9 classes rational");
        // Aut(S₃×S₃) = Aut(S₃) ≀ C₂ = (S₃×S₃)⋊C₂, order 72; Inn = 36 (trivial centre) ⇒ Out = C₂.
        assert_eq!(cp.automorphism_order, Some(72), "|Aut(S₃×S₃)| = 72");
        assert_eq!(cp.outer_automorphism_order, Some(2), "Out(S₃×S₃) = C₂ (factor swap)");
        assert_eq!(cp.assignment_orbits, Some(36), "2⁹ assignments up to S₃×S₃ = 36 binary 3×3 matrices");
        assert_eq!(cp.abelianization, Some((4, 2)), "(S₃×S₃)ᵃᵇ = C₂×C₂ (order 4, exponent 2)");
        assert!(cp.subgroups.is_some(), "the S₃×S₃ subgroup lattice is computed");
        assert_eq!(cp.simple, Some(false), "S₃×S₃ is not simple");
        assert_eq!(cp.composition_factors, Some(vec![2, 2, 3, 3]), "S₃×S₃ = C₂², C₃² (product 36)");
        assert_eq!(cp.sylow, Some(vec![(2, 9), (3, 1)]), "S₃×S₃: 3² = 9 Sylow-2, 1 Sylow-3");
        // Irreducibles of a direct product are the pairwise products of the factors' irreducibles:
        // {1,1,2} ⊗ {1,1,2} = {1,1,1,1,2,2,2,2,4}, and Σ dᵢ² = 4·1 + 4·4 + 16 = 36 = |G|.
        assert_eq!(
            cp.irreducible_degrees,
            Some(vec![1, 1, 1, 1, 2, 2, 2, 2, 4]),
            "S₃×S₃ irreps = products of the S₃ irreps"
        );
        assert_eq!(
            cp.frobenius_schur,
            Some(vec![1, 1, 1, 1, 1, 1, 1, 1, 1]),
            "S₃×S₃ is totally real ⇒ all nine indicators +1"
        );
        {
            // The permutation rep on the 9 grid cells decomposes with Σ m·d = 9 and ⟨π,π⟩ = rank = 4.
            let iso = cp.isotypic_multiplicities.as_ref().expect("S₃×S₃ isotypic decomposition");
            let degs = cp.irreducible_degrees.as_ref().unwrap();
            assert_eq!(iso.iter().zip(degs).map(|(m, d)| m * d).sum::<u128>(), 9, "Σ m·d = 9 cells");
            assert_eq!(iso.iter().map(|m| m * m).sum::<u128>(), cp.rank as u128, "⟨π,π⟩ = rank = 4");
        }

        // No symmetry → the trivial profile.
        let asym = vec![vec![p(0)], vec![n(1), p(2)]];
        let ap = symmetry_structure(3, &asym);
        assert_eq!(ap.order, 1, "no symmetry ⇒ trivial group");
        assert!(ap.coherent_rank.is_some(), "the scheme rank is always computed from the clauses");
        assert_eq!(ap.irreducible_degrees, Some(vec![1]), "trivial group: one trivial irreducible");
        assert_eq!(ap.frobenius_schur, Some(vec![1]), "trivial group: its character is real");
        assert_eq!(
            ap.isotypic_multiplicities,
            Some(vec![3]),
            "trivial group on 3 vars: the perm rep is 3 copies of the trivial irreducible"
        );
        assert_eq!(ap.rational_classes, Some(1), "trivial group: its one class is rational");
        assert_eq!(ap.automorphism_order, Some(1), "trivial group: Aut is trivial");
        assert_eq!(ap.outer_automorphism_order, Some(1), "trivial group: Out is trivial");
    }

    #[test]
    fn models_up_to_symmetry_enumerates_orbits_and_counts_exactly() {
        let p = |v| Lit::new(v, true);
        let n = |v| Lit::new(v, false);

        // Independent oracle: (#models, #orbits) under the variable-symmetry generators, by brute force.
        let oracle = |nv: usize, cl: &[Vec<Lit>]| -> (u128, usize) {
            let models: Vec<Vec<bool>> = (0u64..(1u64 << nv))
                .filter_map(|x| {
                    let a: Vec<bool> = (0..nv).map(|i| (x >> i) & 1 == 1).collect();
                    cl.iter().all(|c| c.iter().any(|l| a[l.var() as usize] == l.is_positive())).then_some(a)
                })
                .collect();
            let gens = crate::sym_break::variable_automorphism_generators(nv, cl).unwrap_or_default();
            let model_set: std::collections::HashSet<Vec<bool>> = models.iter().cloned().collect();
            let mut seen = std::collections::HashSet::new();
            let mut orbits = 0;
            for m in &models {
                if seen.contains(m) {
                    continue;
                }
                orbits += 1;
                let mut stack = vec![m.clone()];
                while let Some(cur) = stack.pop() {
                    if !seen.insert(cur.clone()) {
                        continue;
                    }
                    for g in &gens {
                        let mut pm = vec![false; nv];
                        for v in 0..nv {
                            pm[g[v]] = cur[v];
                        }
                        if model_set.contains(&pm) && !seen.contains(&pm) {
                            stack.push(pm);
                        }
                    }
                }
            }
            (models.len() as u128, orbits)
        };

        // exactly-one-of-3 (one-hot): 3 models, all one orbit under S₃.
        let exactly1 = vec![
            vec![p(0), p(1), p(2)],
            vec![n(0), n(1)], vec![n(0), n(2)], vec![n(1), n(2)],
        ];
        // at-most-one-of-3: 4 models {000, 100, 010, 001} → 2 orbits ({000}, the three weight-1).
        let atmost1 = vec![vec![n(0), n(1)], vec![n(0), n(2)], vec![n(1), n(2)]];

        for cl in [&exactly1, &atmost1] {
            let (m_exact, m_orbits) = oracle(3, cl);
            let sc = models_up_to_symmetry(3, cl, 1000);
            assert!(sc.exhaustive, "the small instance is enumerated to exhaustion");
            assert_eq!(sc.total_models, m_exact, "exact model count = sum of orbit sizes");
            assert_eq!(sc.representatives.len(), m_orbits, "one representative per orbit");
            // Cross-check the orbit count against Burnside — two independent routes must agree.
            if let Some(burnside) = crate::sym_break::count_models_modulo_symmetry(3, cl) {
                assert_eq!(sc.representatives.len(), burnside, "enumeration agrees with Burnside");
            }
            // Every representative is a genuine model, and they lie in distinct orbits (all distinct).
            for r in &sc.representatives {
                assert!(cl.iter().all(|c| c.iter().any(|l| r[l.var() as usize] == l.is_positive())), "valid model");
            }
            let distinct: std::collections::HashSet<&Vec<bool>> = sc.representatives.iter().collect();
            assert_eq!(distinct.len(), sc.representatives.len(), "representatives are distinct");
        }

        // Specific counts, to pin the numbers.
        assert_eq!(models_up_to_symmetry(3, &exactly1, 1000).total_models, 3);
        assert_eq!(models_up_to_symmetry(3, &exactly1, 1000).representatives.len(), 1);
        assert_eq!(models_up_to_symmetry(3, &atmost1, 1000).total_models, 4);
        assert_eq!(models_up_to_symmetry(3, &atmost1, 1000).representatives.len(), 2);
    }

    #[test]
    fn declared_symmetry_is_verified_then_broken() {
        let p = |v| Lit::new(v, true);
        let brute = |cl: &[Vec<Lit>], nv: usize| {
            (0u64..(1u64 << nv)).any(|x| {
                let a: Vec<bool> = (0..nv).map(|i| (x >> i) & 1 == 1).collect();
                cl.iter().all(|c| c.iter().any(|l| a[l.var() as usize] == l.is_positive()))
            })
        };

        // clique_coloring(3,3), x[v][c] = 3v+c. A declared vertex swap 0↔1 is a genuine symmetry — verified
        // and used.
        let (cnf, _) = crate::families::clique_coloring(3, 3);
        let nv = cnf.num_vars;
        let mut vswap: Vec<usize> = (0..nv).collect();
        for c in 0..3 {
            vswap.swap(c, 3 + c);
        }
        assert!(is_declared_symmetry(nv, &cnf.clauses, &vswap), "a vertex swap is a genuine clique symmetry");
        let s = solve_with_declared_symmetry(nv, &cnf.clauses, &[vswap]);
        assert_eq!(s.via, Route::DeclaredSymmetry);
        match &s.answer {
            Answer::Sat(m) => assert!(cnf.clauses.iter().all(|c| c.iter().any(|l| m[l.var() as usize] == l.is_positive()))),
            Answer::Unsat => panic!("clique_coloring(3,3) is SAT"),
        }

        // A BOGUS declared generator (a valid permutation but NOT a symmetry) is rejected — the verdict is
        // unaffected (a wrong declaration cannot corrupt the result).
        let mut bogus: Vec<usize> = (0..nv).collect();
        bogus.swap(0, 5); // x[0][0] ↔ x[1][2] is not a system symmetry
        assert!(!is_declared_symmetry(nv, &cnf.clauses, &bogus), "a non-symmetry must be rejected");
        let wb = solve_with_declared_symmetry(nv, &cnf.clauses, &[bogus]);
        assert!(matches!(wb.answer, Answer::Sat(_)), "a bogus declaration must not corrupt the SAT verdict");
        if let Answer::Sat(m) = &wb.answer {
            assert!(cnf.clauses.iter().all(|c| c.iter().any(|l| m[l.var() as usize] == l.is_positive())));
        }

        // A SEMANTIC declared symmetry (not syntactic): a↔b in (a∨x)(b∨x)(a∨x∨y). Accepted via implication.
        let f = vec![vec![p(0), p(2)], vec![p(1), p(2)], vec![p(0), p(2), p(3)]];
        let mut ab: Vec<usize> = (0..4).collect();
        ab.swap(0, 1);
        assert!(is_declared_symmetry(4, &f, &ab), "a↔b is a semantic symmetry, verified by implication");
        let sem = solve_with_declared_symmetry(4, &f, &[ab]);
        assert_eq!(sem.via, Route::DeclaredSymmetry);
        assert_eq!(matches!(sem.answer, Answer::Sat(_)), brute(&f, 4), "declared-symmetry verdict matches brute force");

        // A malformed generator (wrong length) is rejected without panicking; the verdict is still correct.
        assert!(!is_declared_symmetry(4, &f, &[0usize, 1]), "a wrong-length permutation is rejected");
        let mf = solve_with_declared_symmetry(4, &f, &[vec![0usize, 1]]);
        assert_eq!(
            matches!(mf.answer, Answer::Sat(_)),
            matches!(solve_comprehensive(4, &f).answer, Answer::Sat(_)),
            "a malformed declaration is dropped, verdict unchanged"
        );
    }

    #[test]
    fn almost_symmetry_breaks_a_near_miss_conditionally() {
        let p = |v| Lit::new(v, true);
        let brute = |cl: &[Vec<Lit>], nv: usize| {
            (0u64..(1u64 << nv)).any(|x| {
                let a: Vec<bool> = (0..nv).map(|i| (x >> i) & 1 == 1).collect();
                cl.iter().all(|c| c.iter().any(|l| a[l.var() as usize] == l.is_positive()))
            })
        };
        // a=0,b=1,x=2,y=3. (a∨x)(b∨x) is symmetric in a,b; the extra (a∨y) breaks it, and its swap (b∨y)
        // is NOT implied — so a↔b is an ALMOST-symmetry (one broken clause), not a semantic one.
        let f = vec![vec![p(0), p(2)], vec![p(1), p(2)], vec![p(0), p(3)]];

        // It is NOT a semantic symmetry (the broken image is not implied).
        let (sem, _) = semantic_symmetry_pairs(4, &f);
        assert!(!sem.contains(&(0, 1)), "a,b is not a SEMANTIC symmetry here: {sem:?}");

        // It IS an almost-symmetry: a↔b breaks exactly one clause.
        let almost = almost_symmetry_pairs(4, &f, 2);
        assert!(
            almost.iter().any(|(a, b, imgs)| *a == 0 && *b == 1 && imgs.len() == 1),
            "a↔b breaks exactly one clause: {almost:?}"
        );

        // Solved correctly — the conditional (guarded) break is sound.
        let s = almost_symmetry_solve(4, &f).expect("an almost-symmetry is detected and conditionally broken");
        assert_eq!(s.via, Route::AlmostSymmetry);
        assert_eq!(matches!(s.answer, Answer::Sat(_)), brute(&f, 4), "almost-symmetry verdict matches brute force");
        if let Answer::Sat(m) = &s.answer {
            assert!(f.iter().all(|c| c.iter().any(|l| m[l.var() as usize] == l.is_positive())), "valid model");
        }

        // UNSAT path: force every variable off — (a∨x) cannot hold. The conditional break preserves UNSAT.
        let mut un = f.clone();
        un.extend([
            vec![Lit::new(0, false)],
            vec![Lit::new(1, false)],
            vec![Lit::new(2, false)],
            vec![Lit::new(3, false)],
        ]);
        let u = almost_symmetry_solve(4, &un).expect("almost-symmetry still detected");
        assert_eq!(u.via, Route::AlmostSymmetry);
        assert!(matches!(u.answer, Answer::Unsat), "all variables off makes (a∨x) UNSAT");
        assert_eq!(matches!(u.answer, Answer::Sat(_)), brute(&un, 4), "UNSAT verdict matches brute force");
    }

    #[test]
    fn semantic_symmetry_breaks_a_non_syntactic_interchange() {
        let p = |v| Lit::new(v, true);
        // a=0, b=1, x=2, y=3. (a∨x) ∧ (b∨x) ∧ (a∨x∨y): the third clause is IMPLIED by (a∨x), so it is
        // redundant — F's models are symmetric in a,b. But its swap (b∨x∨y) is not a clause, so swapping
        // a,b does NOT preserve the clause set: a,b are SEMANTICALLY but not SYNTACTICALLY interchangeable.
        let f = vec![vec![p(0), p(2)], vec![p(1), p(2)], vec![p(0), p(2), p(3)]];

        // The semantic detector finds (a,b), flagged non-syntactic.
        let (pairs, non_syntactic) = semantic_symmetry_pairs(4, &f);
        assert!(pairs.contains(&(0, 1)) && non_syntactic, "a,b are a semantic, non-syntactic symmetry: {pairs:?}");

        // Confirm it really is non-syntactic: swapping a,b changes the clause set.
        let canon_set = |swap: bool| -> std::collections::HashSet<Vec<(u32, bool)>> {
            f.iter()
                .map(|c| {
                    let mut k: Vec<(u32, bool)> = c
                        .iter()
                        .map(|l| {
                            let v = l.var() as usize;
                            let nv = if swap && v == 0 { 1 } else if swap && v == 1 { 0 } else { v };
                            (nv as u32, l.is_positive())
                        })
                        .collect();
                    k.sort_unstable();
                    k
                })
                .collect()
        };
        assert_ne!(canon_set(true), canon_set(false), "the a↔b swap changes the clause set — not syntactic");

        // It is broken and solved correctly.
        let s = semantic_symmetry_solve(4, &f).expect("a semantic symmetry is detected and broken");
        assert_eq!(s.via, Route::SemanticSymmetry);
        let brute = (0u64..16).any(|x| {
            let a: Vec<bool> = (0..4).map(|i| (x >> i) & 1 == 1).collect();
            f.iter().all(|c| c.iter().any(|l| a[l.var() as usize] == l.is_positive()))
        });
        assert_eq!(matches!(s.answer, Answer::Sat(_)), brute, "semantic-symmetry verdict matches brute force");
        if let Answer::Sat(m) = &s.answer {
            assert!(f.iter().all(|c| c.iter().any(|l| m[l.var() as usize] == l.is_positive())), "valid model");
        }

        // Declines when the only symmetry is already syntactic: (a∨x)∧(b∨x), a↔b preserves the clause set,
        // so symmetry_break_solve already covers it and the semantic route bows out.
        let syntactic = vec![vec![p(0), p(2)], vec![p(1), p(2)]];
        assert!(
            semantic_symmetry_solve(3, &syntactic).is_none(),
            "a purely syntactic symmetry is left to the syntactic route"
        );
    }

    #[test]
    fn nested_block_tower_breaks_multidimensional_symmetry() {
        let p = |v| Lit::new(v, true);
        let brute = |nv: usize, cl: &[Vec<Lit>]| -> bool {
            (0u64..(1u64 << nv)).any(|x| {
                let a: Vec<bool> = (0..nv).map(|i| (x >> i) & 1 == 1).collect();
                cl.iter().all(|c| c.iter().any(|l| a[l.var() as usize] == l.is_positive()))
            })
        };

        // One block level (2-D), SAT: clique_coloring(3,3) — vertex × colour, phase-free.
        let (sat, _) = crate::families::clique_coloring(3, 3);
        let s = nested_symmetry_solve(sat.num_vars, &sat.clauses).expect("a grid symmetry has a block system");
        assert_eq!(s.via, Route::NestedSymmetry);
        match &s.answer {
            Answer::Sat(m) => assert!(sat.clauses.iter().all(|c| c.iter().any(|l| m[l.var() as usize] == l.is_positive()))),
            Answer::Unsat => panic!("clique_coloring(3,3) is SAT"),
        }
        assert_eq!(matches!(s.answer, Answer::Sat(_)), brute(sat.num_vars, &sat.clauses), "verdict matches brute");

        // One block level, UNSAT: clique_coloring(4,3) — K₄ is not 3-colourable.
        let (unsat, _) = crate::families::clique_coloring(4, 3);
        let u = nested_symmetry_solve(unsat.num_vars, &unsat.clauses).expect("a grid symmetry has a block system");
        assert_eq!(u.via, Route::NestedSymmetry);
        assert!(matches!(u.answer, Answer::Unsat), "clique_coloring(4,3) is UNSAT");
        assert_eq!(matches!(u.answer, Answer::Sat(_)), brute(unsat.num_vars, &unsat.clauses), "verdict matches brute");

        // MULTI-LEVEL TOWER: the cube graph Q₃ (8 vertices, "cover every edge" = xᵤ ∨ xᵥ per edge). It is
        // vertex-transitive, phase-free, and imprimitive with NESTED blocks (antipodal pairs ⊂ … ⊂ whole),
        // so the tower ascends past the single block system a 2-D break would stop at. SAT, broken soundly.
        let mut cube: Vec<Vec<Lit>> = Vec::new();
        for v in 0u32..8 {
            for b in 0..3 {
                let w = v ^ (1 << b);
                if v < w {
                    cube.push(vec![p(v), p(w)]);
                }
            }
        }
        let c = nested_symmetry_solve(8, &cube).expect("the cube's nested block tower engages");
        assert_eq!(c.via, Route::NestedSymmetry);
        match &c.answer {
            Answer::Sat(m) => assert!(cube.iter().all(|cl| cl.iter().any(|l| m[l.var() as usize] == l.is_positive()))),
            Answer::Unsat => panic!("covering every cube edge is SAT (e.g. all true)"),
        }
        assert_eq!(matches!(c.answer, Answer::Sat(_)), brute(8, &cube), "the nested-tower verdict matches brute force");
    }

    #[test]
    fn symmetry_revealed_by_simplification_is_unlocked_and_broken() {
        let p = |v| Lit::new(v, true);
        let n = |v| Lit::new(v, false);
        // Raw F lacks the x2↔x3 symmetry (the ¬x0 tag sits only on x2's clause); propagating x0=T strips
        // the tag, revealing (x1∨x2)∧(x1∨x3) — symmetric in x2,x3. The route detects the unlocked
        // symmetry, solves the residual with the arsenal, and re-applies x0=T.
        let unlock = vec![vec![p(0)], vec![p(1), p(2), n(0)], vec![p(1), p(3)]];
        let s = symmetry_via_simplification_solve(4, &unlock).expect("simplification unlocks symmetry");
        assert_eq!(s.via, Route::SymmetrySimplify);
        match &s.answer {
            Answer::Sat(m) => {
                assert!(unlock.iter().all(|c| c.iter().any(|l| m[l.var() as usize] == l.is_positive())));
                assert!(m[0], "the forced literal x0 is re-applied to the model");
            }
            Answer::Unsat => panic!("the instance is SAT"),
        }
        let brute = (0u64..16).any(|x| {
            let a: Vec<bool> = (0..4).map(|i| (x >> i) & 1 == 1).collect();
            unlock.iter().all(|c| c.iter().any(|l| a[l.var() as usize] == l.is_positive()))
        });
        assert_eq!(matches!(s.answer, Answer::Sat(_)), brute, "verdict matches brute force");

        // Declines when the symmetry is already visible on the raw formula (no unit hides it).
        let already = vec![vec![p(0)], vec![p(1), p(2)], vec![p(1), p(3)]];
        assert!(
            symmetry_via_simplification_solve(4, &already).is_none(),
            "symmetry already present on the raw formula ⟹ nothing unlocked"
        );

        // A unit-propagation conflict is reported as UNSAT.
        let conflict = vec![vec![p(0)], vec![p(1)], vec![n(0), n(1)]];
        let u = symmetry_via_simplification_solve(2, &conflict).expect("BCP reaches a conflict");
        assert_eq!(u.via, Route::SymmetrySimplify);
        assert!(matches!(u.answer, Answer::Unsat), "x0 ∧ x1 ∧ ¬(x0∧x1) is UNSAT");
    }

    #[test]
    fn symmetric_binary_inference_learns_implication_orbits() {
        let p = |v| Lit::new(v, true);
        let n = |v| Lit::new(v, false);
        // Chain xᵢ → a → y with x0,x1,x2 interchangeable (S₃; a=var3, y=var4). BCP under x0 derives the
        // TRANSITIVE x0→y (not a clause), and symmetry adds x1→y and x2→y from that single probe.
        let chain = vec![
            vec![n(0), p(3)], vec![n(1), p(3)], vec![n(2), p(3)], // xᵢ → a
            vec![n(3), p(4)],                                     // a → y
        ];
        let s = symmetric_binary_inference_solve(5, &chain)
            .expect("a derived implication orbit is learned");
        assert_eq!(s.via, Route::SymmetricBinary);
        match &s.answer {
            Answer::Sat(m) => assert!(
                chain.iter().all(|c| c.iter().any(|l| m[l.var() as usize] == l.is_positive())),
                "the strengthened formula yields a valid model"
            ),
            Answer::Unsat => panic!("the chain is SAT"),
        }
        // Soundness vs brute force.
        let brute = (0u64..32).any(|x| {
            let a: Vec<bool> = (0..5).map(|i| (x >> i) & 1 == 1).collect();
            chain.iter().all(|c| c.iter().any(|l| a[l.var() as usize] == l.is_positive()))
        });
        assert_eq!(matches!(s.answer, Answer::Sat(_)), brute, "binary-inference verdict matches brute force");

        // Declines when BCP yields no NEW binary: here xᵢ→y are already clauses, so there is nothing to
        // learn and the route bows out (the cheap routes / CDCL decide).
        let direct = vec![vec![n(0), p(2)], vec![n(1), p(2)]]; // x0→y, x1→y directly (y=var2), S₂
        assert!(
            symmetric_binary_inference_solve(3, &direct).is_none(),
            "no new implication to learn ⟹ declines"
        );
    }

    #[test]
    fn symmetric_component_decomposition_solves_copies_once() {
        let p = |v| Lit::new(v, true);
        let n = |v| Lit::new(v, false);
        // Two interchangeable copies of (x∨y): components {0,1} and {2,3}, swapped by (0↔2, 1↔3). SAT —
        // the representative is solved once and its model is replicated through the symmetry.
        let sat = vec![vec![p(0), p(1)], vec![p(2), p(3)]];
        let s = symmetric_component_solve(4, &sat).expect("two symmetric components decompose");
        assert_eq!(s.via, Route::SymmetricComponent);
        match &s.answer {
            Answer::Sat(m) => assert!(
                sat.iter().all(|c| c.iter().any(|l| m[l.var() as usize] == l.is_positive())),
                "the assembled model is valid"
            ),
            Answer::Unsat => panic!("two copies of (x∨y) is SAT"),
        }

        // Two interchangeable copies of an UNSAT gadget x∧y∧¬(x∧y). Each copy is UNSAT, and the mixed
        // clause widths give it no phase symmetry, so the variable group (with the copy swap) is seen — so
        // F is UNSAT, detected by solving just ONE copy.
        let unsat = vec![
            vec![p(0)], vec![p(1)], vec![n(0), n(1)],
            vec![p(2)], vec![p(3)], vec![n(2), n(3)],
        ];
        let u = symmetric_component_solve(4, &unsat).expect("two symmetric components decompose");
        assert_eq!(u.via, Route::SymmetricComponent);
        assert!(matches!(u.answer, Answer::Unsat), "an UNSAT component makes F UNSAT");

        // Soundness vs brute force on both.
        for cl in [&sat, &unsat] {
            let brute = (0u64..16).any(|x| {
                let a: Vec<bool> = (0..4).map(|i| (x >> i) & 1 == 1).collect();
                cl.iter().all(|c| c.iter().any(|l| a[l.var() as usize] == l.is_positive()))
            });
            let got = symmetric_component_solve(4, cl).expect("decomposition fires");
            assert_eq!(
                matches!(got.answer, Answer::Sat(_)),
                brute,
                "component-decomposition verdict matches brute force"
            );
        }

        // A single connected component (clique_coloring(3,3)) has nothing to decompose — it declines.
        let (clique, _) = crate::families::clique_coloring(3, 3);
        assert!(
            symmetric_component_solve(clique.num_vars, &clique.clauses).is_none(),
            "a single component is not decomposable"
        );
    }

    #[test]
    fn plain_component_decomposition_solves_asymmetric_independent_parts() {
        let p = |v| Lit::new(v, true);
        let n = |v| Lit::new(v, false);
        // Heterogeneous independent components: A = (x0∨x1) on {0,1}, B = (x2 ≠ x3) on {2,3}. Their different
        // structure means no automorphism maps one onto the other, so the SYMMETRIC route declines — this is
        // exactly the case plain component decomposition exists to cover.
        let sat = vec![vec![p(0), p(1)], vec![p(2), p(3)], vec![n(2), n(3)]];
        assert!(symmetric_component_solve(4, &sat).is_none(), "asymmetric components: the symmetric route declines");
        let s = solve_by_components(4, &sat);
        assert_eq!(s.via, Route::Component);
        match &s.answer {
            Answer::Sat(m) => assert!(
                sat.iter().all(|c| c.iter().any(|l| m[l.var() as usize] == l.is_positive())),
                "the assembled model satisfies every clause"
            ),
            Answer::Unsat => panic!("both components are satisfiable, so F is SAT"),
        }

        // One component UNSAT (x2 ∧ ¬x2) ⟹ F is UNSAT, found without ever touching the other component.
        let unsat = vec![vec![p(0), p(1)], vec![p(2)], vec![n(2)]];
        let u = solve_by_components(3, &unsat);
        assert_eq!(u.via, Route::Component);
        assert!(matches!(u.answer, Answer::Unsat), "an UNSAT component makes F UNSAT");

        // A single connected component has nothing to split — component_solve declines to the fallback.
        let single = vec![vec![p(0), p(1)], vec![n(0), p(1)]];
        assert_ne!(solve_by_components(2, &single).via, Route::Component, "one component is not decomposable");

        // Soundness vs brute force on both decomposable instances.
        for (nv, cl) in [(4usize, &sat), (3, &unsat)] {
            let brute = (0u64..(1 << nv)).any(|x| {
                let a: Vec<bool> = (0..nv).map(|i| (x >> i) & 1 == 1).collect();
                cl.iter().all(|c| c.iter().any(|l| a[l.var() as usize] == l.is_positive()))
            });
            assert_eq!(matches!(solve_by_components(nv, cl).answer, Answer::Sat(_)), brute, "matches brute force");
        }
    }

    #[test]
    fn symmetry_propagation_breaks_during_search_and_is_correct() {
        // clique_coloring(3,3): SAT and variable-symmetric. The lex-leader propagator prunes non-canonical
        // assignments DURING the search (no static clauses, no aux vars) and returns a re-checked model.
        let (sat, _) = crate::families::clique_coloring(3, 3);
        let s = symmetry_propagate_solve(sat.num_vars, &sat.clauses)
            .expect("a phase-free variable symmetry drives the propagator");
        assert_eq!(s.via, Route::SymmetryPropagate);
        match &s.answer {
            Answer::Sat(m) => assert!(
                sat.clauses.iter().all(|c| c.iter().any(|l| m[l.var() as usize] == l.is_positive())),
                "the propagator returns a valid model"
            ),
            Answer::Unsat => panic!("clique_coloring(3,3) is SAT"),
        }

        // clique_coloring(4,3): UNSAT (K₄ needs 4 colours). The propagator preserves satisfiability, so the
        // dynamic break decides UNSAT.
        let (unsat, _) = crate::families::clique_coloring(4, 3);
        let u = symmetry_propagate_solve(unsat.num_vars, &unsat.clauses).expect("variable symmetry");
        assert_eq!(u.via, Route::SymmetryPropagate);
        assert!(matches!(u.answer, Answer::Unsat), "clique_coloring(4,3) is UNSAT");

        // Soundness: the dynamic verdict matches an independent brute-force decision on both.
        for (cnf, _) in [crate::families::clique_coloring(3, 3), crate::families::clique_coloring(4, 3)] {
            let nv = cnf.num_vars;
            let brute = (0u64..(1u64 << nv)).any(|x| {
                let a: Vec<bool> = (0..nv).map(|i| (x >> i) & 1 == 1).collect();
                cnf.clauses.iter().all(|c| c.iter().any(|l| a[l.var() as usize] == l.is_positive()))
            });
            let got = symmetry_propagate_solve(nv, &cnf.clauses).expect("propagator fires");
            assert_eq!(
                matches!(got.answer, Answer::Sat(_)),
                brute,
                "symmetry-propagation verdict matches brute force (nv={nv})"
            );
        }
    }

    #[test]
    fn orbit_weight_quotient_collapses_a_full_symmetric_instance() {
        let p = |v| Lit::new(v, true);
        // at-least-2 of {x0,x1,x2}: fully interchangeable (G = S₃, no phase symmetry), SAT at weight 2.
        let at_least_2 = vec![vec![p(0), p(1)], vec![p(0), p(2)], vec![p(1), p(2)]];
        let s = orbit_weight_quotient_solve(3, &at_least_2)
            .expect("a full symmetric group collapses to weight classes");
        assert_eq!(s.via, Route::OrbitWeightQuotient);
        match &s.answer {
            Answer::Sat(m) => {
                assert!(at_least_2.iter().all(|c| c.iter().any(|l| m[l.var() as usize] == l.is_positive())));
                assert!(m.iter().filter(|&&b| b).count() >= 2, "the witness has weight ≥ 2");
            }
            Answer::Unsat => panic!("at-least-2 of 3 is SAT"),
        }

        // at-least-3 ∧ at-most-1 over {x0..x3}: S₄-symmetric, and UNSAT (≥3 and ≤1 true cannot both hold).
        // The clause widths differ (ternary at-least-3, binary at-most-1), so global complement does NOT
        // preserve the set — no phase symmetry — and the quotient sees the full S₄ variable group. It
        // checks weights 0..4 and finds none satisfying — an exact refutation.
        let subsets3 = [[0u32, 1, 2], [0, 1, 3], [0, 2, 3], [1, 2, 3]];
        let pairs = [[0u32, 1], [0, 2], [0, 3], [1, 2], [1, 3], [2, 3]];
        let mut card: Vec<Vec<Lit>> = Vec::new();
        for sub in subsets3 {
            card.push(sub.iter().map(|&v| Lit::new(v, true)).collect()); // at-least-3
        }
        for pr in pairs {
            card.push(pr.iter().map(|&v| Lit::new(v, false)).collect()); // at-most-1
        }
        let u = orbit_weight_quotient_solve(4, &card).expect("S₄ is the full symmetric group on the orbit");
        assert_eq!(u.via, Route::OrbitWeightQuotient);
        assert!(matches!(u.answer, Answer::Unsat), "≥3 and ≤1 true is unsatisfiable");

        // Soundness vs brute force on both.
        for (nv, cl) in [(3usize, &at_least_2), (4, &card)] {
            let brute = (0u64..(1u64 << nv)).any(|x| {
                let a: Vec<bool> = (0..nv).map(|i| (x >> i) & 1 == 1).collect();
                cl.iter().all(|c| c.iter().any(|l| a[l.var() as usize] == l.is_positive()))
            });
            let got = orbit_weight_quotient_solve(nv, cl).expect("quotient fires");
            assert_eq!(
                matches!(got.answer, Answer::Sat(_)),
                brute,
                "orbit-weight-quotient verdict matches brute force (nv={nv})"
            );
        }

        // The gate DECLINES when the group is not the FULL product: clique_coloring(3,3)'s symmetry is
        // S₃(vertices)×S₃(colours) on the 3×3 grid — not the full S₉ on the cells — so a pure cell-swap is
        // not in the group and the quotient correctly bows out (the breaking routes handle it instead).
        let (clique, _) = crate::families::clique_coloring(3, 3);
        assert!(
            orbit_weight_quotient_solve(clique.num_vars, &clique.clauses).is_none(),
            "a product symmetry that is not the full symmetric group is not a weight quotient"
        );
    }

    #[test]
    fn symmetric_probe_infers_a_whole_orbit_from_one_failed_literal() {
        let y = |b| Lit::new(3, b);
        let nx = |v| Lit::new(v, false);
        // x0,x1,x2 are interchangeable (S₃) and each is a FAILED literal: xᵢ=true forces y ∧ ¬y.
        let base: Vec<Vec<Lit>> =
            (0u32..3).flat_map(|v| [vec![nx(v), y(true)], vec![nx(v), y(false)]]).collect();

        // SAT — all xᵢ false (y free). A single probe of x0 fails, and symmetry forces ¬x0,¬x1,¬x2 from
        // that one probe (the inference, not a search over the three).
        let s = symmetric_probe_solve(4, &base).expect("a symmetric failed literal engages the probe");
        assert_eq!(s.via, Route::SymmetricProbe);
        match &s.answer {
            Answer::Sat(m) => {
                assert!(base.iter().all(|c| c.iter().any(|l| m[l.var() as usize] == l.is_positive())));
                assert!(!m[0] && !m[1] && !m[2], "the whole orbit was inferred false from one probe");
            }
            Answer::Unsat => panic!("the base instance is SAT"),
        }

        // UNSAT — add x0∨x1∨x2: once the orbit is forced false, the cardinality clause cannot hold.
        let mut unsat = base.clone();
        unsat.push(vec![Lit::new(0, true), Lit::new(1, true), Lit::new(2, true)]);
        let u = symmetric_probe_solve(4, &unsat).expect("the probe engages");
        assert_eq!(u.via, Route::SymmetricProbe);
        assert!(matches!(u.answer, Answer::Unsat), "forcing the orbit false refutes the cardinality clause");

        // Soundness: the verdict matches an independent brute-force decision on both.
        for cl in [&base, &unsat] {
            let brute = (0u64..16).any(|x| {
                let a: Vec<bool> = (0..4).map(|i| (x >> i) & 1 == 1).collect();
                cl.iter().all(|c| c.iter().any(|l| a[l.var() as usize] == l.is_positive()))
            });
            let got = symmetric_probe_solve(4, cl).expect("probe fires");
            assert_eq!(
                matches!(got.answer, Answer::Sat(_)),
                brute,
                "symmetric-probe verdict matches brute force"
            );
        }
    }

    #[test]
    fn local_symmetry_solve_exploits_branch_symmetry_and_is_correct() {
        // Globally asymmetric (the clause x0∨x1 singles out x1), but branching on x0 reveals an x1↔x2
        // symmetry in the residual. The local-symmetry route splits on x0, breaks each residual's
        // symmetry, and decides it correctly with a re-checked model.
        let f = vec![
            vec![Lit::new(0, false), Lit::new(1, true), Lit::new(2, true)], // ¬x0 ∨ x1 ∨ x2
            vec![Lit::new(0, false), Lit::new(1, false), Lit::new(2, false)], // ¬x0 ∨ ¬x1 ∨ ¬x2
            vec![Lit::new(0, true), Lit::new(1, true)], // x0 ∨ x1
        ];
        let brute = (0u64..8).any(|x| {
            let a: Vec<bool> = (0..3).map(|i| (x >> i) & 1 == 1).collect();
            f.iter().all(|c| c.iter().any(|l| a[l.var() as usize] == l.is_positive()))
        });
        let solved = local_symmetry_solve(3, &f).expect("a branch reveals local symmetry");
        assert_eq!(solved.via, Route::LocalSymmetry);
        assert_eq!(matches!(solved.answer, Answer::Sat(_)), brute, "local-symmetry verdict matches brute force");
        if let Answer::Sat(m) = &solved.answer {
            assert!(
                f.iter().all(|c| c.iter().any(|l| m[l.var() as usize] == l.is_positive())),
                "the model from a local-symmetry branch satisfies F"
            );
        }
    }

    #[test]
    fn symmetry_breaking_scales_to_a_large_group_via_partial_breaking() {
        // clique_coloring(6,6): |Aut| = 6!·6! = 518400 — too large to enumerate (> the 50k cap), so the
        // symmetry route falls back to sound per-generator PARTIAL breaking and still solves it with a
        // re-checked model. (The point: symmetry breaking runs even where the group cannot be enumerated.)
        let (cnf, _) = crate::families::clique_coloring(6, 6);
        let s = symmetry_break_solve(cnf.num_vars, &cnf.clauses)
            .expect("a large phase-free symmetry group still drives partial breaking");
        assert_eq!(s.via, Route::SymmetryBreak);
        match s.answer {
            Answer::Sat(m) => assert!(
                cnf.clauses.iter().all(|c| c.iter().any(|l| m[l.var() as usize] == l.is_positive())),
                "partial breaking returns a valid model"
            ),
            Answer::Unsat => panic!("clique_coloring(6,6) is SAT"),
        }
    }

    #[test]
    fn the_composite_lift_verdict_matches_the_supplied_equations() {
        // A composite one-hot CNF has m bits per group, so a Boolean brute force (2^(edges·m)) is
        // intractable — the right oracle is the supplied ℤ/m system the family also returns: the lifted
        // CNF verdict must agree with running modm::solve directly on those equations, and with the
        // family's declared expectation. (The equation-level engine is itself brute-force-verified in
        // modm's own tests.)
        use crate::modm::{solve as modm_solve, ModmOutcome};
        for (eqs, cnf, want) in [
            crate::families::mod_p_tseitin_expander(6, 6, 7),
            crate::families::mod_p_consistent_onehot(6, 6, 7),
        ] {
            let solved = solve_structured(cnf.num_vars, &cnf.clauses);
            assert_eq!(solved.via, Route::ModM, "a composite instance must take the ℤ/m route");
            let num_gf_vars = cnf.num_vars / 6; // one ℤ/6 variable (edge) per 6 one-hot bits
            let supplied_unsat = matches!(modm_solve(&eqs, num_gf_vars, 6), Some(ModmOutcome::Unsat { .. }));
            assert_eq!(
                matches!(solved.answer, Answer::Unsat),
                supplied_unsat,
                "the lifted-CNF verdict must match modm::solve on the supplied equations"
            );
            assert_eq!(
                matches!(solved.answer, Answer::Unsat),
                matches!(want, crate::families::ExpectedVerdict::Unsat),
                "and the family's declared expectation"
            );
        }
    }

    #[test]
    fn mine_clauses_derives_an_implied_unit_via_probing() {
        // {a} ∧ {¬a ∨ b} ⇒ b is implied (a forces b); failed-literal probing must mine the unit b.
        let clauses = vec![vec![Lit::new(0, true)], vec![Lit::new(0, false), Lit::new(1, true)]];
        let mined = mine_clauses(2, &clauses);
        assert!(
            mined.iter().any(|c| c.len() == 1 && c[0].var() == 1 && c[0].is_positive()),
            "expected mined implied unit b; got {mined:?}"
        );
    }

    #[test]
    fn every_mined_clause_is_implied() {
        // Soundness: each mined clause must hold in EVERY model of the formula (brute force).
        let clauses = vec![
            xor_gadget(&[0, 1, 2], false),
            vec![vec![Lit::new(0, true)]],
            vec![vec![Lit::new(1, true)]],
        ]
        .concat();
        let n = 3;
        let mined = mine_clauses(n, &clauses);
        assert!(!mined.is_empty(), "this instance has implied structure to mine");
        for mask in 0u32..(1 << n) {
            let asg: Vec<bool> = (0..n).map(|v| (mask >> v) & 1 == 1).collect();
            let is_model = clauses.iter().all(|c| c.iter().any(|l| asg[l.var() as usize] == l.is_positive()));
            if is_model {
                for mc in &mined {
                    assert!(
                        mc.iter().any(|l| asg[l.var() as usize] == l.is_positive()),
                        "mined clause {mc:?} is not implied (fails model {asg:?})"
                    );
                }
            }
        }
    }

    /// **The exact-cover lift crushes modular counting and the chessboard — one harvest, every
    /// modulus, certified.** Exactly-one groups (an all-positive clause whose variable pairs all
    /// carry at-most-one clauses) yield the equations `Σ_{v∈g} x_v = 1`, valid consequences over
    /// EVERY modulus; Gaussian elimination over `GF(2)`, `GF(3)`, `GF(5)` then finds whatever
    /// counting obstruction exists — the parity sum for `Count_2`, the black-minus-white
    /// combination for the mutilated chessboard over `GF(3)`, the mod-3/5 sums for `Count_{3,5}` —
    /// each refutation re-checked fail-closed before the route reports. This turns the satbench
    /// CONTROL rows (`Count_2` odd-matching fell through to CDCL at ~40–60 ms) into µs crushes with
    /// zero search. Soundness: the route only ever reports UNSAT (with a re-checked certificate);
    /// satisfiable coverings decline through to the rest of the chain, and a SAT control is pinned.
    #[test]
    fn exact_cover_lift_crushes_modular_counting_and_the_chessboard() {
        // Count_2 on odd n — previously a CONTROL row (CDCL); now the GF(2) point-sum, no search.
        for n in [7usize, 9, 11] {
            let (cnf, _) = crate::families::mod_counting(n, 2);
            let s = solve_structured(cnf.num_vars, &cnf.clauses);
            assert!(matches!(s.answer, Answer::Unsat), "Count_2({n}) is UNSAT");
            assert_eq!(s.via, Route::ExactCover, "Count_2({n}): the exact-cover lift fires");
            assert_eq!(s.conflicts, 0, "Count_2({n}): zero search");
        }
        // Count_3 and Count_5 — the same harvest, higher rungs of the modulus ladder.
        for (n, q) in [(7usize, 3usize), (8, 3), (6, 5), (7, 5)] {
            let (cnf, _) = crate::families::mod_counting(n, q);
            let s = solve_structured(cnf.num_vars, &cnf.clauses);
            assert!(matches!(s.answer, Answer::Unsat), "Count_{q}({n}) is UNSAT");
            assert_eq!(s.via, Route::ExactCover, "Count_{q}({n}): the exact-cover lift fires");
        }
        // The mutilated chessboard: the ±1 (black/white) combination lives in GF(3) — Gaussian
        // finds it from the raw covering equations, no bipartite reasoning supplied.
        let (cb, _) = crate::families::mutilated_chessboard(4);
        let s = solve_structured(cb.num_vars, &cb.clauses);
        assert!(matches!(s.answer, Answer::Unsat), "chessboard(4) is UNSAT");
        assert_eq!(s.conflicts, 0, "chessboard(4): a specialist decides — zero search");
        assert_ne!(s.via, Route::Cdcl, "chessboard(4): never the fallback");
        // SAT control: a satisfiable covering never yields a false UNSAT from the lift.
        let (sat6, _) = crate::families::mod_counting(6, 3);
        let s = solve_structured(sat6.num_vars, &sat6.clauses);
        match s.answer {
            Answer::Sat(model) => {
                assert!(
                    sat6.clauses.iter().all(|c| c.iter().any(|l| model[l.var() as usize] == l.is_positive())),
                    "Count_3(6): the SAT model re-checks"
                );
            }
            Answer::Unsat => panic!("Count_3(6) is satisfiable — the lift must decline"),
        }
    }

    /// **Measurement: the symmetry arsenal against Ramsey.** Ramsey formulas carry the huge `Sₙ`
    /// vertex symmetry induced on edge variables — clique geometry, no counting or parity to lift,
    /// so the exact-cover and algebraic routes rightly decline and the question is whether the
    /// symmetry routes (detection + orbital branching / SEL / lex-leader, `solve_comprehensive`)
    /// beat the raw CDCL fallback the fast chain uses. Wall time, route, and conflicts per engine,
    /// on `ramsey(3,3;6)` and `ramsey(3,4;9)` — the measured groundwork for the crushable frontier
    /// (`ramsey(3,5;14)`, where plain CDCL walls). Verdict correctness asserted; the comparison is
    /// reported, not presumed.
    #[test]
    #[ignore = "scale measurement — symmetry-arsenal wall time on Ramsey; run explicitly"]
    fn ramsey_symmetric_attack_is_measured() {
        for (s, t, n) in [(3usize, 3usize, 6usize), (3, 4, 9)] {
            let (cnf, _) = crate::families::ramsey(s, t, n);
            let t0 = std::time::Instant::now();
            let fast = solve_structured(cnf.num_vars, &cnf.clauses);
            let fast_ms = t0.elapsed().as_millis();
            assert!(matches!(fast.answer, Answer::Unsat), "ramsey({s},{t};{n}) is UNSAT");
            let t1 = std::time::Instant::now();
            let full = solve_comprehensive(cnf.num_vars, &cnf.clauses);
            let full_ms = t1.elapsed().as_millis();
            assert!(matches!(full.answer, Answer::Unsat), "ramsey({s},{t};{n}) is UNSAT (arsenal)");
            eprintln!(
                "RAMSEY | ({s},{t};{n}) [{} vars]: fast={:?} {}ms {} conflicts | arsenal={:?} {}ms {} conflicts",
                cnf.num_vars, fast.via, fast_ms, fast.conflicts, full.via, full_ms, full.conflicts
            );
        }
    }
}
