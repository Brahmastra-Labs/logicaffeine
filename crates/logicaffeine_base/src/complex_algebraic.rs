//! Complex algebraic numbers `re + im·i` with real algebraic parts, and every complex root of a
//! polynomial with rational coefficients.
//!
//! A non-real root `z = a + b·i` of an irreducible `g` has its conjugate as a root too, so `2a =
//! z + z̄` is a real root of `S(x) = Res_y(g(y), g(x − y))`, whose roots are the sums of two roots
//! of `g`; and `z − z̄ = 2b·i`, so `−4b²` is a negative real root of `E`, where `E(x²) =
//! Res_y(g(y), g(x + y)) / x^deg g` has the nonzero differences of roots for roots. A
//! floating-point approximation of each root orders the candidates nearest first, and a candidate
//! pair is accepted only once `g(a + b·i) = 0` is checked exactly — the approximation guides the
//! search and never decides it.

use std::fmt;

use crate::algebraic::{resultant_in_y, Algebraic};
use crate::numeric::Rational;
use crate::poly::Poly;

/// `re + im·i` with real algebraic parts.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ComplexAlgebraic {
    pub re: Algebraic,
    pub im: Algebraic,
}

impl ComplexAlgebraic {
    pub fn new(re: Algebraic, im: Algebraic) -> ComplexAlgebraic {
        ComplexAlgebraic { re, im }
    }

    pub fn real(re: Algebraic) -> ComplexAlgebraic {
        ComplexAlgebraic {
            re,
            im: Algebraic::from_i64(0),
        }
    }

    pub fn imag(im: Algebraic) -> ComplexAlgebraic {
        ComplexAlgebraic {
            re: Algebraic::from_i64(0),
            im,
        }
    }

    pub fn from_rational(r: Rational) -> ComplexAlgebraic {
        ComplexAlgebraic::real(Algebraic::from_rational(r))
    }

    pub fn from_i64(n: i64) -> ComplexAlgebraic {
        ComplexAlgebraic::real(Algebraic::from_i64(n))
    }

    pub fn zero() -> ComplexAlgebraic {
        ComplexAlgebraic::from_i64(0)
    }

    pub fn one() -> ComplexAlgebraic {
        ComplexAlgebraic::from_i64(1)
    }

    pub fn is_zero(&self) -> bool {
        self.re.is_zero() && self.im.is_zero()
    }

    pub fn is_real(&self) -> bool {
        self.im.is_zero()
    }

    pub fn add(&self, o: &ComplexAlgebraic) -> ComplexAlgebraic {
        ComplexAlgebraic {
            re: self.re.add(&o.re),
            im: self.im.add(&o.im),
        }
    }

    pub fn sub(&self, o: &ComplexAlgebraic) -> ComplexAlgebraic {
        self.add(&o.neg())
    }

    pub fn neg(&self) -> ComplexAlgebraic {
        ComplexAlgebraic {
            re: self.re.neg(),
            im: self.im.neg(),
        }
    }

    pub fn conj(&self) -> ComplexAlgebraic {
        ComplexAlgebraic {
            re: self.re.clone(),
            im: self.im.neg(),
        }
    }

    pub fn mul(&self, o: &ComplexAlgebraic) -> ComplexAlgebraic {
        if self.is_real() && o.is_real() {
            return ComplexAlgebraic::real(self.re.mul(&o.re));
        }
        ComplexAlgebraic {
            re: self.re.mul(&o.re).sub(&self.im.mul(&o.im)),
            im: self.re.mul(&o.im).add(&self.im.mul(&o.re)),
        }
    }

    /// `self·r` for a rational `r`.
    pub fn scale(&self, r: &Rational) -> ComplexAlgebraic {
        let r = Algebraic::from_rational(r.clone());
        ComplexAlgebraic {
            re: self.re.mul(&r),
            im: self.im.mul(&r),
        }
    }

    /// `None` at zero.
    pub fn recip(&self) -> Option<ComplexAlgebraic> {
        if self.is_real() {
            return Some(ComplexAlgebraic::real(self.re.recip()?));
        }
        let norm = self.re.mul(&self.re).add(&self.im.mul(&self.im));
        let inv = norm.recip()?;
        Some(ComplexAlgebraic {
            re: self.re.mul(&inv),
            im: self.im.neg().mul(&inv),
        })
    }

    pub fn div(&self, o: &ComplexAlgebraic) -> Option<ComplexAlgebraic> {
        Some(self.mul(&o.recip()?))
    }

    pub fn pow(&self, e: u32) -> ComplexAlgebraic {
        let mut acc = ComplexAlgebraic::one();
        let mut base = self.clone();
        let mut e = e;
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

    /// `p(self)` by Horner's rule.
    pub fn eval(p: &Poly, z: &ComplexAlgebraic) -> ComplexAlgebraic {
        p.coeffs()
            .iter()
            .rev()
            .fold(ComplexAlgebraic::zero(), |acc, c| {
                acc.mul(z).add(&ComplexAlgebraic::from_rational(c.clone()))
            })
    }

    /// The nearest pair of doubles.
    pub fn to_f64(&self) -> (f64, f64) {
        (self.re.to_f64(), self.im.to_f64())
    }
}

impl fmt::Display for ComplexAlgebraic {
    /// `a`, `b·i`, `a + b·i` or `a - b·i`, a unit imaginary part as a bare `i`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_real() {
            return write!(f, "{}", self.re);
        }
        let magnitude = if self.im.signum() < 0 {
            self.im.neg()
        } else {
            self.im.clone()
        };
        let unit = magnitude == Algebraic::from_i64(1);
        let text = magnitude.to_string();
        let imaginary = if unit {
            "i".to_string()
        } else if text.contains(' ') || text.contains('/') {
            format!("({text})·i")
        } else {
            format!("{text}·i")
        };
        match (self.re.is_zero(), self.im.signum() < 0) {
            (true, false) => write!(f, "{imaginary}"),
            (true, true) => write!(f, "-{imaginary}"),
            (false, false) => write!(f, "{} + {imaginary}", self.re),
            (false, true) => write!(f, "{} - {imaginary}", self.re),
        }
    }
}

/// The real roots of `p` once each, ascending.
fn distinct_real_roots(p: &Poly) -> Vec<Algebraic> {
    let mut out: Vec<Algebraic> = Vec::new();
    for (factor, _) in p.factor_over_q() {
        for k in 1..=factor.count_real_roots() {
            if let Some(r) = Algebraic::root_of(&factor, k) {
                if !out.contains(&r) {
                    out.push(r);
                }
            }
        }
    }
    out.sort();
    out
}

/// Floating-point approximations of every root of `p` (degree ≥ 1) by Durand–Kerner iteration.
fn approximate_roots(p: &Poly) -> Vec<(f64, f64)> {
    let degree = p.degree().unwrap_or(0);
    let lead = p.coeff(degree).to_f64();
    let c: Vec<f64> = (0..=degree).map(|i| p.coeff(i).to_f64() / lead).collect();
    let eval = |x: (f64, f64)| {
        let mut acc = (0.0f64, 0.0f64);
        for k in (0..=degree).rev() {
            acc = (acc.0 * x.0 - acc.1 * x.1 + c[k], acc.0 * x.1 + acc.1 * x.0);
        }
        acc
    };
    let seed = (0.4f64, 0.9f64);
    let mut z: Vec<(f64, f64)> = Vec::with_capacity(degree);
    let mut w = (1.0f64, 0.0f64);
    for _ in 0..degree {
        z.push(w);
        w = (w.0 * seed.0 - w.1 * seed.1, w.0 * seed.1 + w.1 * seed.0);
    }
    for _ in 0..2000 {
        let mut moved = 0.0f64;
        for i in 0..degree {
            let num = eval(z[i]);
            let mut den = (1.0f64, 0.0f64);
            for (j, zj) in z.iter().enumerate() {
                if j != i {
                    let d = (z[i].0 - zj.0, z[i].1 - zj.1);
                    den = (den.0 * d.0 - den.1 * d.1, den.0 * d.1 + den.1 * d.0);
                }
            }
            let norm = den.0 * den.0 + den.1 * den.1;
            if norm == 0.0 {
                continue;
            }
            let step = (
                (num.0 * den.0 + num.1 * den.1) / norm,
                (num.1 * den.0 - num.0 * den.1) / norm,
            );
            z[i] = (z[i].0 - step.0, z[i].1 - step.1);
            moved = moved.max(step.0.abs() + step.1.abs());
        }
        if moved < 1e-15 {
            break;
        }
    }
    z
}

/// The non-real roots of an irreducible `g` with positive imaginary part.
fn upper_roots(g: &Poly) -> Vec<ComplexAlgebraic> {
    let degree = g.degree().unwrap_or(0);
    let pairs = (degree - g.count_real_roots()) / 2;
    if pairs == 0 {
        return Vec::new();
    }
    let sums = resultant_in_y(
        g,
        |x0| g.compose(&Poly::from_coeffs(vec![x0.clone(), Rational::from_i64(-1)])),
        degree * degree,
    );
    let differences = resultant_in_y(
        g,
        |x0| g.compose(&Poly::from_coeffs(vec![x0.clone(), Rational::from_i64(1)])),
        degree * degree,
    );
    let half = Algebraic::from_rational(Rational::from_ratio_i64(1, 2).expect("2 ≠ 0"));
    let real_parts: Vec<Algebraic> = distinct_real_roots(&sums).iter().map(|s| s.mul(&half)).collect();
    let even: Vec<Rational> = differences.coeffs()[degree..]
        .iter()
        .step_by(2)
        .cloned()
        .collect();
    let quarter = Algebraic::from_rational(Rational::from_ratio_i64(-1, 4).expect("4 ≠ 0"));
    let imaginary_parts: Vec<Algebraic> = distinct_real_roots(&Poly::from_coeffs(even))
        .into_iter()
        .filter(|u| u.signum() < 0)
        .filter_map(|u| u.mul(&quarter).sqrt())
        .collect();
    let mut found: Vec<ComplexAlgebraic> = Vec::new();
    for (x, y) in approximate_roots(g).into_iter().filter(|(_, y)| *y > 0.0) {
        let mut candidates: Vec<(f64, &Algebraic, &Algebraic)> = Vec::new();
        for a in &real_parts {
            for b in &imaginary_parts {
                let (da, db) = (a.to_f64() - x, b.to_f64() - y);
                candidates.push((da * da + db * db, a, b));
            }
        }
        candidates.sort_by(|p, q| p.0.total_cmp(&q.0));
        for (_, a, b) in candidates {
            let z = ComplexAlgebraic::new(a.clone(), b.clone());
            if found.contains(&z) {
                continue;
            }
            if ComplexAlgebraic::eval(g, &z).is_zero() {
                found.push(z);
                break;
            }
        }
        if found.len() == pairs {
            break;
        }
    }
    assert_eq!(found.len(), pairs, "every non-real root of {g} is found and checked exactly");
    found
}

/// Every complex root of `p` (nonzero), once each: the real roots ascending, then each conjugate
/// pair by its upper root, upper before lower.
pub fn complex_roots(p: &Poly) -> Vec<ComplexAlgebraic> {
    let mut real = Vec::new();
    let mut pairs = Vec::new();
    for (g, _) in p.factor_over_q() {
        for k in 1..=g.count_real_roots() {
            real.push(Algebraic::root_of(&g, k).expect("a real root of its own polynomial"));
        }
        pairs.extend(upper_roots(&g));
    }
    real.sort();
    pairs.sort();
    let mut out: Vec<ComplexAlgebraic> = real.into_iter().map(ComplexAlgebraic::real).collect();
    for z in pairs {
        let lower = z.conj();
        out.push(z);
        out.push(lower);
    }
    out
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_base/complex_algebraic/tests.rs"]
mod tests;
