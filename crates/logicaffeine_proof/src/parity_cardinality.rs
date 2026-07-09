//! Fast refuter for the coupled **exactly-one + parity** family. An exactly-one group (an at-least-one
//! clause plus the full at-most-one clique over its variables) forces EXACTLY one of its selectors true,
//! so their XOR is `1` (odd). If the formula's parity substructure forces that same XOR to `0` (even),
//! the two are inconsistent and the formula is UNSAT — the mixed obstruction neither theory sees alone.
//!
//! This decides it by pure GF(2) linear algebra: recover the implied XOR system, add the equation
//! `⊕S = 1` for an exactly-one group `S`, and test the augmented system for inconsistency. That is
//! microseconds — where the general fused route builds a whole CDCL solver with three theories (parity,
//! cardinality, symmetry) to reach the same verdict in milliseconds.
//!
//! **Soundness:** the XOR equations are consequences of the formula's gadget clauses, and any model of
//! the exactly-one clauses has `⊕S = 1`; so if `eqs ∪ {⊕S = 1}` is GF(2)-inconsistent, no model satisfies
//! both — and the formula contains both — hence UNSAT. Conservative: fires only on a recovered
//! exactly-one group whose augmented parity system is inconsistent; `false` otherwise.

use crate::cdcl::Lit;
use crate::xorsat::{self, XorEquation, XorOutcome};
use std::collections::HashSet;

/// Refute a coupled exactly-one + parity formula. `true` iff some exactly-one group's odd-parity
/// requirement contradicts the formula's XOR system. Never a false refutation.
pub fn refute(num_vars: usize, clauses: &[Vec<Lit>]) -> bool {
    if num_vars == 0 {
        return false;
    }
    let key2 = |x: u32, y: u32| if x < y { (x, y) } else { (y, x) };
    // Binary at-most-one clauses (¬a ∨ ¬b).
    let mut amo: HashSet<(u32, u32)> = HashSet::new();
    for c in clauses {
        if c.len() == 2 && !c[0].is_positive() && !c[1].is_positive() {
            amo.insert(key2(c[0].var(), c[1].var()));
        }
    }
    if amo.is_empty() {
        return false;
    }
    // XOR equations implied by the formula's gadget clauses.
    let eqs = crate::lyapunov::extract_xor(num_vars, clauses);
    if eqs.is_empty() {
        return false;
    }
    // Each all-positive clause is an at-least-one candidate; if its variables form a full at-most-one
    // clique it is an exactly-one group, so exactly one selector is true and their XOR is 1.
    for c in clauses {
        if c.len() < 2 || !c.iter().all(|l| l.is_positive()) {
            continue;
        }
        let set: HashSet<u32> = c.iter().map(|l| l.var()).collect();
        if set.len() != c.len() {
            continue; // a repeated variable
        }
        let mut full_clique = true;
        'pairs: for i in 0..c.len() {
            for j in (i + 1)..c.len() {
                if !amo.contains(&key2(c[i].var(), c[j].var())) {
                    full_clique = false;
                    break 'pairs;
                }
            }
        }
        if !full_clique {
            continue;
        }
        // exactly-one over S ⟹ ⊕S = 1. If the XOR system forces ⊕S = 0 the augmented system is
        // GF(2)-inconsistent — the coupled contradiction.
        let s: Vec<usize> = c.iter().map(|l| l.var() as usize).collect();
        let mut augmented = eqs.clone();
        augmented.push(XorEquation::new(s, true));
        if matches!(xorsat::solve(&augmented, num_vars), XorOutcome::Unsat(_)) {
            return true;
        }
    }
    false
}
