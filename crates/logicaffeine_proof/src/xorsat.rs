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
        XorEquation { vars: vars.into(), rhs }
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
    lhs: Vec<u64>,  // variable bitset
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
            Row { lhs, rhs: eq.rhs, prov }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn eq(vars: &[usize], rhs: bool) -> XorEquation {
        XorEquation::new(vars.to_vec(), rhs)
    }

    #[test]
    fn simple_consistent_system_is_solved() {
        // x0 ⊕ x1 = 1, x1 = 1  ⇒  x1 = 1, x0 = 0.
        let sys = vec![eq(&[0, 1], true), eq(&[1], true)];
        match solve(&sys, 2) {
            XorOutcome::Sat(a) => {
                assert!(satisfies(&sys, &a), "assignment must satisfy: {a:?}");
                assert_eq!(a, vec![false, true]);
            }
            o => panic!("expected Sat, got {o:?}"),
        }
    }

    #[test]
    fn direct_contradiction_is_refuted() {
        // x0 ⊕ x1 = 0 and x0 ⊕ x1 = 1 — summing them gives 0 = 1.
        let sys = vec![eq(&[0, 1], false), eq(&[0, 1], true)];
        match solve(&sys, 2) {
            XorOutcome::Unsat(r) => {
                assert!(is_refutation(&sys, 2, &r), "refutation must re-check: {r:?}");
                assert_eq!(r.len(), 2, "both equations are needed");
            }
            o => panic!("expected Unsat, got {o:?}"),
        }
    }

    #[test]
    fn parity_chain_summing_to_one_is_refuted() {
        // x_i ⊕ x_{i+1} = 0 for a chain, plus x0 ⊕ x_{n-1} = 1 — all equal yet endpoints differ.
        let n = 8;
        let mut sys: Vec<XorEquation> = (0..n - 1).map(|i| eq(&[i, i + 1], false)).collect();
        sys.push(eq(&[0, n - 1], true));
        match solve(&sys, n) {
            XorOutcome::Unsat(r) => assert!(is_refutation(&sys, n, &r), "refutation invalid: {r:?}"),
            o => panic!("inconsistent chain must be Unsat, got {o:?}"),
        }
    }

    #[test]
    fn duplicate_variables_cancel() {
        // x0 ⊕ x0 ⊕ x1 = 1  ≡  x1 = 1.
        let sys = vec![eq(&[0, 0, 1], true)];
        match solve(&sys, 2) {
            XorOutcome::Sat(a) => assert!(a[1], "x1 must be true: {a:?}"),
            o => panic!("expected Sat, got {o:?}"),
        }
    }

    #[test]
    fn empty_system_is_trivially_sat() {
        assert!(matches!(solve(&[], 3), XorOutcome::Sat(_)));
    }

    #[test]
    fn matches_brute_force_on_random_systems() {
        // Independent oracle: enumerate all 2^num_vars assignments; the system is SAT iff some
        // assignment satisfies every equation. Cross-check verdict + re-check every witness.
        let mut s: u64 = 0xD1B54A32D192ED03;
        let mut next = || {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            s
        };
        for _ in 0..400 {
            let num_vars = (next() % 6) as usize + 1; // 1..=6
            let m = (next() % 8) as usize + 1; // 1..=8 equations
            let sys: Vec<XorEquation> = (0..m)
                .map(|_| {
                    let vars: Vec<usize> =
                        (0..num_vars).filter(|_| next() % 2 == 0).collect();
                    eq(&vars, next() % 2 == 0)
                })
                .collect();
            let brute_sat = (0..(1u32 << num_vars)).any(|mask| {
                let a: Vec<bool> = (0..num_vars).map(|i| (mask >> i) & 1 == 1).collect();
                satisfies(&sys, &a)
            });
            match solve(&sys, num_vars) {
                XorOutcome::Sat(a) => {
                    assert!(brute_sat, "we said SAT but brute force says UNSAT: {sys:?}");
                    assert!(satisfies(&sys, &a), "returned assignment is wrong: {a:?}");
                }
                XorOutcome::Unsat(r) => {
                    assert!(!brute_sat, "we said UNSAT but brute force found a model: {sys:?}");
                    assert!(is_refutation(&sys, num_vars, &r), "bogus refutation {r:?}");
                }
            }
        }
    }

    #[test]
    fn a_bad_refutation_is_rejected() {
        let sys = vec![eq(&[0, 1], false), eq(&[0, 1], true)];
        assert!(!is_refutation(&sys, 2, &[]), "empty is not a refutation");
        assert!(!is_refutation(&sys, 2, &[0]), "one consistent equation is not 0=1");
        assert!(is_refutation(&sys, 2, &[0, 1]), "the pair sums to 0=1");
    }
}
