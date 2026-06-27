//! The **PR** (propagation-redundancy) checker — the trust tier for *model-removing* clause
//! additions, and the keystone that makes certified symmetry breaking possible.
//!
//! RUP ([`crate::rup`]) certifies only clauses *implied* by the formula. A symmetry-breaking
//! predicate is different in kind: it deletes satisfying assignments (keeping at least one per
//! symmetry orbit), so it is satisfiability-preserving but **not** implied — RUP must and does
//! reject it. Propagation redundancy (Heule, Kiesl & Biere, CADE 2017) closes exactly this
//! gap: a clause `C` is redundant w.r.t. `F` with witness `ω` iff `ω` satisfies `C` and
//! `F|α ⊢₁ F|ω`, where `α = ¬C` is the assignment falsifying `C` and `⊢₁` is implication by
//! unit propagation. Adding such a `C` preserves satisfiability, with an independently
//! checkable certificate.
//!
//! Two witness forms are checked, each with its own well-specified criterion:
//!
//! - [`Witness::Assignment`] `ω` — the classic PR criterion `ω ⊨ C` and `F|α ⊢₁ F|ω`. This is
//!   the general redundancy tier (vivification, BVE, …).
//! - [`Witness::Substitution`] `σ` — the **substitution-redundancy** criterion for a symmetry.
//!   Repair a model `τ` of `F ∧ ¬C` by composing with `σ`: the result `τ∘σ` satisfies `F`
//!   automatically when `σ(F) = F`, and satisfies `C` exactly when `τ ⊨ σ(C)`. So `C` is
//!   redundant iff **`σ` is an automorphism of the database AND `F ∧ ¬C ⊢₁ σ(C)`** (Heule &
//!   Biere, substitution redundancy). The automorphism is checked against the *current*
//!   database, so an earlier predicate that breaks a later generator's symmetry simply makes
//!   that check fail — fail-closed, no generator-ordering hazard.
//!
//! The checker reuses the very same tiny unit-propagation core as [`crate::rup`]: its
//! simplicity IS the trust, and it is wrapped by a brute-force equisatisfiability oracle over
//! BOTH witness forms in the tests so it can never bless a clause that turns a satisfiable
//! formula unsatisfiable.

use crate::cdcl::Lit;
use crate::proof::{ProofStep, Witness};
use crate::rup;

/// Verify a refutation made of [`ProofStep`]s over `original`. Each step's added clause must
/// check (RUP or PR) against the database built so far; then the empty clause must be RUP.
/// Returns `false` if any step fails to check — a bogus or unsound proof is rejected.
pub fn check_pr_refutation(num_vars: usize, original: &[Vec<Lit>], steps: &[ProofStep]) -> bool {
    let mut db: Vec<Vec<Lit>> = original.to_vec();
    for step in steps {
        match step {
            ProofStep::Rup(c) => {
                if !rup::is_rup(num_vars, &db, c) {
                    return false;
                }
                db.push(c.clone());
            }
            ProofStep::Pr { clause, witness } => {
                if !is_pr(num_vars, &db, clause, witness) {
                    return false;
                }
                db.push(clause.clone());
            }
        }
    }
    rup::is_rup(num_vars, &db, &[])
}

/// Is `clause` propagation-redundant w.r.t. `db` under `witness`?
pub fn is_pr(num_vars: usize, db: &[Vec<Lit>], clause: &[Lit], witness: &Witness) -> bool {
    match witness {
        Witness::Assignment(omega) => assignment_pr(num_vars, db, clause, omega),
        Witness::Substitution(sigma) => substitution_sr(num_vars, db, clause, sigma),
    }
}

/// The substitution-redundancy criterion for a symmetry witness `σ`: `σ` must be an
/// automorphism of `db` (so the σ-repair of any model stays a model), and `db ∧ ¬C ⊢₁ σ(C)`
/// (so that repair also satisfies `C`). Both conditions are independently checkable, and the
/// automorphism is re-verified against the *current* `db` — fail-closed against generator
/// ordering.
fn substitution_sr(num_vars: usize, db: &[Vec<Lit>], clause: &[Lit], sigma: &crate::proof::Perm) -> bool {
    if !crate::symmetry_detect::perm_is_automorphism(db, sigma) {
        return false;
    }
    // `db ∧ ¬C ⊢₁ σ(C)`: assume α = ¬C together with ¬σ(C) and propagate db; require a conflict.
    let sigma_c = sigma.apply_clause(clause);
    let mut assume: Vec<Lit> = clause.iter().map(|l| l.negated()).collect();
    assume.extend(sigma_c.iter().map(|l| l.negated()));
    up_conflict(num_vars, db, &assume)
}

/// The propagation-redundancy criterion with an explicit assignment witness `ω` (given as the
/// set of literals it sets true): `ω` satisfies `C`, and for every clause `D ∈ db`,
/// `F|α ⊢₁ D|ω` where `α = ¬C`.
fn assignment_pr(num_vars: usize, db: &[Vec<Lit>], clause: &[Lit], omega_true: &[Lit]) -> bool {
    // Materialise ω as an assignment; a self-contradictory witness is no witness.
    let mut omega: Vec<Option<bool>> = vec![None; num_vars];
    for &l in omega_true {
        if !rup::set_true(&mut omega, l) {
            return false;
        }
    }
    // Precondition: ω must satisfy the clause being added.
    if !clause.iter().any(|&l| rup::lit_val(&omega, l) == Some(true)) {
        return false;
    }
    // α = ¬C, as the set of literals it sets true.
    let alpha: Vec<Lit> = clause.iter().map(|&l| l.negated()).collect();

    for d in db {
        // Clauses ω satisfies carry no obligation (D|ω is a tautology).
        if d.iter().any(|&m| rup::lit_val(&omega, m) == Some(true)) {
            continue;
        }
        // D|ω keeps the ω-unassigned literals (ω-false ones are dropped). The obligation
        // `F|α ⊢₁ D|ω` is checked by assuming α together with ¬(D|ω) and propagating db: it
        // must conflict.
        let mut assume = alpha.clone();
        for &m in d {
            if rup::lit_val(&omega, m).is_none() {
                assume.push(m.negated());
            }
        }
        if !up_conflict(num_vars, db, &assume) {
            return false;
        }
    }
    true
}

/// Assume every literal of `assume` and unit-propagate `db`; `true` iff a conflict results
/// (an immediate clash among the assumptions counts).
fn up_conflict(num_vars: usize, db: &[Vec<Lit>], assume: &[Lit]) -> bool {
    let mut assign: Vec<Option<bool>> = vec![None; num_vars];
    for &l in assume {
        if !rup::set_true(&mut assign, l) {
            return true;
        }
    }
    rup::propagate(db, &mut assign)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cdcl::Lit;
    use crate::proof::Perm;

    fn p(v: u32) -> Lit {
        Lit::pos(v)
    }
    fn n(v: u32) -> Lit {
        Lit::neg(v)
    }

    /// Brute-force satisfiability over `num_vars` variables — the independent oracle.
    fn sat_brute(num_vars: usize, clauses: &[Vec<Lit>]) -> bool {
        for mask in 0u32..(1u32 << num_vars) {
            let model: Vec<bool> = (0..num_vars).map(|v| (mask >> v) & 1 == 1).collect();
            if clauses.iter().all(|c| c.iter().any(|l| model[l.var() as usize] == l.is_positive())) {
                return true;
            }
        }
        false
    }

    #[test]
    fn pr_accepts_a_model_removing_clause_that_rup_rejects() {
        // F = (a ∨ b) is satisfiable and symmetric under a↔b. The lex-leader C = (¬a ∨ b)
        // deletes the model a=1,b=0 — model-removing, so NOT RUP — but is satisfiability-
        // preserving, certified by the assignment witness ω = {a=0, b=1}.
        let (a, b) = (0u32, 1u32);
        let f = vec![vec![p(a), p(b)]];
        let c = vec![n(a), p(b)];

        assert!(!rup::is_rup(2, &f, &c), "a model-removing clause is not RUP");
        let omega = Witness::Assignment(vec![n(a), p(b)]);
        assert!(is_pr(2, &f, &c, &omega), "but it IS propagation-redundant");
        // And the addition really is satisfiability-preserving.
        let mut fc = f.clone();
        fc.push(c.clone());
        assert_eq!(sat_brute(2, &f), sat_brute(2, &fc));
    }

    #[test]
    fn pr_accepts_via_a_substitution_witness() {
        // The same break, certified by the symmetry σ = (a↔b) instead of a hand-written ω.
        let (a, b) = (0u32, 1u32);
        let f = vec![vec![p(a), p(b)]];
        let c = vec![n(a), p(b)];
        let sigma = Perm::from_images(vec![p(b), p(a)]); // +a↦+b, +b↦+a
        assert!(is_pr(2, &f, &c, &Witness::Substitution(sigma)));
    }

    #[test]
    fn pr_rejects_a_witness_that_does_not_satisfy_the_clause() {
        // ω = α itself (a=1,b=0) falsifies C, so it is not a witness — reject, fail-closed.
        let (a, b) = (0u32, 1u32);
        let f = vec![vec![p(a), p(b)]];
        let c = vec![n(a), p(b)];
        assert!(!is_pr(2, &f, &c, &Witness::Assignment(vec![p(a), n(b)])));
    }

    #[test]
    fn pr_rejects_a_bogus_substitution_on_an_asymmetric_formula() {
        // F = (a ∨ b) ∧ (¬b) forces the unique model a=1,b=0; it is NOT symmetric under a↔b
        // (σ maps ¬b to ¬a, which is not a clause of F). The lex-leader C = (¬a ∨ b) deletes
        // that only model, so blindly trusting σ would flip SAT→UNSAT — PR must reject it.
        let (a, b) = (0u32, 1u32);
        let f = vec![vec![p(a), p(b)], vec![n(b)]];
        let c = vec![n(a), p(b)];
        let sigma = Perm::from_images(vec![p(b), p(a)]);
        assert!(!is_pr(2, &f, &c, &Witness::Substitution(sigma)));
        // Confirm the trap: adding C really would have been unsound.
        let mut fc = f.clone();
        fc.push(c.clone());
        assert!(sat_brute(2, &f) && !sat_brute(2, &fc), "C would flip SAT→UNSAT");
    }

    #[test]
    fn check_pr_refutation_agrees_with_rup_on_pure_rup_proofs() {
        // With only RUP steps, the PR driver must behave exactly like the RUP checker.
        let (a, b) = (0u32, 1u32);
        let f = vec![
            vec![p(a), p(b)],
            vec![p(a), n(b)],
            vec![n(a), p(b)],
            vec![n(a), n(b)],
        ];
        let steps = vec![ProofStep::Rup(vec![p(a)]), ProofStep::Rup(vec![n(a)])];
        assert!(check_pr_refutation(2, &f, &steps));
        // Without the resolvents it cannot close.
        assert!(!check_pr_refutation(2, &f, &[]));
    }

    #[test]
    fn pr_never_blesses_an_unsound_addition_random() {
        // Robustness to the point of absurdity: over many seeded random tiny formulas and
        // random candidate (clause, witness) pairs, WHENEVER is_pr accepts, brute force must
        // confirm the addition preserved satisfiability. A single false-accept is a hard fail.
        let mut state = 0x9E3779B97F4A7C15u64;
        let mut next = || {
            // SplitMix64 — deterministic, no wall-clock seeding.
            state = state.wrapping_add(0x9E3779B97F4A7C15);
            let mut z = state;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
            z ^ (z >> 31)
        };
        let num_vars = 4usize;
        let rand_clause = |next: &mut dyn FnMut() -> u64| -> Vec<Lit> {
            let len = 1 + (next() as usize % 3);
            let mut c = Vec::new();
            for _ in 0..len {
                let v = (next() as u32) % num_vars as u32;
                let lit = Lit::new(v, next() & 1 == 0);
                if !c.contains(&lit) && !c.contains(&lit.negated()) {
                    c.push(lit);
                }
            }
            c
        };

        let mut accepted = 0;
        for _ in 0..20_000 {
            // Random small formula.
            let nclauses = next() as usize % 5;
            let f: Vec<Vec<Lit>> = (0..nclauses).map(|_| rand_clause(&mut next)).filter(|c| !c.is_empty()).collect();
            let c = rand_clause(&mut next);
            if c.is_empty() {
                continue;
            }
            // Random assignment witness over a random subset of variables.
            let mut omega = Vec::new();
            for v in 0..num_vars as u32 {
                if next() & 1 == 0 {
                    omega.push(Lit::new(v, next() & 1 == 0));
                }
            }
            if is_pr(num_vars, &f, &c, &Witness::Assignment(omega)) {
                accepted += 1;
                let before = sat_brute(num_vars, &f);
                let mut fc = f.clone();
                fc.push(c.clone());
                let after = sat_brute(num_vars, &fc);
                assert_eq!(before, after, "PR accepted a clause that changed satisfiability: F={f:?} C={c:?}");
            }
        }
        assert!(accepted > 0, "the test must actually exercise acceptances");
    }

    #[test]
    fn sr_never_blesses_an_unsound_addition_random() {
        // The substitution-redundancy path's soundness net: over many seeded random formulas,
        // random literal permutations, and random clauses, WHENEVER is_pr accepts under a
        // substitution witness, brute force must confirm the addition kept satisfiability.
        let mut state = 0xD1B54A32D192ED03u64;
        let mut next = || {
            state = state.wrapping_add(0x9E3779B97F4A7C15);
            let mut z = state;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
            z ^ (z >> 31)
        };
        let num_vars = 4usize;
        let rand_clause = |next: &mut dyn FnMut() -> u64| -> Vec<Lit> {
            let len = 1 + (next() as usize % 3);
            let mut c = Vec::new();
            for _ in 0..len {
                let v = (next() as u32) % num_vars as u32;
                let lit = Lit::new(v, next() & 1 == 0);
                if !c.contains(&lit) && !c.contains(&lit.negated()) {
                    c.push(lit);
                }
            }
            c
        };
        // A random literal permutation: a Fisher-Yates shuffle of variables, each image given a
        // random phase (identity a quarter of the time, to reliably exercise acceptances).
        let rand_perm = |next: &mut dyn FnMut() -> u64| -> Perm {
            if next() % 4 == 0 {
                return Perm::identity(num_vars);
            }
            let mut order: Vec<u32> = (0..num_vars as u32).collect();
            for i in (1..num_vars).rev() {
                let j = next() as usize % (i + 1);
                order.swap(i, j);
            }
            Perm::from_images((0..num_vars).map(|v| Lit::new(order[v], next() & 1 == 0)).collect())
        };

        let mut accepted = 0;
        for _ in 0..20_000 {
            let nclauses = next() as usize % 5;
            let f: Vec<Vec<Lit>> =
                (0..nclauses).map(|_| rand_clause(&mut next)).filter(|c| !c.is_empty()).collect();
            let c = rand_clause(&mut next);
            if c.is_empty() {
                continue;
            }
            let sigma = rand_perm(&mut next);
            if is_pr(num_vars, &f, &c, &Witness::Substitution(sigma)) {
                accepted += 1;
                let before = sat_brute(num_vars, &f);
                let mut fc = f.clone();
                fc.push(c.clone());
                let after = sat_brute(num_vars, &fc);
                assert_eq!(before, after, "SR accepted a clause that changed satisfiability: F={f:?} C={c:?}");
            }
        }
        assert!(accepted > 0, "the substitution path must actually be exercised");
    }
}
