//! Elimination: polynomials in several variables over the rationals, held sparse, and their resultants.
//!
//! A resultant eliminates one variable from two polynomials: it vanishes exactly where they share a
//! root in that variable (or both leading coefficients vanish). The norm of a polynomial whose
//! coefficients are algebraic numbers is a tower of resultants against their minimal polynomials,
//! and the projection of a cylindrical decomposition is built from them.
//!
//! The resultant is computed by evaluation and interpolation. Specializing every other variable at a
//! point where neither polynomial's leading coefficient in the eliminated variable vanishes commutes
//! with taking the resultant, and the resultant's degree in each other variable is bounded by the
//! degrees of the two polynomials, so the resultant in `v` of polynomials in `n` variables is
//! interpolated, one variable at a time, from resultants of polynomials in one.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use crate::numeric::Rational;
use crate::poly::Poly;

/// A monomial's exponents, variable `i` at index `i`, with no trailing zero.
type Exponents = Vec<u32>;

fn trimmed(mut e: Exponents) -> Exponents {
    while e.last() == Some(&0) {
        e.pop();
    }
    e
}

/// A polynomial in variables numbered from 0, with rational coefficients. No coefficient is zero, so
/// equal polynomials are structurally equal.
#[derive(Clone, PartialEq, Eq, Hash, Default)]
pub struct Polynomial {
    terms: BTreeMap<Exponents, Rational>,
}

impl Polynomial {
    pub fn zero() -> Polynomial {
        Polynomial::default()
    }

    pub fn one() -> Polynomial {
        Polynomial::constant(Rational::one())
    }

    pub fn constant(c: Rational) -> Polynomial {
        Polynomial::monomial(c, Vec::new())
    }

    /// Variable `i`.
    pub fn var(i: usize) -> Polynomial {
        let mut e = vec![0; i + 1];
        e[i] = 1;
        Polynomial::monomial(Rational::one(), e)
    }

    /// `c · Π x_i^e_i`.
    pub fn monomial(c: Rational, exponents: Vec<u32>) -> Polynomial {
        let mut terms = BTreeMap::new();
        if !c.is_zero() {
            terms.insert(trimmed(exponents), c);
        }
        Polynomial { terms }
    }

    /// The univariate `p` in variable `v`.
    pub fn from_univariate(p: &Poly, v: usize) -> Polynomial {
        let mut out = Polynomial::zero();
        for (k, c) in p.coeffs().iter().enumerate() {
            let mut e = vec![0; v + 1];
            e[v] = k as u32;
            out.add_term(e, c.clone());
        }
        out
    }

    pub fn is_zero(&self) -> bool {
        self.terms.is_empty()
    }

    /// The value of a polynomial with no variable; `None` for any other.
    pub fn as_constant(&self) -> Option<Rational> {
        match self.terms.len() {
            0 => Some(Rational::zero()),
            1 => self.terms.get(&Vec::new()).cloned(),
            _ => None,
        }
    }

    /// The terms, exponents with no trailing zero, in increasing order of exponents.
    pub fn terms(&self) -> impl Iterator<Item = (&[u32], &Rational)> {
        self.terms.iter().map(|(e, c)| (e.as_slice(), c))
    }

    fn add_term(&mut self, e: Exponents, c: Rational) {
        if c.is_zero() {
            return;
        }
        let e = trimmed(e);
        let sum = match self.terms.get(&e) {
            Some(old) => old.add(&c),
            None => c,
        };
        if sum.is_zero() {
            self.terms.remove(&e);
        } else {
            self.terms.insert(e, sum);
        }
    }

    pub fn add(&self, o: &Polynomial) -> Polynomial {
        let mut out = self.clone();
        for (e, c) in &o.terms {
            out.add_term(e.clone(), c.clone());
        }
        out
    }

    pub fn neg(&self) -> Polynomial {
        Polynomial {
            terms: self.terms.iter().map(|(e, c)| (e.clone(), c.negated())).collect(),
        }
    }

    pub fn sub(&self, o: &Polynomial) -> Polynomial {
        self.add(&o.neg())
    }

    pub fn scale(&self, k: &Rational) -> Polynomial {
        if k.is_zero() {
            return Polynomial::zero();
        }
        Polynomial {
            terms: self.terms.iter().map(|(e, c)| (e.clone(), c.mul(k))).collect(),
        }
    }

    pub fn mul(&self, o: &Polynomial) -> Polynomial {
        let mut out = Polynomial::zero();
        for (e1, c1) in &self.terms {
            for (e2, c2) in &o.terms {
                let n = e1.len().max(e2.len());
                let e: Exponents = (0..n)
                    .map(|i| e1.get(i).copied().unwrap_or(0) + e2.get(i).copied().unwrap_or(0))
                    .collect();
                out.add_term(e, c1.mul(c2));
            }
        }
        out
    }

    pub fn pow(&self, k: u32) -> Polynomial {
        let mut acc = Polynomial::one();
        let mut base = self.clone();
        let mut e = k;
        while e > 0 {
            if e & 1 == 1 {
                acc = acc.mul(&base);
            }
            e >>= 1;
            if e > 0 {
                base = base.mul(&base);
            }
        }
        acc
    }

    /// The degree in variable `v`; zero for the zero polynomial.
    pub fn degree_in(&self, v: usize) -> u32 {
        self.terms.keys().map(|e| e.get(v).copied().unwrap_or(0)).max().unwrap_or(0)
    }

    /// The variables that occur.
    pub fn variables(&self) -> BTreeSet<usize> {
        let mut out = BTreeSet::new();
        for e in self.terms.keys() {
            for (i, k) in e.iter().enumerate() {
                if *k > 0 {
                    out.insert(i);
                }
            }
        }
        out
    }

    /// The coefficient of `x_v^k`, a polynomial in the other variables.
    pub fn coefficient_in(&self, v: usize, k: u32) -> Polynomial {
        let mut out = Polynomial::zero();
        for (e, c) in &self.terms {
            if e.get(v).copied().unwrap_or(0) == k {
                let mut e = e.clone();
                if v < e.len() {
                    e[v] = 0;
                }
                out.add_term(e, c.clone());
            }
        }
        out
    }

    /// The coefficient of the highest power of `x_v`.
    pub fn leading_in(&self, v: usize) -> Polynomial {
        self.coefficient_in(v, self.degree_in(v))
    }

    /// `x_v` set to `x`.
    pub fn substitute(&self, v: usize, x: &Rational) -> Polynomial {
        let mut out = Polynomial::zero();
        for (e, c) in &self.terms {
            let k = e.get(v).copied().unwrap_or(0);
            let mut e = e.clone();
            if v < e.len() {
                e[v] = 0;
            }
            let factor = x.pow(k as i32).expect("a nonnegative power");
            out.add_term(e, c.mul(&factor));
        }
        out
    }

    /// `x_v` replaced by the polynomial `p`.
    pub fn compose(&self, v: usize, p: &Polynomial) -> Polynomial {
        let mut out = Polynomial::zero();
        for k in 0..=self.degree_in(v) {
            let c = self.coefficient_in(v, k);
            if !c.is_zero() {
                out = out.add(&c.mul(&p.pow(k)));
            }
        }
        out
    }

    /// `∂/∂x_v`.
    pub fn derivative(&self, v: usize) -> Polynomial {
        let mut out = Polynomial::zero();
        for (e, c) in &self.terms {
            let k = e.get(v).copied().unwrap_or(0);
            if k == 0 {
                continue;
            }
            let mut e = e.clone();
            e[v] = k - 1;
            out.add_term(e, c.mul(&Rational::from_i64(i64::from(k))));
        }
        out
    }

    /// The polynomial as one in `x_v` alone; `None` when another variable occurs.
    pub fn to_univariate(&self, v: usize) -> Option<Poly> {
        if self.variables().iter().any(|w| *w != v) {
            return None;
        }
        let mut c = vec![Rational::zero(); self.degree_in(v) as usize + 1];
        for (e, k) in &self.terms {
            c[e.get(v).copied().unwrap_or(0) as usize] = k.clone();
        }
        Some(Poly::from_coeffs(c))
    }

    /// The resultant in `x_v`: `lc^{deg o} · Π o(α)` over the roots `α` in `x_v` of `self`, a
    /// polynomial in the other variables, by the univariate resultant's convention ([`Poly::resultant`]).
    /// Zero when either is zero; `self^{deg o}` when `self` does not hold `x_v`.
    pub fn resultant(&self, o: &Polynomial, v: usize) -> Polynomial {
        if self.is_zero() || o.is_zero() {
            return Polynomial::zero();
        }
        let (da, db) = (self.degree_in(v), o.degree_in(v));
        if da == 0 {
            return self.pow(db);
        }
        if db == 0 {
            return o.pow(da);
        }
        let others: BTreeSet<usize> = self.variables().union(&o.variables()).copied().filter(|w| *w != v).collect();
        let Some(&w) = others.iter().next_back() else {
            let a = self.to_univariate(v).expect("only x_v occurs");
            let b = o.to_univariate(v).expect("only x_v occurs");
            return Polynomial::constant(a.resultant(&b));
        };
        let bound = da * o.degree_in(w) + db * self.degree_in(w);
        let (la, lb) = (self.leading_in(v), o.leading_in(v));
        let mut nodes: Vec<Rational> = Vec::new();
        let mut values: Vec<Polynomial> = Vec::new();
        for x in points() {
            if nodes.len() > bound as usize {
                break;
            }
            if la.substitute(w, &x).is_zero() || lb.substitute(w, &x).is_zero() {
                continue;
            }
            values.push(self.substitute(w, &x).resultant(&o.substitute(w, &x), v));
            nodes.push(x);
        }
        interpolate(w, &nodes, values)
    }
}

/// `0, 1, -1, 2, -2, …`: the points a variable is specialized at.
fn points() -> impl Iterator<Item = Rational> {
    (0i64..).map(|i| Rational::from_i64(if i % 2 == 1 { (i + 1) / 2 } else { -(i / 2) }))
}

/// The polynomial in `x_w` of degree below `nodes.len()` taking `values[i]` at `nodes[i]`, by
/// Newton's divided differences.
fn interpolate(w: usize, nodes: &[Rational], mut values: Vec<Polynomial>) -> Polynomial {
    let n = nodes.len();
    for j in 1..n {
        for i in (j..n).rev() {
            let gap = nodes[i].sub(&nodes[i - j]);
            let inv = gap.recip().expect("the nodes are distinct");
            values[i] = values[i].sub(&values[i - 1]).scale(&inv);
        }
    }
    let mut out = values[n - 1].clone();
    for i in (0..n - 1).rev() {
        let factor = Polynomial::var(w).sub(&Polynomial::constant(nodes[i].clone()));
        out = out.mul(&factor).add(&values[i]);
    }
    out
}

impl fmt::Debug for Polynomial {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self}")
    }
}

impl fmt::Display for Polynomial {
    /// Terms from the highest exponents down, variable `i` written `x{i}`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_zero() {
            return f.write_str("0");
        }
        for (n, (e, c)) in self.terms.iter().rev().enumerate() {
            let monomial: Vec<String> = e
                .iter()
                .enumerate()
                .filter(|(_, k)| **k > 0)
                .map(|(i, k)| if *k == 1 { format!("x{i}") } else { format!("x{i}^{k}") })
                .collect();
            let negative = c.is_negative();
            let magnitude = if negative { c.negated() } else { c.clone() };
            let body = match (monomial.is_empty(), magnitude == Rational::one()) {
                (true, _) => magnitude.to_string(),
                (false, true) => monomial.join("·"),
                (false, false) => format!("{magnitude}·{}", monomial.join("·")),
            };
            match (n, negative) {
                (0, true) => write!(f, "-{body}")?,
                (0, false) => write!(f, "{body}")?,
                (_, true) => write!(f, " - {body}")?,
                (_, false) => write!(f, " + {body}")?,
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_base/elimination/tests.rs"]
mod tests;
