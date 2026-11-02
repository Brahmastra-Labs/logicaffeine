//! XL — algebraic cryptanalysis of multivariate quadratic (MQ) systems by linearization.
//!
//! Multivariate signatures (Oil-and-Vinegar, Rainbow, HFE) hide a secret behind a system of
//! quadratic equations over a finite field; forging a signature or recovering a key reduces
//! to *solving* such a system. XL does it by **linearization**: multiply every equation by
//! every monomial up to a chosen degree `D`, treat each distinct monomial (including the
//! quadratics) as an independent unknown, and solve the resulting *linear* system. When the
//! equations are dense enough — past the *degree of regularity* Prof. Ding formalized — the
//! linear system pins the original variables and the secret falls out.
//!
//! The engine is field-generic over [`Field`]: [`Gf2`] (the multilinear ring where `x² = x`)
//! and [`Gf256`] (the field multivariate signatures actually use, delegating to the
//! pit-of-success-tested [`logicaffeine_base::gf256`]). The linear algebra is exact Gaussian
//! elimination — no floating point.
//!
//! **Contract:** [`xl_solve`] returns `Some(x)` *only after verifying* `x` satisfies the
//! original system. It can therefore never report a spurious break — at worst it misses (needs
//! a higher degree) and returns `None`. A returned solution is a genuine preimage/forgery.

use crate::macaulay::{self, one, Monomial};
use std::collections::{BTreeMap, HashMap};

/// A finite field small enough to enumerate — the arithmetic XL runs its linear algebra over.
pub trait Field: Copy + Eq + std::fmt::Debug {
    /// Whether the polynomial ring is multilinear (`x² = x`). True for GF(2), where we impose
    /// the field equations; false for GF(256), where quadratics are genuine.
    const MULTILINEAR: bool;
    fn zero() -> Self;
    fn one() -> Self;
    fn add(self, o: Self) -> Self;
    fn sub(self, o: Self) -> Self;
    fn mul(self, o: Self) -> Self;
    /// Multiplicative inverse; caller guarantees `self` is non-zero.
    fn inv(self) -> Self;
    fn is_zero(self) -> bool {
        self == Self::zero()
    }
}

/// GF(2) as the multilinear ring: elements `0`/`1`, addition is XOR, multiplication is AND.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Gf2(pub u8);

impl Field for Gf2 {
    const MULTILINEAR: bool = true;
    fn zero() -> Self {
        Gf2(0)
    }
    fn one() -> Self {
        Gf2(1)
    }
    fn add(self, o: Self) -> Self {
        Gf2(self.0 ^ o.0)
    }
    fn sub(self, o: Self) -> Self {
        Gf2(self.0 ^ o.0)
    }
    fn mul(self, o: Self) -> Self {
        Gf2(self.0 & o.0)
    }
    fn inv(self) -> Self {
        debug_assert_eq!(self.0, 1, "GF(2) inverse of 0");
        Gf2(1)
    }
}

/// GF(2^8) — the field of the multivariate-signature attack surface. All arithmetic delegates
/// to [`logicaffeine_base::gf256`] (the exhaustively-verified shared field substrate).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Gf256(pub u8);

impl Field for Gf256 {
    const MULTILINEAR: bool = false;
    fn zero() -> Self {
        Gf256(0)
    }
    fn one() -> Self {
        Gf256(1)
    }
    fn add(self, o: Self) -> Self {
        Gf256(logicaffeine_base::gf256::add(self.0, o.0))
    }
    fn sub(self, o: Self) -> Self {
        Gf256(logicaffeine_base::gf256::sub(self.0, o.0))
    }
    fn mul(self, o: Self) -> Self {
        Gf256(logicaffeine_base::gf256::mul(self.0, o.0))
    }
    fn inv(self) -> Self {
        Gf256(logicaffeine_base::gf256::inv(self.0))
    }
}

/// A polynomial: monomial → coefficient, only non-zero terms stored.
pub type Poly<F> = BTreeMap<Monomial, F>;

/// A multivariate quadratic system in `n` variables: solutions are the common zeros of `polys`.
#[derive(Clone, Debug)]
pub struct MQSystem<F: Field> {
    pub n: usize,
    pub polys: Vec<Poly<F>>,
}

/// Evaluate a polynomial at an assignment `x` (length ≥ every variable index used).
pub fn eval<F: Field>(p: &Poly<F>, x: &[F]) -> F {
    let mut acc = F::zero();
    for (m, &c) in p {
        let mut term = c;
        for &v in m {
            term = term.mul(x[v as usize]);
        }
        acc = acc.add(term);
    }
    acc
}

impl<F: Field> MQSystem<F> {
    /// Does `x` satisfy every equation (i.e. is it a common zero)?
    pub fn satisfied_by(&self, x: &[F]) -> bool {
        self.polys.iter().all(|p| eval(p, x).is_zero())
    }
}

/// Multiply a polynomial by a single monomial in the (possibly multilinear) ring.
fn poly_mul_mono<F: Field>(p: &Poly<F>, t: &Monomial, multilinear: bool) -> Poly<F> {
    let mut r: Poly<F> = BTreeMap::new();
    for (m, &c) in p {
        let mm = macaulay::mul(m, t, multilinear);
        let s = r.get(&mm).copied().unwrap_or_else(F::zero).add(c);
        if s.is_zero() {
            r.remove(&mm);
        } else {
            r.insert(mm, s);
        }
    }
    r
}

/// Reduce augmented `rows` to reduced row-echelon form, pivoting only in the first
/// `pivot_limit` columns. Returns the pivot column of each pivot row, in row order.
fn rref<F: Field>(rows: &mut [Vec<F>], pivot_limit: usize) -> Vec<usize> {
    let mut pivots = Vec::new();
    if rows.is_empty() {
        return pivots;
    }
    let width = rows[0].len();
    let mut r = 0usize;
    for c in 0..pivot_limit {
        let mut piv = None;
        for i in r..rows.len() {
            if !rows[i][c].is_zero() {
                piv = Some(i);
                break;
            }
        }
        let Some(pi) = piv else { continue };
        rows.swap(r, pi);
        let invp = rows[r][c].inv();
        for j in 0..width {
            rows[r][j] = rows[r][j].mul(invp);
        }
        for i in 0..rows.len() {
            if i != r && !rows[i][c].is_zero() {
                let f = rows[i][c];
                for j in 0..width {
                    let t = f.mul(rows[r][j]);
                    rows[i][j] = rows[i][j].sub(t);
                }
            }
        }
        pivots.push(c);
        r += 1;
        if r == rows.len() {
            break;
        }
    }
    pivots
}

/// A particular solution of `A x = b` over `F` (free variables set to zero), or `None` if the
/// system is inconsistent.
fn solve<F: Field>(a: &[Vec<F>], b: &[F]) -> Option<Vec<F>> {
    if a.is_empty() {
        return None;
    }
    let n = a[0].len();
    let mut rows: Vec<Vec<F>> = a
        .iter()
        .zip(b.iter())
        .map(|(row, &rhs)| {
            let mut r = row.clone();
            r.push(rhs);
            r
        })
        .collect();
    let pivots = rref(&mut rows, n);
    for row in &rows {
        if row[..n].iter().all(|c| c.is_zero()) && !row[n].is_zero() {
            return None; // 0 = nonzero
        }
    }
    let mut x = vec![F::zero(); n];
    for (ri, &c) in pivots.iter().enumerate() {
        x[c] = rows[ri][n];
    }
    Some(x)
}

/// Solve an MQ system by XL: linearize at increasing degrees `d` up to `max_degree`, and at
/// each degree read the linear variables off the linear solution and verify them against the
/// original system. Returns a verified common zero, or `None` if none was found by
/// `max_degree`.
pub fn xl_solve<F: Field>(sys: &MQSystem<F>, max_degree: usize) -> Option<Vec<F>> {
    let ml = F::MULTILINEAR;
    let n = sys.n as u32;
    let sys_deg = sys
        .polys
        .iter()
        .flat_map(|p| p.keys())
        .map(|m| m.len())
        .max()
        .unwrap_or(0);
    let start = sys_deg.max(1);
    for d in start..=max_degree {
        // Columns = every monomial up to degree d; the constant (index 0) is known = 1, so it
        // becomes the right-hand side and the remaining monomials are the linearized unknowns.
        let cols = macaulay::monomials_up_to_degree(n, d, ml);
        let mut idx: HashMap<Monomial, usize> = HashMap::with_capacity(cols.len());
        for (i, m) in cols.iter().enumerate() {
            idx.insert(m.clone(), i);
        }
        let num_unknowns = cols.len() - 1; // minus the constant

        let mut a_rows: Vec<Vec<F>> = Vec::new();
        let mut b: Vec<F> = Vec::new();
        for f in &sys.polys {
            let fdeg = f.keys().map(|m| m.len()).max().unwrap_or(0);
            if fdeg > d {
                continue;
            }
            for t in macaulay::monomials_up_to_degree(n, d - fdeg, ml) {
                let ext = poly_mul_mono(f, &t, ml);
                let mut row = vec![F::zero(); num_unknowns];
                let mut rhs = F::zero();
                for (m, &c) in &ext {
                    if m.is_empty() {
                        rhs = rhs.sub(c); // move the constant term to the RHS: A u = −const
                    } else {
                        let j = idx[m] - 1; // shift past the constant column
                        row[j] = row[j].add(c);
                    }
                }
                a_rows.push(row);
                b.push(rhs);
            }
        }

        if let Some(u) = solve(&a_rows, &b) {
            let mut x = vec![F::zero(); sys.n];
            for i in 0..sys.n {
                if let Some(&ci) = idx.get(&vec![i as u32]) {
                    x[i] = u[ci - 1];
                }
            }
            if sys.satisfied_by(&x) {
                return Some(x);
            }
        }
    }
    None
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_proof/xl/tests.rs"]
mod tests;
