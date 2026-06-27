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

use crate::cdcl::{Lit, SolveResult, Var};
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
    match solver.solve() {
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
