//! Propositional SAT-discharge of a `ProofExpr` obligation — the engine behind in-browser,
//! Z3-free hardware proving.
//!
//! A bounded hardware property (an SVA assertion unrolled to discrete timesteps, or a
//! Kripke-lowered FOL spec) reduces to a quantifier-free propositional formula over
//! `signal@t` atoms. This module discharges two questions over that fragment, reusing the
//! existing trust tiers ([`crate::cnf`] Tseitin → [`crate::cdcl`] CDCL → [`crate::rup`] RUP
//! certification) so the answers are certified, not merely asserted:
//!
//! - [`find_model`] — is the obligation satisfiable, and if so, a distinguishing assignment.
//!   This is `∃trace. φ`, the core of bounded model checking and counterexample extraction.
//! - [`prove_equivalence`] — do two formulas denote the same Boolean function? `F ≡ S` iff
//!   `F ↔ S` is a tautology, certified via RUP; otherwise a concrete counterexample trace.
//!
//! Everything here is pure Rust with no Z3 dependency, so it runs unchanged in the browser
//! (wasm32) and in native tests where its verdicts are checked against Z3 as the oracle.

use crate::cdcl::{BudgetedResult, Lit, SolveResult, Solver, Var};
use crate::cnf::Cnf;
use crate::rup;
use crate::ProofExpr;
use std::collections::HashMap;
use std::collections::BTreeSet;

/// The result of a satisfiability query over a propositional obligation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ModelOutcome {
    /// Satisfiable, with a model over the source atoms (sorted by name). For a hardware
    /// obligation these are the `signal@t` bindings of a witnessing trace.
    Sat(Vec<(String, bool)>),
    /// Unsatisfiable — no assignment satisfies the obligation.
    Unsat,
    /// Not a quantifier-free propositional formula over recognisable atoms, so the SAT
    /// engine cannot speak to it (the caller must escalate, e.g. bit-blast first).
    Unsupported,
}

/// The result of an unsatisfiability query — the shared primitive behind equivalence,
/// bounded model checking, k-induction, and vacuity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UnsatOutcome {
    /// The formula is unsatisfiable — RUP-certified (the refutation replays to empty).
    Refuted,
    /// The formula is satisfiable, with a witnessing model over its atoms.
    Sat(Vec<(String, bool)>),
    /// Not a quantifier-free propositional formula, or the refutation could not be
    /// certified — fail-closed (never a false `Refuted`).
    Unsupported,
}

/// The result of an equivalence query between two formulas.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EquivOutcome {
    /// The two formulas denote the same Boolean function. The verdict is RUP-certified:
    /// `F ↔ S` was replayed to the empty clause, not merely reported UNSAT.
    Equivalent,
    /// The formulas differ. The assignment is a concrete counterexample: under it exactly
    /// one of the two formulas holds. For hardware, this is the distinguishing waveform.
    Differ(Vec<(String, bool)>),
    /// Not purely propositional over recognisable atoms — the caller must escalate.
    Unsupported,
}

/// Is `e` satisfiable? Returns a witnessing model over its atoms if so.
///
/// `∃assignment. e`. Tseitin-clausifies `e`, runs CDCL, and on SAT decodes the model back
/// to the source atoms appearing in `e` (Tseitin auxiliaries are dropped). Used directly by
/// bounded model checking (`∃trace. ¬property`) and for counterexample extraction.
pub fn find_model(e: &ProofExpr) -> ModelOutcome {
    let mut cnf = Cnf::new();
    if cnf.assert(e).is_none() {
        return ModelOutcome::Unsupported;
    }
    // Move the atom table out alongside the solver — no clause-database clone just to decode.
    let (mut solver, atom_of) = cnf.into_solver_with_atoms();
    match solver.solve() {
        SolveResult::Unsat => ModelOutcome::Unsat,
        SolveResult::Sat(model) => ModelOutcome::Sat(decode_model_from(&atom_of, &model, &[e])),
    }
}

/// Are `a` and `b` equivalent? `F ≡ S` iff `F ↔ S` is valid, i.e. `¬(F ↔ S)` is UNSAT.
///
/// One solve discharges both outcomes: a satisfying assignment is the counterexample
/// (`Differ`); UNSAT is RUP-certified into `Equivalent` (the certified trust tier — a
/// solver bug that can't be replayed yields `Unsupported`, never a false `Equivalent`).
/// Structurally identical formulas short-circuit with no solve at all.
pub fn prove_equivalence(a: &ProofExpr, b: &ProofExpr) -> EquivOutcome {
    // Sound fast-path: identical formulas are trivially equivalent (no solve).
    if a == b {
        return EquivOutcome::Equivalent;
    }
    // `F ≡ S` iff `¬(F ↔ S)` is unsatisfiable.
    let neg_iff = ProofExpr::Not(Box::new(ProofExpr::Iff(
        Box::new(a.clone()),
        Box::new(b.clone()),
    )));
    match prove_unsat(&neg_iff) {
        UnsatOutcome::Refuted => EquivOutcome::Equivalent,
        UnsatOutcome::Sat(model) => EquivOutcome::Differ(model),
        UnsatOutcome::Unsupported => EquivOutcome::Unsupported,
    }
}

/// Is `e` unsatisfiable? One CDCL solve decides it: `Sat` carries a witnessing model,
/// `Unsat` is independently RUP-certified into `Refuted` (a refutation the trusted checker
/// cannot replay yields `Unsupported`, never a false `Refuted`). This is the shared certified
/// Does `e` reduce to an inconsistent mod-`m` linear system? Clausifies `e`, recovers the one-hot-encoded
/// congruences ([`crate::modp::recover_from_cnf`]), and refutes them by certified modular Gaussian
/// elimination — over the prime field `GF(p)` ([`crate::modp`]) when the recovered modulus is prime, or
/// over `ℤ/m` by CRT across its prime factors ([`crate::modm`]) when it is composite. Fail-closed:
/// recovery returns `None` on anything that is not a faithful mod-`m` encoding, and the produced linear
/// refutation is independently re-checked, so a `true` is always a sound UNSAT. This is the engine behind
/// the cascade's modular fast-path — the same lift `solve_structured` routes to `ModP`/`ModM`.
fn refutes_modular(e: &ProofExpr) -> bool {
    let mut cnf = Cnf::new();
    if cnf.assert(e).is_none() {
        return false;
    }
    let Some(rec) = crate::modp::recover_from_cnf(cnf.num_vars(), cnf.clauses()) else {
        return false;
    };
    if crate::modp::is_prime(rec.modulus) {
        match crate::modp::solve(&rec.equations, rec.num_vars, rec.modulus) {
            crate::modp::ModpOutcome::Unsat(combo) => {
                crate::modp::is_refutation(&rec.equations, rec.num_vars, rec.modulus, &combo)
            }
            _ => false,
        }
    } else {
        match crate::modm::solve(&rec.equations, rec.num_vars, rec.modulus) {
            Some(crate::modm::ModmOutcome::Unsat { modulus, combo }) => {
                crate::modm::is_refutation(&rec.equations, rec.num_vars, modulus, &combo)
            }
            _ => false,
        }
    }
}

/// Does `e` collapse under an auto-discovered covering symmetry, an asymmetric cardinality cover, or a
/// parity structure? Clausifies `e` and runs [`crate::lyapunov::auto_collapse`], which synthesizes a
/// Lyapunov measure and certifies the descent to a contradiction. This catches the *irregularly-encoded*
/// coverings and counting cores that the strict structural recognizers (pigeonhole, cutting-planes) miss
/// — the small-`n` census finds 756 such minimal-UNSAT families at n=4 alone. Fail-closed: each
/// `auto_collapse` variant is returned only once its contradiction is actually reached/checked
/// (`ranked.refuted` / `reached_goal`), and its XOR/covering recovery is conservative, so a `true` is a
/// sound UNSAT. Runs after the cheap structural cuts, before search.
fn refutes_by_collapse(e: &ProofExpr) -> bool {
    let mut cnf = Cnf::new();
    if cnf.assert(e).is_none() {
        return false;
    }
    !matches!(
        crate::lyapunov::auto_collapse(cnf.num_vars(), cnf.clauses()),
        crate::lyapunov::AutoCollapse::None
    )
}

/// Does `e` have a low-degree **Nullstellensatz** refutation over GF(2)? Clausifies `e` and asks whether
/// `1` lies in the degree-`d` GF(2)-span of the clause polynomials (`crate::polycalc`) — the universal
/// algebraic cut that *subsumes* the narrow ones (parity is its degree-1 fragment) and, at degree ≥ 2,
/// certifies the irregular low-degree-algebraic cores no structural recognizer matches. The degree is
/// **size-gated**: we take the largest `d` whose monomial basis `Σ_{i≤d} C(n,i)` fits a fixed budget —
/// degree 1 (cheap, general GF(2)-affine) for large formulas, higher degree only when `n` is small — so
/// the cut never threatens the hot path. Nullstellensatz refutability is monotone in `d`, so the single
/// call at the gated degree decides it. Sound: a certificate exists only for an unsatisfiable formula.
/// The largest algebraic degree whose monomial basis `Σ_{i≤d} C(n,i)` fits a fixed budget — the
/// size-gate shared by the Nullstellensatz and Polynomial-Calculus cuts. Degree 1 (cheap, general
/// GF(2)-affine) for large formulas, higher degree only when `n` is small; `0` when even degree 1 is too
/// wide (leave it to search). Keeps the algebraic cuts off the hot path.
fn gated_algebraic_degree(n: usize) -> usize {
    const MONO_BUDGET: u128 = 4000;
    let mut dmax = 0usize;
    let mut monos: u128 = 1; // the constant monomial (degree 0)
    let mut binom: u128 = 1; // C(n, d), updated as C(n,d) = C(n,d-1)·(n-d+1)/d
    for d in 1..=n {
        binom = binom * (n - d + 1) as u128 / d as u128;
        if monos + binom > MONO_BUDGET {
            break;
        }
        monos += binom;
        dmax = d;
    }
    dmax
}

fn refutes_by_nullstellensatz(e: &ProofExpr) -> bool {
    let mut cnf = Cnf::new();
    if cnf.assert(e).is_none() {
        return false;
    }
    let n = cnf.num_vars();
    let d = gated_algebraic_degree(n);
    if d == 0 {
        return false;
    }
    crate::polycalc::nullstellensatz_refutes(n, cnf.clauses(), d)
}

/// Does `e` have a low-degree **Polynomial Calculus** refutation over GF(2)? PC is the dynamic
/// strengthening of Nullstellensatz — it closes the clause polynomials under linear combination *and*
/// multiply-by-variable, so an intermediate cancellation lets it certify at the gated degree a strict
/// superset of what the NS cut above reaches (measured: real families at n=3). Same size-gate, runs only
/// when the cheaper NS cut has already declined, sound (`1` derivable ⟹ unsatisfiable).
fn refutes_by_polynomial_calculus(e: &ProofExpr) -> bool {
    let mut cnf = Cnf::new();
    if cnf.assert(e).is_none() {
        return false;
    }
    let n = cnf.num_vars();
    let d = gated_algebraic_degree(n);
    if d == 0 {
        return false;
    }
    crate::polycalc::polynomial_calculus_refutes(n, cnf.clauses(), d)
}

/// Variable-count ceiling under which `prove_unsat` runs the certified recursive symmetry breaker as its
/// terminal rung. The symmetric-UNSAT instances that survive every algebraic cut and reach that rung are
/// small; above the cap the automorphism re-detection is not worth its cost, so the cheap single-pass row
/// break takes over.
const CERTIFIED_SYMMETRY_VAR_CAP: usize = 64;

/// Conflict budget for the first, cheap pass of the complete search in [`prove_unsat`]. Easy
/// instances finish far below it; a hard one exhausts it and thereby EARNS the expensive terminal
/// certified-symmetry rung. Keeping the rung escalation-only is the invariant that keeps
/// microsecond obligations at microseconds (the rung costs ~50ms even on tiny formulas).
const EASY_SEARCH_CONFLICTS: u64 = 2_000;

/// Refute `e` with the **certified recursive symmetry breaker** and return the proof. Each round
/// re-detects the residual automorphism group, certifies ONE lex-leader lead clause as a PR step (its
/// generator a fresh automorphism of the current database), and re-detects — looping to a fixpoint that
/// breaks the COMPLETE group, not just the adjacent positive-row swaps [`crate::symmetry::break_symmetries`]
/// sees. The returned stream is the certified PR breaks followed by the closing RUP learned clauses, and
/// it is already `check_pr_refutation`-verified against `e`'s CNF. `None` if `e` is not CNF-assertable, is
/// satisfiable, or the breaker did not certify a refutation — the wired, checkable companion of
/// [`prove_unsat`]'s coarse verdict.
pub fn prove_unsat_certified(e: &ProofExpr) -> Option<Vec<crate::proof::ProofStep>> {
    let mut cnf = Cnf::new();
    cnf.assert(e)?;
    let r = crate::sym_certify::certified_unsat_auto(cnf.num_vars(), cnf.clauses());
    r.refuted.then_some(r.steps)
}

/// core for equivalence, bounded model checking, k-induction, and vacuity.
pub fn prove_unsat(e: &ProofExpr) -> UnsatOutcome {
    // Pigeonhole fast-path: a conjunction of at-least-one rows + fully-encoded at-most-one columns is
    // a bipartite-matching question that costs CDCL *exponentially* many resolution steps, but the
    // matching reasoner decides in *polynomial* time with a re-verified Hall witness — a sound UNSAT
    // certificate. Fires only on a faithfully-recognized, infeasible pigeonhole structure (never a
    // false `Refuted`); everything else falls through to the certified CDCL core below.
    if crate::pigeonhole::decide_pigeonhole_unsat(e) {
        return UnsatOutcome::Refuted;
    }
    // Ordering-principle fast-path: a complete GT(n) core — a strict total order (totality +
    // antisymmetry + transitivity) with no maximal element — is unsatisfiable, since a finite strict
    // total order always has a maximum. Recognized structurally and certified from that structure in
    // polynomial time, where the general cascade searches super-polynomially. Faithful/fail-closed
    // (never a false `Refuted`); falls through for anything that is not a complete ordering core.
    if crate::ordering::refutes_ordering_principle(e) {
        return UnsatOutcome::Refuted;
    }
    // Cutting-planes fast-path: recover the at-most-one cardinality from the exclusion cliques and
    // sum-refute to `0 ≥ 1` — a POLYNOMIAL cutting-plane proof of a pigeonhole/cardinality CNF that
    // resolution (CDCL) refutes only exponentially. Sound (each step a cutting-plane inference over
    // verified at-most-one cliques, never a false `Refuted`); falls through for anything else.
    if crate::pseudo_boolean::refute_clausal(e) {
        return UnsatOutcome::Refuted;
    }
    // Parity (GF(2)) fast-path: a Tseitin/XOR cover is a linear system over GF(2). Its CNF encoding is
    // resolution-hard (CDCL blows up exponentially on expander instances), but Gaussian elimination
    // decides it in POLYNOMIAL time. We recognize the wrong-parity clause bundles, recover the XOR
    // equations they imply, and refute the linear subsystem — sound (each equation a consequence of
    // `e`, the refutation re-checkable), falling through for anything without inconsistent parity.
    if crate::xorsat::refute_via_parity(e) {
        return UnsatOutcome::Refuted;
    }
    // Modular (GF(p) / ℤ-mod-m) fast-path: the parity cut above speaks only GF(2), but a mod-`m`
    // counting/Tseitin obstruction is a linear system over ℤ/m. Its one-hot CNF encoding is
    // resolution-hard — CDCL blows up *exponentially* (and Z3/Kissat time out), and the GF(2) cut is
    // blind to odd characteristic — yet Gaussian elimination over the *right* modulus decides it in
    // polynomial time. We recover the congruence system and refute it over the prime field `GF(p)` when
    // `m` is prime, or over ℤ/m by CRT across its prime factors when it is composite. Sound and
    // fail-closed: recovery declines on non-encodings and every modular refutation is re-checked, so a
    // `Refuted` here is never false. This carries the parity cut to every modulus.
    if refutes_modular(e) {
        return UnsatOutcome::Refuted;
    }
    // The apex algebraic rung — Sum-of-Squares / Positivstellensatz over the ordered field
    // (`crate::sos`) — would slot here, certifying integrality gaps the GF(2) cuts above structurally
    // cannot. It is *deliberately not wired*: with `sos::MAX_VARS = 6` it only runs where the
    // Nullstellensatz cut already runs at full degree (`2ⁿ ≤ MONO_BUDGET`, i.e. `n ≤ 11`), and there NS
    // is *complete* — so SoS would never fire. It earns a slot only once its reach extends into the
    // `n ≥ 12` band where NS is gated below full degree, which needs the symmetry-reduced / PSD SoS that
    // scales past the Fourier–Motzkin wall. The wiring is then one size-gated `sos::sos_refutes` call.
    // Symmetry breaking: a symmetric formula forces CDCL to re-derive the same conflict once per
    // symmetric copy (the pigeon symmetry multiplies the refutation by `n!`). Augmenting with sound,
    // *verified-automorphism* lex-leader SBPs collapses each orbit so the solver searches the
    // quotient. The SBPs preserve satisfiability, so refuting `e ∧ SBP` refutes `e`; the model decode
    // and the `e` reported on `Sat` stay over the original atoms (auxiliaries are skipped).
    let augmented = crate::symmetry::break_symmetries(e);
    let mut cnf = Cnf::new();
    if cnf.assert(&augmented).is_none() {
        return UnsatOutcome::Unsupported;
    }
    let num_vars = cnf.num_vars();
    // Move the atom table out alongside the solver (the RUP checker reads the original clauses
    // back from the solver via `original_clauses()`, so no clone of the CNF is needed).
    let (mut solver, atom_of) = cnf.into_solver_with_atoms();
    // The complete search runs FIRST, conflict-budgeted: an easy instance — the overwhelming
    // majority of equivalence/BMC obligations — gets its verdict in microseconds and never pays
    // for the terminal rung below. Only an instance that exhausts the budget (the exponential
    // families the rung exists for) escalates.
    let first = match solver.solve_budgeted(EASY_SEARCH_CONFLICTS) {
        BudgetedResult::Sat(model) => SolveResult::Sat(model),
        BudgetedResult::Unsat => SolveResult::Unsat,
        BudgetedResult::Budget => {
            // The instance has EARNED escalation: the cheap complete search could not decide it
            // within the budget, so it is exactly the class the expensive certified cuts exist
            // for. Easy obligations never reach any of these.
            //
            // Covering / cardinality / algebraic collapse: auto-discover a covering *symmetry* (the
            // pigeonhole class beyond the strict recognizer), an asymmetric cardinality cover (the
            // mutilated-chessboard class), or a parity collapse, and certify it via a synthesized
            // Lyapunov descent. The census proves it closes 756 minimal-UNSAT families at n=4 that
            // every narrow upfront cut is blind to. Fail-closed.
            if refutes_by_collapse(e) {
                return UnsatOutcome::Refuted;
            }
            // Nullstellensatz: the universal *algebraic* cut. Where the structural recognizers each
            // match one shape, this asks the shape-free question — is `1` in the low-degree
            // GF(2)-span of the clause polynomials? — and certifies any instance with a low-degree
            // algebraic refutation, the census's "rigid" residue that nonetheless has a degree-2/3
            // certificate. Size-gated; sound (a certificate implies UNSAT).
            if refutes_by_nullstellensatz(e) {
                return UnsatOutcome::Refuted;
            }
            // Polynomial Calculus: the *dynamic* algebraic cut, one rung above Nullstellensatz. By
            // closing the clause polynomials under linear combination AND multiply-by-variable
            // (intermediate cancellations and all), it certifies at the same gated degree a strict
            // superset of NS — the thin but real sliver NS leaves behind. Runs only after NS
            // declines; size-gated; sound.
            if refutes_by_polynomial_calculus(e) {
                return UnsatOutcome::Refuted;
            }
            // Certified RECURSIVE symmetry break (the terminal certified rung). Detect the residual
            // automorphism group of the current database, certify ONE lex-leader lead clause as a PR
            // step, re-detect, and loop to a fixpoint — breaking the COMPLETE group rather than the
            // adjacent positive-row swaps the single-pass break above sees (e.g. clique-coloring's
            // COLOR-permutation symmetry, a column swap the row break is structurally blind to). The
            // composed stream (certified PR breaks + closing RUP learned clauses) is
            // `check_pr_refutation`-verified against this formula's CNF inside `certified_unsat_auto`,
            // so a `refuted` here is a fully checked refutation. Size-gated — larger instances fall to
            // the cheap single-pass row break — and only claims UNSAT (a satisfiable formula leaves
            // `refuted == false`, so its model is recovered by resuming the search).
            let mut base = Cnf::new();
            if base.assert(e).is_some()
                && base.num_vars() <= CERTIFIED_SYMMETRY_VAR_CAP
                && crate::sym_certify::certified_unsat_auto(base.num_vars(), base.clauses()).refuted
            {
                return UnsatOutcome::Refuted;
            }
            // Complete the search on a FRESH solver over the original clauses: a budget-stopped
            // solver is not re-enterable (inprocessing assumes a virgin DB), and rebuilding keeps
            // the refutation certificate self-contained — the RUP checker replays only clauses
            // this solve derived, against only the formula's own clauses.
            let originals: Vec<Vec<Lit>> = solver.original_clauses().to_vec();
            solver = Solver::new(num_vars);
            for c in originals {
                solver.add_clause(c);
            }
            solver.solve()
        }
    };
    match first {
        SolveResult::Sat(model) => UnsatOutcome::Sat(decode_model_from(&atom_of, &model, &[e])),
        SolveResult::Unsat => {
            let learned: Vec<Vec<Lit>> = solver.learned().iter().map(|c| c.lits.clone()).collect();
            if rup::check_refutation(num_vars, solver.original_clauses(), &learned) {
                UnsatOutcome::Refuted
            } else {
                UnsatOutcome::Unsupported
            }
        }
    }
}

/// Decode a SAT `model` back to `(atom, value)` bindings for every source atom appearing in
/// `exprs` (Tseitin auxiliaries carry no source meaning and are skipped), sorted by name.
pub fn decode_model(cnf: &Cnf, model: &[bool], exprs: &[&ProofExpr]) -> Vec<(String, bool)> {
    let mut atoms = BTreeSet::new();
    for e in exprs {
        collect_atoms(e, &mut atoms);
    }
    atoms
        .into_iter()
        .filter_map(|name| {
            cnf.atom_value(&ProofExpr::Atom(name.clone()), model)
                .map(|v| (name, v))
        })
        .collect()
}

/// Decode a SAT `model` from a pre-extracted atom→variable map (the table `Cnf` already holds),
/// so a solve need not clone the whole clause database just to read its model back. Equivalent to
/// [`decode_model`] for the propositional atoms `collect_atoms` gathers.
pub fn decode_model_from(
    atom_of: &HashMap<String, Var>,
    model: &[bool],
    exprs: &[&ProofExpr],
) -> Vec<(String, bool)> {
    let mut atoms = BTreeSet::new();
    for e in exprs {
        collect_atoms(e, &mut atoms);
    }
    atoms
        .into_iter()
        .filter_map(|name| {
            // Atoms are interned under the `atom:` key (see `cnf::atom_key`).
            atom_of
                .get(&format!("atom:{name}"))
                .and_then(|&v| model.get(v as usize).copied())
                .map(|val| (name, val))
        })
        .collect()
}

/// Collect the names of every propositional [`ProofExpr::Atom`] reachable through the
/// Boolean fragment (`∧ ∨ ¬ → ↔`). Non-Boolean nodes are ignored — they cannot appear in a
/// bounded hardware obligation, and silently skipping them keeps the decode total.
fn collect_atoms(e: &ProofExpr, out: &mut BTreeSet<String>) {
    match e {
        ProofExpr::Atom(name) => {
            out.insert(name.clone());
        }
        ProofExpr::Not(p) => collect_atoms(p, out),
        ProofExpr::And(p, q)
        | ProofExpr::Or(p, q)
        | ProofExpr::Implies(p, q)
        | ProofExpr::Iff(p, q) => {
            collect_atoms(p, out);
            collect_atoms(q, out);
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn atom(s: &str) -> ProofExpr {
        ProofExpr::Atom(s.to_string())
    }
    fn not(e: ProofExpr) -> ProofExpr {
        ProofExpr::Not(Box::new(e))
    }
    fn and(a: ProofExpr, b: ProofExpr) -> ProofExpr {
        ProofExpr::And(Box::new(a), Box::new(b))
    }
    fn or(a: ProofExpr, b: ProofExpr) -> ProofExpr {
        ProofExpr::Or(Box::new(a), Box::new(b))
    }
    fn implies(a: ProofExpr, b: ProofExpr) -> ProofExpr {
        ProofExpr::Implies(Box::new(a), Box::new(b))
    }

    /// Evaluate a Boolean `ProofExpr` under an assignment — the independent oracle that
    /// proves a returned counterexample genuinely distinguishes two formulas (robust-to-
    /// absurdity: we never trust the solver's model without re-checking it ourselves).
    fn eval(e: &ProofExpr, env: &[(String, bool)]) -> bool {
        match e {
            ProofExpr::Atom(n) => env.iter().find(|(k, _)| k == n).map(|(_, v)| *v).unwrap_or(false),
            ProofExpr::Not(p) => !eval(p, env),
            ProofExpr::And(p, q) => eval(p, env) && eval(q, env),
            ProofExpr::Or(p, q) => eval(p, env) || eval(q, env),
            ProofExpr::Implies(p, q) => !eval(p, env) || eval(q, env),
            ProofExpr::Iff(p, q) => eval(p, env) == eval(q, env),
            _ => panic!("non-boolean node in test eval"),
        }
    }

    #[test]
    fn reflexive_equivalence_is_certified() {
        // `req |-> ack` at t0 against itself — the trivial but load-bearing identity.
        let f = implies(atom("req@0"), atom("ack@0"));
        assert_eq!(prove_equivalence(&f, &f), EquivOutcome::Equivalent);
    }

    #[test]
    fn de_morgan_is_equivalent() {
        // ¬(a ∧ b) ≡ (¬a ∨ ¬b)
        let lhs = not(and(atom("a@0"), atom("b@0")));
        let rhs = or(not(atom("a@0")), not(atom("b@0")));
        assert_eq!(prove_equivalence(&lhs, &rhs), EquivOutcome::Equivalent);
    }

    #[test]
    fn distributivity_is_equivalent() {
        // a ∧ (b ∨ c) ≡ (a ∧ b) ∨ (a ∧ c)
        let lhs = and(atom("a@0"), or(atom("b@0"), atom("c@0")));
        let rhs = or(and(atom("a@0"), atom("b@0")), and(atom("a@0"), atom("c@0")));
        assert_eq!(prove_equivalence(&lhs, &rhs), EquivOutcome::Equivalent);
    }

    #[test]
    fn distinct_tautologies_are_equivalent() {
        // Two excluded-middle tautologies over different atoms are both constantly true,
        // so they are equivalent even with no shared variables.
        let lhs = or(atom("p@0"), not(atom("p@0")));
        let rhs = or(atom("q@0"), not(atom("q@0")));
        assert_eq!(prove_equivalence(&lhs, &rhs), EquivOutcome::Equivalent);
    }

    #[test]
    fn implication_is_not_its_consequent() {
        // `req → ack`  vs  `ack` differ: at req=0, ack=0 the implication holds but ack does
        // not. The verdict must be Differ AND the counterexample must genuinely distinguish.
        let f = implies(atom("req@0"), atom("ack@0"));
        let s = atom("ack@0");
        match prove_equivalence(&f, &s) {
            EquivOutcome::Differ(model) => {
                assert_ne!(
                    eval(&f, &model),
                    eval(&s, &model),
                    "counterexample {:?} must distinguish the two formulas",
                    model
                );
            }
            other => panic!("expected Differ, got {:?}", other),
        }
    }

    #[test]
    fn implication_is_not_its_converse() {
        // `req → ack` vs `ack → req` differ; verify the witness is real.
        let f = implies(atom("req@0"), atom("ack@0"));
        let s = implies(atom("ack@0"), atom("req@0"));
        match prove_equivalence(&f, &s) {
            EquivOutcome::Differ(model) => {
                assert_ne!(eval(&f, &model), eval(&s, &model));
            }
            other => panic!("expected Differ, got {:?}", other),
        }
    }

    #[test]
    fn find_model_of_contradiction_is_unsat() {
        assert_eq!(find_model(&and(atom("a@0"), not(atom("a@0")))), ModelOutcome::Unsat);
    }

    #[test]
    fn find_model_of_satisfiable_returns_witness() {
        // a ∧ (a → b) forces a=true, b=true.
        let e = and(atom("a@0"), implies(atom("a@0"), atom("b@0")));
        match find_model(&e) {
            ModelOutcome::Sat(model) => {
                assert!(eval(&e, &model), "returned model must actually satisfy the formula");
                assert!(model.iter().any(|(k, v)| k == "a@0" && *v));
                assert!(model.iter().any(|(k, v)| k == "b@0" && *v));
            }
            other => panic!("expected Sat, got {:?}", other),
        }
    }
}
