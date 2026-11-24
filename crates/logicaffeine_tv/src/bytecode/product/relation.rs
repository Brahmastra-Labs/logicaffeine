//! The algebraic relations among the values a loop head holds, read off concrete runs: the candidate invariants for a
//! loop the compiled code computes in closed form.
//!
//! Each row is one state a run reached the head in, each column a variable (a value the head holds, or an input of the
//! run). A polynomial of degree at most `d` that vanishes on every row is a vector in the null space of the matrix of
//! its monomials' values, found exactly, over the rationals. The relations are read off by increasing degree. The
//! variables a linear relation determines are dropped first, and at each higher degree only a relation that is not a
//! lower one times a polynomial is kept, so `2·total + i = i²` is offered once, not again times `i` and times `total`.
//! A relation holds on the runs it was read from; the proof keeps only the inductive ones.

use std::collections::HashMap;

use logicaffeine_base::numeric::{BigInt, Rational};

/// A monomial: each variable's exponent.
pub type Monomial = Vec<u32>;

/// `Σ coefficient · monomial = 0`, with integer coefficients whose gcd is one, the first one positive.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Relation {
    pub terms: Vec<(BigInt, Monomial)>,
}

impl Relation {
    pub fn degree(&self) -> u32 {
        self.terms.iter().map(|(_, m)| m.iter().sum::<u32>()).max().unwrap_or(0)
    }
}

/// Every relation of degree at most `max_degree` the rows satisfy, up to the multiples of lower ones, in the variables
/// a linear relation does not determine; the search stops at the first degree with more than `max_monomials`
/// monomials.
pub fn relations(rows: &[Vec<BigInt>], max_degree: u32, max_monomials: usize) -> Vec<Relation> {
    let Some(width) = rows.first().map(Vec::len) else {
        return Vec::new();
    };
    let rows: Vec<Vec<Rational>> =
        rows.iter().map(|r| r.iter().map(|x| Rational::from_bigint(x.clone())).collect()).collect();
    let all: Vec<usize> = (0..width).collect();

    let linear = monomials(&all, width, 1);
    let mut determined = vec![false; width];
    let mut found: Vec<Relation> = Vec::new();
    let mut echelon = Echelon::default();
    for v in null_space(&rows, &linear) {
        if let Some(reduced) = echelon.insert(v) {
            if let Some(var) = linear[leading(&reduced)].iter().position(|&e| e == 1) {
                determined[var] = true;
            }
            found.push(relation(&reduced, &linear));
        }
    }

    let free: Vec<usize> = all.iter().copied().filter(|&v| !determined[v]).collect();
    let mut higher: Vec<Relation> = Vec::new();
    for degree in 2..=max_degree {
        let cols = monomials(&free, width, degree);
        if cols.len() > max_monomials {
            break;
        }
        let index: HashMap<&Monomial, usize> = cols.iter().enumerate().map(|(i, m)| (m, i)).collect();
        let mut span = Echelon::default();
        for r in &higher {
            for m in monomials(&free, width, degree - r.degree()) {
                let mut v = vec![Rational::zero(); cols.len()];
                for (c, term) in &r.terms {
                    let product: Monomial = term.iter().zip(&m).map(|(a, b)| a + b).collect();
                    v[index[&product]] = Rational::from_bigint(c.clone());
                }
                span.insert(v);
            }
        }
        let mut new = Vec::new();
        for v in null_space(&rows, &cols) {
            if let Some(reduced) = span.insert(v) {
                new.push(relation(&reduced, &cols));
            }
        }
        higher.extend(new);
    }
    found.extend(higher);
    found
}

/// Every monomial in `vars` of degree at most `degree`, highest degree first, the constant last.
fn monomials(vars: &[usize], width: usize, degree: u32) -> Vec<Monomial> {
    fn go(vars: &[usize], left: u32, at: Monomial, out: &mut Vec<Monomial>) {
        match vars.split_first() {
            None => out.push(at),
            Some((&v, rest)) => {
                for e in (0..=left).rev() {
                    let mut m = at.clone();
                    m[v] = e;
                    go(rest, left - e, m, out);
                }
            }
        }
    }
    let mut out = Vec::new();
    go(vars, degree, vec![0; width], &mut out);
    out.sort_by(|a, b| b.iter().sum::<u32>().cmp(&a.iter().sum::<u32>()).then_with(|| b.cmp(a)));
    out
}

fn value(m: &Monomial, row: &[Rational]) -> Rational {
    let mut acc = Rational::one();
    for (x, &e) in row.iter().zip(m) {
        for _ in 0..e {
            acc = acc.mul(x);
        }
    }
    acc
}

/// A basis of the polynomials over `cols` that vanish on every row. A row every polynomial found so far vanishes on
/// lies in the span of the rows already taken and is passed over, so only a row that breaks one is reduced.
fn null_space(rows: &[Vec<Rational>], cols: &[Monomial]) -> Vec<Vec<Rational>> {
    let mut echelon = Echelon::default();
    let mut basis = echelon.null(cols.len());
    for row in rows {
        let v: Vec<Rational> = cols.iter().map(|m| value(m, row)).collect();
        if basis.iter().all(|b| dot(b, &v).is_zero()) {
            continue;
        }
        echelon.insert(v);
        if echelon.rows.len() == cols.len() {
            return Vec::new();
        }
        basis = echelon.null(cols.len());
    }
    basis
}

fn dot(a: &[Rational], b: &[Rational]) -> Rational {
    a.iter().zip(b).fold(Rational::zero(), |acc, (x, y)| acc.add(&x.mul(y)))
}

fn leading(v: &[Rational]) -> usize {
    v.iter().position(|x| !x.is_zero()).expect("a nonzero vector")
}

/// A reduced row echelon form, kept reduced as rows are added.
#[derive(Default)]
struct Echelon {
    rows: Vec<(usize, Vec<Rational>)>,
}

impl Echelon {
    /// Add `v`; its remainder against the rows already in, scaled so its leading entry is one, when that is not zero.
    fn insert(&mut self, mut v: Vec<Rational>) -> Option<Vec<Rational>> {
        for (p, row) in &self.rows {
            if !v[*p].is_zero() {
                let f = v[*p].clone();
                for (x, r) in v.iter_mut().zip(row) {
                    *x = x.sub(&f.mul(r));
                }
            }
        }
        let p = v.iter().position(|x| !x.is_zero())?;
        let lead = v[p].clone();
        for x in v.iter_mut() {
            *x = x.div(&lead).expect("a nonzero leading entry");
        }
        for (_, row) in self.rows.iter_mut() {
            if !row[p].is_zero() {
                let f = row[p].clone();
                for (x, r) in row.iter_mut().zip(&v) {
                    *x = x.sub(&f.mul(r));
                }
            }
        }
        self.rows.push((p, v.clone()));
        Some(v)
    }

    /// The null space of the rows: one vector per column without a pivot.
    fn null(&self, width: usize) -> Vec<Vec<Rational>> {
        let pivots: Vec<usize> = self.rows.iter().map(|(p, _)| *p).collect();
        (0..width)
            .filter(|c| !pivots.contains(c))
            .map(|free| {
                let mut v = vec![Rational::zero(); width];
                v[free] = Rational::one();
                for (p, row) in &self.rows {
                    v[*p] = row[free].negated();
                }
                v
            })
            .collect()
    }
}

/// The vector over `cols` as a relation with coprime integer coefficients, the first one positive.
fn relation(v: &[Rational], cols: &[Monomial]) -> Relation {
    let mut den = BigInt::from_i64(1);
    for x in v {
        let d = x.denominator();
        let g = den.gcd(d);
        den = den.mul(d).div_rem(&g).expect("a gcd is not zero").0;
    }
    let ints: Vec<BigInt> = v
        .iter()
        .map(|x| x.mul(&Rational::from_bigint(den.clone())).to_bigint().expect("scaled by every denominator"))
        .collect();
    let g = ints.iter().fold(BigInt::zero(), |g, x| g.gcd(x));
    let lead_negative = ints.iter().find(|x| !x.is_zero()).is_some_and(BigInt::is_negative);
    let terms = ints
        .into_iter()
        .zip(cols)
        .filter(|(c, _)| !c.is_zero())
        .map(|(c, m)| {
            let c = c.div_rem(&g).expect("the gcd of a nonzero vector is not zero").0;
            (if lead_negative { c.negated() } else { c }, m.clone())
        })
        .collect();
    Relation { terms }
}

#[cfg(test)]
#[path = "../../../../logicaffeine_tests/unit/logicaffeine_tv/bytecode/product/relation/tests.rs"]
mod tests;
