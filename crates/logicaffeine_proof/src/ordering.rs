//! The **ordering-principle specialist** — a polynomial-time recognizer + refuter for GT(n), the
//! linear-ordering contradiction. GT(n) asserts a strict total order (totality + antisymmetry +
//! transitivity) in which every element has a strictly greater one ("no maximum"). That is impossible:
//! a finite strict total order always has a maximum. So a complete GT(n) core is unsatisfiable, and any
//! formula containing it (a superset of an UNSAT core) is UNSAT.
//!
//! The general cascade decides GT(n) only by super-polynomial search (measured: GT(20) ≈ 2.7s, ~68k
//! conflicts, growing ~10x every 4 steps). This module recognizes the structure directly and certifies
//! UNSAT from it — polynomial, and instant where search walls.
//!
//! **Soundness** is by faithful, conservative recognition, in the style of [`crate::pigeonhole`]: the
//! recognizer verifies, for a single consistent element/edge identification, that ALL of totality,
//! antisymmetry, transitivity (every ordered triple), and the no-maximum clause (every element) are
//! present. Only then is a complete GT(n) core certified present — and it is genuinely UNSAT. Any
//! missing, ambiguous, or extra-shaped piece makes the recognizer return `None` (never a false
//! refutation); the caller then falls through to the general engine. Full transitivity is essential:
//! without it a "no-maximum tournament" can be a satisfiable cycle (e.g. 0<1<2<0), so an incomplete
//! structure must not be refuted.
//!
//! The refutation object is an [`OrderingCert`]: the element/edge identification, which a checker
//! re-verifies against the raw clauses ([`check_ordering_cert`]) with zero trust in how it was produced.

use crate::cdcl::Lit;
use std::collections::{HashMap, HashSet};

/// A re-checkable ordering-principle refutation: the recovered element/edge identification of a complete
/// GT(n) core. `edge[i * n + j]` is the comparison variable `x_ij` ("i < j") for the ordered pair
/// `(i, j)`; diagonal entries (`i == j`) are [`u32::MAX`] and unused. Given this map a checker
/// re-verifies that every totality, antisymmetry, transitivity, and no-maximum clause is present — the
/// whole certificate, no trust in the recognizer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OrderingCert {
    /// The number of ordered elements.
    pub n: usize,
    /// The comparison variable `x_ij` at flat index `i * n + j` (diagonal = `u32::MAX`).
    pub edge: Vec<u32>,
}

impl OrderingCert {
    fn get(&self, i: usize, j: usize) -> u32 {
        self.edge[i * self.n + j]
    }

    /// The serialized size in bytes: the element count plus one 32-bit variable id per off-diagonal
    /// ordered pair — the `O(n²)` identification a checker consumes.
    pub fn byte_len(&self) -> usize {
        8 + self.n * (self.n - 1) * 4
    }
}

/// Recover a complete ordering-principle (GT(n)) core from `clauses`, or `None` if there is none. On
/// `Some(cert)` the formula is unsatisfiable (a finite strict total order has a maximum, contradicting
/// the no-maximum clauses), and `cert` re-checks via [`check_ordering_cert`]. Conservative / fail-closed
/// — never a false certificate. See the module docs.
pub fn ordering_certificate(num_vars: usize, clauses: &[Vec<Lit>]) -> Option<OrderingCert> {
    if num_vars < 2 {
        return None;
    }
    let key2 = |x: u32, y: u32| if x < y { (x, y) } else { (y, x) };

    // ---- 1. Index clauses by shape. ----
    // {a,b} both positive → totality candidate; {a,b} both negative → antisymmetry candidate;
    // [¬a, ¬b, c] → transitivity; any all-positive clause → no-maximum candidate.
    let mut pos2: HashSet<(u32, u32)> = HashSet::new();
    let mut neg2: HashSet<(u32, u32)> = HashSet::new();
    let mut trans: HashSet<(u32, u32, u32)> = HashSet::new(); // (a, b, c) for the clause ¬a ∨ ¬b ∨ c
    let mut positive_clauses: Vec<Vec<u32>> = Vec::new();
    for c in clauses {
        if c.iter().all(|l| l.is_positive()) {
            positive_clauses.push(c.iter().map(|l| l.var()).collect());
        }
        match c.len() {
            2 => match (c[0].is_positive(), c[1].is_positive()) {
                (true, true) => {
                    pos2.insert(key2(c[0].var(), c[1].var()));
                }
                (false, false) => {
                    neg2.insert(key2(c[0].var(), c[1].var()));
                }
                _ => {}
            },
            3 => {
                let negs: Vec<u32> = c.iter().filter(|l| !l.is_positive()).map(|l| l.var()).collect();
                let poss: Vec<u32> = c.iter().filter(|l| l.is_positive()).map(|l| l.var()).collect();
                if negs.len() == 2 && poss.len() == 1 {
                    trans.insert((negs[0], negs[1], poss[0]));
                    trans.insert((negs[1], negs[0], poss[0]));
                }
            }
            _ => {}
        }
    }

    // ---- 2. Comparison pairs: {a,b} carrying BOTH totality and antisymmetry (the two directions of one
    // element pair). Every comparison variable must belong to exactly one such pair. ----
    let comparisons: Vec<(u32, u32)> = pos2.intersection(&neg2).copied().collect();
    if comparisons.len() < 2 {
        return None;
    }
    let mut partner: HashMap<u32, u32> = HashMap::new();
    for &(a, b) in &comparisons {
        if partner.insert(a, b).is_some() || partner.insert(b, a).is_some() {
            return None; // a variable in two comparison pairs — not a clean ordering encoding
        }
    }
    let comparison_vars: HashSet<u32> = partner.keys().copied().collect();

    // ---- 3. No-maximum clauses partition the comparison variables into per-element OUTGOING groups. A
    // no-maximum clause for element i is {x_ij : j≠i}: all positive comparison variables, pairwise
    // NON-partners. A totality clause [x_ij, x_ji] is positive too, but its vars ARE partners. ----
    let mut groups: Vec<Vec<u32>> = Vec::new();
    let mut group_of: HashMap<u32, usize> = HashMap::new();
    for pc in &positive_clauses {
        if pc.is_empty() || !pc.iter().all(|v| comparison_vars.contains(v)) {
            continue;
        }
        let set: HashSet<u32> = pc.iter().copied().collect();
        if set.len() != pc.len() {
            continue; // a repeated variable
        }
        if pc.iter().any(|v| set.contains(&partner[v])) {
            continue; // contains a comparison pair → a totality clause, not a no-maximum clause
        }
        if pc.iter().any(|v| group_of.contains_key(v)) {
            continue; // overlaps an already-claimed group — require a clean partition
        }
        let gid = groups.len();
        for &v in pc {
            group_of.insert(v, gid);
        }
        groups.push(pc.clone());
    }
    let n = groups.len();
    if n < 2
        || group_of.len() != comparison_vars.len()
        || comparison_vars.len() != n * (n - 1)
        || groups.iter().any(|g| g.len() != n - 1)
    {
        return None;
    }

    // ---- 4. Directed edge map: edge[(i,j)] = x_ij. For a comparison pair {a,b}, a lies in group i and
    // b in group j, so a is the i→j edge and b the j→i edge. ----
    let mut edge_map: HashMap<(usize, usize), u32> = HashMap::new();
    for (&a, &b) in &partner {
        let (i, j) = (group_of[&a], group_of[&b]);
        if i == j || edge_map.insert((i, j), a).is_some() {
            return None; // partner within one element, or two edges for the same ordered pair
        }
    }
    if edge_map.len() != n * (n - 1) {
        return None;
    }
    let mut edge = vec![u32::MAX; n * n];
    for i in 0..n {
        for j in 0..n {
            if i == j {
                continue;
            }
            match edge_map.get(&(i, j)) {
                Some(&v) => edge[i * n + j] = v,
                None => return None, // a missing directed comparison — not a complete tournament
            }
        }
    }
    // each group must be EXACTLY element i's outgoing set {edge[(i,j)] : j≠i}.
    for (i, g) in groups.iter().enumerate() {
        let expected: HashSet<u32> = (0..n).filter(|&j| j != i).map(|j| edge[i * n + j]).collect();
        if expected != g.iter().copied().collect() {
            return None;
        }
    }

    // ---- 5. Verify COMPLETE transitivity: every ordered distinct triple (i,j,k) has ¬x_ij ∨ ¬x_jk ∨
    // x_ik. Essential for soundness — a tournament missing transitivity can be a satisfiable cycle. ----
    for i in 0..n {
        for j in 0..n {
            if j == i {
                continue;
            }
            for k in 0..n {
                if k == i || k == j {
                    continue;
                }
                if !trans.contains(&(edge[i * n + j], edge[j * n + k], edge[i * n + k])) {
                    return None;
                }
            }
        }
    }

    Some(OrderingCert { n, edge })
}

/// Re-check an [`OrderingCert`] against the raw `clauses` from scratch, trusting nothing about how it was
/// produced: verify that for its element/edge identification EVERY totality, antisymmetry, transitivity,
/// and no-maximum clause is present. `true` iff the certificate genuinely witnesses a complete GT(n)
/// core (which is unsatisfiable), so the formula is UNSAT.
pub fn check_ordering_cert(cert: &OrderingCert, clauses: &[Vec<Lit>]) -> bool {
    let n = cert.n;
    if n < 2 || cert.edge.len() != n * n {
        return false;
    }
    // every off-diagonal edge is a real variable, and the n·(n−1) of them are distinct.
    let mut seen = HashSet::new();
    for i in 0..n {
        for j in 0..n {
            if i == j {
                continue;
            }
            let v = cert.get(i, j);
            if v == u32::MAX || !seen.insert(v) {
                return false;
            }
        }
    }
    // build clause lookups.
    let key2 = |x: u32, y: u32| if x < y { (x, y) } else { (y, x) };
    let mut pos2 = HashSet::new();
    let mut neg2 = HashSet::new();
    let mut trans = HashSet::new();
    let mut positive: HashSet<Vec<u32>> = HashSet::new();
    for c in clauses {
        if c.iter().all(|l| l.is_positive()) {
            let mut vs: Vec<u32> = c.iter().map(|l| l.var()).collect();
            vs.sort_unstable();
            positive.insert(vs);
        }
        match c.len() {
            2 => match (c[0].is_positive(), c[1].is_positive()) {
                (true, true) => {
                    pos2.insert(key2(c[0].var(), c[1].var()));
                }
                (false, false) => {
                    neg2.insert(key2(c[0].var(), c[1].var()));
                }
                _ => {}
            },
            3 => {
                let negs: Vec<u32> = c.iter().filter(|l| !l.is_positive()).map(|l| l.var()).collect();
                let poss: Vec<u32> = c.iter().filter(|l| l.is_positive()).map(|l| l.var()).collect();
                if negs.len() == 2 && poss.len() == 1 {
                    trans.insert((negs[0], negs[1], poss[0]));
                }
            }
            _ => {}
        }
    }
    // totality + antisymmetry for every unordered pair.
    for i in 0..n {
        for j in (i + 1)..n {
            let (a, b) = (cert.get(i, j), cert.get(j, i));
            if !pos2.contains(&key2(a, b)) || !neg2.contains(&key2(a, b)) {
                return false;
            }
        }
    }
    // transitivity for every ordered distinct triple.
    for i in 0..n {
        for j in 0..n {
            if j == i {
                continue;
            }
            for k in 0..n {
                if k == i || k == j {
                    continue;
                }
                let want = (cert.get(i, j), cert.get(j, k), cert.get(i, k));
                let alt = (cert.get(j, k), cert.get(i, j), cert.get(i, k));
                if !trans.contains(&want) && !trans.contains(&alt) {
                    return false;
                }
            }
        }
    }
    // no-maximum clause for every element (its complete outgoing set, in sorted order).
    for i in 0..n {
        let mut out: Vec<u32> = (0..n).filter(|&j| j != i).map(|j| cert.get(i, j)).collect();
        out.sort_unstable();
        if !positive.contains(&out) {
            return false;
        }
    }
    true
}

/// Refute a formula that contains a complete ordering-principle core. `true` iff a certificate is
/// recovered — see [`ordering_certificate`]. Never a false refutation.
pub fn refute_ordering(num_vars: usize, clauses: &[Vec<Lit>]) -> bool {
    ordering_certificate(num_vars, clauses).is_some()
}

/// The ordering cut over a `ProofExpr`: clausify and run the specialist. This is the entry the cascade
/// ([`crate::sat::prove_unsat`]) calls.
pub fn refutes_ordering_principle(e: &crate::ProofExpr) -> bool {
    let mut cnf = crate::cnf::Cnf::new();
    if cnf.assert(e).is_none() {
        return false;
    }
    refute_ordering(cnf.num_vars(), cnf.clauses())
}
