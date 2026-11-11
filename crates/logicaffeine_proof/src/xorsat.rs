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

/// Solve a parity system over `0..num_vars` by elimination over GF(2) ([`eliminate`]). Returns a satisfying
/// assignment or a certified `0 = 1` refutation.
pub fn solve(equations: &[XorEquation], num_vars: usize) -> XorOutcome {
    eliminate(equations, num_vars).outcome
}

/// The rank of a parity system over `0..num_vars`: how many of its equations are linearly independent.
pub fn rank(equations: &[XorEquation], num_vars: usize) -> usize {
    eliminate(equations, num_vars).rank
}

/// A parity system eliminated: its rank, and an assignment of every variable satisfying each equation or a set of
/// equations summing to `0 = 1`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Elimination {
    /// How many equations are linearly independent.
    pub rank: usize,
    /// The assignment (free variables 0) or the refutation (equation indices, ascending).
    pub outcome: XorOutcome,
}

/// The sparsest active column's row count past which the rest of an elimination runs on dense bit rows.
const DENSE_WEIGHT: usize = 24;

/// Eliminate a parity system over GF(2). Each equation is a row of its variables, a repeated variable cancelling in
/// pairs and one past `num_vars` ignored.
///
/// The rows are eliminated sparsely, Markowitz-fashion: the active column in fewest active rows is pivoted next, on
/// the shortest of its rows, which is added into its other rows and retired. A chain of equivalences eliminates along
/// itself, a gate's private input retires its row without touching another, and fill-in stays where the structure
/// puts it. Once the sparsest column is in more than [`DENSE_WEIGHT`] rows the remainder has filled in, and it is
/// eliminated on dense bit rows over its own columns, a word of columns per XOR.
///
/// A retired row is never changed again, so each addition `target += source` adds the source's final value. Replaying
/// the additions backwards from a `0 = 1` row therefore marks exactly the original equations it is the sum of: the
/// refutation, with no provenance carried through the elimination. The assignment back-substitutes the retired rows in
/// reverse — each holds only columns pivoted after it or never — with every column no row pivots on taking 0.
pub fn eliminate(equations: &[XorEquation], num_vars: usize) -> Elimination {
    use std::cmp::Reverse;
    use std::collections::BinaryHeap;
    let m = equations.len();
    let mut rows: Vec<Vec<u32>> = equations
        .iter()
        .map(|eq| {
            let mut vars: Vec<u32> = eq.vars.iter().filter(|&&v| v < num_vars).map(|&v| v as u32).collect();
            vars.sort_unstable();
            cancel_pairs(&mut vars);
            vars
        })
        .collect();
    let mut rhs: Vec<bool> = equations.iter().map(|eq| eq.rhs).collect();
    let mut active: Vec<bool> = rows.iter().map(|r| !r.is_empty()).collect();
    let mut contradiction: Option<Vec<usize>> = (0..m).find(|&i| rows[i].is_empty() && rhs[i]).map(|i| vec![i]);
    let mut holders: Vec<Vec<u32>> = vec![Vec::new(); num_vars];
    let mut weight = vec![0usize; num_vars];
    for (i, row) in rows.iter().enumerate() {
        for &c in row {
            holders[c as usize].push(i as u32);
            weight[c as usize] += 1;
        }
    }
    let mut queue: BinaryHeap<Reverse<(usize, u32)>> =
        (0..num_vars).filter(|&c| weight[c] > 0).map(|c| Reverse((weight[c], c as u32))).collect();
    let mut retired: Vec<(usize, u32)> = Vec::new();
    let mut additions: Vec<(u32, u32)> = Vec::new();
    let mut filled_in = false;
    while let Some(Reverse((w, c))) = queue.pop() {
        let col = c as usize;
        if weight[col] != w || w == 0 {
            continue;
        }
        if w > DENSE_WEIGHT {
            filled_in = true;
            break;
        }
        let mut with: Vec<u32> = std::mem::take(&mut holders[col])
            .into_iter()
            .filter(|&r| active[r as usize] && rows[r as usize].binary_search(&c).is_ok())
            .collect();
        with.sort_unstable();
        with.dedup();
        debug_assert_eq!(with.len(), w, "a column's weight counts its active rows");
        let pivot = *with.iter().min_by_key(|&&r| (rows[r as usize].len(), r)).expect("a column of positive weight has a row");
        let pivot = pivot as usize;
        active[pivot] = false;
        let pivot_row = std::mem::take(&mut rows[pivot]);
        for &other in &pivot_row {
            weight[other as usize] -= 1;
            if other != c && weight[other as usize] > 0 {
                queue.push(Reverse((weight[other as usize], other)));
            }
        }
        for &r in &with {
            let r = r as usize;
            if r == pivot {
                continue;
            }
            let before = std::mem::take(&mut rows[r]);
            let mut merged = Vec::with_capacity(before.len() + pivot_row.len());
            let (mut i, mut j) = (0, 0);
            while i < before.len() || j < pivot_row.len() {
                match (before.get(i), pivot_row.get(j)) {
                    (Some(&a), Some(&b)) if a == b => {
                        weight[a as usize] -= 1;
                        if a != c && weight[a as usize] > 0 {
                            queue.push(Reverse((weight[a as usize], a)));
                        }
                        i += 1;
                        j += 1;
                    }
                    (Some(&a), Some(&b)) if a < b => {
                        merged.push(a);
                        i += 1;
                    }
                    (Some(&a), None) => {
                        merged.push(a);
                        i += 1;
                    }
                    (_, Some(&b)) => {
                        merged.push(b);
                        weight[b as usize] += 1;
                        holders[b as usize].push(r as u32);
                        queue.push(Reverse((weight[b as usize], b)));
                        j += 1;
                    }
                    (None, None) => unreachable!(),
                }
            }
            rhs[r] ^= rhs[pivot];
            additions.push((r as u32, pivot as u32));
            if merged.is_empty() {
                active[r] = false;
                if rhs[r] && contradiction.is_none() {
                    contradiction = Some(vec![r]);
                }
            }
            rows[r] = merged;
        }
        rows[pivot] = pivot_row;
        retired.push((pivot, c));
    }
    let mut rank = retired.len();
    let dense = filled_in.then(|| {
        let core: Vec<usize> = (0..m).filter(|&r| active[r]).collect();
        let dense = eliminate_dense(&core, &rows, &rhs, num_vars, contradiction.is_none());
        rank += dense.retired.len();
        if contradiction.is_none() {
            contradiction = dense.contradiction.clone();
        }
        dense
    });
    let outcome = match contradiction {
        Some(root) => {
            let mut marked = vec![false; m];
            for r in root {
                marked[r] ^= true;
            }
            for &(target, source) in additions.iter().rev() {
                if marked[target as usize] {
                    marked[source as usize] ^= true;
                }
            }
            XorOutcome::Unsat((0..m).filter(|&i| marked[i]).collect())
        }
        None => {
            let mut value = vec![false; num_vars];
            if let Some(dense) = &dense {
                dense.back_substitute(&mut value);
            }
            for &(r, c) in retired.iter().rev() {
                value[c as usize] = rows[r].iter().filter(|&&o| o != c).fold(rhs[r], |acc, &o| acc ^ value[o as usize]);
            }
            XorOutcome::Sat(value)
        }
    };
    Elimination { rank, outcome }
}

/// Remove adjacent equal pairs from a sorted list, so each value is left once if it occurred an odd number of times.
fn cancel_pairs(sorted: &mut Vec<u32>) {
    let mut kept = 0;
    let mut i = 0;
    while i < sorted.len() {
        if i + 1 < sorted.len() && sorted[i] == sorted[i + 1] {
            i += 2;
        } else {
            sorted[kept] = sorted[i];
            kept += 1;
            i += 1;
        }
    }
    sorted.truncate(kept);
}

/// The dense phase of [`eliminate`]: the core rows as bit rows over the core's own columns, eliminated forward — each
/// pivot row added into the other rows holding its column, then retired.
struct Dense {
    /// The core's columns, by dense index.
    columns: Vec<u32>,
    /// The retired rows in pivot order: the bit row, its rhs, its pivot's dense index.
    retired: Vec<(Vec<u64>, bool, usize)>,
    /// The core rows (by global index) summing to `0 = 1`, when the core is inconsistent and one was asked for.
    contradiction: Option<Vec<usize>>,
}

fn eliminate_dense(core: &[usize], rows: &[Vec<u32>], rhs: &[bool], num_vars: usize, want_refutation: bool) -> Dense {
    let mut index = vec![u32::MAX; num_vars];
    let mut columns: Vec<u32> = Vec::new();
    for &r in core {
        for &c in &rows[r] {
            if index[c as usize] == u32::MAX {
                index[c as usize] = columns.len() as u32;
                columns.push(c);
            }
        }
    }
    let words = columns.len().div_ceil(64).max(1);
    let proof_words = if want_refutation { core.len().div_ceil(64).max(1) } else { 0 };
    let mut bits: Vec<(Vec<u64>, bool, Vec<u64>)> = core
        .iter()
        .enumerate()
        .map(|(k, &r)| {
            let mut lhs = vec![0u64; words];
            for &c in &rows[r] {
                flip(&mut lhs, index[c as usize] as usize);
            }
            let mut proof = vec![0u64; proof_words];
            if want_refutation {
                flip(&mut proof, k);
            }
            (lhs, rhs[r], proof)
        })
        .collect();
    let mut live: Vec<usize> = (0..bits.len()).collect();
    let mut retired = Vec::new();
    for col in 0..columns.len() {
        let Some(at) = live.iter().position(|&k| get(&bits[k].0, col)) else { continue };
        let pivot = live.swap_remove(at);
        let (lhs, value, proof) = std::mem::take(&mut bits[pivot]);
        for &k in &live {
            if get(&bits[k].0, col) {
                xor_assign(&mut bits[k].0, &lhs);
                bits[k].1 ^= value;
                xor_assign(&mut bits[k].2, &proof);
            }
        }
        retired.push((lhs, value, col));
    }
    let contradiction = live
        .iter()
        .find(|&&k| bits[k].1 && is_zero(&bits[k].0))
        .filter(|_| want_refutation)
        .map(|&k| set_indices(&bits[k].2).into_iter().map(|j| core[j]).collect());
    Dense { columns, retired, contradiction }
}

impl Dense {
    fn back_substitute(&self, value: &mut [bool]) {
        for (lhs, rhs, pivot) in self.retired.iter().rev() {
            let v = set_indices(lhs).into_iter().filter(|&k| k != *pivot).fold(*rhs, |acc, k| acc ^ value[self.columns[k] as usize]);
            value[self.columns[*pivot] as usize] = v;
        }
    }
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

/// The XOR equations a CNF spells out gadget by gadget. A parity constraint `x_{a} ⊕ … ⊕ x_{z} = r` over `k`
/// variables is encoded as exactly the `2^{k-1}` clauses that forbid the wrong-parity assignments, so every set of
/// `k ≤ max_width` variables whose clauses — each over those variables, each once, none repeated — are exactly
/// `2^{k-1}`, forbid pairwise distinct assignments, and all forbid assignments of one parity `p`, gives the equation
/// `⊕ vars = ¬p`. A clause with a repeated or complementary literal, or a variable past `num_vars`, belongs to no
/// gadget.
///
/// Every equation is a consequence of the clauses: its gadget forbids exactly the assignments the equation forbids.
/// The clauses are grouped by sorting them on their variable sets, with no hashing, and the equations come out in
/// that order — by width, then by variables — whatever order the clauses came in.
pub fn xor_gadgets(num_vars: usize, clauses: &[Vec<crate::cdcl::Lit>], max_width: usize) -> Vec<XorEquation> {
    let max_width = max_width.min(31);
    let mut vars: Vec<u32> = Vec::new();
    // (offset of the clause's sorted variables in `vars`, width, the assignment it forbids over them)
    let mut candidates: Vec<(usize, usize, u32)> = Vec::new();
    for c in clauses {
        let k = c.len();
        if k == 0 || k > max_width {
            continue;
        }
        let offset = vars.len();
        vars.extend(c.iter().map(|l| l.var()));
        let sorted = &mut vars[offset..];
        sorted.sort_unstable();
        if sorted.windows(2).any(|w| w[0] == w[1]) || sorted[k - 1] as usize >= num_vars {
            vars.truncate(offset);
            continue;
        }
        let sorted = &vars[offset..];
        let forbidden = c
            .iter()
            .filter(|l| !l.is_positive())
            .map(|l| 1u32 << sorted.binary_search(&l.var()).expect("a clause's variable is among its sorted variables"))
            .fold(0, |a, b| a | b);
        candidates.push((offset, k, forbidden));
    }
    let set = |&(offset, k, _): &(usize, usize, u32)| &vars[offset..offset + k];
    candidates.sort_unstable_by(|a, b| (a.1, set(a)).cmp(&(b.1, set(b))).then(a.2.cmp(&b.2)));
    let mut eqs = Vec::new();
    let mut i = 0;
    while i < candidates.len() {
        let k = candidates[i].1;
        let mut j = i + 1;
        while j < candidates.len() && candidates[j].1 == k && set(&candidates[j]) == set(&candidates[i]) {
            j += 1;
        }
        let group = &candidates[i..j];
        let parity = group[0].2.count_ones() % 2;
        if group.len() == 1 << (k - 1)
            && group.iter().all(|g| g.2.count_ones() % 2 == parity)
            && group.windows(2).all(|w| w[0].2 != w[1].2)
        {
            eqs.push(XorEquation::new(set(&group[0]).iter().map(|&v| v as usize).collect::<Vec<usize>>(), parity == 0));
        }
        i = j;
    }
    eqs
}

/// Recognize the XOR (parity) gadgets inside a CNF ([`xor_gadgets`], up to 31 variables a gadget) and refute the
/// recovered linear system by Gaussian elimination — the GF(2) shadow, as a fast-path for the dispatcher.
///
/// **Soundness (never a false `true`):** a fully-present gadget's clauses *imply* its XOR equation, so the recovered
/// equations are all logical consequences of the clauses. If that recognized linear subsystem is inconsistent over
/// GF(2), the formula is unsatisfiable; the refutation is re-checked by [`is_refutation`] before it is believed.
/// Partial or malformed gadgets are simply not recognized — we fall through, never guess.
pub fn refute_via_parity_clauses(num_vars: usize, clauses: &[Vec<crate::cdcl::Lit>]) -> bool {
    let eqs = xor_gadgets(num_vars, clauses, 31);
    if eqs.is_empty() {
        return false;
    }
    matches!(solve(&eqs, num_vars), XorOutcome::Unsat(r) if is_refutation(&eqs, num_vars, &r))
}

/// [`refute_via_parity_clauses`] on a CNF `ProofExpr` — a conjunction of disjunctions of literals over atoms, as
/// [`crate::sat::prove_unsat`] holds it; anything else is declined.
pub fn refute_via_parity(e: &crate::ProofExpr) -> bool {
    use std::collections::HashMap;
    let mut idx: HashMap<String, usize> = HashMap::new();
    let mut clauses: Vec<Vec<(usize, bool)>> = Vec::new();
    if !collect_clauses(e, &mut clauses, &mut idx) {
        return false;
    }
    let clauses: Vec<Vec<crate::cdcl::Lit>> =
        clauses.iter().map(|c| c.iter().map(|&(v, positive)| crate::cdcl::Lit::new(v as u32, positive)).collect()).collect();
    refute_via_parity_clauses(idx.len(), &clauses)
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
