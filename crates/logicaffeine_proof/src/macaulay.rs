//! Monomials and the Macaulay matrix — the combinatorial skeleton of the XL algorithm.
//!
//! XL ("eXtended Linearization", Courtois–Klimov–Patarin–Shamir 2000) solves a system of
//! multivariate polynomial equations by *linearizing*: multiply every equation by every
//! monomial up to a chosen degree, treat each distinct monomial as an independent unknown,
//! and solve the resulting linear system. This is the engine behind Gröbner-basis-style
//! algebraic cryptanalysis of multivariate schemes (Rainbow / Oil-and-Vinegar / HFE) — the
//! attack surface Prof. Ding formalized through the *degree of regularity*.
//!
//! This module owns only the field-agnostic bookkeeping: how monomials are represented,
//! enumerated up to a degree bound, and multiplied. The field arithmetic and the linear
//! solve live in [`crate::xl`].
//!
//! A monomial is the **sorted multiset of its variable indices** — `[]` is the constant `1`,
//! `[i]` is `x_i`, `[i, i]` is `x_i²`, `[i, j]` (i ≤ j) is `x_i x_j`. The degree is the length.
//! In a multilinear ring (GF(2), where `x² = x`) monomials are squarefree (strictly
//! increasing); otherwise repeats encode true exponents.

/// A monomial: the sorted multiset of variable indices it contains. `Vec::new()` is `1`.
pub type Monomial = Vec<u32>;

/// The constant monomial `1`.
pub fn one() -> Monomial {
    Vec::new()
}

/// The degree of a monomial (its total number of variable factors).
pub fn degree(m: &Monomial) -> usize {
    m.len()
}

/// The product of two monomials. When `multilinear`, repeated variables collapse (`x² = x`),
/// the correct rule for GF(2); otherwise exponents accumulate.
pub fn mul(a: &Monomial, b: &Monomial, multilinear: bool) -> Monomial {
    let mut m: Monomial = a.iter().chain(b.iter()).copied().collect();
    m.sort_unstable();
    if multilinear {
        m.dedup();
    }
    m
}

/// Every monomial in `n` variables of degree exactly `deg`, listed with variable indices in
/// nondecreasing order. Strictly increasing (squarefree) when `multilinear`.
pub fn monomials_of_degree(n: u32, deg: usize, multilinear: bool) -> Vec<Monomial> {
    fn rec(
        start: u32,
        n: u32,
        remaining: usize,
        multilinear: bool,
        cur: &mut Monomial,
        out: &mut Vec<Monomial>,
    ) {
        if remaining == 0 {
            out.push(cur.clone());
            return;
        }
        for v in start..n {
            cur.push(v);
            let next = if multilinear { v + 1 } else { v };
            rec(next, n, remaining - 1, multilinear, cur, out);
            cur.pop();
        }
    }
    let mut out = Vec::new();
    let mut cur = Vec::new();
    rec(0, n, deg, multilinear, &mut cur, &mut out);
    out
}

/// Every monomial in `n` variables of degree `0..=max_deg`. The constant `1` is always first
/// (index `0`), which the XL solver relies on to read off constant terms.
pub fn monomials_up_to_degree(n: u32, max_deg: usize, multilinear: bool) -> Vec<Monomial> {
    let mut out = vec![one()];
    for deg in 1..=max_deg {
        out.extend(monomials_of_degree(n, deg, multilinear));
    }
    out
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_proof/macaulay/tests.rs"]
mod tests;
