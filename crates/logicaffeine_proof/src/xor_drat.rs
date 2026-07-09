//! The CNF→GF(2) bridge: compile a Gaussian (XOR linear-dependency) refutation into a **strict DRAT
//! proof** that an external checker (`drat-trim`) accepts.
//!
//! Our parity route refutes Tseitin/XOR formulas by linear algebra: it finds a set `S` of recovered
//! XOR equations whose GF(2) sum telescopes to `0 = 1` ([`crate::xorsat::solve`] → `Unsat(S)`). That
//! certificate is sound and re-checkable *algebraically* — but it is not the Boolean clausal proof
//! the competition toolchain consumes. This module closes that gap.
//!
//! **The construction (why each emitted line is RUP).** Every equation in `S` is encoded in the CNF
//! by its full parity gadget (the `2^{k-1}` clauses forbidding wrong-parity assignments). The
//! conjunction of the gadgets of `S` is unsatisfiable (their linear combination is `0 = 1`), so the
//! empty clause is derivable from them by *resolution*. We produce that derivation by Davis–Putnam
//! variable elimination over the support, in min-degree order, emitting each resolvent as a proof
//! line. A resolvent of two clauses already in the formula is **RUP** (reverse-unit-propagation)
//! against that formula, and we only ever grow the formula — so every line checks, and the final
//! line is the empty clause. The proof is polynomial when the elimination keeps clauses narrow
//! (bounded cut-/tree-width); it can blow up on high-expansion instances, which is the honest limit
//! of the resolution route (the polynomial extension-variable construction is the next rung).
//!
//! Soundness is independent of how the equations were recovered: the gadget clauses we resolve over
//! are entailed by the CNF, and the emitted proof is checked by `drat-trim` against the *original*
//! formula, so a faulty recovery cannot produce an accepted proof.

use std::collections::{BTreeSet, HashSet};

use crate::cdcl::Lit;
use crate::xorsat::XorEquation;

/// The full parity gadget of `eq`: one clause per wrong-parity assignment of its variables. These are
/// exactly the CNF clauses a Tseitin/XOR encoding carries for the constraint.
pub fn gadget_clauses(eq: &XorEquation) -> Vec<Vec<Lit>> {
    let k = eq.vars.len();
    let mut out = Vec::new();
    for mask in 0u32..(1u32 << k) {
        // `mask` is the assignment (bit i = value of vars[i]); it violates the parity when its
        // popcount-parity disagrees with the rhs. The forbidding clause is false under `mask`.
        if (mask.count_ones() % 2 == 1) != eq.rhs {
            out.push((0..k).map(|i| Lit::new(eq.vars[i] as u32, (mask >> i) & 1 == 0)).collect());
        }
    }
    out
}

/// Canonical key for a clause (sorted, deduped literal codes), for dedup/subsumption-free bookkeeping.
fn key(c: &[Lit]) -> Vec<u32> {
    let mut k: Vec<u32> = c.iter().map(|l| l.var() * 2 + u32::from(!l.is_positive())).collect();
    k.sort_unstable();
    k.dedup();
    k
}

/// Resolve `a` and `b` on variable `v` (which appears positively in one, negatively in the other).
/// Returns the resolvent with `v` removed, or `None` if it is a tautology (contains `x` and `¬x`).
fn resolve(a: &[Lit], b: &[Lit], v: u32) -> Option<Vec<Lit>> {
    let mut seen: std::collections::HashMap<u32, bool> = std::collections::HashMap::new();
    for l in a.iter().chain(b.iter()) {
        if l.var() == v {
            continue;
        }
        if let Some(&prev) = seen.get(&l.var()) {
            if prev != l.is_positive() {
                return None; // x and ¬x ⇒ tautology
            }
        } else {
            seen.insert(l.var(), l.is_positive());
        }
    }
    let mut out: Vec<Lit> = seen.into_iter().map(|(var, pos)| Lit::new(var, pos)).collect();
    out.sort_by_key(|l| l.var() * 2 + u32::from(!l.is_positive()));
    Some(out)
}

/// Derive the empty clause from an unsatisfiable clause set by Davis–Putnam variable elimination,
/// returning the ordered list of resolvents (each RUP against the original set plus the earlier
/// resolvents). Eliminates in min-degree order to keep intermediate clauses small. Returns `None`
/// only if the input was not actually unsatisfiable (no refutation exists).
/// Resolvent budget: the resolution route is polynomial only when the elimination keeps clauses
/// narrow; past this many emitted clauses we declare the route blown (fail-closed) rather than hang.
const RESOLUTION_BUDGET: usize = 40_000;
/// Raw-work budget: a single elimination can attempt `|pos|·|neg|` resolutions, which explodes long
/// before the emitted-clause budget trips (most attempts are duplicates/tautologies). Cap the attempts
/// so a blown instance bails in seconds rather than minutes.
const RESOLUTION_WORK_BUDGET: u64 = 4_000_000;

fn resolution_refutation(support: &[Vec<Lit>]) -> Option<Vec<Vec<Lit>>> {
    // Active working set, deduped; and the set of every clause key already in play (so a resolvent we
    // re-derive is not emitted twice).
    let mut active: Vec<Vec<Lit>> = Vec::new();
    let mut present: HashSet<Vec<u32>> = HashSet::new();
    for c in support {
        let k = key(c);
        if present.insert(k) {
            active.push({
                let mut c = c.clone();
                c.sort_by_key(|l| l.var() * 2 + u32::from(!l.is_positive()));
                c.dedup();
                c
            });
        }
    }
    let mut emitted: Vec<Vec<Lit>> = Vec::new();
    let mut work: u64 = 0;
    if active.iter().any(|c| c.is_empty()) {
        emitted.push(Vec::new());
        return Some(emitted);
    }

    loop {
        let vars: BTreeSet<u32> = active.iter().flat_map(|c| c.iter().map(|l| l.var())).collect();
        if vars.is_empty() {
            return None; // nothing left to eliminate but no empty clause: input was satisfiable
        }
        // Min-product (min-fill proxy): eliminate the variable whose resolution fan-out `|pos|·|neg|`
        // is smallest — that bounds both the work and the number of new clauses, keeping the proof as
        // narrow as the instance's structure allows (the lever before extension variables).
        let v = *vars
            .iter()
            .min_by_key(|&&v| {
                let (mut p, mut n) = (0usize, 0usize);
                for c in &active {
                    match c.iter().find(|l| l.var() == v) {
                        Some(l) if l.is_positive() => p += 1,
                        Some(_) => n += 1,
                        None => {}
                    }
                }
                p * n
            })
            .unwrap();

        let mut pos: Vec<Vec<Lit>> = Vec::new();
        let mut neg: Vec<Vec<Lit>> = Vec::new();
        let mut rest: Vec<Vec<Lit>> = Vec::new();
        for c in active.drain(..) {
            match c.iter().find(|l| l.var() == v) {
                Some(l) if l.is_positive() => pos.push(c),
                Some(_) => neg.push(c),
                None => rest.push(c),
            }
        }

        let mut next = rest;
        for cp in &pos {
            for cn in &neg {
                work += 1;
                if work > RESOLUTION_WORK_BUDGET {
                    return None; // raw-work blowup: bail fast, fail-closed.
                }
                let Some(r) = resolve(cp, cn, v) else { continue };
                if r.is_empty() {
                    emitted.push(Vec::new());
                    return Some(emitted);
                }
                let k = key(&r);
                if present.insert(k) {
                    emitted.push(r.clone());
                    next.push(r);
                    if emitted.len() > RESOLUTION_BUDGET {
                        return None; // the resolution route blew up on this instance's width.
                    }
                }
            }
        }
        active = next;
        // The clauses mentioning `v` are now eliminated; they stay in the DRAT formula (we never
        // delete), so every future resolvent remains RUP against the growing database.
    }
}

/// Compile the GF(2) linear-dependency refutation `refutation` (equation indices whose XOR is `0=1`,
/// from [`crate::xorsat::solve`]) into a DRAT proof: the ordered resolvent clauses, ending in the
/// empty clause. Pair these with the original CNF for [`crate::rup::check_refutation`] or `drat-trim`.
pub fn emit_xor_drat(equations: &[XorEquation], refutation: &[usize]) -> Option<Vec<Vec<Lit>>> {
    let mut support: Vec<Vec<Lit>> = Vec::new();
    for &i in refutation {
        support.extend(gadget_clauses(&equations[i]));
    }
    resolution_refutation(&support)
}

/// Resolution-refute the sub-CNF confined to the `support` variables — the convention-agnostic bridge.
/// The emitted clauses are resolvents of *real* CNF clauses, so each is RUP against the original
/// formula by construction and a `drat-trim` check holds. `None` if that sub-CNF is not UNSAT (so the
/// caller's algebraic verdict could not be reproduced clausally — a fail-closed signal).
pub fn emit_drat_over_support(clauses: &[Vec<Lit>], support: &BTreeSet<u32>) -> Option<Vec<Vec<Lit>>> {
    let sub: Vec<Vec<Lit>> = clauses
        .iter()
        .filter(|c| c.iter().all(|l| support.contains(&l.var())))
        .cloned()
        .collect();
    resolution_refutation(&sub)
}

/// The CNF→GF(p) bridge: compile a *modular* (mod-p / counting) UNSAT verdict into a strict DRAT
/// proof. We recover the one-hot modular system from `clauses`, solve it over the prime field, and —
/// on UNSAT — resolution-refute the sub-CNF confined to the one-hot bits of the involved groups. That
/// sub-CNF (the forbidden-tuple gadgets of the combination's equations together with their groups'
/// at-least-one/at-most-one clauses) is unsatisfiable, so DP elimination over those Boolean variables
/// yields the empty clause, every line RUP against the original CNF. `None` if the formula is not a
/// recoverable mod-p encoding or its modular system is satisfiable.
pub fn emit_modp_drat(num_vars: usize, clauses: &[Vec<Lit>]) -> Option<Vec<Vec<Lit>>> {
    let rec = crate::modp::recover_from_cnf(num_vars, clauses)?;
    if !crate::modp::is_prime(rec.modulus) {
        return None; // composite moduli go through the ring engine; the prime field is the bridge here.
    }
    let combo = match crate::modp::solve(&rec.equations, rec.num_vars, rec.modulus) {
        crate::modp::ModpOutcome::Unsat(combo) => combo,
        crate::modp::ModpOutcome::Sat(_) => return None,
    };
    // The groups (modular variables) that actually appear in the refuting linear combination.
    let mut groups_involved: BTreeSet<usize> = BTreeSet::new();
    for &(eq_idx, mult) in &combo {
        if mult % rec.modulus == 0 {
            continue;
        }
        for &(g, _) in &rec.equations[eq_idx].coeffs {
            groups_involved.insert(g);
        }
    }
    let support: BTreeSet<u32> =
        groups_involved.iter().flat_map(|&g| rec.groups[g].iter().copied()).collect();
    emit_drat_over_support(clauses, &support)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xorsat::{self, XorOutcome};

    #[test]
    fn xor_linear_dependency_compiles_to_a_drat_refutation_checked_by_rup() {
        // x0⊕x1=0, x1⊕x2=0, x0⊕x2=1 — their GF(2) sum is 0=1, so UNSAT. The CNF is the union of the
        // three parity gadgets. We compile the linear-dependency certificate to DRAT and require our
        // INDEPENDENT RUP checker to accept it, ending in the empty clause.
        let eqs = vec![
            XorEquation::new(vec![0, 1], false),
            XorEquation::new(vec![1, 2], false),
            XorEquation::new(vec![0, 2], true),
        ];
        let num_vars = 3;
        let clauses: Vec<Vec<Lit>> = eqs.iter().flat_map(gadget_clauses).collect();

        let refutation = match xorsat::solve(&eqs, num_vars) {
            XorOutcome::Unsat(s) => s,
            XorOutcome::Sat(_) => panic!("the system is UNSAT"),
        };
        let drat = emit_xor_drat(&eqs, &refutation).expect("a linear dependency must compile to DRAT");
        assert!(drat.last().is_some_and(|c| c.is_empty()), "the proof must end in the empty clause");
        assert!(
            crate::rup::check_refutation(num_vars, &clauses, &drat),
            "every DRAT line must be RUP and the proof must refute the CNF"
        );
    }
}
