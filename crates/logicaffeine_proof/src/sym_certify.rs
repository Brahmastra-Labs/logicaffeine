//! Certified symmetry breaking — the centerpiece. Given a formula and a set of verified
//! symmetry generators, add lex-leader symmetry-breaking predicates as **PR steps** (each
//! self-checked, fail-closed), solve the augmented formula, and emit a single composed
//! refutation that an independent checker accepts against the *original* formula alone.
//!
//! This closes the soundness gap of the old path (where the symmetry-broken formula was RUP-
//! certified but the model-removing addition was only argued informally): here every SBP clause
//! carries a propagation-redundancy witness derived from its symmetry, so the whole UNSAT
//! result — symmetry steps included — is machine-checkable. The decisive wiring invariant is
//! that `check_pr_refutation` runs against `formula` ALONE; the SBP clauses appear only as PR
//! steps, never as free original clauses.

use crate::cdcl::{Lit, SolveResult, Solver, Var};
use crate::pr::{check_pr_refutation, is_pr};
use crate::proof::{Perm, ProofStep, Witness};
use crate::symmetry_detect::find_generators;

/// The outcome of a certified-symmetry-breaking solve.
#[derive(Clone, Debug)]
pub struct CertifiedRefutation {
    /// Whether the formula was refuted (proven UNSAT) AND the composed PR proof checks.
    pub refuted: bool,
    /// How many lex-leader SBP clauses were PR-certified and added.
    pub sbp_clauses: usize,
    /// The composed proof stream: PR symmetry steps followed by RUP learned clauses.
    pub steps: Vec<ProofStep>,
}

/// The first-index lex-leader clause for `sigma`: over the smallest variable `v` that `sigma`
/// moves, assert `v ⟹ sigma(v)` (the leading bit of `x ≤ₗₑₓ sigma(x)`). Returns `None` if
/// `sigma` is the identity.
fn lex_leader_lead_clause(num_vars: usize, sigma: &Perm) -> Option<Vec<Lit>> {
    for v in 0..num_vars as Var {
        let image = sigma.apply(Lit::pos(v));
        if image != Lit::pos(v) {
            return Some(vec![Lit::neg(v), image]);
        }
    }
    None
}

/// Solve `formula` with certified symmetry breaking under the given `generators` (each must be
/// a genuine automorphism — they are re-checked implicitly by the per-clause PR self-check).
///
/// For each generator we propose its lead lex-leader clause, PR-self-check it against the
/// database built so far (so a generator invalidated by earlier SBPs is simply skipped), add
/// the survivors as PR steps, solve the augmented formula, and — if UNSAT — append the solver's
/// learned clauses as RUP steps. The composed stream is verified against `formula` alone.
pub fn certified_unsat(num_vars: usize, formula: &[Vec<Lit>], generators: &[Perm]) -> CertifiedRefutation {
    let mut db: Vec<Vec<Lit>> = formula.to_vec();
    let mut steps: Vec<ProofStep> = Vec::new();

    for sigma in generators {
        let Some(clause) = lex_leader_lead_clause(num_vars, sigma) else { continue };
        let witness = Witness::Substitution(sigma.clone());
        if is_pr(num_vars, &db, &clause, &witness) {
            db.push(clause.clone());
            steps.push(ProofStep::Pr { clause, witness });
        }
    }
    let sbp_clauses = steps.len();

    // Solve the augmented formula F ∧ SBP and collect its learned clauses as RUP steps.
    let mut solver = Solver::new(num_vars);
    for c in &db {
        solver.add_clause(c.clone());
    }
    let refuted = match solver.solve() {
        SolveResult::Sat(_) => false,
        SolveResult::Unsat => {
            for lc in solver.learned() {
                steps.push(ProofStep::Rup(lc.lits.clone()));
            }
            // The decisive check: the whole stream is replayed against the ORIGINAL formula.
            check_pr_refutation(num_vars, formula, &steps)
        }
    };

    CertifiedRefutation { refuted, sbp_clauses, steps }
}

/// A safety cap on the number of symmetry-breaking rounds — far above any real need (the group
/// is finite and strictly shrinks each round), a guard against pathological inputs.
const MAX_SBP_ROUNDS: usize = 100_000;

/// Solve `formula` with FULL certified symmetry breaking, discovering the symmetries itself.
///
/// Each round: detect the residual symmetry group of the current database, certify ONE lead
/// lex-leader predicate as a PR step (always sound — its generator is a fresh automorphism of
/// the current database, so the SR check's conflict is immediate), and re-detect. Adding the
/// predicate strictly shrinks the automorphism group, so the loop terminates with the whole group
/// broken; then the augmented formula is solved and the learned clauses appended as RUP steps. The
/// entire composed stream is verified against `formula` ALONE.
///
/// This breaks the *complete* group rather than one clause per generator, by re-detecting the
/// stabilizer after each predicate — the natural "lift and shift" of detection + the SR checker.
pub fn certified_unsat_auto(num_vars: usize, formula: &[Vec<Lit>]) -> CertifiedRefutation {
    let mut db: Vec<Vec<Lit>> = formula.to_vec();
    let mut steps: Vec<ProofStep> = Vec::new();

    for _ in 0..MAX_SBP_ROUNDS {
        let mut progressed = false;
        for sigma in find_generators(num_vars, &db) {
            let Some(clause) = lex_leader_lead_clause(num_vars, &sigma) else { continue };
            let witness = Witness::Substitution(sigma);
            if is_pr(num_vars, &db, &clause, &witness) {
                db.push(clause.clone());
                steps.push(ProofStep::Pr { clause, witness });
                progressed = true;
                break;
            }
        }
        if !progressed {
            break;
        }
    }
    let sbp_clauses = steps.len();

    let mut solver = Solver::new(num_vars);
    for c in &db {
        solver.add_clause(c.clone());
    }
    let refuted = match solver.solve() {
        SolveResult::Sat(_) => false,
        SolveResult::Unsat => {
            for lc in solver.learned() {
                steps.push(ProofStep::Rup(lc.lits.clone()));
            }
            check_pr_refutation(num_vars, formula, &steps)
        }
    };

    CertifiedRefutation { refuted, sbp_clauses, steps }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cdcl::Lit;
    use crate::families;
    use crate::symmetry_detect::perm_is_automorphism;

    /// Swap two pigeon rows of PHP(n) — a known automorphism, used to feed the certifier known
    /// generators while the general detector is built out separately.
    fn swap_pigeon_rows(n: usize, p0: usize, p1: usize) -> Perm {
        let holes = n - 1;
        Perm::from_images(
            (0..n * holes)
                .map(|v| {
                    let (p, h) = (v / holes, v % holes);
                    let np = if p == p0 {
                        p1
                    } else if p == p1 {
                        p0
                    } else {
                        p
                    };
                    Lit::pos((np * holes + h) as u32)
                })
                .collect(),
        )
    }

    #[test]
    fn php3_is_refuted_with_a_pr_certified_symmetry_proof() {
        let (cnf, _) = families::php(3);
        // Adjacent pigeon-row swaps generate the pigeon symmetry group S_3.
        let gens: Vec<Perm> = [(0usize, 1usize), (1, 2)].iter().map(|&(a, b)| swap_pigeon_rows(3, a, b)).collect();
        for g in &gens {
            assert!(perm_is_automorphism(&cnf.clauses, g), "fed generators must be real symmetries");
        }

        let result = certified_unsat(cnf.num_vars, &cnf.clauses, &gens);
        assert!(result.refuted, "PHP(3) must be refuted and the composed PR proof must check");
        assert!(result.sbp_clauses >= 1, "at least one symmetry-breaking predicate was certified");
        // Independent re-check of the full composed stream against the ORIGINAL formula alone.
        assert!(check_pr_refutation(cnf.num_vars, &cnf.clauses, &result.steps));
    }

    #[test]
    fn php4_is_refuted_with_a_pr_certified_symmetry_proof() {
        let (cnf, _) = families::php(4);
        let gens: Vec<Perm> =
            [(0usize, 1usize), (1, 2), (2, 3)].iter().map(|&(a, b)| swap_pigeon_rows(4, a, b)).collect();
        let result = certified_unsat(cnf.num_vars, &cnf.clauses, &gens);
        assert!(result.refuted);
        assert!(result.sbp_clauses >= 1);
        assert!(check_pr_refutation(cnf.num_vars, &cnf.clauses, &result.steps));
    }

    #[test]
    fn a_bogus_generator_is_not_certified_but_the_refutation_still_holds() {
        // Feed a NON-symmetry: its lead clause must fail the PR self-check and be dropped, yet
        // the formula is still refuted (by RUP on the learned clauses) and the proof checks.
        let (cnf, _) = families::php(3);
        let holes = 2;
        let bogus = Perm::from_images(
            (0..cnf.num_vars)
                .map(|v| {
                    let (p, h) = (v / holes, v % holes);
                    Lit::pos((if p == 0 { 1 } else { p } * holes + h) as u32)
                })
                .collect(),
        );
        assert!(!perm_is_automorphism(&cnf.clauses, &bogus));
        let result = certified_unsat(cnf.num_vars, &cnf.clauses, &[bogus]);
        assert_eq!(result.sbp_clauses, 0, "a non-symmetry yields no certified SBP");
        assert!(result.refuted, "the formula is still refuted, soundly");
        assert!(check_pr_refutation(cnf.num_vars, &cnf.clauses, &result.steps));
    }

    #[test]
    fn php_is_refuted_with_auto_discovered_generators() {
        // The full pipeline with NO hand-fed generators: detect symmetries, certify the SBPs as
        // PR steps, solve, and machine-check the composed refutation against the original formula.
        use crate::symmetry_detect::find_generators;
        for n in 3..=4 {
            let (cnf, _) = families::php(n);
            let gens = find_generators(cnf.num_vars, &cnf.clauses);
            let result = certified_unsat(cnf.num_vars, &cnf.clauses, &gens);
            assert!(result.refuted, "PHP({n}) refuted via discovered symmetries");
            assert!(result.sbp_clauses >= 1, "at least one SBP certified from a discovered generator");
            assert!(check_pr_refutation(cnf.num_vars, &cnf.clauses, &result.steps));
        }
    }

    // --- full iterative symmetry breaking (certified_unsat_auto) ---

    fn pr_clauses(steps: &[ProofStep]) -> Vec<Vec<Lit>> {
        steps
            .iter()
            .filter_map(|s| if let ProofStep::Pr { clause, .. } = s { Some(clause.clone()) } else { None })
            .collect()
    }

    #[test]
    fn auto_breaks_the_whole_group_and_refutes_php() {
        for n in 3..=4 {
            let (cnf, _) = families::php(n);
            let r = certified_unsat_auto(cnf.num_vars, &cnf.clauses);
            assert!(r.refuted, "PHP({n}) refuted");
            assert!(r.sbp_clauses >= 2, "a real chain, not one clause (n={n}, got {})", r.sbp_clauses);
            assert!(check_pr_refutation(cnf.num_vars, &cnf.clauses, &r.steps), "composed PR proof checks");
            // The whole symmetry group must be gone: F + SBP has no non-trivial automorphism.
            let mut full = cnf.clauses.clone();
            full.extend(pr_clauses(&r.steps));
            assert!(
                find_generators(cnf.num_vars, &full).iter().all(|g| g.is_identity()),
                "every symmetry of PHP({n}) is broken"
            );
        }
    }

    #[test]
    fn auto_on_an_asymmetric_unsat_formula_adds_no_sbp_but_refutes() {
        // (a) ∧ (¬a ∨ b) ∧ (¬b): forces a, then b, then ¬b — UNSAT, and asymmetric (a and b play
        // different roles, no phase symmetry preserves the set). So no SBP, refuted by RUP alone.
        let f = vec![vec![Lit::pos(0)], vec![Lit::neg(0), Lit::pos(1)], vec![Lit::neg(1)]];
        let r = certified_unsat_auto(2, &f);
        assert_eq!(r.sbp_clauses, 0, "no symmetry to break");
        assert!(r.refuted);
        assert!(check_pr_refutation(2, &f, &r.steps));
    }

    #[test]
    fn auto_does_not_refute_a_satisfiable_symmetric_formula() {
        // Exactly-one(a,b): satisfiable, symmetric under a↔b. Breaking the symmetry is sound but
        // there is no refutation — the result must NOT claim one.
        let f = vec![vec![Lit::pos(0), Lit::pos(1)], vec![Lit::neg(0), Lit::neg(1)]];
        let r = certified_unsat_auto(2, &f);
        assert!(!r.refuted, "a satisfiable formula is never refuted");
    }

    #[test]
    fn auto_handles_a_lone_empty_clause() {
        // PHP(1) is a single empty clause over zero variables — immediately UNSAT, the degenerate
        // edge for the symmetry machinery (num_vars == 0 short-circuits the finder).
        let (cnf, _) = families::php(1);
        assert_eq!(cnf.num_vars, 0);
        let r = certified_unsat_auto(cnf.num_vars, &cnf.clauses);
        assert_eq!(r.sbp_clauses, 0);
        assert!(r.refuted);
        assert!(check_pr_refutation(cnf.num_vars, &cnf.clauses, &r.steps));
    }

    #[test]
    fn auto_is_deterministic() {
        let (cnf, _) = families::php(3);
        let a = certified_unsat_auto(cnf.num_vars, &cnf.clauses);
        let b = certified_unsat_auto(cnf.num_vars, &cnf.clauses);
        assert_eq!(a.sbp_clauses, b.sbp_clauses, "no wall-clock or hashing nondeterminism");
        assert_eq!(a.steps.len(), b.steps.len());
    }
}
