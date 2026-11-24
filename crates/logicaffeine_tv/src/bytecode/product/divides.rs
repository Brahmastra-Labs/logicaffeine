//! The facts a solver needs about a division by a constant that a polynomial always meets exactly. For a polynomial
//! `P` with integer coefficients and a constant `c > 0`, `P(x) mod c` depends only on `x mod c`, so `c` divides `P` at
//! every integer point exactly when it divides it at each of the `c^k` points whose coordinates are residues. When it
//! does, `c · (P div c) = P` is a lemma the solver may use: `n·(n+1) div 2` is then half of `n·(n+1)`, which nonlinear
//! arithmetic does not find on its own, and a closed form that divides a product of consecutive integers is proved as
//! the sum it replaces.

use std::collections::{BTreeMap, BTreeSet};

use logicaffeine_base::numeric::BigInt;
use z3::ast::{Ast, Bool, Dynamic};
use z3::DeclKind;

use crate::bytecode::value::numeral;

/// The most residue points one divisibility check evaluates.
const POINTS: u64 = 4096;

/// A polynomial: each monomial (each variable's exponent, by name) with its coefficient.
type Poly = BTreeMap<BTreeMap<String, u32>, BigInt>;

/// `c · (P div c) = P` for every division by a positive constant in `terms` that its polynomial always meets exactly.
pub fn lemmas(terms: &[Dynamic]) -> Vec<Bool> {
    let mut divisions: Vec<Dynamic> = Vec::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut work: Vec<Dynamic> = terms.to_vec();
    while let Some(t) = work.pop() {
        if !t.is_app() || !seen.insert(t.to_string()) {
            continue;
        }
        if t.decl().kind() == DeclKind::Idiv {
            divisions.push(t.clone());
        }
        work.extend(t.children());
    }
    divisions
        .into_iter()
        .filter_map(|d| {
            let [p, c] = d.children().try_into().ok()?;
            let divisor = constant(&c).filter(|c| *c > BigInt::zero())?;
            let poly = polynomial(&p)?;
            always_divides(&poly, &divisor).then(|| {
                let (d, p, c) = (d.as_int()?, p.as_int()?, c.as_int()?);
                Some((&c * &d).eq(p))
            })?
        })
        .collect()
}

/// A numeral: a term without arguments that prints as one. (Read from the text, not the declaration's kind, whose
/// numbering differs between Z3 releases for the kinds without arguments.)
fn constant(t: &Dynamic) -> Option<BigInt> {
    if t.is_app() && t.num_children() == 0 {
        BigInt::parse_decimal(&numeral(&t.to_string())?)
    } else {
        None
    }
}

/// `t` as a polynomial over its integer constants, when it is one.
fn polynomial(t: &Dynamic) -> Option<Poly> {
    if !t.is_app() {
        return None;
    }
    if let Some(n) = constant(t) {
        return Some(Poly::from([(BTreeMap::new(), n)]));
    }
    let kids = t.children();
    let polys = || kids.iter().map(polynomial).collect::<Option<Vec<Poly>>>();
    match t.decl().kind() {
        _ if kids.is_empty() && t.as_int().is_some() => {
            Some(Poly::from([(BTreeMap::from([(t.to_string(), 1)]), BigInt::from_i64(1))]))
        }
        DeclKind::Add => Some(polys()?.into_iter().fold(Poly::new(), |a, b| add(&a, &b, false))),
        DeclKind::Sub => {
            let ps = polys()?;
            let (first, rest) = ps.split_first()?;
            Some(rest.iter().fold(first.clone(), |a, b| add(&a, b, true)))
        }
        DeclKind::Uminus => Some(add(&Poly::new(), &polys()?.into_iter().next()?, true)),
        DeclKind::Mul => Some(polys()?.into_iter().fold(Poly::from([(BTreeMap::new(), BigInt::from_i64(1))]), |a, b| mul(&a, &b))),
        _ => None,
    }
}

fn add(a: &Poly, b: &Poly, negate: bool) -> Poly {
    let mut out = a.clone();
    for (m, c) in b {
        let c = if negate { c.negated() } else { c.clone() };
        let sum = out.get(m).map_or(c.clone(), |x| x.add(&c));
        if sum.is_zero() {
            out.remove(m);
        } else {
            out.insert(m.clone(), sum);
        }
    }
    out
}

fn mul(a: &Poly, b: &Poly) -> Poly {
    let mut out = Poly::new();
    for (ma, ca) in a {
        for (mb, cb) in b {
            let mut m = ma.clone();
            for (v, e) in mb {
                *m.entry(v.clone()).or_insert(0) += e;
            }
            out = add(&out, &Poly::from([(m, ca.mul(cb))]), false);
        }
    }
    out
}

/// `c` divides `p` at every integer point: at every point whose coordinates are residues mod `c`.
fn always_divides(p: &Poly, c: &BigInt) -> bool {
    let vars: Vec<&String> = p.keys().flat_map(|m| m.keys()).collect::<BTreeSet<_>>().into_iter().collect();
    let Some(base) = c.to_i64().and_then(|c| u64::try_from(c).ok()) else {
        return false;
    };
    let Some(points) = base.checked_pow(vars.len() as u32).filter(|&n| n <= POINTS) else {
        return false;
    };
    (0..points).all(|mut index| {
        let point: BTreeMap<&String, BigInt> = vars
            .iter()
            .map(|v| {
                let r = index % base;
                index /= base;
                (*v, BigInt::from_i64(r as i64))
            })
            .collect();
        let value = p.iter().fold(BigInt::zero(), |acc, (m, coef)| {
            acc.add(&m.iter().fold(coef.clone(), |t, (v, e)| (0..*e).fold(t, |t, _| t.mul(&point[v]))))
        });
        value.div_rem(c).is_some_and(|(_, r)| r.is_zero())
    })
}

#[cfg(test)]
#[path = "../../../../logicaffeine_tests/unit/logicaffeine_tv/bytecode/product/divides/tests.rs"]
mod tests;
