//! Real algebraic numbers: exact roots of integer polynomials, with decidable equality and order.
//!
//! An [`Algebraic`] is the `index`-th real root (counting from 1 in increasing order) of its
//! **minimal polynomial** — irreducible over ℚ, primitive over ℤ, positive leading coefficient —
//! together with an isolating interval for that root. The pair (minimal polynomial, index) is the
//! value's canonical name, so equality and hashing read only it and two routes to `√2` are the same
//! value; the interval is a refinable cache that comparison, floor and printing narrow as needed.
//! A rational is the degree-1 case, held with its exact point interval.
//!
//! Arithmetic builds a polynomial the result is known to be a root of, then cuts it down by
//! factoring and picks the one irreducible factor with exactly one root inside an interval
//! enclosure of the result, shrinking the enclosure until only one candidate root remains:
//!
//! - `α + β` is a root of `Res_y(m_α(y), m_β(x − y))`, `α·β` of `Res_y(m_α(y), y^{deg} m_β(x/y))`,
//!   and `α^e` of `Res_y(m_α(y), x − y^e)` — each resultant interpolated from univariate
//!   resultants at integer points, which is exact because the leading coefficients in `y` do not
//!   depend on `x`;
//! - a rational shift or scale, negation, and reciprocal transform the minimal polynomial
//!   directly (they keep it irreducible), and `ᵏ√α` is a root of `m_α(x^k)`.
//!
//! Every decision is exact: the order of two distinct values is found by refining both intervals
//! until they separate, which always happens because distinct numbers have a positive distance.

use crate::numeric::{rational_from_f64_exact, rational_to_f64_nearest, BigInt, Rational};
use crate::poly::{Poly, RootInterval};
use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash, Hasher};

/// A real algebraic number. See the module documentation.
#[derive(Clone, Debug)]
pub struct Algebraic {
    /// Irreducible over ℚ, primitive over ℤ, positive leading coefficient.
    min: Poly,
    /// Which real root of `min`, counting from 1 in increasing order.
    index: usize,
    /// `lo == hi` for a rational; otherwise an open interval holding this root of `min` and no
    /// other, with neither end a root.
    iv: RootInterval,
}

/// `⌊√n⌋` for `n ≥ 0`, by Newton's method on integers.
fn isqrt(n: &BigInt) -> BigInt {
    if n.is_zero() {
        return BigInt::zero();
    }
    let mut x = BigInt::from_i64(1).shl_bits(n.bit_len() / 2 + 1);
    loop {
        let y = x.add(&n.div_rem(&x).expect("x > 0").0).shr_bits(1).expect("nonnegative");
        if y >= x {
            return x;
        }
        x = y;
    }
}

/// A quadratic irrational `a + b·√d`, `d` a positive integer that is no square.
struct Quadratic {
    a: Rational,
    b: Rational,
    d: BigInt,
}

fn half() -> Rational {
    Rational::from_ratio_i64(1, 2).expect("2 is nonzero")
}

fn pow2_recip(bits: u32) -> Rational {
    Rational::new(BigInt::from_i64(1), BigInt::from_i64(1).shl_bits(bits as u64))
        .expect("a power of two is nonzero")
}

/// The largest integer `s ≥ 0` with `sᵏ ≤ n`, for `n ≥ 0`.
fn integer_root_floor(n: &BigInt, k: u32) -> BigInt {
    if n.is_zero() {
        return BigInt::zero();
    }
    let top = n.bit_len() / k as u64 + 1;
    let mut s = BigInt::zero();
    for bit in (0..=top).rev() {
        let t = s.add(&BigInt::from_i64(1).shl_bits(bit));
        if t.pow(k) <= *n {
            s = t;
        }
    }
    s
}

/// Rational bounds `(l, u)` with `lᵏ ≤ x ≤ uᵏ` and `u − l ≤ 2^-bits`, for `x ≥ 0`.
fn rational_root_bounds(x: &Rational, k: u32, bits: u32) -> (Rational, Rational) {
    let scale = BigInt::from_i64(1).shl_bits(k as u64 * bits as u64);
    let scaled = x.mul(&Rational::from_bigint(scale));
    let lo_int = integer_root_floor(&scaled.floor(), k);
    let ceil = scaled.ceil();
    let mut hi_int = integer_root_floor(&ceil, k);
    if hi_int.pow(k) < ceil {
        hi_int = hi_int.add(&BigInt::from_i64(1));
    }
    let den = BigInt::from_i64(1).shl_bits(bits as u64);
    (
        Rational::new(lo_int, den.clone()).expect("nonzero"),
        Rational::new(hi_int, den).expect("nonzero"),
    )
}

/// `x^e` over a closed interval.
fn interval_pow(lo: &Rational, hi: &Rational, e: u32) -> (Rational, Rational) {
    let pl = lo.pow(e as i32).expect("nonnegative powers exist");
    let ph = hi.pow(e as i32).expect("nonnegative powers exist");
    if e % 2 == 1 || !lo.is_negative() {
        (pl, ph)
    } else if !hi.is_positive() {
        (ph, pl)
    } else {
        (Rational::zero(), pl.max(ph))
    }
}

fn interval_mul(a: &(Rational, Rational), b: &(Rational, Rational)) -> (Rational, Rational) {
    let products = [a.0.mul(&b.0), a.0.mul(&b.1), a.1.mul(&b.0), a.1.mul(&b.1)];
    let lo = products.iter().min().expect("four products").clone();
    let hi = products.iter().max().expect("four products").clone();
    (lo, hi)
}

/// `Res_y(p(y), q_at(x₀)(y))` as a polynomial in `x`, interpolated through `degree + 1` integer
/// points by Newton's divided differences.
pub(crate) fn resultant_in_y(p: &Poly, q_at: impl Fn(&Rational) -> Poly, degree: usize) -> Poly {
    let xs: Vec<Rational> = (0..=degree as i64).map(Rational::from_i64).collect();
    let mut d: Vec<Rational> = xs.iter().map(|x| p.resultant(&q_at(x))).collect();
    for j in 1..=degree {
        for i in (j..=degree).rev() {
            let num = d[i].sub(&d[i - 1]);
            let den = xs[i].sub(&xs[i - j]);
            d[i] = num.div(&den).expect("interpolation points are distinct");
        }
    }
    let mut poly = Poly::constant(d[degree].clone());
    for j in (0..degree).rev() {
        let factor = Poly::from_coeffs(vec![xs[j].negated(), Rational::one()]);
        poly = poly.mul(&factor).add(&Poly::constant(d[j].clone()));
    }
    poly
}

/// The square-free decomposition `n = k²·d` (`n > 0`) by trial division with the primes below
/// 10 000, then a perfect-square test of whatever cofactor remains.
fn split_square(n: &BigInt) -> (BigInt, BigInt) {
    let mut rest = n.clone();
    let mut k = BigInt::from_i64(1);
    let mut d = BigInt::from_i64(1);
    let mut p = 2i64;
    while p < 10_000 {
        let pb = BigInt::from_i64(p);
        if pb.mul(&pb) > rest {
            break;
        }
        let mut e = 0u32;
        loop {
            let (q, r) = rest.div_rem(&pb).expect("p is nonzero");
            if !r.is_zero() {
                break;
            }
            rest = q;
            e += 1;
        }
        k = k.mul(&pb.pow(e / 2));
        if e % 2 == 1 {
            d = d.mul(&pb);
        }
        p += if p == 2 { 1 } else { 2 };
    }
    let s = integer_root_floor(&rest, 2);
    if s.mul(&s) == rest {
        k = k.mul(&s);
    } else {
        d = d.mul(&rest);
    }
    (k, d)
}

impl Algebraic {
    pub fn from_i64(n: i64) -> Algebraic {
        Self::from_rational(Rational::from_i64(n))
    }

    pub fn from_rational(r: Rational) -> Algebraic {
        let min = Poly::from_bigints(&[r.numerator().negated(), r.denominator().clone()]);
        Algebraic {
            min,
            index: 1,
            iv: RootInterval {
                lo: r.clone(),
                hi: r,
            },
        }
    }

    /// The exact value of a finite float. `None` for NaN and the infinities.
    pub fn from_f64(f: f64) -> Option<Algebraic> {
        rational_from_f64_exact(f).map(Self::from_rational)
    }

    /// The `k`-th real root of `p` (counting from 1 in increasing order, each distinct root once).
    pub fn root_of(p: &Poly, k: usize) -> Option<Algebraic> {
        if k == 0 || p.degree().unwrap_or(0) == 0 {
            return None;
        }
        let roots = p.isolate_real_roots();
        let iv = roots.get(k - 1)?;
        if iv.lo == iv.hi {
            return Some(Self::from_rational(iv.lo.clone()));
        }
        let factors: Vec<Poly> = p.factor_over_q().into_iter().map(|(g, _)| g).collect();
        let g = factors
            .into_iter()
            .find(|g| g.count_real_roots_in(&iv.lo, &iv.hi) == 1)
            .expect("the root lies on exactly one irreducible factor");
        Some(Self::from_irreducible(g, iv.lo.clone(), iv.hi.clone()))
    }

    /// The root of the irreducible primitive `g` in the open interval `(lo, hi)`, which holds that
    /// root and no other root of `g`, with neither end a root.
    fn from_irreducible(g: Poly, lo: Rational, hi: Rational) -> Algebraic {
        if g.degree() == Some(1) {
            let r = g.coeff(0).negated().div(&g.coeff(1)).expect("degree 1");
            return Self::from_rational(r);
        }
        let bound = g.root_bound();
        let index = g.count_real_roots_in(&bound.negated(), &lo) + 1;
        Algebraic {
            min: g,
            index,
            iv: RootInterval { lo, hi },
        }
    }

    /// The one root, among the roots of `candidate`, that `enclose` encloses: the irreducible
    /// factors of `candidate` are tested against enclosures of increasing precision until exactly
    /// one factor has exactly one root in the closed enclosure.
    fn select(candidate: &Poly, enclose: impl Fn(u32) -> (Rational, Rational)) -> Algebraic {
        let factors: Vec<Poly> = candidate.factor_over_q().into_iter().map(|(g, _)| g).collect();
        let mut bits = 4u32;
        loop {
            let (lo, hi) = enclose(bits);
            if lo == hi {
                return Self::from_rational(lo);
            }
            let mut hits = factors.iter().filter_map(|g| {
                let n = g.count_real_roots_in(&lo, &hi) + usize::from(g.eval(&lo).is_zero());
                (n > 0).then_some((g, n))
            });
            if let (Some((g, 1)), None) = (hits.next(), hits.next()) {
                if g.degree() == Some(1) {
                    let r = g.coeff(0).negated().div(&g.coeff(1)).expect("degree 1");
                    return Self::from_rational(r);
                }
                return Self::from_irreducible(g.clone(), lo, hi);
            }
            bits = bits.saturating_mul(2);
        }
    }

    pub fn degree(&self) -> usize {
        self.min.degree().expect("a minimal polynomial is nonzero")
    }

    pub fn minimal_polynomial(&self) -> &Poly {
        &self.min
    }

    /// Which real root of the minimal polynomial this is, counting from 1 in increasing order.
    pub fn root_index(&self) -> usize {
        self.index
    }

    pub fn to_rational(&self) -> Option<Rational> {
        (self.degree() == 1).then(|| self.iv.lo.clone())
    }

    pub fn is_zero(&self) -> bool {
        self.degree() == 1 && self.iv.lo.is_zero()
    }

    pub fn signum(&self) -> i32 {
        if let Some(r) = self.to_rational() {
            return if r.is_zero() {
                0
            } else if r.is_negative() {
                -1
            } else {
                1
            };
        }
        let mut iv = self.iv.clone();
        loop {
            if !iv.lo.is_negative() {
                return 1;
            }
            if !iv.hi.is_positive() {
                return -1;
            }
            iv = self.bisect(&iv);
        }
    }

    /// Halve this value's isolating interval.
    fn bisect(&self, iv: &RootInterval) -> RootInterval {
        if iv.lo == iv.hi {
            return iv.clone();
        }
        let mid = iv.lo.add(&iv.hi).mul(&half());
        let at_mid = self.min.eval(&mid);
        if at_mid.is_zero() {
            return RootInterval {
                lo: mid.clone(),
                hi: mid,
            };
        }
        if at_mid.is_negative() == self.min.eval(&iv.lo).is_negative() {
            RootInterval {
                lo: mid,
                hi: iv.hi.clone(),
            }
        } else {
            RootInterval {
                lo: iv.lo.clone(),
                hi: mid,
            }
        }
    }

    /// A closed interval `[lo, hi]` holding the value, at most `2^-bits` wide; a rational encloses
    /// itself exactly.
    pub fn enclosure(&self, bits: u32) -> (Rational, Rational) {
        let eps = pow2_recip(bits);
        let mut iv = self.iv.clone();
        while iv.hi.sub(&iv.lo) > eps {
            iv = self.bisect(&iv);
        }
        (iv.lo, iv.hi)
    }

    pub fn neg(&self) -> Algebraic {
        if let Some(r) = self.to_rational() {
            return Self::from_rational(r.negated());
        }
        let min = Poly::from_bigints(&self.min.neg_var().primitive_int());
        let real_roots = min.count_real_roots();
        Algebraic {
            index: real_roots + 1 - self.index,
            iv: RootInterval {
                lo: self.iv.hi.negated(),
                hi: self.iv.lo.negated(),
            },
            min,
        }
    }

    fn add_rational(&self, r: &Rational) -> Algebraic {
        if let Some(s) = self.to_rational() {
            return Self::from_rational(s.add(r));
        }
        let shifted = self
            .min
            .compose(&Poly::from_coeffs(vec![r.negated(), Rational::one()]));
        Algebraic {
            min: Poly::from_bigints(&shifted.primitive_int()),
            index: self.index,
            iv: RootInterval {
                lo: self.iv.lo.add(r),
                hi: self.iv.hi.add(r),
            },
        }
    }

    fn mul_rational(&self, r: &Rational) -> Algebraic {
        if r.is_zero() {
            return Self::from_i64(0);
        }
        if let Some(s) = self.to_rational() {
            return Self::from_rational(s.mul(r));
        }
        let scaled = self.min.compose(&Poly::from_coeffs(vec![
            Rational::zero(),
            r.recip().expect("r is nonzero"),
        ]));
        let min = Poly::from_bigints(&scaled.primitive_int());
        let (lo, hi) = (self.iv.lo.mul(r), self.iv.hi.mul(r));
        if r.is_positive() {
            Algebraic {
                min,
                index: self.index,
                iv: RootInterval { lo, hi },
            }
        } else {
            let real_roots = min.count_real_roots();
            Algebraic {
                min,
                index: real_roots + 1 - self.index,
                iv: RootInterval { lo: hi, hi: lo },
            }
        }
    }

    pub fn add(&self, o: &Algebraic) -> Algebraic {
        if let Some(r) = o.to_rational() {
            return self.add_rational(&r);
        }
        if let Some(r) = self.to_rational() {
            return o.add_rational(&r);
        }
        if let Some((x, y)) = self.one_quadratic_field(o) {
            return Self::from_quadratic(x.a.add(&y.a), x.b.add(&y.b), &x.d);
        }
        let mb = o.min.clone();
        let candidate = resultant_in_y(
            &self.min,
            |x0| mb.compose(&Poly::from_coeffs(vec![x0.clone(), Rational::from_i64(-1)])),
            self.degree() * o.degree(),
        );
        Self::select(&candidate, |bits| {
            let (alo, ahi) = self.enclosure(bits + 1);
            let (blo, bhi) = o.enclosure(bits + 1);
            (alo.add(&blo), ahi.add(&bhi))
        })
    }

    pub fn sub(&self, o: &Algebraic) -> Algebraic {
        self.add(&o.neg())
    }

    pub fn mul(&self, o: &Algebraic) -> Algebraic {
        if let Some(r) = o.to_rational() {
            return self.mul_rational(&r);
        }
        if let Some(r) = self.to_rational() {
            return o.mul_rational(&r);
        }
        if let Some((x, y)) = self.one_quadratic_field(o) {
            let d = Rational::from_bigint(x.d.clone());
            let a = x.a.mul(&y.a).add(&x.b.mul(&y.b).mul(&d));
            let b = x.a.mul(&y.b).add(&y.a.mul(&x.b));
            return Self::from_quadratic(a, b, &x.d);
        }
        let db = o.degree();
        let mb = o.min.clone();
        let candidate = resultant_in_y(
            &self.min,
            |x0| {
                let mut c = vec![Rational::zero(); db + 1];
                let mut xp = Rational::one();
                for j in 0..=db {
                    c[db - j] = mb.coeff(j).mul(&xp);
                    xp = xp.mul(x0);
                }
                Poly::from_coeffs(c)
            },
            self.degree() * db,
        );
        Self::select(&candidate, |bits| {
            let a = self.enclosure(bits + 4);
            let b = o.enclosure(bits + 4);
            interval_mul(&a, &b)
        })
    }

    pub fn recip(&self) -> Option<Algebraic> {
        if self.is_zero() {
            return None;
        }
        if let Some(r) = self.to_rational() {
            return Some(Self::from_rational(r.recip().expect("nonzero")));
        }
        let candidate = Poly::from_bigints(&self.min.reverse().primitive_int());
        Some(Self::select(&candidate, |bits| {
            let mut b = bits;
            loop {
                let (lo, hi) = self.enclosure(b);
                if lo.is_positive() || hi.is_negative() {
                    return (
                        hi.recip().expect("0 is outside the enclosure"),
                        lo.recip().expect("0 is outside the enclosure"),
                    );
                }
                b += 4;
            }
        }))
    }

    pub fn div(&self, o: &Algebraic) -> Option<Algebraic> {
        Some(self.mul(&o.recip()?))
    }

    /// `self^e`; `None` only for zero to a negative power.
    pub fn pow(&self, e: i32) -> Option<Algebraic> {
        if e < 0 {
            return self.recip()?.pow(-e);
        }
        if e == 0 {
            return Some(Self::from_i64(1));
        }
        if let Some(r) = self.to_rational() {
            return Some(Self::from_rational(r.pow(e).expect("nonnegative power")));
        }
        let e = e as u32;
        let candidate = resultant_in_y(
            &self.min,
            |x0| {
                let mut c = vec![Rational::zero(); e as usize + 1];
                c[0] = x0.clone();
                c[e as usize] = Rational::from_i64(-1);
                Poly::from_coeffs(c)
            },
            self.degree(),
        );
        Some(Self::select(&candidate, |bits| {
            let (lo, hi) = self.enclosure(bits + 2 * e + 8);
            interval_pow(&lo, &hi, e)
        }))
    }

    /// The nonnegative square root; `None` for a negative number.
    pub fn sqrt(&self) -> Option<Algebraic> {
        self.nth_root(2)
    }

    /// The real `k`-th root: the nonnegative one for even `k` (`None` for a negative number), the
    /// unique one for odd `k`. `None` for `k = 0`.
    pub fn nth_root(&self, k: u32) -> Option<Algebraic> {
        if k == 0 {
            return None;
        }
        if k == 1 || self.is_zero() {
            return Some(self.clone());
        }
        let negative = self.signum() < 0;
        if negative {
            if k.is_multiple_of(2) {
                return None;
            }
            return Some(self.neg().nth_root(k)?.neg());
        }
        let mut c = vec![Rational::zero(); self.degree() * k as usize + 1];
        for (i, a) in self.min.coeffs().iter().enumerate() {
            c[i * k as usize] = a.clone();
        }
        let candidate = Poly::from_coeffs(c);
        Some(Self::select(&candidate, |bits| {
            let (lo, hi) = self.enclosure(bits * k + 4);
            let lo = if lo.is_negative() { Rational::zero() } else { lo };
            let (l, _) = rational_root_bounds(&lo, k, bits + 2);
            let (_, u) = rational_root_bounds(&hi, k, bits + 2);
            (l, u)
        }))
    }

    /// The distinct real roots of `Σ coeffs[i]·x^i`, a polynomial with real algebraic coefficients,
    /// in increasing order, each with its multiplicity; none for a constant.
    ///
    /// Every root is a root of the polynomial's norm over the rationals — the product of its
    /// conjugates under every embedding of its coefficients, a tower of resultants against their
    /// minimal polynomials ([`crate::elimination::Polynomial::resultant`]) — so the roots are found among the
    /// real roots of the norm's irreducible factors, each kept when the polynomial vanishes there
    /// exactly.
    pub fn real_roots(coeffs: &[Algebraic]) -> Vec<(Algebraic, usize)> {
        let mut c = coeffs.to_vec();
        while c.last().is_some_and(Algebraic::is_zero) {
            c.pop();
        }
        if c.len() <= 1 {
            return Vec::new();
        }
        let rational: Option<Vec<Rational>> = c.iter().map(Algebraic::to_rational).collect();
        let norm = match &rational {
            Some(r) => Poly::from_coeffs(r.clone()),
            None => norm_over_q(&c),
        };
        let mut out = Vec::new();
        for (g, _) in norm.factor_over_q() {
            for k in 1..=g.count_real_roots() {
                let r = Self::root_of(&g, k).expect("an index within the real roots");
                if rational.is_none() && !eval_at(&c, &r).is_zero() {
                    continue;
                }
                let mut multiplicity = 1;
                let mut d = derivative(&c);
                while !d.is_empty() && eval_at(&d, &r).is_zero() {
                    multiplicity += 1;
                    d = derivative(&d);
                }
                out.push((r, multiplicity));
            }
        }
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    }

    pub fn floor(&self) -> BigInt {
        if let Some(r) = self.to_rational() {
            return r.floor();
        }
        let mut iv = self.iv.clone();
        loop {
            let (fl, fh) = (iv.lo.floor(), iv.hi.floor());
            if fl == fh {
                return fl;
            }
            iv = self.bisect(&iv);
        }
    }

    pub fn ceil(&self) -> BigInt {
        if let Some(r) = self.to_rational() {
            return r.ceil();
        }
        let mut iv = self.iv.clone();
        loop {
            let (cl, ch) = (iv.lo.ceil(), iv.hi.ceil());
            if cl == ch {
                return cl;
            }
            iv = self.bisect(&iv);
        }
    }

    /// The nearest f64 (ties to even): the interval is refined until both ends round alike.
    pub fn to_f64(&self) -> f64 {
        if let Some(r) = self.to_rational() {
            return rational_to_f64_nearest(&r);
        }
        let mut iv = self.iv.clone();
        loop {
            let (fl, fh) = (rational_to_f64_nearest(&iv.lo), rational_to_f64_nearest(&iv.hi));
            if fl.to_bits() == fh.to_bits() {
                return fl;
            }
            iv = self.bisect(&iv);
        }
    }

    /// The radical form of a quadratic irrational: `(r ± s√d)/t`.
    /// A quadratic irrational as `a + b·√d`, `d` the discriminant of its minimal polynomial
    /// `c₂x² + c₁x + c₀` (`c₂ > 0`): of the two roots `(−c₁ ± √d)/(2c₂)`, the larger has `+`. `None` for
    /// any other degree.
    fn quadratic(&self) -> Option<Quadratic> {
        if self.degree() != 2 {
            return None;
        }
        let c = self.min.primitive_int();
        let (c0, c1, c2) = (&c[0], &c[1], &c[2]);
        let d = c1.mul(c1).sub(&BigInt::from_i64(4).mul(c2).mul(c0));
        let two_c2 = Rational::from_bigint(c2.add(c2));
        let a = Rational::from_bigint(c1.negated()).div(&two_c2).expect("c₂ is nonzero");
        let b = Rational::one().div(&two_c2).expect("c₂ is nonzero");
        let b = if self.index == 2 { b } else { b.negated() };
        Some(Quadratic { a, b, d })
    }

    /// Both values as `a + b·√d` over one `d`, in either order, when both are quadratic irrationals of
    /// one field: their discriminants' product is a square `r²`, so `√d₂ = (r/d₁)·√d₁`, and their sum
    /// and product are that field's own, with no resultant. The smaller discriminant is the one kept.
    fn one_quadratic_field(&self, o: &Algebraic) -> Option<(Quadratic, Quadratic)> {
        let (x, y) = (self.quadratic()?, o.quadratic()?);
        if x.d == y.d {
            return Some((x, y));
        }
        let product = x.d.mul(&y.d);
        let r = isqrt(&product);
        if r.mul(&r) != product {
            return None;
        }
        let (keep, other) = if x.d <= y.d { (x, y) } else { (y, x) };
        let scale = Rational::from_bigint(r).div(&Rational::from_bigint(keep.d.clone())).expect("d is positive");
        let moved = Quadratic {
            a: other.a,
            b: other.b.mul(&scale),
            d: keep.d.clone(),
        };
        Some((keep, moved))
    }

    /// `a + b·√d` in its one form, `d` a positive integer that is no square: a rational when `b` is
    /// zero, otherwise the root `a ± |b|·√d` of the irreducible `(x − a)² − b²·d`, the larger when `b`
    /// is positive. With `s < √d < s + 1`, `a ± |b|·(s, s + 1)` holds that root and not the other,
    /// which lies `2|b|·√d > |b|` away, and neither end is a root, both being rational.
    fn from_quadratic(a: Rational, b: Rational, d: &BigInt) -> Algebraic {
        if b.is_zero() {
            return Self::from_rational(a);
        }
        let c0 = a.mul(&a).sub(&b.mul(&b).mul(&Rational::from_bigint(d.clone())));
        let c1 = a.add(&a).negated();
        let p = Poly::from_coeffs(vec![c0, c1, Rational::one()]);
        let min = Poly::from_bigints(&p.primitive_int());
        let larger = b.is_positive();
        let m = if larger { b } else { b.negated() };
        let s = Rational::from_bigint(isqrt(d));
        let (near, far) = (m.mul(&s), m.mul(&s.add(&Rational::one())));
        let (lo, hi) = if larger { (a.add(&near), a.add(&far)) } else { (a.sub(&far), a.sub(&near)) };
        Algebraic {
            min,
            index: if larger { 2 } else { 1 },
            iv: RootInterval { lo, hi },
        }
    }

    fn fmt_quadratic(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let c = self.min.primitive_int();
        let (cc, b, a) = (&c[0], &c[1], &c[2]);
        let four = BigInt::from_i64(4);
        let disc = b.mul(b).sub(&four.mul(a).mul(cc));
        let (k, d) = split_square(&disc);
        let two_a = a.add(a);
        let g = b.abs().gcd(&k).gcd(&two_a);
        let r = b.negated().div_rem(&g).expect("g divides b").0;
        let s = k.div_rem(&g).expect("g divides k").0;
        let t = two_a.div_rem(&g).expect("g divides 2a").0;
        let plus = self.index == 2;
        let one = BigInt::from_i64(1);
        let radical = if s == one {
            format!("√{d}")
        } else {
            format!("{s}√{d}")
        };
        if r.is_zero() {
            let sign = if plus { "" } else { "-" };
            if t == one {
                write!(f, "{sign}{radical}")
            } else {
                write!(f, "{sign}{radical}/{t}")
            }
        } else {
            let op = if plus { "+" } else { "-" };
            if t == one {
                write!(f, "{r} {op} {radical}")
            } else {
                write!(f, "({r} {op} {radical})/{t}")
            }
        }
    }
}

/// `Σ c[i]·x^i` at `x`, by Horner's rule.
fn eval_at(c: &[Algebraic], x: &Algebraic) -> Algebraic {
    c.iter().rev().fold(Algebraic::from_i64(0), |acc, a| acc.mul(x).add(a))
}

fn derivative(c: &[Algebraic]) -> Vec<Algebraic> {
    c.iter()
        .enumerate()
        .skip(1)
        .map(|(i, a)| a.mul_rational(&Rational::from_i64(i as i64)))
        .collect()
}

/// The norm over ℚ of `Σ c[i]·x^i`: each distinct irrational coefficient is a variable `z_j`, and
/// each `z_j` is eliminated against its minimal polynomial.
fn norm_over_q(c: &[Algebraic]) -> Poly {
    use crate::elimination::Polynomial;
    let mut symbols: Vec<&Algebraic> = Vec::new();
    for a in c {
        if a.to_rational().is_none() && !symbols.contains(&a) {
            symbols.push(a);
        }
    }
    let mut f = Polynomial::zero();
    for (i, a) in c.iter().enumerate() {
        let coefficient = match a.to_rational() {
            Some(r) => Polynomial::constant(r),
            None => Polynomial::var(1 + symbols.iter().position(|s| *s == a).expect("collected")),
        };
        f = f.add(&coefficient.mul(&Polynomial::var(0).pow(i as u32)));
    }
    for (j, a) in symbols.iter().enumerate() {
        let minimal = Polynomial::from_univariate(a.minimal_polynomial(), j + 1);
        f = minimal.resultant(&f, j + 1);
    }
    f.to_univariate(0).expect("every coefficient eliminated")
}

impl PartialEq for Algebraic {
    fn eq(&self, o: &Self) -> bool {
        self.index == o.index && self.min == o.min
    }
}

impl Eq for Algebraic {}

impl Hash for Algebraic {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.min.hash(state);
        self.index.hash(state);
    }
}

impl Ord for Algebraic {
    fn cmp(&self, o: &Self) -> Ordering {
        if self == o {
            return Ordering::Equal;
        }
        if let (Some(a), Some(b)) = (self.to_rational(), o.to_rational()) {
            return a.cmp(&b);
        }
        let (mut a, mut b) = (self.iv.clone(), o.iv.clone());
        loop {
            if a.hi <= b.lo {
                return Ordering::Less;
            }
            if b.hi <= a.lo {
                return Ordering::Greater;
            }
            if a.hi.sub(&a.lo) >= b.hi.sub(&b.lo) {
                a = self.bisect(&a);
            } else {
                b = o.bisect(&b);
            }
        }
    }
}

impl PartialOrd for Algebraic {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}

impl fmt::Display for Algebraic {
    /// A rational as a rational; a quadratic irrational in simplified radical form
    /// (`(1 + √5)/2`, `3√2/4`); anything of higher degree as `root k of <minimal polynomial>`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.degree() {
            1 => write!(f, "{}", self.iv.lo),
            2 => self.fmt_quadratic(f),
            _ => write!(f, "root {} of {}", self.index, self.min),
        }
    }
}

impl crate::exact_codec::ExactCodec for Algebraic {
    /// The minimal polynomial, then which of its real roots, counting from 1.
    fn write(&self, out: &mut Vec<u8>) {
        self.min.write(out);
        (self.index as u64).write(out);
    }

    /// Refused unless the polynomial is primitive with a positive lead, of degree at most
    /// [`crate::exact_codec::MAX_WIRE_DEGREE`], irreducible, and has that many real roots.
    fn read(buf: &[u8], pos: &mut usize) -> Option<Self> {
        let min = Poly::read(buf, pos)?;
        let index = usize::try_from(u64::read(buf, pos)?).ok()?;
        let degree = min.degree()?;
        if degree == 0 || degree > crate::exact_codec::MAX_WIRE_DEGREE || index == 0 {
            return None;
        }
        if Poly::from_bigints(&min.primitive_int()) != min || !min.is_irreducible() {
            return None;
        }
        let value = Algebraic::root_of(&min, index)?;
        (value.min == min && value.index == index).then_some(value)
    }
}

/// Why the even `n`-th root of a negative number is refused, in the words every exact type uses.
pub(crate) fn negative_root(n: u32) -> String {
    if n == 2 {
        "square root of a negative number".to_string()
    } else {
        format!("an even root ({n}th) of a negative number")
    }
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_base/algebraic/tests.rs"]
mod tests;
