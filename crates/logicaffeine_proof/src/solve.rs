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
use crate::xor_engine::{IncXor, WatchedXor};
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

/// The verdict; a model on SAT, with any UNSAT proof the route carries in [`Solved::proof`].
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
    /// The UNSAT proof the route carries at no extra cost: the CDCL route's preprocessing steps and learned
    /// clauses, the hybrid-XOR route's learned clauses (over the formula and the short clauses Gaussian
    /// elimination implies, so not always checkable against the formula alone), 2-SAT's two RUP units, the covering collapse's and the symmetry routes' PR streams. Empty for a specialist whose verdict
    /// rests on its own certificate (see [`certified_proof`] for which are re-checked). [`certified_proof`] gives a
    /// proof checked against the formula alone.
    pub proof: Vec<ProofStep>,
    /// CDCL conflicts spent (0 whenever a specialist collapsed the instance without search).
    pub conflicts: u64,
}

impl Solved {
    fn unsat(via: Route) -> Self {
        Solved {
            answer: Answer::Unsat,
            via,
            proof: Vec::new(),
            conflicts: 0,
        }
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
    // 1. 2-SAT: a complete polynomial decision procedure. The model is re-checked; the refutation — `x` and `¬x`
    //    in one strongly connected component — is re-derived by reachability and carried as the RUP steps `(¬x)`,
    //    `(x)`: each implication chain is unit propagation. A certificate that does not re-check declines onward.
    if let Some(binary) = as_two_sat(clauses) {
        match crate::twosat::solve(&binary, num_vars) {
            crate::twosat::TwoSatOutcome::Sat(model) if crate::twosat::satisfies(&binary, &model) => {
                return Some(Solved { answer: Answer::Sat(model), via: Route::TwoSat, proof: Vec::new(), conflicts: 0 });
            }
            crate::twosat::TwoSatOutcome::Unsat(x) if crate::twosat::is_refutation(&binary, num_vars, x) => {
                let x = x as u32;
                return Some(Solved {
                    answer: Answer::Unsat,
                    via: Route::TwoSat,
                    proof: vec![ProofStep::Rup(vec![Lit::neg(x)]), ProofStep::Rup(vec![Lit::pos(x)])],
                    conflicts: 0,
                });
            }
            _ => {}
        }
    }

    // 2. Horn: the least model by forward chaining, re-checked, or the refutation, re-derived. A Horn formula with
    //    no model is refuted by unit propagation alone, so the empty stream is its proof.
    if let Some(horn) = as_horn(clauses) {
        match crate::hornsat::solve(&horn, num_vars) {
            crate::hornsat::HornOutcome::Sat(model) if crate::hornsat::satisfies(&horn, &model) => {
                return Some(Solved { answer: Answer::Sat(model), via: Route::Horn, proof: Vec::new(), conflicts: 0 });
            }
            crate::hornsat::HornOutcome::Unsat(r) if crate::hornsat::is_refutation(&horn, num_vars, &r) => {
                return Some(Solved::unsat(Route::Horn));
            }
            _ => {}
        }
    }

    // 3. LLL: satisfiability from sparsity. If the local-lemma condition holds a model is guaranteed;
    //    construct it with Moser–Tardos. The check is O(clauses) and an empty clause fails it, so this
    //    only fires on genuinely sparse (SAT) instances and never claims SAT without a re-checked model.
    if !clauses.is_empty() && crate::lll::lll_certifies_sat(clauses).is_some() {
        let budget = 1000 + 64 * clauses.len();
        if let Some(model) =
            crate::lll::moser_tardos_witness(num_vars, clauses, 0x10C0_5EED_C0DE_F00D, budget)
        {
            // Fail-closed: only accept a witness that genuinely satisfies every clause.
            if clauses.iter().all(|c| {
                c.iter().any(|l| {
                    model.get(l.var() as usize).copied().unwrap_or(false) == l.is_positive()
                })
            }) {
                return Some(Solved {
                    answer: Answer::Sat(model),
                    via: Route::Lll,
                    proof: Vec::new(),
                    conflicts: 0,
                });
            }
        }
    }

    // 4-6. Polynomial UNSAT recognizers. Matching and cutting planes read a covering over the formula's ProofExpr
    //      view, built only when the formula is one; parity reads the clauses.
    if clauses.iter().all(|c| !c.is_empty()) {
        if covering_shaped(clauses) {
            if let Some(expr) = cnf_to_expr(clauses) {
                if crate::pigeonhole::decide_pigeonhole_unsat(&expr) {
                    return Some(Solved::unsat(Route::Pigeonhole));
                }
                if crate::pseudo_boolean::refute_clausal(&expr) {
                    return Some(Solved::unsat(Route::CuttingPlanes));
                }
            }
        }
        if crate::xorsat::refute_via_parity_clauses(num_vars, clauses) {
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
    // A covering collapse carries its PR refutation, already checked; the cardinality and parity collapses rest on
    // the descent reaching its goal.
    match auto_collapse(num_vars, clauses) {
        AutoCollapse::None => {}
        AutoCollapse::Geometric { ranked, .. } => {
            return Some(Solved { answer: Answer::Unsat, via: Route::Collapse, proof: ranked.steps, conflicts: 0 });
        }
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
    if num_vars <= 6
        && crate::sos::sos_certificate(num_vars, clauses)
            .is_some_and(|cert| crate::sos::check_sos_certificate(num_vars, clauses, &cert))
    {
        return Some(Solved::unsat(Route::Sos));
    }

    // 7¾. Bounded-degree **Nullstellensatz (degree ≤ 3)** — the algebraic obstruction *beyond* SoS's
    //      degree-2. The residue-map census showed the symmetric-but-uncrushed families sit at
    //      Nullstellensatz degree 3: their symmetry is *already fully broken* (0 underbroken), so no
    //      symmetry route can touch them — the only remaining lens is the deeper algebraic one. Small-core
    //      gated (num_vars ≤ 12) so the fast router stays bounded; sound (a genuine degree-d NS refutation).
    //      The verdict is re-checked by the full-degree partition-of-unity certificate, cheap at this size.
    if num_vars <= 12 {
        for d in 2..=num_vars.min(3) {
            if crate::polycalc::nullstellensatz_refutes(num_vars, clauses, d) {
                if matches!(crate::polycalc::build_ns_certificate(num_vars, clauses), Ok(cert) if cert.verify(clauses)) {
                    return Some(Solved::unsat(Route::Nullstellensatz));
                }
                break;
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
        let full_amo = (0..g.len()).all(|i| (i + 1..g.len()).all(|j| amo.contains(&(g[i], g[j]))));
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
        .map(|g| {
            crate::xorsat::XorEquation::new(g.iter().map(|&v| v as usize).collect::<Vec<_>>(), true)
        })
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

/// The clauses the CDCL route's preprocessing lets an elimination add beyond those it removes. On an exact cover, each
/// variable in four exactly-one constraints, no variable eliminates at bound 0 and a sixth of them at 4. On the SAT
/// Competition 2024 sample at 60 s the route decides 57 of 62 formulas at bound 4 against 47 at 0 (PAR-2 1049 against
/// 2320), among them all ten of its tilings, nine of which bound 0 does not decide; on the SATLIB and structured arena
/// it decides the same 108 of 133.
const ROUTE_ELIMINATION_BOUND: usize = 4;

/// The CDCL route: the formula preprocessed ([`crate::preprocess`]: level-0 units, congruence closure, equivalent
/// literals, subsumption, bounded variable addition and elimination), then the CDCL core on the simplified formula,
/// over its variables and those bounded variable addition introduced. Every clause the simplified formula keeps or
/// derives follows from the formula or, over an introduced variable, is redundant for it, so the formula and the
/// simplified one are satisfiable together, and a model of the simplified one extends back to a model of the
/// formula. The proof is the preprocessing steps (RUP and PR additions, deletions) followed by the search's learned
/// clauses. An extended model is
/// checked against the formula; one that failed would be a preprocessing defect, and the formula is then decided by
/// the core on the formula itself ([`cdcl_on_the_formula`]). An elimination may add up to [`ROUTE_ELIMINATION_BOUND`]
/// clauses beyond those it removes.
fn cdcl_fallback(num_vars: usize, clauses: &[Vec<Lit>]) -> Solved {
    let options = crate::preprocess::Options { factor: true, bound: ROUTE_ELIMINATION_BOUND, ..Default::default() };
    let pre = crate::preprocess::preprocess(num_vars, clauses, &options);
    if pre.unsat {
        return Solved { answer: Answer::Unsat, via: Route::Cdcl, proof: pre.steps, conflicts: 0 };
    }
    let mut solver = Solver::new(pre.num_vars);
    for c in &pre.clauses {
        solver.add_clause(c.clone());
    }
    match solver.solve() {
        SolveResult::Sat(mut model) => {
            pre.extend(&mut model);
            model.truncate(num_vars);
            if clauses.iter().all(|c| c.iter().any(|l| model[l.var() as usize] == l.is_positive())) {
                return Solved { answer: Answer::Sat(model), via: Route::Cdcl, proof: Vec::new(), conflicts: solver.conflicts() };
            }
            debug_assert!(false, "a model of the preprocessed formula failed to extend to the formula");
            cdcl_on_the_formula(num_vars, clauses)
        }
        SolveResult::Unsat => {
            let mut proof = pre.steps;
            proof.extend(solver.learned().iter().map(|lc| ProofStep::Rup(lc.lits.clone())));
            Solved { answer: Answer::Unsat, via: Route::Cdcl, proof, conflicts: solver.conflicts() }
        }
    }
}

/// The CDCL core on the formula itself, with no preprocessing.
fn cdcl_on_the_formula(num_vars: usize, clauses: &[Vec<Lit>]) -> Solved {
    let mut solver = Solver::new(num_vars);
    for c in clauses {
        solver.add_clause(c.clone());
    }
    match solver.solve() {
        SolveResult::Sat(model) => Solved {
            answer: Answer::Sat(model),
            via: Route::Cdcl,
            proof: Vec::new(),
            conflicts: solver.conflicts(),
        },
        SolveResult::Unsat => {
            let proof = solver
                .learned()
                .iter()
                .map(|lc| ProofStep::Rup(lc.lits.clone()))
                .collect();
            Solved {
                answer: Answer::Unsat,
                via: Route::Cdcl,
                proof,
                conflicts: solver.conflicts(),
            }
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
    if let crate::inprocess::EquivResult::Unsat(steps) =
        crate::inprocess::equivalent_literal_scc(num_vars, clauses)
    {
        return Solved {
            answer: Answer::Unsat,
            via: Route::EquivLit,
            proof: steps,
            conflicts: 0,
        };
    }
    let (reduced, steps) = crate::inprocess::bve(num_vars, clauses);
    if reduced.iter().any(|c| c.is_empty()) {
        return Solved {
            answer: Answer::Unsat,
            via: Route::BoundedVarElim,
            proof: steps,
            conflicts: 0,
        };
    }
    // Certified bucket elimination (tree-width): full Davis–Putnam in min-degree order, capping resolvent width.
    // A refutation within the cap is a `2^cap·n` resolution certificate — crushes bounded-treewidth families
    // BVE's non-growing rule misses. Declines on the high-treewidth residue (width exceeds the cap).
    if let Some(steps) = crate::inprocess::bucket_elimination_refute(num_vars, clauses, 12) {
        return Solved {
            answer: Answer::Unsat,
            via: Route::TreeWidth,
            proof: steps,
            conflicts: 0,
        };
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
            let proof = drat
                .map(|res| res.into_iter().map(ProofStep::Rup).collect())
                .unwrap_or_default();
            return Solved {
                answer: Answer::Unsat,
                via: Route::Parity,
                proof,
                conflicts: 0,
            };
        }
        crate::affine::AffineCanon::Canonical(canon) => {
            // The canonical RREF break: solve the formula reduced to its free generators, then lift the
            // model back through the affine quotient. UNSAT of the reduced ⇒ UNSAT of the original.
            let sub = cdcl_fallback(canon.num_vars, &canon.clauses);
            return match sub.answer {
                Answer::Unsat => Solved {
                    answer: Answer::Unsat,
                    via: sub.via,
                    proof: Vec::new(),
                    conflicts: sub.conflicts,
                },
                Answer::Sat(model) => {
                    let lifted = canon.lift(&model);
                    // Fail-closed: the lifted model must satisfy the original formula.
                    if clauses.iter().all(|c| {
                        c.iter()
                            .any(|l| lifted[l.var() as usize] == l.is_positive())
                    }) {
                        Solved {
                            answer: Answer::Sat(lifted),
                            via: sub.via,
                            proof: Vec::new(),
                            conflicts: sub.conflicts,
                        }
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
            let proof = drat
                .map(|res| res.into_iter().map(ProofStep::Rup).collect())
                .unwrap_or_default();
            return Solved {
                answer: Answer::Unsat,
                via: Route::ModP,
                proof,
                conflicts: 0,
            };
        }
        crate::affine_gfp::AffinePCanon::Canonical(canon) => {
            let sub = cdcl_fallback(canon.num_vars, &canon.clauses);
            return match sub.answer {
                Answer::Unsat => Solved {
                    answer: Answer::Unsat,
                    via: sub.via,
                    proof: Vec::new(),
                    conflicts: sub.conflicts,
                },
                Answer::Sat(model) => {
                    let lifted = canon.lift(&model);
                    if clauses.iter().all(|c| {
                        c.iter()
                            .any(|l| lifted[l.var() as usize] == l.is_positive())
                    }) {
                        Solved {
                            answer: Answer::Sat(lifted),
                            via: sub.via,
                            proof: Vec::new(),
                            conflicts: sub.conflicts,
                        }
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
            let proof = drat
                .map(|res| res.into_iter().map(ProofStep::Rup).collect())
                .unwrap_or_default();
            return Solved {
                answer: Answer::Unsat,
                via: Route::ModM,
                proof,
                conflicts: 0,
            };
        }
        crate::affine_gfp::AffinePCanon::Canonical(canon) => {
            let sub = cdcl_fallback(canon.num_vars, &canon.clauses);
            return match sub.answer {
                Answer::Unsat => Solved {
                    answer: Answer::Unsat,
                    via: sub.via,
                    proof: Vec::new(),
                    conflicts: sub.conflicts,
                },
                Answer::Sat(model) => {
                    let lifted = canon.lift(&model);
                    if clauses.iter().all(|c| {
                        c.iter()
                            .any(|l| lifted[l.var() as usize] == l.is_positive())
                    }) {
                        Solved {
                            answer: Answer::Sat(lifted),
                            via: sub.via,
                            proof: Vec::new(),
                            conflicts: sub.conflicts,
                        }
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
                .all(|c| {
                    c.iter()
                        .any(|l| projected[l.var() as usize] == l.is_positive())
                })
                .then_some(Solved {
                    answer: Answer::Sat(projected),
                    via: Route::RecursiveBreak,
                    proof: Vec::new(),
                    conflicts: solver.conflicts(),
                })
        }
        SolveResult::Unsat => Some(Solved {
            answer: Answer::Unsat,
            via: Route::RecursiveBreak,
            proof: Vec::new(),
            conflicts: solver.conflicts(),
        }),
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
    Some(Solved {
        answer: Answer::Unsat,
        via: Route::LocalSymmetry,
        proof: Vec::new(),
        conflicts,
    })
}

/// Dynamic in-CDCL symmetry breaking — Symmetric Explanation Learning ([`crate::sym_dynamic::sel_refute`]).
/// During a budgeted CDCL search, each learned clause is multiplied by the formula's symmetry group, so
/// symmetric conflicts are learned for free and symmetric subtrees are never re-explored. Sound:
/// UNSAT carries a checked PR/RUP proof, SAT a model, else an honest Unknown. Gated by a quick symmetry
/// check (skip if the formula is asymmetric — SEL would just burn budget) and `num_vars ≤ 64`.
fn dynamic_sel(num_vars: usize, clauses: &[Vec<Lit>]) -> Option<Solved> {
    use crate::sym_dynamic::{sel_refute, SelOutcome};
    if num_vars > 64
        || crate::sym_break::literal_automorphism_generators(num_vars, clauses).is_empty()
    {
        return None;
    }
    match sel_refute(num_vars, clauses) {
        SelOutcome::Unsat {
            steps, conflicts, ..
        } => Some(Solved {
            answer: Answer::Unsat,
            via: Route::Sel,
            proof: steps,
            conflicts,
        }),
        SelOutcome::Sat(model) => Some(Solved {
            answer: Answer::Sat(model),
            via: Route::Sel,
            proof: Vec::new(),
            conflicts: 0,
        }),
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
    let point_gens: Vec<_> = gens
        .iter()
        .map(|s| crate::sym_break::litsym_to_points(s, num_vars))
        .collect();
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
            &pts.iter()
                .map(|p| crate::sym_break::litsym_from_points(p, num_vars))
                .collect::<Vec<_>>(),
        ),
        None => crate::sym_break::hierarchical_break(num_vars, clauses).unwrap_or_else(|| {
            let mut s = gens;
            s.extend(
                bsgs.transversal_elements()
                    .iter()
                    .map(|p| crate::sym_break::litsym_from_points(p, num_vars)),
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
                .all(|c| {
                    c.iter()
                        .any(|l| projected[l.var() as usize] == l.is_positive())
                })
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
    if !crate::permgroup::orbits(num_vars, &gens)
        .iter()
        .any(|o| o.len() >= 3)
    {
        return None;
    }
    let mut conflicts = 0u64;
    let mut budget = 1024u64; // node cap; on exhaustion a node finishes its residual with plain CDCL
    let answer = orbital_node(
        num_vars,
        clauses,
        &gens,
        &mut Vec::new(),
        &mut budget,
        &mut conflicts,
    )?;
    Some(Solved {
        answer,
        via: Route::OrbitalBranch,
        proof: Vec::new(),
        conflicts,
    })
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
        gens.iter()
            .filter(|g| committed_vars.iter().all(|&v| g[v] == v))
            .cloned()
            .collect()
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
    let point_gens: Vec<_> = gens
        .iter()
        .map(|s| crate::sym_break::litsym_to_points(s, num_vars))
        .collect();
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
                .all(|c| {
                    c.iter()
                        .any(|l| projected[l.var() as usize] == l.is_positive())
                })
                .then_some(Solved {
                    answer: Answer::Sat(projected),
                    via: Route::SymmetricProbe,
                    proof: Vec::new(),
                    conflicts,
                })
        }
        SolveResult::Unsat => Some(Solved {
            answer: Answer::Unsat,
            via: Route::SymmetricProbe,
            proof: Vec::new(),
            conflicts,
        }),
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
    let mut seen: HashSet<[(u32, bool); 2]> = clauses
        .iter()
        .filter(|c| c.len() == 2)
        .map(|c| key2(c[0], c[1]))
        .collect();

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
                .all(|c| {
                    c.iter()
                        .any(|l| projected[l.var() as usize] == l.is_positive())
                })
                .then_some(Solved {
                    answer: Answer::Sat(projected),
                    via: Route::SymmetricBinary,
                    proof: Vec::new(),
                    conflicts,
                })
        }
        SolveResult::Unsat => Some(Solved {
            answer: Answer::Unsat,
            via: Route::SymmetricBinary,
            proof: Vec::new(),
            conflicts,
        }),
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
        clauses
            .iter()
            .all(|c| c.iter().any(|l| a[l.var() as usize] == l.is_positive()))
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
    let mut theories: Vec<Box<dyn crate::cdcl::Theory>> = vec![Box::new(LexLeaderTheory {
        num_vars,
        generators: gens,
    })];
    match solver.solve_with(&mut theories) {
        SolveResult::Sat(model) => {
            let projected: Vec<bool> = model[..num_vars].to_vec();
            clauses
                .iter()
                .all(|c| {
                    c.iter()
                        .any(|l| projected[l.var() as usize] == l.is_positive())
                })
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
    let roots: Vec<usize> = (0..num_vars)
        .filter(|&r| !comp_clauses[r].is_empty())
        .collect();
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
                return Some(Solved {
                    answer: Answer::Unsat,
                    via: Route::Component,
                    proof: Vec::new(),
                    conflicts,
                });
            }
            Answer::Sat(m) => {
                let vars: std::collections::BTreeSet<usize> = comp_clauses[r]
                    .iter()
                    .flatten()
                    .map(|l| l.var() as usize)
                    .collect();
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
        .then_some(Solved {
            answer: Answer::Sat(model),
            via: Route::Component,
            proof: Vec::new(),
            conflicts,
        })
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
    let roots: Vec<usize> = (0..num_vars)
        .filter(|&r| !comp_vars[r].is_empty())
        .collect();
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
        .then_some(Solved {
            answer: Answer::Sat(model),
            via: Route::SymmetricComponent,
            proof: Vec::new(),
            conflicts,
        })
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
    let forced: Vec<Lit> = (0..num_vars)
        .filter_map(|v| val[v].map(|b| Lit::new(v as u32, b)))
        .collect();
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
    let res_gens =
        crate::sym_break::variable_automorphism_generators(num_vars, &detect).unwrap_or_default();
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
            let mut img: Vec<(u32, bool)> = c
                .iter()
                .map(|l| (g[l.var() as usize] as u32, l.is_positive()))
                .collect();
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
fn nested_block_tower_break(
    num_vars: usize,
    clauses: &[Vec<Lit>],
) -> Option<(Vec<Vec<Lit>>, usize)> {
    let gens = crate::sym_break::variable_automorphism_generators(num_vars, clauses)?;
    if gens.is_empty() {
        return None;
    }
    let bsgs = crate::permgroup::schreier_sims(num_vars, &gens);
    let to_litsym =
        |p: &[usize]| -> Vec<Lit> { (0..num_vars).map(|v| Lit::pos(p[v] as u32)).collect() };

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
                .all(|c| {
                    c.iter()
                        .any(|l| projected[l.var() as usize] == l.is_positive())
                })
                .then_some(Solved {
                    answer: Answer::Sat(projected),
                    via: Route::NestedSymmetry,
                    proof: Vec::new(),
                    conflicts: solver.conflicts(),
                })
        }
        SolveResult::Unsat => Some(Solved {
            answer: Answer::Unsat,
            via: Route::NestedSymmetry,
            proof: Vec::new(),
            conflicts: solver.conflicts(),
        }),
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
pub fn semantic_symmetry_pairs(
    num_vars: usize,
    clauses: &[Vec<Lit>],
) -> (Vec<(usize, usize)>, bool) {
    let clause_set: HashSet<Vec<(u32, bool)>> = clauses.iter().map(|c| canon_clause(c)).collect();
    let mut pairs = Vec::new();
    let mut any_non_syntactic = false;
    for a in 0..num_vars {
        for b in (a + 1)..num_vars {
            let syntactic = clauses
                .iter()
                .all(|c| clause_set.contains(&canon_clause(&swap_clause_vars(c, a, b))));
            let semantic = syntactic
                || clauses.iter().all(|c| {
                    let sc = swap_clause_vars(c, a, b);
                    clause_set.contains(&canon_clause(&sc))
                        || clause_is_implied(num_vars, clauses, &sc)
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
                .all(|c| {
                    c.iter()
                        .any(|l| projected[l.var() as usize] == l.is_positive())
                })
                .then_some(Solved {
                    answer: Answer::Sat(projected),
                    via: Route::SemanticSymmetry,
                    proof: Vec::new(),
                    conflicts: solver.conflicts(),
                })
        }
        SolveResult::Unsat => Some(Solved {
            answer: Answer::Unsat,
            via: Route::SemanticSymmetry,
            proof: Vec::new(),
            conflicts: solver.conflicts(),
        }),
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
/// few clauses is a partial symmetry: it maps a model `m` of `F` to a model *exactly when* `m` satisfies the
/// images `σ(B)` of the broken clauses. [`crate::sym_certify::conditional_symmetry_breaks`] adds the guarded
/// break `(⋀ σ(B)) → (a ≤ b)` of every such transposition, each clause a substitution-redundant PR step checked
/// against the database before it, so the breaks of several transpositions compose. A SAT model is re-checked
/// against `F`; an UNSAT verdict carries the PR breaks and the learned clauses as its proof, checked against
/// `F` alone, and falls back to a plain CDCL refutation of `F` should that check fail. `None` when no
/// transposition breaks few enough clauses.
fn almost_symmetry_solve(num_vars: usize, clauses: &[Vec<Lit>]) -> Option<Solved> {
    if num_vars == 0 || num_vars > 20 || clauses.len() > 256 {
        return None;
    }
    if almost_symmetry_pairs(num_vars, clauses, 2).is_empty() {
        return None;
    }
    let breaks = crate::sym_certify::conditional_symmetry_breaks(num_vars, clauses, 2);
    let mut solver = Solver::new(num_vars);
    for c in &breaks.db {
        solver.add_clause(c.clone());
    }
    match solver.solve() {
        SolveResult::Sat(model) => clauses
            .iter()
            .all(|c| c.iter().any(|l| model[l.var() as usize] == l.is_positive()))
            .then_some(Solved {
                answer: Answer::Sat(model),
                via: Route::AlmostSymmetry,
                proof: Vec::new(),
                conflicts: solver.conflicts(),
            }),
        SolveResult::Unsat => {
            let mut proof = breaks.steps;
            proof.extend(solver.learned().iter().map(|lc| ProofStep::Rup(lc.lits.clone())));
            if !crate::pr::check_pr_refutation(num_vars, clauses, &proof) {
                proof = crate::sdcl::plain_cdcl_refutation(num_vars, clauses);
                if !crate::pr::check_pr_refutation(num_vars, clauses, &proof) {
                    return None;
                }
            }
            Some(Solved {
                answer: Answer::Unsat,
                via: Route::AlmostSymmetry,
                proof,
                conflicts: solver.conflicts(),
            })
        }
    }
}

/// A clause with its variables permuted by `g`.
fn apply_perm_to_clause(c: &[Lit], g: &[usize]) -> Vec<Lit> {
    c.iter()
        .map(|l| Lit::new(g[l.var() as usize] as u32, l.is_positive()))
        .collect()
}

/// Whether the variable permutation `g` preserves the clause SET of `clauses` (`σ(F) = F`) — i.e. `g` is a
/// phase-free automorphism of the formula. `g` is a variable bijection, so it maps the (deduplicated)
/// clause set injectively into itself; if every image is present, it permutes the set.
fn perm_preserves_clause_set(clauses: &[Vec<Lit>], g: &[usize]) -> bool {
    let set: HashSet<Vec<(u32, bool)>> = clauses.iter().map(|c| canon_clause(c)).collect();
    clauses
        .iter()
        .all(|c| set.contains(&canon_clause(&apply_perm_to_clause(c, g))))
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
    let mut candidates =
        crate::sym_break::variable_automorphism_generators(num_vars, f).unwrap_or_default();
    candidates.extend(
        crate::sym_break::variable_automorphism_generators(num_vars, s).unwrap_or_default(),
    );
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
    let index: HashMap<Vec<(u32, bool)>, usize> = clauses
        .iter()
        .enumerate()
        .map(|(i, c)| (canon_clause(c), i))
        .collect();
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
    let mut groups: std::collections::BTreeMap<usize, Vec<usize>> =
        std::collections::BTreeMap::new();
    for i in 0..n {
        let r = find(&mut parent, i);
        groups.entry(r).or_default().push(i);
    }
    groups.into_values().collect()
}

/// A model of `clauses ∧ ¬c` — a witness that `c` is NOT entailed by `clauses` — or `None` if `clauses ⊨ c`.
fn entailment_counterexample(
    num_vars: usize,
    clauses: &[Vec<Lit>],
    c: &[Lit],
) -> Option<Vec<bool>> {
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
    (
        clause_orbits(s, &gens).len() + clause_orbits(f, &gens).len(),
        s.len() + f.len(),
    )
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
            broken.push(vec![Lit::new(v as u32, false), Lit::new(g[v] as u32, true)]);
            // ¬x_v ∨ x_{g[v]}
        }
    }
    broken
}

/// Enumerate the models of `clauses` (CDCL + blocking) and return the minimum-weight one with the number of
/// models visited: `(optimum, witness, models_enumerated)`, or `None` if unsatisfiable. A FRESH solver is
/// built each round with the accumulated blocking clauses — our CDCL core is not incrementally re-solvable
/// (the same pattern the model-counting routines use), so reusing one solver across `solve()` calls would
/// loop.
fn min_weight_model(
    num_vars: usize,
    clauses: &[Vec<Lit>],
    weights: &[i64],
) -> Option<(i64, Vec<bool>, usize)> {
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
                let w: i64 = (0..num_vars)
                    .filter(|&i| model[i])
                    .map(|i| weights[i])
                    .sum();
                if best.as_ref().map_or(true, |(bw, _)| w < *bw) {
                    best = Some((w, model[..num_vars].to_vec()));
                }
                working.push(
                    (0..num_vars)
                        .map(|i| Lit::new(i as u32, !model[i]))
                        .collect(),
                );
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
pub fn optimize_enumeration_counts(
    num_vars: usize,
    clauses: &[Vec<Lit>],
    weights: &[i64],
) -> (usize, usize) {
    let broken = optimization_break(num_vars, clauses, weights);
    let with = min_weight_model(num_vars, &broken, weights).map_or(0, |(_, _, c)| c);
    let without = min_weight_model(num_vars, clauses, weights).map_or(0, |(_, _, c)| c);
    (with, without)
}

/// The full orbit of a Boolean assignment `m` under the group generated by `gens` (action `σ(m)[g[v]] =
/// m[v]`), as a set of complete assignments — no projection, so it is exact for counting.
fn full_assignment_orbit(
    num_vars: usize,
    m: &[bool],
    gens: &[crate::permgroup::Perm],
) -> Vec<Vec<bool>> {
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
    let gens =
        crate::sym_break::variable_automorphism_generators(num_vars, clauses).unwrap_or_default();
    let weight_of = |m: &[bool]| -> i128 {
        (0..num_vars)
            .map(|v| {
                if m[v] {
                    weight[v].1 as i128
                } else {
                    weight[v].0 as i128
                }
            })
            .product()
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
    let gens =
        crate::sym_break::variable_automorphism_generators(num_vars, clauses).unwrap_or_default();
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
    sigs.iter()
        .map(|s| distinct.binary_search(s).expect("present"))
        .collect()
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
    let mut clause_color: Vec<usize> =
        rank_signatures(&clauses.iter().map(|c| c.len()).collect::<Vec<_>>());
    loop {
        // Recolour clauses by the multiset of incident (variable colour, sign).
        let clause_sig: Vec<(usize, Vec<(usize, bool)>)> = clauses
            .iter()
            .enumerate()
            .map(|(ci, c)| {
                let mut nbrs: Vec<(usize, bool)> = c
                    .iter()
                    .map(|l| (var_color[l.var() as usize], l.is_positive()))
                    .collect();
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
        let var_sig: Vec<(usize, Vec<(usize, bool)>)> = (0..num_vars)
            .map(|v| (var_color[v], std::mem::take(&mut var_nbrs[v])))
            .collect();
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
    color_refinement(num_vars, clauses)
        .iter()
        .copied()
        .max()
        .map_or(0, |m| m + 1)
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
                let mut nbrs: Vec<(usize, i64)> = (0..num_vars)
                    .filter(|&v| a[u][v] != 0)
                    .map(|v| (color[v], a[u][v]))
                    .collect();
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
/// (`equitable_partition_of`): `B[u][v] = 1/|cell(u)|` if `u,v` share a cell, else `0`. By Tinhofer's
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
pub fn is_fractional_automorphism(
    num_vars: usize,
    clauses: &[Vec<Lit>],
    partition: &[usize],
) -> bool {
    let a = cooccurrence_matrix(num_vars, clauses);
    let d = partition.iter().copied().max().map_or(0, |m| m + 1);
    let mut cell: Vec<Vec<usize>> = vec![Vec::new(); d];
    for (v, &c) in partition.iter().enumerate() {
        cell[c].push(v);
    }
    let sum_over = |rows: &[usize], w: usize, by_row: bool| -> i64 {
        rows.iter()
            .map(|&x| if by_row { a[x][w] } else { a[w][x] })
            .sum()
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
        let lits: Vec<(usize, bool)> = c
            .iter()
            .map(|l| (l.var() as usize, l.is_positive()))
            .collect();
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
    let mut flat: Vec<(bool, usize, usize, Vec<(usize, bool, bool)>)> =
        Vec::with_capacity(num_vars * num_vars);
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
                let mut tri: Vec<(usize, usize)> = (0..num_vars)
                    .map(|k| (color[at(i, k)], color[at(k, j)]))
                    .collect();
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
    (0..num_vars)
        .map(|i| (0..num_vars).map(|j| color[at(i, j)]).collect())
        .collect()
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
pub fn coherent_configuration_constants(
    num_vars: usize,
    clauses: &[Vec<Lit>],
) -> Option<(usize, Vec<Vec<Vec<u128>>>)> {
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
                                    x[k] = ((x[k] as u128 + cj as u128 * s[jj][k] as u128)
                                        % p as u128)
                                        as u64;
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
pub fn association_scheme_eigenmatrix(
    num_vars: usize,
    clauses: &[Vec<Lit>],
) -> Option<(u64, Vec<Vec<u64>>)> {
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
                .map(|i| {
                    (0..d)
                        .map(|k| (0..d).map(|j| (a[i][j][k] % p as u128) as u64).collect())
                        .collect()
                })
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
                let has_valency = rows
                    .iter()
                    .any(|row| (0..d).all(|i| row[i] as u128 == valency[i] % p as u128));
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
pub fn association_scheme_multiplicities(
    num_vars: usize,
    clauses: &[Vec<Lit>],
) -> Option<Vec<u128>> {
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
                .map(|i| {
                    (0..d)
                        .map(|k| (0..d).map(|j| (a[i][j][k] % p as u128) as u64).collect())
                        .collect()
                })
                .collect();
            if let Some(rows) = gf_simultaneous_eigenvalues(&mmats, d, p) {
                let mut mult = Vec::with_capacity(d);
                let mut ok = true;
                for row in &rows {
                    let mut denom = 0u64;
                    for i in 0..d {
                        let ki = (valency[i] % p as u128) as u64;
                        let term = (row[i] as u128 * row[transpose[i]] as u128 % p as u128) as u64;
                        denom = ((denom as u128
                            + term as u128 * crate::permgroup::mod_inv(ki, p) as u128)
                            % p as u128) as u64;
                    }
                    if denom == 0 {
                        ok = false;
                        break;
                    }
                    let m =
                        (n % p as u128) * crate::permgroup::mod_inv(denom, p) as u128 % p as u128;
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
                    let mut nbr: Vec<(usize, usize, usize)> = (0..n)
                        .map(|w| (color[at(w, j, k)], color[at(i, w, k)], color[at(i, j, w)]))
                        .collect();
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
        .map(|i| {
            (0..n)
                .map(|j| (0..n).map(|k| color[at(i, j, k)]).collect())
                .collect()
        })
        .collect()
}

/// The label-independent **3-WL fingerprint**: the sorted multiset of triple-color class sizes. Separates
/// strongly-regular graphs that share every 2-WL invariant (the rook's vs Shrikhande graph).
pub fn three_wl_fingerprint(num_vars: usize, clauses: &[Vec<Lit>]) -> Vec<usize> {
    let c = three_wl_colors(num_vars, clauses);
    let max = c
        .iter()
        .flatten()
        .flatten()
        .copied()
        .max()
        .map_or(0, |m| m + 1);
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
            let mut lits: Vec<(usize, bool)> = c
                .iter()
                .map(|l| (label[l.var() as usize], l.is_positive()))
                .collect();
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
    ir_canonical(
        num_vars,
        clauses,
        &vec![0usize; num_vars],
        &mut nodes,
        200_000,
    )
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
    let syntactic = clauses
        .iter()
        .all(|c| clause_set.contains(&canon_clause(&apply_perm_to_clause(c, g))));
    syntactic
        || clauses.iter().all(|c| {
            let gc = apply_perm_to_clause(c, g);
            clause_set.contains(&canon_clause(&gc)) || clause_is_implied(num_vars, clauses, &gc)
        })
}

/// **The full arsenal with caller-DECLARED symmetry.** The modeler often knows symmetries the automatic
/// detector cannot afford to find (geometric, semantic, or simply large). This entry point accepts declared
/// variable-permutation generators, **verifies** each is a genuine symmetry (`is_declared_symmetry` —
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
    let mut gens =
        crate::sym_break::variable_automorphism_generators(num_vars, clauses).unwrap_or_default();
    for g in declared {
        if is_declared_symmetry(num_vars, clauses, g) {
            gens.push(g.clone());
        }
    }
    if gens.is_empty() {
        return solve_comprehensive(num_vars, clauses);
    }
    let bsgs = crate::permgroup::schreier_sims(num_vars, &gens);
    let to_litsym =
        |p: &[usize]| -> Vec<Lit> { (0..num_vars).map(|v| Lit::pos(p[v] as u32)).collect() };
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
            if clauses.iter().all(|c| {
                c.iter()
                    .any(|l| projected[l.var() as usize] == l.is_positive())
            }) {
                Solved {
                    answer: Answer::Sat(projected),
                    via: Route::DeclaredSymmetry,
                    proof: Vec::new(),
                    conflicts: solver.conflicts(),
                }
            } else {
                solve_comprehensive(num_vars, clauses) // re-check failed ⟹ authoritative fallback
            }
        }
        SolveResult::Unsat => Solved {
            answer: Answer::Unsat,
            via: Route::DeclaredSymmetry,
            proof: Vec::new(),
            conflicts: solver.conflicts(),
        },
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
fn assignment_orbit(
    num_vars: usize,
    m: &[bool],
    gens: &[crate::permgroup::Perm],
    occurs: &[usize],
) -> Vec<Vec<bool>> {
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
    let gens =
        crate::sym_break::variable_automorphism_generators(num_vars, clauses).unwrap_or_default();

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
    SymmetricCount {
        representatives,
        total_models,
        exhaustive,
    }
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
    let gens =
        crate::sym_break::variable_automorphism_generators(num_vars, clauses).unwrap_or_default();
    let mut profile = profile_of_generators(num_vars, &gens);
    // The coherent (association-scheme) rank is clause-derived — wire it in here (the group profile alone
    // cannot see it). It refines the orbital rank: coherent_rank ≤ rank.
    profile.coherent_rank = Some(
        coherent_rank(num_vars, clauses).unwrap_or_else(|| two_wl_pair_cells(num_vars, clauses)),
    );
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
    let rank = if num_vars <= 48 {
        crate::permgroup::rank(num_vars, &gens)
    } else {
        0
    };
    let transitivity = if num_vars <= 24 {
        crate::permgroup::transitivity_degree(num_vars, &gens, 3)
    } else {
        0
    };
    let primitive = crate::permgroup::is_primitive(num_vars, &gens);
    let blocks = crate::permgroup::minimal_block_system(num_vars, &gens).map(|b| b.len());
    let abelian = crate::permgroup::is_abelian(num_vars, &gens);
    // The series walks are heavier (repeated Schreier–Sims); gate them to moderate sizes. Compute the
    // depths once and read the booleans off them.
    let (solvable, nilpotent, derived_length, nilpotency_class, derived_order) = if num_vars <= 24 {
        let dl = crate::permgroup::derived_length(num_vars, &gens);
        let nc = crate::permgroup::nilpotency_class(num_vars, &gens);
        let d = crate::permgroup::derived_subgroup(num_vars, &gens);
        (
            Some(dl.is_some()),
            Some(nc.is_some()),
            dl,
            nc,
            crate::permgroup::schreier_sims(num_vars, &d).order(),
        )
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
    let frobenius_schur = ctable
        .as_ref()
        .and_then(crate::permgroup::frobenius_schur_from_table);
    let isotypic_multiplicities = ctable
        .as_ref()
        .and_then(|t| crate::permgroup::isotypic_from_table(num_vars, &gens, t));
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
pub fn pb_symmetry_profile(
    num_vars: usize,
    constraints: &[crate::pseudo_boolean::PbConstraint],
) -> SymmetryProfile {
    let gens = crate::pseudo_boolean::coeff_symmetry_generators(num_vars, constraints);
    profile_of_generators(num_vars, &gens)
}

/// The **class-algebra structure constants** of a formula's variable-symmetry group — `a[i][j][k]`, the
/// multiplication coefficients of the conjugacy classes (`Cᵢ·Cⱼ = Σₖ a[i][j][k]·Cₖ`), the foundation of the
/// character table. `None` when the group is too large to enumerate.
pub fn class_algebra_constants(
    num_vars: usize,
    clauses: &[Vec<Lit>],
) -> Option<Vec<Vec<Vec<u128>>>> {
    let gens =
        crate::sym_break::variable_automorphism_generators(num_vars, clauses).unwrap_or_default();
    crate::permgroup::class_multiplication_coefficients(num_vars, &gens, 4096)
}

/// The **character table** of a formula's variable-symmetry group, computed exactly over a finite field by
/// the Burnside–Dixon algorithm (degrees + `GF(p)`-valued irreducible characters). The deepest structural
/// view of the symmetry — `None` when the group is too large for the finite-field diagonalisation.
pub fn character_table(
    num_vars: usize,
    clauses: &[Vec<Lit>],
) -> Option<crate::permgroup::CharacterTable> {
    let gens =
        crate::sym_break::variable_automorphism_generators(num_vars, clauses).unwrap_or_default();
    crate::permgroup::character_table(num_vars, &gens, 4096)
}

/// The **Frobenius–Schur indicators** of a formula's variable-symmetry group — `+1`/`0`/`−1` per
/// irreducible (real/complex/quaternionic), the finest representation-theoretic refinement of the symmetry
/// (it separates groups even when the character table cannot). `None` when out of range.
pub fn frobenius_schur_indicators(num_vars: usize, clauses: &[Vec<Lit>]) -> Option<Vec<i8>> {
    let gens =
        crate::sym_break::variable_automorphism_generators(num_vars, clauses).unwrap_or_default();
    crate::permgroup::frobenius_schur_indicators(num_vars, &gens, 4096)
}

/// The **permutation character** `π(g) = #fixed variables of g` of a formula's variable-symmetry action,
/// valued per conjugacy class. The character of the representation the symmetry group carries on the
/// variables themselves. `None` when the group is too large to enumerate.
pub fn permutation_character(num_vars: usize, clauses: &[Vec<Lit>]) -> Option<Vec<u128>> {
    let gens =
        crate::sym_break::variable_automorphism_generators(num_vars, clauses).unwrap_or_default();
    crate::permgroup::permutation_character(num_vars, &gens, 4096)
}

/// The **isotypic decomposition** of a formula's variable-permutation representation — the multiplicity of
/// each irreducible in `π = Σ_s m_s χ_s`. The representation-theoretic spectrum of the symmetry, tying the
/// character table back to the orbit/orbital structure of the action. `None` when out of range.
pub fn isotypic_multiplicities(num_vars: usize, clauses: &[Vec<Lit>]) -> Option<Vec<u128>> {
    let gens =
        crate::sym_break::variable_automorphism_generators(num_vars, clauses).unwrap_or_default();
    crate::permgroup::isotypic_multiplicities(num_vars, &gens, 4096)
}

/// The **tensor (fusion) decomposition** of a formula's variable-symmetry irreducibles — `N[i][j][k]`, the
/// multiplicity of `χ_k` in `χ_i ⊗ χ_j`, the multiplication table of the representation ring `R(G)`. `None`
/// when the character table is out of range.
pub fn tensor_decomposition(num_vars: usize, clauses: &[Vec<Lit>]) -> Option<Vec<Vec<Vec<u128>>>> {
    let gens =
        crate::sym_break::variable_automorphism_generators(num_vars, clauses).unwrap_or_default();
    crate::permgroup::tensor_decomposition(num_vars, &gens, 4096)
}

/// The **Galois orbits on conjugacy classes** of a formula's variable symmetry — classes fused by the
/// `(ℤ/e)*` action `C ↦ C^t` (algebraic conjugacy). The singletons are the rational classes. `None` when
/// out of range.
pub fn galois_class_orbits(num_vars: usize, clauses: &[Vec<Lit>]) -> Option<Vec<Vec<usize>>> {
    let gens =
        crate::sym_break::variable_automorphism_generators(num_vars, clauses).unwrap_or_default();
    crate::permgroup::galois_class_orbits(num_vars, &gens, 4096)
}

/// The **table of marks** of a formula's variable-symmetry group — the Burnside-ring classification of its
/// `G`-sets (the permutation-representation analogue of [`character_table`]). Returns
/// `(subgroup_class_orders, marks)`. `None` when the subgroup lattice is out of range.
pub fn table_of_marks(
    num_vars: usize,
    clauses: &[Vec<Lit>],
) -> Option<(Vec<u128>, Vec<Vec<u128>>)> {
    let gens =
        crate::sym_break::variable_automorphism_generators(num_vars, clauses).unwrap_or_default();
    crate::permgroup::table_of_marks(num_vars, &gens, 256)
}

/// The **Burnside ring** multiplication of a formula's variable-symmetry group — `N[a][b][l]`, the
/// decomposition of the product G-set `(G/H_a)×(G/H_b)` into transitive G-sets (the G-set analogue of the
/// character table's fusion ring). `None` when the subgroup lattice is out of range.
pub fn burnside_ring_product(num_vars: usize, clauses: &[Vec<Lit>]) -> Option<Vec<Vec<Vec<i128>>>> {
    let gens =
        crate::sym_break::variable_automorphism_generators(num_vars, clauses).unwrap_or_default();
    crate::permgroup::burnside_ring_product(num_vars, &gens, 256)
}

/// The **Möbius number** `μ(1, G)` of the subgroup lattice of a formula's variable-symmetry group. `None`
/// when the subgroup lattice is out of range.
pub fn mobius_number(num_vars: usize, clauses: &[Vec<Lit>]) -> Option<i128> {
    let gens =
        crate::sym_break::variable_automorphism_generators(num_vars, clauses).unwrap_or_default();
    crate::permgroup::mobius_number(num_vars, &gens, 256)
}

/// The number of ordered `k`-tuples of symmetries that **generate** the whole variable-symmetry group `G`
/// (Hall's Eulerian function `e_k(G)`); `e_k(G)/|G|^k` is the probability `k` random symmetries generate it.
/// `None` when out of range.
pub fn generating_tuple_count(num_vars: usize, clauses: &[Vec<Lit>], k: u32) -> Option<i128> {
    let gens =
        crate::sym_break::variable_automorphism_generators(num_vars, clauses).unwrap_or_default();
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
    let gens =
        crate::sym_break::variable_automorphism_generators(num_vars, clauses).unwrap_or_default();
    crate::permgroup::permutation_character_decomposition(num_vars, &gens, 256)
}

/// The order of `Aut(G)` for a formula's variable-symmetry group `G` — the automorphism group of the
/// symmetry itself. `None` when out of range.
pub fn automorphism_group_order(num_vars: usize, clauses: &[Vec<Lit>]) -> Option<u128> {
    let gens =
        crate::sym_break::variable_automorphism_generators(num_vars, clauses).unwrap_or_default();
    crate::permgroup::automorphism_group_order(num_vars, &gens, 256)
}

/// The **weight inventory** of a formula's variable symmetry: entry `w` is the number of essentially-
/// distinct (modulo symmetry) `{0,1}` assignments to the variables with exactly `w` ones — the weighted
/// Pólya refinement of [`SymmetryProfile::assignment_orbits`] (which is the sum). `None` when the group is
/// too large to enumerate.
pub fn assignment_weight_inventory(num_vars: usize, clauses: &[Vec<Lit>]) -> Option<Vec<u128>> {
    let gens =
        crate::sym_break::variable_automorphism_generators(num_vars, clauses).unwrap_or_default();
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
        if x <= y {
            (x.0, x.1, y.0, y.1)
        } else {
            (y.0, y.1, x.0, x.1)
        }
    };
    let mut combined = clauses.to_vec();
    let mut seen: HashSet<(u32, bool, u32, bool)> = combined
        .iter()
        .filter(|c| c.len() == 2)
        .map(|c| key(c[0], c[1]))
        .collect();
    loop {
        let gens = crate::sym_break::variable_automorphism_generators(num_vars, &combined)
            .unwrap_or_default();
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
                let Some(v) = (0..num_vars).find(|&i| g[i] != i) else {
                    continue;
                };
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
pub fn break_all_symmetry_complete(
    num_vars: usize,
    clauses: &[Vec<Lit>],
) -> (Vec<Vec<Lit>>, usize) {
    let gens =
        crate::sym_break::variable_automorphism_generators(num_vars, clauses).unwrap_or_default();
    if gens.is_empty() {
        return (clauses.to_vec(), num_vars);
    }
    let bsgs = crate::permgroup::schreier_sims(num_vars, &gens);
    let to_litsym =
        |p: &[usize]| -> Vec<Lit> { (0..num_vars).map(|v| Lit::pos(p[v] as u32)).collect() };
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
            if clauses
                .iter()
                .all(|c| c.iter().any(|l| model[l.var() as usize] == l.is_positive()))
            {
                Solved {
                    answer: Answer::Sat(model),
                    via: inner.via,
                    proof: inner.proof,
                    conflicts: inner.conflicts,
                }
            } else {
                solve_comprehensive(num_vars, clauses) // re-check failed ⇒ authoritative fallback
            }
        }
        Answer::Unsat => Solved {
            answer: Answer::Unsat,
            via: inner.via,
            proof: inner.proof,
            conflicts: inner.conflicts,
        },
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
            .all(|c| {
                c.iter().any(|l| {
                    model.get(l.var() as usize).copied().unwrap_or(false) == l.is_positive()
                })
            })
            .then_some(model)
    };

    if modp::is_prime(rec.modulus) {
        // Prime modulus: the proven field engine.
        match modp::solve(&rec.equations, rec.num_vars, rec.modulus) {
            ModpOutcome::Unsat(combo) => modp::is_refutation(&rec.equations, rec.num_vars, rec.modulus, &combo)
                .then(|| Solved::unsat(Route::ModP)),
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
            ModmOutcome::Unsat { modulus, combo } => modm::is_refutation(&rec.equations, rec.num_vars, modulus, &combo)
                .then(|| Solved::unsat(Route::ModM)),
            ModmOutcome::Sat(assign) => Some(Solved {
                answer: Answer::Sat(build_model(&assign)?),
                via: Route::ModM,
                proof: Vec::new(),
                conflicts: 0,
            }),
        }
    }
}

/// Whether `steps` refute `clauses` alone. A stream of RUP additions is checked by trimmed LRAT
/// ([`crate::lrat::refutation`], watched-literal propagation); one with PR steps by
/// [`crate::pr::check_pr_refutation_fast`].
fn refutes(num_vars: usize, clauses: &[Vec<Lit>], steps: &[ProofStep]) -> bool {
    let rup_only: Option<Vec<Vec<Lit>>> = steps
        .iter()
        .map(|s| match s {
            ProofStep::Rup(c) => Some(c.clone()),
            _ => None,
        })
        .collect();
    match rup_only {
        Some(lemmas) => crate::lrat::refutation(num_vars, clauses, &lemmas).is_some(),
        None => crate::pr::check_pr_refutation_fast(num_vars, clauses, steps),
    }
}

/// **A proof of an UNSAT verdict, checked against the formula alone**, or `None` when the route has no clausal
/// proof to give. The proof [`Solved::proof`] carries is used when it checks. The hybrid-XOR route's may not: its
/// search ran over the formula and the short clauses Gaussian elimination implied, which are implied but not RUP.
/// A CDCL-family route whose proof does not check is given a fresh CDCL refutation of the formula itself. The parity and GF(p) routes
/// compile their linear refutations to resolution ([`crate::xor_drat`]), within that bridge's budget. The other
/// specialists have no polynomial clausal proof, and their verdicts rest on their own certificates: a Hall witness,
/// an exact-cover or ℤ/m combination, an SoS or Nullstellensatz certificate, each re-checked before the route
/// returned; a cutting-plane sum; a collapse's descent. Whatever is returned has been checked here.
pub fn certified_proof(num_vars: usize, clauses: &[Vec<Lit>], solved: &Solved) -> Option<Vec<ProofStep>> {
    if !matches!(solved.answer, Answer::Unsat) {
        return None;
    }
    if refutes(num_vars, clauses, &solved.proof) {
        return Some(solved.proof.clone());
    }
    let candidate: Vec<ProofStep> = match solved.via {
        Route::Parity => {
            let eqs = extract_xor(num_vars, clauses);
            let XorOutcome::Unsat(subset) = crate::xorsat::solve(&eqs, num_vars) else { return None };
            crate::xor_drat::emit_xor_drat(&eqs, &subset)?.into_iter().map(ProofStep::Rup).collect()
        }
        Route::ModP => crate::xor_drat::emit_modp_drat(num_vars, clauses)?.into_iter().map(ProofStep::Rup).collect(),
        Route::Cdcl | Route::HybridXor | Route::Incompressible => crate::sdcl::plain_cdcl_refutation(num_vars, clauses),
        _ => return None,
    };
    refutes(num_vars, clauses, &candidate).then_some(candidate)
}

/// **The clause-bundle pass.** Run every structure-mining contributor and union the *implied* clauses
/// (no-goods) they discover. Each contributor's soundness contract: every returned clause is implied
/// by the formula, so the union preserves the solution set — a sound, never-worse enrichment that lets
/// one method's discovered structure accelerate the others (and CDCL). Contributors that find nothing
/// are cheap, so this is safe to run before the fallback on any instance.
pub fn mine_clauses(num_vars: usize, clauses: &[Vec<Lit>]) -> Vec<Vec<Lit>> {
    let mut seen: HashSet<Vec<i64>> = clauses.iter().map(|c| canon(c)).collect();
    let mut pool = Vec::new();
    let add =
        |bundle: Vec<Vec<Lit>>, pool: &mut Vec<Vec<Lit>>, seen: &mut HashSet<Vec<i64>>| {
            for c in bundle {
                if seen.insert(canon(&c)) {
                    pool.push(c);
                }
            }
        };
    add(xor_gaussian_bundle(num_vars, clauses), &mut pool, &mut seen);
    add(
        failed_literal_bundle(num_vars, clauses),
        &mut pool,
        &mut seen,
    );
    pool
}

/// Canonical clause key (sorted signed-var ids) for deduping against the formula and each other.
fn canon(c: &[Lit]) -> Vec<i64> {
    let mut k: Vec<i64> = c
        .iter()
        .map(|l| {
            if l.is_positive() {
                l.var() as i64 + 1
            } else {
                -(l.var() as i64 + 1)
            }
        })
        .collect();
    k.sort_unstable();
    k.dedup();
    k
}

/// Max width of clauses the Gaussian bundle emits (tunable for experiments via `LOGOS_XOR_WIDTH`).
fn xor_width() -> usize {
    std::env::var("LOGOS_XOR_WIDTH")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(3)
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
        let pos: Vec<usize> = c
            .iter()
            .filter(|l| l.is_positive())
            .map(|l| l.var() as usize)
            .collect();
        let neg: Vec<usize> = c
            .iter()
            .filter(|l| !l.is_positive())
            .map(|l| l.var() as usize)
            .collect();
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
/// Whether every clause is all-positive or a binary exclusion `¬a ∨ ¬b`: the only clauses the matching
/// ([`crate::pigeonhole::decide_pigeonhole_unsat`]) and cutting-plane ([`crate::pseudo_boolean::refute_clausal`])
/// recognizers read, so on any other formula both decline.
fn covering_shaped(clauses: &[Vec<Lit>]) -> bool {
    clauses.iter().all(|c| c.iter().all(|l| l.is_positive()) || (c.len() == 2 && c.iter().all(|l| !l.is_positive())))
}

fn cnf_to_expr(clauses: &[Vec<Lit>]) -> Option<ProofExpr> {
    let atom = |l: &Lit| {
        let a = ProofExpr::Atom(format!("v{}", l.var()));
        if l.is_positive() {
            a
        } else {
            ProofExpr::Not(Box::new(a))
        }
    };
    let mut clause_exprs = Vec::with_capacity(clauses.len());
    for c in clauses {
        if c.is_empty() {
            return None;
        }
        let atoms: Vec<ProofExpr> = c.iter().map(&atom).collect();
        clause_exprs.push(balanced(atoms, &|a, b| {
            ProofExpr::Or(Box::new(a), Box::new(b))
        })?);
    }
    // Balanced trees keep the And/Or spine depth ~log(n), so the recursive detector traversals do
    // not overflow the stack on competition-scale formulas (tens of thousands of clauses).
    balanced(clause_exprs, &|a, b| {
        ProofExpr::And(Box::new(a), Box::new(b))
    })
}

/// Fold `items` into one expression with a balanced (log-depth) tree under `combine`.
fn balanced(
    mut items: Vec<ProofExpr>,
    combine: &dyn Fn(ProofExpr, ProofExpr) -> ProofExpr,
) -> Option<ProofExpr> {
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
/// The search itself runs with the equations as a watched Gauss–Jordan theory
/// ([`crate::xor_engine::WatchedXor`]), so every assignment the linear system forces is propagated, and
/// every one it refutes is a conflict, at the fixpoint it arises in. A SAT model is re-checked against the
/// clauses (fail-closed).
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
        // DPLL(XOR): the watched Gauss–Jordan matrix as a theory. Optionally restrict CDCL's decisions to
        // the kernel — the variables the reduced system leaves non-basic — so the basic ones are forced by
        // Gaussian propagation rather than branched on.
        let live = WatchedXor::new(num_vars, &eqs);
        if xor_kernel() {
            solver.set_decision_vars(&live.decision_vars());
        }
        let mut theories: Vec<Box<dyn crate::cdcl::Theory>> = vec![Box::new(Retiring { live, retired: false })];
        solver.solve_with(&mut theories)
    } else {
        solver.solve()
    };
    match result {
        SolveResult::Sat(model) => clauses
            .iter()
            .all(|c| c.iter().any(|l| model[l.var() as usize] == l.is_positive()))
            .then_some(Solved {
                answer: Answer::Sat(model),
                via: Route::HybridXor,
                proof: Vec::new(),
                conflicts: solver.conflicts(),
            }),
        SolveResult::Unsat => {
            let proof = solver
                .learned()
                .iter()
                .map(|lc| ProofStep::Rup(lc.lits.clone()))
                .collect();
            Some(Solved {
                answer: Answer::Unsat,
                via: Route::HybridXor,
                proof,
                conflicts: solver.conflicts(),
            })
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
        Box::new(crate::pseudo_boolean::CardinalityTheory::new(
            num_vars, &amo,
        )),
        Box::new(crate::lyapunov::SymmetryTheory::new(
            num_vars,
            crate::lyapunov::fused_symmetry_group(num_vars, clauses),
        )),
    ];
    match solver.solve_with(&mut theories) {
        SolveResult::Sat(model) => clauses
            .iter()
            .all(|c| c.iter().any(|l| model[l.var() as usize] == l.is_positive()))
            .then_some(Solved {
                answer: Answer::Sat(model),
                via: Route::HybridXor,
                proof: Vec::new(),
                conflicts: solver.conflicts(),
            }),
        SolveResult::Unsat => Some(Solved {
            answer: Answer::Unsat,
            via: Route::HybridXor,
            proof: Vec::new(),
            conflicts: solver.conflicts(),
        }),
    }
}

/// Whether [`hybrid_xor`] searches with the watched Gauss–Jordan theory, or with the mined clauses and the seeded
/// phases alone. Default on; `LOGOS_XOR_LIVE=0` turns it off, for an A/B.
/// The calls after which, and after every further run of as many, the hybrid route weighs its Gauss–Jordan theory.
const XOR_WEIGHED_EVERY: u64 = 4096;

/// The refutations in a thousand calls below which the hybrid route lets its Gauss–Jordan theory go.
const XOR_KEPT_PER_MILLE: u64 = 1;

/// The hybrid route's Gauss–Jordan theory, let go once it stops paying. The route keeps every clause of the formula,
/// the equations' own among them, so what the theory forces the clauses force too, a step at a time; what it alone
/// finds early is a refutation, a contradiction among rows no single clause holds. Weighed every
/// [`XOR_WEIGHED_EVERY`] calls, it stays while at least [`XOR_KEPT_PER_MILLE`] in a thousand of its calls have ended
/// in one. On a parity formula most calls refute; on the adders of an ARX cipher, where the matrix spans thousands of
/// rows and costs most of the search's time, almost none do. Let go, it answers every call with a fixpoint, and the
/// literals it forced before are still explained by its matrices as they stood.
struct Retiring {
    live: WatchedXor,
    retired: bool,
}

impl Retiring {
    fn weigh(&mut self) {
        let calls = self.live.calls();
        if calls % XOR_WEIGHED_EVERY == 0 && self.live.refutations() * 1000 < calls * XOR_KEPT_PER_MILLE {
            self.retired = true;
        }
    }
}

impl crate::cdcl::Theory for Retiring {
    fn propagate(&mut self, trail: &[Lit]) -> Vec<Vec<Lit>> {
        if self.retired {
            return Vec::new();
        }
        let out = self.live.propagate(trail);
        self.weigh();
        out
    }

    fn propagate_kept(&mut self, trail: &[Lit], kept: usize) -> Vec<Vec<Lit>> {
        if self.retired {
            return Vec::new();
        }
        let out = self.live.propagate_kept(trail, kept);
        self.weigh();
        out
    }

    fn propagate_lazily(&mut self, trail: &[Lit], kept: usize, forced: &mut Vec<(Lit, u32)>) -> Vec<Vec<Lit>> {
        if self.retired {
            return Vec::new();
        }
        let out = self.live.propagate_lazily(trail, kept, forced);
        self.weigh();
        out
    }

    fn explain(&mut self, token: u32, clause: &mut Vec<Lit>) {
        self.live.explain(token, clause);
    }
}

fn xor_live() -> bool {
    std::env::var("LOGOS_XOR_LIVE")
        .map(|s| s != "0" && !s.is_empty())
        .unwrap_or(true)
}

/// Whether the theory's search decides only the variables the reduced system leaves non-basic. Default off: on the
/// parity-learning family it took more conflicts than deciding every variable. `LOGOS_XOR_KERNEL=1` turns it on.
fn xor_kernel() -> bool {
    std::env::var("LOGOS_XOR_KERNEL")
        .map(|s| s != "0" && !s.is_empty())
        .unwrap_or(false)
}

/// Whether to seed CDCL's phases with the GF(2) witness. Default on; `LOGOS_XOR_SEED=0` disables it.
fn xor_seed() -> bool {
    std::env::var("LOGOS_XOR_SEED")
        .map(|s| s != "0" && !s.is_empty())
        .unwrap_or(true)
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_proof/solve/tests.rs"]
mod tests;
