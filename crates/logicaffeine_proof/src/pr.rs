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

/// The variables a proof over `original` speaks of: at least `num_vars`, and every variable the formula or a step
/// names — the variables a PR step introduces among them, as bounded variable addition's definitions do. A checker
/// sizes its database by this; each step is still checked against the clauses that name its variables, so a
/// variable past `num_vars` is no freer than any other.
pub(crate) fn proof_vars(num_vars: usize, original: &[Vec<Lit>], steps: &[ProofStep]) -> usize {
    let witnessed = steps.iter().flat_map(|s| match s {
        ProofStep::Pr { witness: Witness::Assignment(w), .. } => &w[..],
        _ => &[],
    });
    original
        .iter()
        .flatten()
        .chain(steps.iter().flat_map(|s| s.clause()))
        .chain(witnessed)
        .map(|l| l.var() as usize + 1)
        .max()
        .unwrap_or(0)
        .max(num_vars)
}

/// A forward proof checker by watched literals. Every clause a proof ever holds enters one propagator at the start,
/// switched on as the proof adds it and off as the proof deletes it, so a RUP check costs its propagation, not a scan of
/// the database. A PR step with an assignment witness `ω` is checked by the definition: a clause `D` that `ω` touches
/// only through falsified literals must follow by propagation from `¬C` with its `ω`-free literals negated, and every
/// other clause holds under `ω` as it is — satisfied, or untouched — so only the clauses holding a negated literal of
/// `ω` are visited. Any other witness is decided by [`is_pr`] over the clauses present.
pub(crate) struct Forward {
    num_vars: usize,
    /// The formula's clauses, then the proof's additions in order.
    clauses: Vec<Vec<Lit>>,
    propagator: crate::lrat::Propagator,
    active: Vec<bool>,
    /// Per literal index, the clauses holding it.
    holding: Vec<Vec<u32>>,
    /// The active clauses by their canonical form, for deletion.
    live: std::collections::HashMap<Vec<u32>, Vec<usize>>,
    /// The next addition's clause.
    next: usize,
}

impl Forward {
    pub(crate) fn new(num_vars: usize, original: &[Vec<Lit>], steps: &[ProofStep]) -> Forward {
        let num_vars = proof_vars(num_vars, original, steps);
        let mut clauses: Vec<Vec<Lit>> = original.to_vec();
        clauses.extend(steps.iter().filter(|s| !matches!(s, ProofStep::Delete(_))).map(|s| s.clause().to_vec()));
        let mut propagator = crate::lrat::Propagator::new(num_vars, clauses.iter());
        for ci in original.len()..clauses.len() {
            propagator.deactivate(ci);
        }
        let mut holding: Vec<Vec<u32>> = vec![Vec::new(); 2 * num_vars];
        for (ci, c) in clauses.iter().enumerate() {
            for l in c {
                holding[l.index()].push(ci as u32);
            }
        }
        let mut live: std::collections::HashMap<Vec<u32>, Vec<usize>> = std::collections::HashMap::new();
        for (ci, c) in original.iter().enumerate() {
            live.entry(canon_clause(c)).or_default().push(ci);
        }
        let active = (0..clauses.len()).map(|ci| ci < original.len()).collect();
        Forward { num_vars, clauses, propagator, active, holding, live, next: original.len() }
    }

    /// Whether `c` is RUP over the active clauses.
    pub(crate) fn rup(&mut self, c: &[Lit]) -> bool {
        self.propagator.refutes(c)
    }

    /// Whether `c` is propagation-redundant over the active clauses under `witness`.
    pub(crate) fn pr(&mut self, c: &[Lit], witness: &Witness) -> bool {
        let Witness::Assignment(omega) = witness else {
            return is_pr(self.num_vars, &self.present(), c, witness);
        };
        let value = |l: Lit| -> Option<bool> {
            if omega.contains(&l) {
                Some(true)
            } else if omega.contains(&l.negated()) {
                Some(false)
            } else {
                None
            }
        };
        if omega.iter().any(|&l| omega.contains(&l.negated())) || !c.iter().any(|&l| value(l) == Some(true)) {
            return false;
        }
        let mut visited: Vec<u32> = Vec::new();
        for &l in omega {
            let falsified = l.negated();
            for k in 0..self.holding.get(falsified.index()).map_or(0, |h| h.len()) {
                let d = self.holding[falsified.index()][k];
                if !self.active[d as usize] || visited.contains(&d) {
                    continue;
                }
                visited.push(d);
                let clause = &self.clauses[d as usize];
                if clause.iter().any(|&m| value(m) == Some(true)) {
                    continue;
                }
                let obligation: Vec<Lit> = c.iter().copied().chain(clause.iter().copied().filter(|&m| value(m).is_none())).collect();
                if !self.propagator.refutes(&obligation) {
                    return false;
                }
            }
        }
        true
    }

    /// Switch on the next addition's clause.
    pub(crate) fn admit(&mut self) {
        let ci = self.next;
        self.next += 1;
        self.active[ci] = true;
        self.propagator.activate(ci);
        self.live.entry(canon_clause(&self.clauses[ci])).or_default().push(ci);
    }

    /// Switch off one active clause equal to `c`, if any.
    pub(crate) fn delete(&mut self, c: &[Lit]) {
        if let Some(ci) = self.live.get_mut(&canon_clause(c)).and_then(|ids| ids.pop()) {
            self.active[ci] = false;
            self.propagator.deactivate(ci);
        }
    }

    /// The active clauses.
    pub(crate) fn present(&self) -> Vec<Vec<Lit>> {
        (0..self.clauses.len()).filter(|&ci| self.active[ci]).map(|ci| self.clauses[ci].clone()).collect()
    }
}

/// [`check_pr_refutation`] by the [`Forward`] checker: the same verdict, each step at the cost of its propagation.
fn check_pr_refutation_forward(num_vars: usize, original: &[Vec<Lit>], steps: &[ProofStep]) -> bool {
    let mut forward = Forward::new(num_vars, original, steps);
    for step in steps {
        match step {
            ProofStep::Rup(c) => {
                if !forward.rup(c) {
                    return false;
                }
                forward.admit();
            }
            ProofStep::Pr { clause, witness } => {
                if !forward.pr(clause, witness) {
                    return false;
                }
                forward.admit();
            }
            ProofStep::Delete(c) => forward.delete(c),
        }
    }
    forward.rup(&[])
}

/// Verify a refutation made of [`ProofStep`]s over `original`. Each step's added clause must
/// check (RUP or PR) against the database built so far; then the empty clause must be RUP.
/// Returns `false` if any step fails to check — a bogus or unsound proof is rejected. The steps may name variables
/// past `num_vars` ([`proof_vars`]).
pub fn check_pr_refutation(num_vars: usize, original: &[Vec<Lit>], steps: &[ProofStep]) -> bool {
    let num_vars = proof_vars(num_vars, original, steps);
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
            ProofStep::Delete(clause) => {
                // A deletion is unchecked (always sound); remove the first matching clause.
                let key = canon_clause(clause);
                if let Some(pos) = db.iter().position(|d| canon_clause(d) == key) {
                    db.swap_remove(pos);
                }
            }
        }
    }
    rup::is_rup(num_vars, &db, &[])
}

/// A clause's canonical form (sorted, deduped literal codes) for set-equality comparison.
fn canon_clause(c: &[Lit]) -> Vec<u32> {
    let mut k: Vec<u32> = c
        .iter()
        .map(|l| l.var() * 2 + u32::from(!l.is_positive()))
        .collect();
    k.sort_unstable();
    k.dedup();
    k
}

/// Is `clause` propagation-redundant w.r.t. `db` under `witness`?
pub fn is_pr(num_vars: usize, db: &[Vec<Lit>], clause: &[Lit], witness: &Witness) -> bool {
    match witness {
        Witness::Assignment(omega) => assignment_pr(num_vars, db, clause, omega),
        Witness::Substitution(sigma) => substitution_sr(num_vars, db, clause, sigma),
        Witness::PartialSymmetry(sigma) => partial_symmetry_sr(num_vars, db, clause, sigma),
    }
}

/// [`is_pr`] with the automorphism re-check served by a pre-built incremental
/// `AutomorphismIndex` (which must hold exactly the clauses of `db`). Identical verdict to
/// [`is_pr`] — the index is an acceleration structure — but the substitution path no longer
/// rebuilds the membership set per call, the difference between an `O(n⁴)` and an `O(n³)`
/// certified refutation.
pub fn is_pr_indexed(
    num_vars: usize,
    db: &[Vec<Lit>],
    index: &mut crate::symmetry_detect::AutomorphismIndex,
    clause: &[Lit],
    witness: &Witness,
) -> bool {
    match witness {
        Witness::Assignment(omega) => assignment_pr(num_vars, db, clause, omega),
        Witness::Substitution(sigma) => {
            if !index.is_automorphism(sigma) {
                return false;
            }
            let sigma_c = sigma.apply_clause(clause);
            let mut assume: Vec<Lit> = clause.iter().map(|l| l.negated()).collect();
            assume.extend(sigma_c.iter().map(|l| l.negated()));
            // Occurrence-driven propagation over the index, not a full-database scan.
            let _ = db;
            index.propagate_to_conflict(num_vars, &assume)
        }
        Witness::PartialSymmetry(sigma) => partial_symmetry_sr(num_vars, db, clause, sigma),
    }
}

/// [`check_pr_refutation`] accelerated by an incrementally-maintained `AutomorphismIndex` — the
/// same verdict, but each substitution step's automorphism re-check costs `O(support)` rather than
/// `O(|db|)`. Falls back to the stateless checker if the proof deletes clauses (the index is
/// append-only, and certified symmetry refutations do not delete).
pub fn check_pr_refutation_fast(
    num_vars: usize,
    original: &[Vec<Lit>],
    steps: &[ProofStep],
) -> bool {
    if steps.iter().any(|s| matches!(s, ProofStep::Delete(_))) {
        return check_pr_refutation_forward(num_vars, original, steps);
    }
    let num_vars = proof_vars(num_vars, original, steps);
    let mut db: Vec<Vec<Lit>> = original.to_vec();
    let mut index = crate::symmetry_detect::AutomorphismIndex::with_clauses(num_vars, original);
    for step in steps {
        match step {
            ProofStep::Rup(c) => {
                if !rup::is_rup(num_vars, &db, c) {
                    return false;
                }
                db.push(c.clone());
                index.insert(c.clone());
            }
            ProofStep::Pr { clause, witness } => {
                if !is_pr_indexed(num_vars, &db, &mut index, clause, witness) {
                    return false;
                }
                db.push(clause.clone());
                index.insert(clause.clone());
            }
            ProofStep::Delete(_) => unreachable!("deletes handled by the fallback above"),
        }
    }
    rup::is_rup(num_vars, &db, &[])
}

/// The substitution-redundancy criterion for a symmetry witness `σ`: `σ` must be an
/// automorphism of `db` (so the σ-repair of any model stays a model), and `db ∧ ¬C ⊢₁ σ(C)`
/// (so that repair also satisfies `C`). Both conditions are independently checkable, and the
/// automorphism is re-verified against the *current* `db` — fail-closed against generator
/// ordering.
fn substitution_sr(
    num_vars: usize,
    db: &[Vec<Lit>],
    clause: &[Lit],
    sigma: &crate::proof::Perm,
) -> bool {
    if !crate::symmetry_detect::perm_is_automorphism(db, sigma) {
        return false;
    }
    // `db ∧ ¬C ⊢₁ σ(C)`: assume α = ¬C together with ¬σ(C) and propagate db; require a conflict.
    let sigma_c = sigma.apply_clause(clause);
    let mut assume: Vec<Lit> = clause.iter().map(|l| l.negated()).collect();
    assume.extend(sigma_c.iter().map(|l| l.negated()));
    up_conflict(num_vars, db, &assume)
}

/// The substitution-redundancy criterion without the automorphism requirement (Buss & Thapen): the σ-image of every
/// clause of `db`, and of `C` itself, must follow by unit propagation from `db ∧ ¬C`. Then a model `τ` of `db ∧ ¬C`
/// satisfies every image, so `τ ∘ σ` satisfies `db ∧ C`, and adding `C` keeps the formula satisfiable. An image that is
/// already a clause of `db` follows at once; only the others are propagated. A mapping that names a variable outside
/// `num_vars`, or does not cover every variable the clauses use, certifies nothing.
fn partial_symmetry_sr(num_vars: usize, db: &[Vec<Lit>], clause: &[Lit], sigma: &crate::proof::Perm) -> bool {
    let in_range = |c: &[Lit]| c.iter().all(|l| (l.var() as usize) < num_vars && (l.var() as usize) < sigma.num_vars());
    if !in_range(clause) || !db.iter().all(|d| in_range(d)) {
        return false;
    }
    if (0..sigma.num_vars().min(num_vars) as u32).any(|v| sigma.apply(Lit::pos(v)).var() as usize >= num_vars) {
        return false;
    }
    let present: std::collections::HashSet<Vec<u32>> = db.iter().map(|d| canon_clause(d)).collect();
    let alpha: Vec<Lit> = clause.iter().map(|l| l.negated()).collect();
    db.iter().map(|d| d.as_slice()).chain(std::iter::once(clause)).all(|d| {
        let image = sigma.apply_clause(d);
        if present.contains(&canon_clause(&image)) {
            return true;
        }
        let mut assume = alpha.clone();
        assume.extend(image.iter().map(|l| l.negated()));
        up_conflict(num_vars, db, &assume)
    })
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
    if !clause
        .iter()
        .any(|&l| rup::lit_val(&omega, l) == Some(true))
    {
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
#[path = "../../logicaffeine_tests/unit/logicaffeine_proof/pr/tests.rs"]
mod tests;
