//! XOR-SAT via Gaussian elimination over GF(2) — the parity analog of the pigeonhole/matching
//! supercrush.
//!
//! A system of parity (XOR) constraints — `x₁ ⊕ x₃ ⊕ x₇ = 1`, etc. — is the canonical
//! *resolution-hard* problem: Tseitin formulas over expander graphs need exponentially long
//! resolution refutations, so CDCL solvers (ours and Z3 alike) blow up on the CNF encoding. But the
//! underlying question is just a linear system over GF(2), decided in **polynomial time** by
//! Gaussian elimination — and certified: an inconsistent system yields a subset of equations whose
//! XOR is `0 = 1` (a re-checkable linear-dependency refutation), and a consistent one yields a
//! satisfying assignment. Parity systems are everywhere — cryptanalysis, error-correcting codes,
//! checksum logic — so this is a broad class we decide instantly where SAT/Z3 cannot.

/// A parity equation: the XOR of the variables in `vars` equals `rhs`. (Repeated variables cancel,
/// per GF(2).)
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct XorEquation {
    /// Variable indices (`0..num_vars`) whose XOR is constrained.
    pub vars: Vec<usize>,
    /// The right-hand side of the equation.
    pub rhs: bool,
}

impl XorEquation {
    /// Convenience constructor.
    pub fn new(vars: impl Into<Vec<usize>>, rhs: bool) -> Self {
        XorEquation {
            vars: vars.into(),
            rhs,
        }
    }
}

/// The outcome of solving an XOR system.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum XorOutcome {
    /// Satisfiable, with an assignment over `0..num_vars` (re-checkable via [`satisfies`]).
    Sat(Vec<bool>),
    /// Unsatisfiable, witnessed by a subset of equation indices whose XOR is `0 = 1`
    /// (re-checkable via [`is_refutation`]).
    Unsat(Vec<usize>),
}

#[inline]
fn get(bits: &[u64], i: usize) -> bool {
    (bits[i / 64] >> (i % 64)) & 1 == 1
}

#[inline]
fn flip(bits: &mut [u64], i: usize) {
    bits[i / 64] ^= 1u64 << (i % 64);
}

#[inline]
fn xor_assign(dst: &mut [u64], src: &[u64]) {
    for (d, s) in dst.iter_mut().zip(src) {
        *d ^= *s;
    }
}

#[inline]
fn is_zero(bits: &[u64]) -> bool {
    bits.iter().all(|&w| w == 0)
}

fn set_indices(bits: &[u64]) -> Vec<usize> {
    let mut out = Vec::new();
    for (w, &word) in bits.iter().enumerate() {
        let mut b = word;
        while b != 0 {
            let t = b.trailing_zeros() as usize;
            out.push(w * 64 + t);
            b &= b - 1;
        }
    }
    out
}

#[derive(Clone)]
struct Row {
    lhs: Vec<u64>, // variable bitset
    rhs: bool,
    prov: Vec<u64>, // which original equations XOR to this row (the certificate's provenance)
}

/// Solve a parity system over `0..num_vars` by Gauss–Jordan elimination over GF(2). Returns a
/// satisfying assignment or a certified `0 = 1` refutation. `O(eq · vars · (eq+vars)/64)`.
pub fn solve(equations: &[XorEquation], num_vars: usize) -> XorOutcome {
    let nb = num_vars.div_ceil(64).max(1);
    let pb = equations.len().div_ceil(64).max(1);
    let mut rows: Vec<Row> = equations
        .iter()
        .enumerate()
        .map(|(i, eq)| {
            let mut lhs = vec![0u64; nb];
            for &v in &eq.vars {
                if v < num_vars {
                    flip(&mut lhs, v); // XOR ⇒ duplicate variables cancel
                }
            }
            let mut prov = vec![0u64; pb];
            flip(&mut prov, i);
            Row {
                lhs,
                rhs: eq.rhs,
                prov,
            }
        })
        .collect();

    let mut pivot_for_col = vec![usize::MAX; num_vars];
    let mut rank = 0;
    for c in 0..num_vars {
        let Some(p) = (rank..rows.len()).find(|&i| get(&rows[i].lhs, c)) else {
            continue;
        };
        rows.swap(rank, p);
        let pivot = rows[rank].clone();
        for (i, row) in rows.iter_mut().enumerate() {
            if i != rank && get(&row.lhs, c) {
                xor_assign(&mut row.lhs, &pivot.lhs);
                row.rhs ^= pivot.rhs;
                xor_assign(&mut row.prov, &pivot.prov);
            }
        }
        pivot_for_col[c] = rank;
        rank += 1;
    }

    // A reduced row with empty LHS but rhs = true is `0 = 1` — its provenance is the refutation.
    for row in &rows {
        if is_zero(&row.lhs) && row.rhs {
            return XorOutcome::Unsat(set_indices(&row.prov));
        }
    }

    // Consistent: free variables take 0; each pivot variable then equals its row's rhs (the row, in
    // reduced form, holds only its pivot column plus free columns, all assigned 0).
    let mut assignment = vec![false; num_vars];
    for c in 0..num_vars {
        let pr = pivot_for_col[c];
        if pr != usize::MAX {
            assignment[c] = rows[pr].rhs;
        }
    }
    XorOutcome::Sat(assignment)
}

/// Re-check a satisfying assignment: every equation's variable-XOR equals its rhs.
pub fn satisfies(equations: &[XorEquation], assignment: &[bool]) -> bool {
    equations.iter().all(|eq| {
        let ones = eq
            .vars
            .iter()
            .filter(|&&v| v < assignment.len() && assignment[v])
            .count();
        (ones % 2 == 1) == eq.rhs
    })
}

/// Re-check a refutation: the XOR of the chosen equations is `0 = 1` — their variables all cancel
/// while their right-hand sides sum to 1. A solver-free certificate of unsatisfiability.
pub fn is_refutation(equations: &[XorEquation], num_vars: usize, refutation: &[usize]) -> bool {
    if refutation.is_empty() {
        return false;
    }
    let nb = num_vars.div_ceil(64).max(1);
    let mut lhs = vec![0u64; nb];
    let mut rhs = false;
    for &idx in refutation {
        let Some(eq) = equations.get(idx) else {
            return false;
        };
        for &v in &eq.vars {
            if v < num_vars {
                flip(&mut lhs, v);
            }
        }
        rhs ^= eq.rhs;
    }
    is_zero(&lhs) && rhs
}

/// Recognize the XOR (parity) gadgets inside a CNF `ProofExpr` and refute via Gaussian elimination —
/// the GF(2) shadow, as a fast-path for [`crate::sat::prove_unsat`]. A parity constraint
/// `x_{a} ⊕ … ⊕ x_{z} = r` over `k` variables is encoded in CNF as exactly the `2^{k-1}` clauses that
/// forbid the wrong-parity assignments; we group clauses by their variable set, and whenever a group
/// is precisely such a full wrong-parity bundle we recover its [`XorEquation`].
///
/// **Soundness (never a false `true`):** a fully-present gadget's clauses *imply* its XOR equation
/// (they forbid exactly the assignments the equation forbids), so the recovered equations are all
/// logical consequences of `e`. If that recognized linear subsystem is inconsistent over GF(2), then
/// `e` is unsatisfiable. Partial or malformed gadgets are simply not recognized — we fall through,
/// never guess. The parity refutation is itself re-checkable via [`is_refutation`].
pub fn refute_via_parity(e: &crate::ProofExpr) -> bool {
    use std::collections::HashMap;
    let mut idx: HashMap<String, usize> = HashMap::new();
    let mut clauses: Vec<Vec<(usize, bool)>> = Vec::new();
    if !collect_clauses(e, &mut clauses, &mut idx) {
        return false;
    }
    // Group clauses by their (sorted, deduplicated) variable set.
    let mut groups: HashMap<Vec<usize>, Vec<Vec<(usize, bool)>>> = HashMap::new();
    for c in clauses {
        let mut vars: Vec<usize> = c.iter().map(|&(v, _)| v).collect();
        vars.sort_unstable();
        vars.dedup();
        if vars.len() != c.len() {
            continue; // a repeated variable in one clause — not a clean parity literal set
        }
        groups.entry(vars).or_default().push(c);
    }
    let num_vars = idx.len();
    let mut eqs = Vec::new();
    for (vars, group) in groups {
        let k = vars.len();
        if k == 0 || k > 31 || group.len() != (1usize << (k - 1)) {
            continue; // a gadget over k vars is exactly 2^{k-1} clauses; anything else isn't one
        }
        let pos: HashMap<usize, usize> = vars.iter().enumerate().map(|(i, &v)| (v, i)).collect();
        let mut forbidden = std::collections::HashSet::new();
        let mut parity: Option<u32> = None;
        let mut clean = true;
        for c in &group {
            // The forbidden assignment this clause rules out: bit i is set iff literal i is negative.
            let mut a = 0u32;
            for &(v, positive) in c {
                if !positive {
                    a |= 1 << pos[&v];
                }
            }
            if !forbidden.insert(a) {
                clean = false; // a duplicated forbidden assignment ⟹ not a faithful gadget
                break;
            }
            let p = a.count_ones() % 2;
            match parity {
                None => parity = Some(p),
                Some(q) if q != p => {
                    clean = false; // mixed parities ⟹ not one XOR equation
                    break;
                }
                _ => {}
            }
        }
        if !clean {
            continue;
        }
        // All 2^{k-1} forbidden assignments share parity `p`; the equation is `⊕ vars = ¬p`.
        let p = parity.unwrap_or(0);
        eqs.push(XorEquation::new(vars, p == 0));
    }
    if eqs.is_empty() {
        return false;
    }
    matches!(solve(&eqs, num_vars), XorOutcome::Unsat(_))
}

/// Flatten a CNF `ProofExpr` into clauses of `(var, positive)` literals over a shared atom index.
/// Returns `false` if `e` is not a conjunction of disjunctions of literals (so parity recognition
/// declines rather than misreads).
fn collect_clauses(
    e: &crate::ProofExpr,
    out: &mut Vec<Vec<(usize, bool)>>,
    idx: &mut std::collections::HashMap<String, usize>,
) -> bool {
    use crate::ProofExpr;
    match e {
        ProofExpr::And(l, r) => collect_clauses(l, out, idx) && collect_clauses(r, out, idx),
        other => {
            let mut lits = Vec::new();
            if collect_literals(other, true, &mut lits, idx) {
                out.push(lits);
                true
            } else {
                false
            }
        }
    }
}

fn collect_literals(
    e: &crate::ProofExpr,
    positive: bool,
    out: &mut Vec<(usize, bool)>,
    idx: &mut std::collections::HashMap<String, usize>,
) -> bool {
    use crate::ProofExpr;
    match e {
        ProofExpr::Atom(name) => {
            let n = idx.len();
            let v = *idx.entry(name.clone()).or_insert(n);
            out.push((v, positive));
            true
        }
        ProofExpr::Not(inner) => match inner.as_ref() {
            ProofExpr::Atom(name) => {
                let n = idx.len();
                let v = *idx.entry(name.clone()).or_insert(n);
                out.push((v, !positive));
                true
            }
            _ => false,
        },
        ProofExpr::Or(l, r) if positive => {
            collect_literals(l, positive, out, idx) && collect_literals(r, positive, out, idx)
        }
        _ => false,
    }
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_proof/xorsat/tests.rs"]
mod tests;
