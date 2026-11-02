//! Exact linear algebra over ℚ — the certified core of the LOGOS numerical stack.
//!
//! Floating-point LAPACK *runs* a solve; this *proves* one. Every entry is an exact BigInt-backed
//! [`Rational`], so `A·x = b` is decided as an EXACT identity — the residual is identically zero,
//! not "close enough" — and `det A ≠ 0` certifies nonsingularity outright. These are the primitives
//! the proof engine reasons about, and they are FAIL-CLOSED in the same discipline as the mod-p /
//! SoS routes: the certificate (`residual_zero`) is an independent re-multiplication, never trust in
//! the solver. An ill-conditioned system (e.g. a Hilbert matrix) that floating point cannot solve to
//! zero residual is solved here EXACTLY, and the exactness is a checkable fact.

use logicaffeine_base::Rational;

/// A·x over the rationals, computed exactly.
pub fn matvec(a: &[Vec<Rational>], x: &[Rational]) -> Vec<Rational> {
    a.iter()
        .map(|row| {
            row.iter()
                .zip(x)
                .fold(Rational::zero(), |acc, (aij, xj)| acc.add(&aij.mul(xj)))
        })
        .collect()
}

/// Is `A·x == b` exactly, component by component? The independent re-check behind a certified solve.
pub fn residual_is_zero(a: &[Vec<Rational>], x: &[Rational], b: &[Rational]) -> bool {
    let ax = matvec(a, x);
    ax.len() == b.len() && ax.iter().zip(b).all(|(l, r)| l == r)
}

/// Solve `A·x = b` exactly by Gauss–Jordan elimination with nonzero pivoting (n×n).
/// Returns `None` iff `A` is singular. All arithmetic is exact — the answer `1/3` stays `1/3`.
pub fn solve(a: &[Vec<Rational>], b: &[Rational]) -> Option<Vec<Rational>> {
    let n = a.len();
    if n == 0 || a.iter().any(|r| r.len() != n) || b.len() != n {
        return None;
    }
    // augmented [A | b]
    let mut m: Vec<Vec<Rational>> = a
        .iter()
        .zip(b)
        .map(|(row, bi)| {
            let mut r = row.clone();
            r.push(bi.clone());
            r
        })
        .collect();
    for col in 0..n {
        // pivot: the first row at or below the diagonal with a nonzero entry in this column
        let piv = (col..n).find(|&r| !m[r][col].is_zero())?;
        m.swap(col, piv);
        let inv = m[col][col].recip()?; // pivot is nonzero → Some
                                        // scale the pivot row so the pivot becomes exactly 1
        for j in col..=n {
            m[col][j] = m[col][j].mul(&inv);
        }
        // eliminate this column from every other row
        for r in 0..n {
            if r == col {
                continue;
            }
            let f = m[r][col].clone();
            if f.is_zero() {
                continue;
            }
            for j in col..=n {
                m[r][j] = m[r][j].sub(&f.mul(&m[col][j]));
            }
        }
    }
    Some((0..n).map(|i| m[i][n].clone()).collect())
}

/// Exact determinant via forward elimination (product of pivots × permutation sign).
pub fn det(a: &[Vec<Rational>]) -> Rational {
    let n = a.len();
    if n == 0 || a.iter().any(|r| r.len() != n) {
        return Rational::zero();
    }
    let mut m: Vec<Vec<Rational>> = a.to_vec();
    let mut d = Rational::one();
    for col in 0..n {
        let piv = match (col..n).find(|&r| !m[r][col].is_zero()) {
            Some(p) => p,
            None => return Rational::zero(),
        };
        if piv != col {
            m.swap(col, piv);
            d = d.negated();
        }
        d = d.mul(&m[col][col]);
        let inv = m[col][col].recip().expect("nonzero pivot");
        for r in (col + 1)..n {
            let f = m[r][col].mul(&inv);
            if f.is_zero() {
                continue;
            }
            for j in col..n {
                m[r][j] = m[r][j].sub(&f.mul(&m[col][j]));
            }
        }
    }
    d
}

/// A solve carrying its own certificate: the exact solution plus independently re-checked flags that
/// `A·x = b` holds exactly and `det A ≠ 0`. This is what "prove, not just run" means for a solve.
pub struct Certified {
    pub x: Vec<Rational>,
    /// `A·x == b` verified by re-multiplication over exact rationals (not solver trust).
    pub residual_zero: bool,
    /// `det A ≠ 0` — the system has a unique solution.
    pub nonsingular: bool,
}

/// Solve and certify in one call. `None` iff singular.
pub fn certify_solve(a: &[Vec<Rational>], b: &[Rational]) -> Option<Certified> {
    let x = solve(a, b)?;
    let residual_zero = residual_is_zero(a, &x, b);
    let nonsingular = !det(a).is_zero();
    Some(Certified {
        x,
        residual_zero,
        nonsingular,
    })
}

/// An exact LU factorization `P·A = L·U`: `perm` is the row permutation (P), `l` is unit
/// lower-triangular, `u` is upper-triangular. `None` iff singular.
pub struct Lu {
    pub perm: Vec<usize>,
    pub l: Vec<Vec<Rational>>,
    pub u: Vec<Vec<Rational>>,
}

/// Exact LU with partial (nonzero) pivoting. Every multiplier is an exact rational.
pub fn lu(a: &[Vec<Rational>]) -> Option<Lu> {
    let n = a.len();
    if n == 0 || a.iter().any(|r| r.len() != n) {
        return None;
    }
    let mut u = a.to_vec();
    let mut l: Vec<Vec<Rational>> = (0..n)
        .map(|i| {
            (0..n)
                .map(|j| {
                    if i == j {
                        Rational::one()
                    } else {
                        Rational::zero()
                    }
                })
                .collect()
        })
        .collect();
    let mut perm: Vec<usize> = (0..n).collect();
    for k in 0..n {
        let piv = (k..n).find(|&r| !u[r][k].is_zero())?;
        if piv != k {
            u.swap(k, piv);
            perm.swap(k, piv);
            // the already-computed multipliers (columns < k) travel with their rows
            for j in 0..k {
                let tmp = l[k][j].clone();
                l[k][j] = l[piv][j].clone();
                l[piv][j] = tmp;
            }
        }
        let inv = u[k][k].recip()?;
        for i in (k + 1)..n {
            let f = u[i][k].mul(&inv);
            l[i][k] = f.clone();
            for j in k..n {
                u[i][j] = u[i][j].sub(&f.mul(&u[k][j]));
            }
        }
    }
    Some(Lu { perm, l, u })
}

/// Exact matrix product `A·B`.
pub fn matmul(a: &[Vec<Rational>], b: &[Vec<Rational>]) -> Vec<Vec<Rational>> {
    let (n, m, p) = (a.len(), b.len(), b[0].len());
    (0..n)
        .map(|i| {
            (0..p)
                .map(|j| (0..m).fold(Rational::zero(), |acc, k| acc.add(&a[i][k].mul(&b[k][j]))))
                .collect()
        })
        .collect()
}

/// Certify a factorization: is `P·A == L·U` exactly? (P·A is A with its rows permuted by `perm`.)
pub fn lu_is_exact(a: &[Vec<Rational>], f: &Lu) -> bool {
    let pa: Vec<Vec<Rational>> = f.perm.iter().map(|&i| a[i].clone()).collect();
    matmul(&f.l, &f.u) == pa
}

/// The exact n×n identity matrix.
pub fn identity(n: usize) -> Vec<Vec<Rational>> {
    (0..n)
        .map(|i| {
            (0..n)
                .map(|j| {
                    if i == j {
                        Rational::one()
                    } else {
                        Rational::zero()
                    }
                })
                .collect()
        })
        .collect()
}

/// Exact transpose.
pub fn transpose(a: &[Vec<Rational>]) -> Vec<Vec<Rational>> {
    if a.is_empty() {
        return Vec::new();
    }
    let (n, m) = (a.len(), a[0].len());
    (0..m)
        .map(|j| (0..n).map(|i| a[i][j].clone()).collect())
        .collect()
}

/// Exact matrix inverse via Gauss–Jordan on `[A | I]`. `None` iff singular.
pub fn inverse(a: &[Vec<Rational>]) -> Option<Vec<Vec<Rational>>> {
    let n = a.len();
    if n == 0 || a.iter().any(|r| r.len() != n) {
        return None;
    }
    // augmented [A | I], width 2n
    let mut m: Vec<Vec<Rational>> = a
        .iter()
        .enumerate()
        .map(|(i, row)| {
            let mut r = row.clone();
            for j in 0..n {
                r.push(if i == j {
                    Rational::one()
                } else {
                    Rational::zero()
                });
            }
            r
        })
        .collect();
    for col in 0..n {
        let piv = (col..n).find(|&r| !m[r][col].is_zero())?;
        m.swap(col, piv);
        let inv = m[col][col].recip()?;
        for j in col..(2 * n) {
            m[col][j] = m[col][j].mul(&inv);
        }
        for r in 0..n {
            if r == col {
                continue;
            }
            let f = m[r][col].clone();
            if f.is_zero() {
                continue;
            }
            for j in col..(2 * n) {
                m[r][j] = m[r][j].sub(&f.mul(&m[col][j]));
            }
        }
    }
    Some(
        (0..n)
            .map(|i| (0..n).map(|j| m[i][n + j].clone()).collect())
            .collect(),
    )
}

/// Exact rank (works for rectangular matrices) via row reduction — the number of nonzero pivots.
pub fn rank(a: &[Vec<Rational>]) -> usize {
    if a.is_empty() {
        return 0;
    }
    let (rows, cols) = (a.len(), a[0].len());
    let mut m = a.to_vec();
    let mut prow = 0usize;
    for col in 0..cols {
        if prow == rows {
            break;
        }
        let piv = match (prow..rows).find(|&r| !m[r][col].is_zero()) {
            Some(p) => p,
            None => continue,
        };
        m.swap(prow, piv);
        let inv = m[prow][col].recip().expect("nonzero pivot");
        for r in 0..rows {
            if r == prow {
                continue;
            }
            let f = m[r][col].mul(&inv);
            if f.is_zero() {
                continue;
            }
            for j in col..cols {
                m[r][j] = m[r][j].sub(&f.mul(&m[prow][j]));
            }
        }
        prow += 1;
    }
    prow
}

/// Is `A·B == I` exactly? (independent re-check behind a certified inverse)
pub fn is_inverse(a: &[Vec<Rational>], b: &[Vec<Rational>]) -> bool {
    matmul(a, b) == identity(a.len())
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_proof/linalg/tests.rs"]
mod tests;
