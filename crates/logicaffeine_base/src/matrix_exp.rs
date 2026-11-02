//! The exact matrix exponential `e^{t·M}` of a rational matrix.
//!
//! The characteristic polynomial `χ` of `M` annihilates it, so `e^{t·M} = r(M)` for the polynomial
//! `r` of degree below `n` that agrees with `e^{t·x}` on the roots of `χ` to their multiplicities:
//! `r^{(j)}(λ) = t^j·e^{t·λ}` for `j < m(λ)` (Hermite interpolation). Writing `r = Σ c_k·x^k`, those
//! conditions are a confluent Vandermonde system `V·c = (t^j·e^{t·λ})`, so `c = V⁻¹·(t^j·e^{t·λ})`
//! with `V⁻¹` exact over the complex algebraic numbers, and each entry of `e^{t·M}` is a sum
//! `Σ_λ β_λ·e^{t·λ}`. A conjugate pair of eigenvalues `a ± b·i` contributes
//! `2·e^{t·a}·(Re β·cos(t·b) − Im β·sin(t·b))`, so every entry is a real number in the exponential
//! exact form: `e^{[[0, 1], [−1, 0]]}` is `[[cos(1), sin(1)], [−sin(1), cos(1)]]` exactly.

use crate::algebraic::Algebraic;
use crate::complex_algebraic::{complex_roots, ComplexAlgebraic};
use crate::exact_real::ExactReal;
use crate::numeric::Rational;
use crate::poly::Poly;

/// `det(x·I − M)` by Faddeev–LeVerrier; `None` for a matrix that is not square.
pub fn characteristic_polynomial(m: &[Vec<Rational>]) -> Option<Poly> {
    let n = m.len();
    if n == 0 || m.iter().any(|r| r.len() != n) {
        return None;
    }
    let mul = |a: &[Vec<Rational>], b: &[Vec<Rational>]| -> Vec<Vec<Rational>> {
        (0..n)
            .map(|i| {
                (0..n)
                    .map(|j| (0..n).fold(Rational::zero(), |acc, k| acc.add(&a[i][k].mul(&b[k][j]))))
                    .collect()
            })
            .collect()
    };
    let mut c = vec![Rational::zero(); n + 1];
    c[n] = Rational::one();
    let mut acc = vec![vec![Rational::zero(); n]; n];
    for k in 1..=n {
        acc = mul(m, &acc);
        for (i, row) in acc.iter_mut().enumerate() {
            row[i] = row[i].add(&c[n - k + 1]);
        }
        let am = mul(m, &acc);
        let trace = (0..n).fold(Rational::zero(), |t, i| t.add(&am[i][i]));
        c[n - k] = trace
            .negated()
            .div(&Rational::from_i64(k as i64))
            .expect("k ≥ 1");
    }
    Some(Poly::from_coeffs(c))
}

/// `k!/(k − j)!`, the `j`-th derivative's coefficient of `x^k`; `0` for `j > k`.
fn falling(k: usize, j: usize) -> i64 {
    if j > k {
        return 0;
    }
    ((k - j + 1)..=k).map(|v| v as i64).product::<i64>().max(1)
}

/// The inverse of a square matrix over the complex algebraic numbers; `None` when singular.
fn invert(mut a: Vec<Vec<ComplexAlgebraic>>) -> Option<Vec<Vec<ComplexAlgebraic>>> {
    let n = a.len();
    let mut inv: Vec<Vec<ComplexAlgebraic>> = (0..n)
        .map(|i| {
            (0..n)
                .map(|j| {
                    if i == j {
                        ComplexAlgebraic::one()
                    } else {
                        ComplexAlgebraic::zero()
                    }
                })
                .collect()
        })
        .collect();
    for col in 0..n {
        let pivot = (col..n).find(|&r| !a[r][col].is_zero())?;
        a.swap(col, pivot);
        inv.swap(col, pivot);
        let scale = a[col][col].recip()?;
        for j in 0..n {
            a[col][j] = a[col][j].mul(&scale);
            inv[col][j] = inv[col][j].mul(&scale);
        }
        for r in 0..n {
            if r == col || a[r][col].is_zero() {
                continue;
            }
            let f = a[r][col].clone();
            for j in 0..n {
                let (pa, pi) = (a[col][j].mul(&f), inv[col][j].mul(&f));
                a[r][j] = a[r][j].sub(&pa);
                inv[r][j] = inv[r][j].sub(&pi);
            }
        }
    }
    Some(inv)
}

/// `e^{t·M}`, every entry exact; `None` for a matrix that is not square.
pub fn exp_matrix(m: &[Vec<Rational>], t: &Rational) -> Option<Vec<Vec<ExactReal>>> {
    let n = m.len();
    let chi = characteristic_polynomial(m)?;
    let mut nodes: Vec<(ComplexAlgebraic, usize)> = Vec::new();
    for (factor, multiplicity) in chi.factor_over_q() {
        for lambda in complex_roots(&factor) {
            nodes.push((lambda, multiplicity as usize));
        }
    }
    let mut rows: Vec<Vec<ComplexAlgebraic>> = Vec::with_capacity(n);
    let mut labels: Vec<(usize, usize)> = Vec::with_capacity(n);
    for (i, (lambda, multiplicity)) in nodes.iter().enumerate() {
        for j in 0..*multiplicity {
            rows.push(
                (0..n)
                    .map(|k| {
                        let f = falling(k, j);
                        if f == 0 {
                            ComplexAlgebraic::zero()
                        } else {
                            lambda.pow((k - j) as u32).scale(&Rational::from_i64(f))
                        }
                    })
                    .collect(),
            );
            labels.push((i, j));
        }
    }
    let w = invert(rows)?;
    let identity: Vec<Vec<Rational>> = (0..n)
        .map(|i| {
            (0..n)
                .map(|j| if i == j { Rational::one() } else { Rational::zero() })
                .collect()
        })
        .collect();
    let mut powers = vec![identity];
    for _ in 1..n {
        let last = powers.last().expect("the identity is there");
        let next: Vec<Vec<Rational>> = (0..n)
            .map(|i| {
                (0..n)
                    .map(|j| (0..n).fold(Rational::zero(), |acc, k| acc.add(&last[i][k].mul(&m[k][j]))))
                    .collect()
            })
            .collect();
        powers.push(next);
    }
    let mut out = vec![vec![ExactReal::from_i64(0); n]; n];
    for (r, row) in out.iter_mut().enumerate() {
        for (s, entry) in row.iter_mut().enumerate() {
            let mut weights = vec![ComplexAlgebraic::zero(); nodes.len()];
            for (col, (node, j)) in labels.iter().enumerate() {
                let alpha = (0..n).fold(ComplexAlgebraic::zero(), |acc, k| {
                    acc.add(&w[k][col].scale(&powers[k][r][s]))
                });
                weights[*node] = weights[*node].add(&alpha.scale(&t.pow(*j as i32).expect("t^j")));
            }
            *entry = assemble(&nodes, &weights, t);
        }
    }
    Some(out)
}

/// `Σ_λ β_λ·e^{t·λ}`, a conjugate pair taken once by its upper root.
fn assemble(nodes: &[(ComplexAlgebraic, usize)], weights: &[ComplexAlgebraic], t: &Rational) -> ExactReal {
    let t_alg = Algebraic::from_rational(t.clone());
    let mut total = ExactReal::from_i64(0);
    for ((lambda, _), beta) in nodes.iter().zip(weights) {
        if beta.is_zero() {
            continue;
        }
        let growth = ExactReal::from_algebraic(lambda.re.mul(&t_alg)).exp();
        if lambda.is_real() {
            debug_assert!(beta.is_real(), "a real eigenvalue's weight is real");
            total = total.add(&ExactReal::from_algebraic(beta.re.clone()).mul(&growth));
        } else if lambda.im.signum() > 0 {
            let angle = ExactReal::from_algebraic(lambda.im.mul(&t_alg));
            let two = Algebraic::from_i64(2);
            let cosine = ExactReal::from_algebraic(beta.re.mul(&two)).mul(&angle.cos());
            let sine = ExactReal::from_algebraic(beta.im.mul(&two)).mul(&angle.sin());
            total = total.add(&growth.mul(&cosine.sub(&sine)));
        }
    }
    total
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_base/matrix_exp/tests.rs"]
mod tests;
