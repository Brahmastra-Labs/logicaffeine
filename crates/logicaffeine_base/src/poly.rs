//! Exact univariate polynomials over ℚ — the algebra under the real algebraic numbers.
//!
//! A [`Poly`] is a little-endian vector of exact [`Rational`] coefficients with no trailing zeros
//! (the zero polynomial is empty), so equal polynomials are structurally equal and `Eq`/`Hash` are
//! derived. Beside the ring operations it carries what exact real-root arithmetic needs:
//!
//! - **Sturm chains** on the squarefree part count the distinct real roots in any half-open
//!   interval `(lo, hi]`, and bisection on those counts isolates every real root in a
//!   [`RootInterval`] holding exactly one;
//! - **resultants** by the Euclidean identity `res(f, g) = (−1)^{mn}·lc(g)^{m − deg r}·res(g, r)`,
//!   `r = f mod g`, which is how the polynomial of a sum or product of two roots is built;
//! - **factorization over ℤ** (Zassenhaus): Yun's squarefree decomposition, Berlekamp's algorithm
//!   modulo a small prime, linear Hensel lifting past the Mignotte bound `2·|lc|·2ⁿ·‖f‖₁`, and
//!   recombination of lifted factors by trial division — so a root's defining polynomial can always
//!   be cut down to its minimal polynomial.
//!
//! Everything is exact and deterministic: no floating point, and the prime and every split are
//! chosen by a fixed order, so every run (and every tier mirroring this file) factors identically.

use crate::numeric::{BigInt, Rational};
use std::cmp::Ordering;
use std::fmt;

/// A polynomial over ℚ: `c[i]` is the coefficient of `xⁱ`. INVARIANT: no trailing zero.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct Poly {
    c: Vec<Rational>,
}

/// One isolated real root. `lo == hi` is the root itself (a rational root the bisection landed
/// on); otherwise the root is the only root in the open interval `(lo, hi)` and neither end is a
/// root, so the squarefree part changes sign across it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RootInterval {
    pub lo: Rational,
    pub hi: Rational,
}

fn trimmed(mut c: Vec<Rational>) -> Vec<Rational> {
    while c.last().is_some_and(Rational::is_zero) {
        c.pop();
    }
    c
}

fn sign(r: &Rational) -> i32 {
    if r.is_zero() {
        0
    } else if r.is_negative() {
        -1
    } else {
        1
    }
}

fn half_of_sum(a: &Rational, b: &Rational) -> Rational {
    a.add(b).mul(&Rational::from_ratio_i64(1, 2).expect("2 is nonzero"))
}

impl Poly {
    pub fn zero() -> Poly {
        Poly { c: Vec::new() }
    }

    pub fn one() -> Poly {
        Poly {
            c: vec![Rational::one()],
        }
    }

    pub fn x() -> Poly {
        Poly {
            c: vec![Rational::zero(), Rational::one()],
        }
    }

    pub fn constant(c: Rational) -> Poly {
        Poly { c: trimmed(vec![c]) }
    }

    /// From little-endian coefficients (`c[i]` multiplies `xⁱ`); trailing zeros are dropped.
    pub fn from_coeffs(c: Vec<Rational>) -> Poly {
        Poly { c: trimmed(c) }
    }

    pub fn from_i64s(c: &[i64]) -> Poly {
        Self::from_coeffs(c.iter().map(|&x| Rational::from_i64(x)).collect())
    }

    pub fn from_bigints(c: &[BigInt]) -> Poly {
        Self::from_coeffs(c.iter().cloned().map(Rational::from_bigint).collect())
    }

    pub fn coeffs(&self) -> &[Rational] {
        &self.c
    }

    /// The coefficient of `xⁱ` (zero past the degree).
    pub fn coeff(&self, i: usize) -> Rational {
        self.c.get(i).cloned().unwrap_or_else(Rational::zero)
    }

    /// `None` for the zero polynomial.
    pub fn degree(&self) -> Option<usize> {
        self.c.len().checked_sub(1)
    }

    pub fn is_zero(&self) -> bool {
        self.c.is_empty()
    }

    pub fn leading(&self) -> Option<&Rational> {
        self.c.last()
    }

    pub fn add(&self, o: &Poly) -> Poly {
        let n = self.c.len().max(o.c.len());
        Self::from_coeffs((0..n).map(|i| self.coeff(i).add(&o.coeff(i))).collect())
    }

    pub fn sub(&self, o: &Poly) -> Poly {
        let n = self.c.len().max(o.c.len());
        Self::from_coeffs((0..n).map(|i| self.coeff(i).sub(&o.coeff(i))).collect())
    }

    pub fn neg(&self) -> Poly {
        Poly {
            c: self.c.iter().map(Rational::negated).collect(),
        }
    }

    pub fn mul(&self, o: &Poly) -> Poly {
        if self.is_zero() || o.is_zero() {
            return Poly::zero();
        }
        let mut out = vec![Rational::zero(); self.c.len() + o.c.len() - 1];
        for (i, a) in self.c.iter().enumerate() {
            if a.is_zero() {
                continue;
            }
            for (j, b) in o.c.iter().enumerate() {
                if b.is_zero() {
                    continue;
                }
                out[i + j] = out[i + j].add(&a.mul(b));
            }
        }
        Self::from_coeffs(out)
    }

    pub fn scale(&self, k: &Rational) -> Poly {
        Self::from_coeffs(self.c.iter().map(|a| a.mul(k)).collect())
    }

    pub fn pow(&self, mut e: u32) -> Poly {
        let mut base = self.clone();
        let mut acc = Poly::one();
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

    /// Euclidean division over ℚ: `self = q·d + r` with `deg r < deg d`. `None` when `d` is zero.
    pub fn div_rem(&self, d: &Poly) -> Option<(Poly, Poly)> {
        let dd = d.degree()?;
        let lc_inv = d.c[dd].recip()?;
        if self.c.len() <= dd {
            return Some((Poly::zero(), self.clone()));
        }
        let mut r = self.c.clone();
        let mut q = vec![Rational::zero(); r.len() - dd];
        for k in (0..q.len()).rev() {
            let coef = r[k + dd].mul(&lc_inv);
            if !coef.is_zero() {
                for (j, dj) in d.c.iter().enumerate() {
                    r[k + j] = r[k + j].sub(&coef.mul(dj));
                }
            }
            q[k] = coef;
        }
        r.truncate(dd);
        Some((Self::from_coeffs(q), Self::from_coeffs(r)))
    }

    /// The monic greatest common divisor; `gcd(0, 0) = 0`.
    pub fn gcd(&self, o: &Poly) -> Poly {
        let (mut a, mut b) = (self.monic(), o.monic());
        while !b.is_zero() {
            let r = a.div_rem(&b).expect("b is nonzero").1;
            a = b;
            b = r.monic();
        }
        a
    }

    /// `self` divided by its leading coefficient; zero stays zero.
    pub fn monic(&self) -> Poly {
        match self.leading() {
            None => Poly::zero(),
            Some(lc) => self.scale(&lc.recip().expect("the leading coefficient is nonzero")),
        }
    }

    pub fn derivative(&self) -> Poly {
        Self::from_coeffs(
            self.c
                .iter()
                .enumerate()
                .skip(1)
                .map(|(i, a)| a.mul(&Rational::from_i64(i as i64)))
                .collect(),
        )
    }

    /// Horner evaluation at an exact rational point.
    pub fn eval(&self, x: &Rational) -> Rational {
        self.c
            .iter()
            .rev()
            .fold(Rational::zero(), |acc, a| acc.mul(x).add(a))
    }

    /// `self(inner(x))`.
    pub fn compose(&self, inner: &Poly) -> Poly {
        self.c.iter().rev().fold(Poly::zero(), |acc, a| {
            acc.mul(inner).add(&Poly::constant(a.clone()))
        })
    }

    /// `x^deg · self(1/x)`: the polynomial whose roots are the reciprocals of the nonzero roots.
    pub fn reverse(&self) -> Poly {
        let mut c = self.c.clone();
        c.reverse();
        Self::from_coeffs(c)
    }

    /// `self(−x)`: the polynomial whose roots are the negated roots.
    pub fn neg_var(&self) -> Poly {
        Poly {
            c: self
                .c
                .iter()
                .enumerate()
                .map(|(i, a)| if i % 2 == 1 { a.negated() } else { a.clone() })
                .collect(),
        }
    }

    /// The monic polynomial with the same roots, each once: `self / gcd(self, self′)`.
    pub fn squarefree_part(&self) -> Poly {
        if self.is_zero() {
            return Poly::zero();
        }
        let g = self.gcd(&self.derivative());
        self.div_rem(&g).expect("the gcd of a nonzero polynomial is nonzero").0.monic()
    }

    /// The integer polynomial with the same roots: denominators cleared, content divided out,
    /// leading coefficient positive. Empty for the zero polynomial.
    pub fn primitive_int(&self) -> Vec<BigInt> {
        if self.is_zero() {
            return Vec::new();
        }
        let den = self
            .c
            .iter()
            .fold(BigInt::from_i64(1), |l, r| lcm(&l, r.denominator()));
        let ints: Vec<BigInt> = self
            .c
            .iter()
            .map(|r| {
                let scale = den.div_rem(r.denominator()).expect("denominators are nonzero").0;
                r.numerator().mul(&scale)
            })
            .collect();
        z_primitive(&ints)
    }

    /// Cauchy's bound `1 + max |aᵢ/aₙ|`: every complex root lies strictly inside it.
    pub fn root_bound(&self) -> Rational {
        let Some(n) = self.degree() else {
            return Rational::one();
        };
        let lc = self.c[n].abs();
        let max = self.c[..n]
            .iter()
            .map(|a| a.abs().div(&lc).expect("the leading coefficient is nonzero"))
            .max()
            .unwrap_or_else(Rational::zero);
        Rational::one().add(&max)
    }

    /// The number of distinct real roots.
    pub fn count_real_roots(&self) -> usize {
        match self.degree() {
            None | Some(0) => 0,
            Some(_) => {
                let chain = self.squarefree_part().sturm_chain();
                variations_at_neg_inf(&chain) - variations_at_pos_inf(&chain)
            }
        }
    }

    /// The number of distinct real roots in the half-open interval `(lo, hi]`.
    pub fn count_real_roots_in(&self, lo: &Rational, hi: &Rational) -> usize {
        match self.degree() {
            None | Some(0) => 0,
            Some(_) if lo >= hi => 0,
            Some(_) => {
                let chain = self.squarefree_part().sturm_chain();
                variations_at(&chain, lo) - variations_at(&chain, hi)
            }
        }
    }

    /// Every distinct real root, isolated, in increasing order.
    pub fn isolate_real_roots(&self) -> Vec<RootInterval> {
        match self.degree() {
            None | Some(0) => Vec::new(),
            Some(_) => {
                let s = self.squarefree_part();
                let chain = s.sturm_chain();
                let hi = s.root_bound();
                let lo = hi.negated();
                let n = variations_at(&chain, &lo) - variations_at(&chain, &hi);
                let mut out = Vec::new();
                isolate(&s, &chain, lo, hi, n, &mut out);
                out
            }
        }
    }

    /// Halve an isolating interval of one of `self`'s roots, keeping the root inside. An exact
    /// root stays as it is; a midpoint that is the root becomes the exact interval.
    pub fn refine(&self, iv: &RootInterval) -> RootInterval {
        if iv.lo == iv.hi {
            return iv.clone();
        }
        let s = self.squarefree_part();
        let mid = half_of_sum(&iv.lo, &iv.hi);
        let at_mid = sign(&s.eval(&mid));
        if at_mid == 0 {
            RootInterval {
                lo: mid.clone(),
                hi: mid,
            }
        } else if at_mid == sign(&s.eval(&iv.lo)) {
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

    /// The resultant `lc(self)^{deg o} · ∏ o(α)` over the roots `α` of `self`; zero exactly when
    /// the two share a root, and zero when either is the zero polynomial.
    pub fn resultant(&self, o: &Poly) -> Rational {
        let (Some(m), Some(n)) = (self.degree(), o.degree()) else {
            return Rational::zero();
        };
        if n == 0 {
            return o.c[0].pow(m as i32).expect("a nonzero constant has every power");
        }
        if m == 0 {
            return self.c[0].pow(n as i32).expect("a nonzero constant has every power");
        }
        let r = self.div_rem(o).expect("o is nonzero").1;
        let Some(dr) = r.degree() else {
            return Rational::zero();
        };
        let lc = o.c[n]
            .pow((m - dr) as i32)
            .expect("a nonzero leading coefficient has every power");
        let v = lc.mul(&o.resultant(&r));
        if (m * n) % 2 == 1 {
            v.negated()
        } else {
            v
        }
    }

    /// The factorization over ℚ into irreducible primitive integer polynomials (positive leading
    /// coefficient) with multiplicities, in [`Poly::canonical_order`]. The constant content is
    /// dropped; a constant or zero polynomial has no factors.
    pub fn factor_over_q(&self) -> Vec<(Poly, u32)> {
        match self.degree() {
            None | Some(0) => return Vec::new(),
            Some(_) => {}
        }
        let mut out = Vec::new();
        for (part, mult) in self.squarefree_decomposition() {
            for g in factor_squarefree_primitive(part.primitive_int()) {
                out.push((Poly::from_bigints(&g), mult));
            }
        }
        out.sort_by(|a, b| Poly::canonical_order(&a.0, &b.0));
        out
    }

    /// Irreducible over ℚ: positive degree and no proper factorization.
    pub fn is_irreducible(&self) -> bool {
        if self.degree().unwrap_or(0) == 0 {
            return false;
        }
        let fs = self.factor_over_q();
        fs.len() == 1 && fs[0].1 == 1
    }

    /// The total order factors are listed in: by degree, then by coefficients from the highest
    /// power down.
    pub fn canonical_order(a: &Poly, b: &Poly) -> Ordering {
        a.degree().cmp(&b.degree()).then_with(|| {
            for i in (0..a.c.len()).rev() {
                match a.c[i].cmp(&b.c[i]) {
                    Ordering::Equal => continue,
                    other => return other,
                }
            }
            Ordering::Equal
        })
    }

    /// Yun's algorithm: `self = c · ∏ aᵢ^i` with each `aᵢ` monic, squarefree and pairwise coprime;
    /// the constant parts are omitted.
    fn squarefree_decomposition(&self) -> Vec<(Poly, u32)> {
        let f = self.monic();
        let fp = f.derivative();
        let a0 = f.gcd(&fp);
        let mut b = f.div_rem(&a0).expect("gcd is nonzero").0;
        let c = fp.div_rem(&a0).expect("gcd is nonzero").0;
        let mut d = c.sub(&b.derivative());
        let mut i = 1u32;
        let mut out = Vec::new();
        while b.degree().unwrap_or(0) > 0 {
            let a = b.gcd(&d);
            if a.degree().unwrap_or(0) > 0 {
                out.push((a.clone(), i));
            }
            b = b.div_rem(&a).expect("gcd is nonzero").0;
            let c_next = d.div_rem(&a).expect("gcd is nonzero").0;
            d = c_next.sub(&b.derivative());
            i += 1;
        }
        out
    }

    /// The Sturm chain `p, p′, −rem(p, p′), …`, each member scaled by a positive constant to keep
    /// its coefficients small (a positive scale leaves every sign, so every count, unchanged).
    fn sturm_chain(&self) -> Vec<Poly> {
        let mut chain = vec![self.clone()];
        let d = self.derivative();
        if d.is_zero() {
            return chain;
        }
        chain.push(d);
        loop {
            let n = chain.len();
            let r = chain[n - 2].div_rem(&chain[n - 1]).expect("chain members are nonzero").1;
            if r.is_zero() {
                return chain;
            }
            let lc = r.leading().expect("r is nonzero").abs();
            chain.push(r.neg().scale(&lc.recip().expect("lc is nonzero")));
        }
    }
}

fn sign_changes(signs: impl Iterator<Item = i32>) -> usize {
    let mut prev = 0;
    let mut n = 0;
    for s in signs {
        if s == 0 {
            continue;
        }
        if prev != 0 && s != prev {
            n += 1;
        }
        prev = s;
    }
    n
}

fn variations_at(chain: &[Poly], x: &Rational) -> usize {
    sign_changes(chain.iter().map(|p| sign(&p.eval(x))))
}

fn variations_at_pos_inf(chain: &[Poly]) -> usize {
    sign_changes(chain.iter().map(|p| sign(p.leading().expect("chain members are nonzero"))))
}

fn variations_at_neg_inf(chain: &[Poly]) -> usize {
    sign_changes(chain.iter().map(|p| {
        let s = sign(p.leading().expect("chain members are nonzero"));
        if p.degree().unwrap_or(0) % 2 == 1 {
            -s
        } else {
            s
        }
    }))
}

/// Bisect `(lo, hi)`, which holds `n` roots of the squarefree `s` and has no root at either end.
/// A midpoint that lands on a root is recorded exactly, and the bisection steps around it by a
/// margin that leaves it alone in its window, so every open interval it reports has nonzero ends.
fn isolate(
    s: &Poly,
    chain: &[Poly],
    lo: Rational,
    hi: Rational,
    n: usize,
    out: &mut Vec<RootInterval>,
) {
    if n == 0 {
        return;
    }
    if n == 1 {
        out.push(RootInterval { lo, hi });
        return;
    }
    let mid = half_of_sum(&lo, &hi);
    if !s.eval(&mid).is_zero() {
        let left = variations_at(chain, &lo) - variations_at(chain, &mid);
        isolate(s, chain, lo, mid.clone(), left, out);
        isolate(s, chain, mid, hi, n - left, out);
        return;
    }
    let quarter = Rational::from_ratio_i64(1, 4).expect("4 is nonzero");
    let half = Rational::from_ratio_i64(1, 2).expect("2 is nonzero");
    let mut delta = hi.sub(&lo).mul(&quarter);
    let (a, b) = loop {
        let a = mid.sub(&delta);
        let b = mid.add(&delta);
        if !s.eval(&a).is_zero()
            && !s.eval(&b).is_zero()
            && variations_at(chain, &a) - variations_at(chain, &b) == 1
        {
            break (a, b);
        }
        delta = delta.mul(&half);
    };
    let left = variations_at(chain, &lo) - variations_at(chain, &a);
    isolate(s, chain, lo, a, left, out);
    out.push(RootInterval {
        lo: mid.clone(),
        hi: mid,
    });
    isolate(s, chain, b, hi, n - left - 1, out);
}

impl fmt::Display for Poly {
    /// Descending powers: `x^3 - x - 1`, `-x^2 + 1`, `x^2 - (1/3)x + 1/2`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_zero() {
            return write!(f, "0");
        }
        let mut first = true;
        for i in (0..self.c.len()).rev() {
            let c = &self.c[i];
            if c.is_zero() {
                continue;
            }
            let neg = c.is_negative();
            if first {
                if neg {
                    write!(f, "-")?;
                }
            } else {
                write!(f, "{}", if neg { " - " } else { " + " })?;
            }
            first = false;
            let a = c.abs();
            let coef = if i == 0 || a.is_integer() {
                if i > 0 && a == Rational::one() {
                    String::new()
                } else {
                    a.to_string()
                }
            } else {
                format!("({a})")
            };
            match i {
                0 => write!(f, "{coef}")?,
                1 => write!(f, "{coef}x")?,
                _ => write!(f, "{coef}x^{i}")?,
            }
        }
        Ok(())
    }
}

// ── integer polynomials ───────────────────────────────────────────────────────────────────────

fn lcm(a: &BigInt, b: &BigInt) -> BigInt {
    let g = a.gcd(b);
    a.div_rem(&g).expect("gcd of nonzero is nonzero").0.mul(b).abs()
}

fn z_trim(mut v: Vec<BigInt>) -> Vec<BigInt> {
    while v.last().is_some_and(BigInt::is_zero) {
        v.pop();
    }
    v
}

fn z_add(a: &[BigInt], b: &[BigInt]) -> Vec<BigInt> {
    let n = a.len().max(b.len());
    let zero = BigInt::zero();
    z_trim(
        (0..n)
            .map(|i| a.get(i).unwrap_or(&zero).add(b.get(i).unwrap_or(&zero)))
            .collect(),
    )
}

fn z_sub(a: &[BigInt], b: &[BigInt]) -> Vec<BigInt> {
    let n = a.len().max(b.len());
    let zero = BigInt::zero();
    z_trim(
        (0..n)
            .map(|i| a.get(i).unwrap_or(&zero).sub(b.get(i).unwrap_or(&zero)))
            .collect(),
    )
}

fn z_mul(a: &[BigInt], b: &[BigInt]) -> Vec<BigInt> {
    if a.is_empty() || b.is_empty() {
        return Vec::new();
    }
    let mut out = vec![BigInt::zero(); a.len() + b.len() - 1];
    for (i, x) in a.iter().enumerate() {
        if x.is_zero() {
            continue;
        }
        for (j, y) in b.iter().enumerate() {
            out[i + j] = out[i + j].add(&x.mul(y));
        }
    }
    z_trim(out)
}

fn z_scale(a: &[BigInt], k: &BigInt) -> Vec<BigInt> {
    z_trim(a.iter().map(|x| x.mul(k)).collect())
}

fn mod_nonneg(x: &BigInt, m: &BigInt) -> BigInt {
    let r = x.div_rem(m).expect("the modulus is nonzero").1;
    if r.is_negative() {
        r.add(m)
    } else {
        r
    }
}

fn zm_reduce(a: &[BigInt], m: &BigInt) -> Vec<BigInt> {
    z_trim(a.iter().map(|x| mod_nonneg(x, m)).collect())
}

/// The representative of `x mod m` in `(−m/2, m/2]`.
fn symmetric(x: &BigInt, m: &BigInt) -> BigInt {
    let r = mod_nonneg(x, m);
    if r.add(&r) > *m {
        r.sub(m)
    } else {
        r
    }
}

/// Divide out the content and make the leading coefficient positive.
fn z_primitive(a: &[BigInt]) -> Vec<BigInt> {
    let a = z_trim(a.to_vec());
    let Some(lead) = a.last() else {
        return a;
    };
    let content = a.iter().fold(BigInt::zero(), |g, x| g.gcd(x));
    let content = if lead.is_negative() {
        content.negated()
    } else {
        content
    };
    a.iter()
        .map(|x| x.div_rem(&content).expect("content of a nonzero polynomial is nonzero").0)
        .collect()
}

/// `f / g` over ℤ when `g` divides `f` exactly with an integer quotient.
fn z_div_exact(f: &[BigInt], g: &[BigInt]) -> Option<Vec<BigInt>> {
    let dg = g.len().checked_sub(1)?;
    if f.len() <= dg {
        return if f.is_empty() { Some(Vec::new()) } else { None };
    }
    let lc = &g[dg];
    let mut r = f.to_vec();
    let mut q = vec![BigInt::zero(); r.len() - dg];
    for k in (0..q.len()).rev() {
        let (coef, rem) = r[k + dg].div_rem(lc).expect("lc is nonzero");
        if !rem.is_zero() {
            return None;
        }
        if !coef.is_zero() {
            for (j, gj) in g.iter().enumerate() {
                r[k + j] = r[k + j].sub(&coef.mul(gj));
            }
        }
        q[k] = coef;
    }
    if r[..dg].iter().all(BigInt::is_zero) {
        Some(z_trim(q))
    } else {
        None
    }
}

fn bigint_mod_inverse(a: &BigInt, m: &BigInt) -> BigInt {
    let (mut r0, mut r1) = (mod_nonneg(a, m), m.clone());
    let (mut s0, mut s1) = (BigInt::from_i64(1), BigInt::zero());
    while !r1.is_zero() {
        let (q, r) = r0.div_rem(&r1).expect("r1 is nonzero");
        r0 = std::mem::replace(&mut r1, r);
        let s2 = s0.sub(&q.mul(&s1));
        s0 = std::mem::replace(&mut s1, s2);
    }
    mod_nonneg(&s0, m)
}

// ── polynomials over GF(p), p a small prime ──────────────────────────────────────────────────

fn fp_trim(mut v: Vec<u64>) -> Vec<u64> {
    while v.last() == Some(&0) {
        v.pop();
    }
    v
}

fn pow_mod(mut b: u64, mut e: u64, p: u64) -> u64 {
    let mut acc = 1u64;
    b %= p;
    while e > 0 {
        if e & 1 == 1 {
            acc = acc * b % p;
        }
        b = b * b % p;
        e >>= 1;
    }
    acc
}

fn fp_inv(a: u64, p: u64) -> u64 {
    pow_mod(a, p - 2, p)
}

fn modp(x: &BigInt, p: u64) -> u64 {
    mod_nonneg(x, &BigInt::from_u64(p))
        .to_u64()
        .expect("a residue below p fits u64")
}

fn to_fp(f: &[BigInt], p: u64) -> Vec<u64> {
    fp_trim(f.iter().map(|c| modp(c, p)).collect())
}

fn from_fp(v: &[u64]) -> Vec<BigInt> {
    v.iter().map(|&x| BigInt::from_u64(x)).collect()
}

fn fp_add(a: &[u64], b: &[u64], p: u64) -> Vec<u64> {
    let n = a.len().max(b.len());
    fp_trim(
        (0..n)
            .map(|i| (a.get(i).copied().unwrap_or(0) + b.get(i).copied().unwrap_or(0)) % p)
            .collect(),
    )
}

fn fp_sub(a: &[u64], b: &[u64], p: u64) -> Vec<u64> {
    let n = a.len().max(b.len());
    fp_trim(
        (0..n)
            .map(|i| (a.get(i).copied().unwrap_or(0) + p - b.get(i).copied().unwrap_or(0)) % p)
            .collect(),
    )
}

fn fp_mul(a: &[u64], b: &[u64], p: u64) -> Vec<u64> {
    if a.is_empty() || b.is_empty() {
        return Vec::new();
    }
    let mut out = vec![0u64; a.len() + b.len() - 1];
    for (i, &x) in a.iter().enumerate() {
        if x == 0 {
            continue;
        }
        for (j, &y) in b.iter().enumerate() {
            out[i + j] = (out[i + j] + x * y) % p;
        }
    }
    fp_trim(out)
}

fn fp_scale(a: &[u64], k: u64, p: u64) -> Vec<u64> {
    fp_trim(a.iter().map(|&x| x * k % p).collect())
}

fn fp_divrem(a: &[u64], b: &[u64], p: u64) -> (Vec<u64>, Vec<u64>) {
    let db = b.len() - 1;
    if a.len() <= db {
        return (Vec::new(), a.to_vec());
    }
    let inv = fp_inv(b[db], p);
    let mut r = a.to_vec();
    let mut q = vec![0u64; r.len() - db];
    for k in (0..q.len()).rev() {
        let coef = r[k + db] * inv % p;
        if coef != 0 {
            for (j, &bj) in b.iter().enumerate() {
                r[k + j] = (r[k + j] + p - coef * bj % p) % p;
            }
        }
        q[k] = coef;
    }
    r.truncate(db);
    (fp_trim(q), fp_trim(r))
}

fn fp_monic(a: &[u64], p: u64) -> Vec<u64> {
    match a.last() {
        None => Vec::new(),
        Some(&lc) => fp_scale(a, fp_inv(lc, p), p),
    }
}

fn fp_gcd(a: &[u64], b: &[u64], p: u64) -> Vec<u64> {
    let (mut a, mut b) = (fp_monic(a, p), fp_monic(b, p));
    while !b.is_empty() {
        let r = fp_divrem(&a, &b, p).1;
        a = b;
        b = fp_monic(&r, p);
    }
    a
}

/// `(g, s, t)` with `s·a + t·b = g`, `g` the monic gcd.
fn fp_ext_gcd(a: &[u64], b: &[u64], p: u64) -> (Vec<u64>, Vec<u64>, Vec<u64>) {
    let (mut r0, mut r1) = (a.to_vec(), b.to_vec());
    let (mut s0, mut s1) = (vec![1u64], Vec::new());
    let (mut t0, mut t1) = (Vec::new(), vec![1u64]);
    while !r1.is_empty() {
        let (q, r) = fp_divrem(&r0, &r1, p);
        r0 = std::mem::replace(&mut r1, r);
        let s2 = fp_sub(&s0, &fp_mul(&q, &s1, p), p);
        s0 = std::mem::replace(&mut s1, s2);
        let t2 = fp_sub(&t0, &fp_mul(&q, &t1, p), p);
        t0 = std::mem::replace(&mut t1, t2);
    }
    let inv = fp_inv(*r0.last().expect("gcd of nonzero polynomials is nonzero"), p);
    (fp_scale(&r0, inv, p), fp_scale(&s0, inv, p), fp_scale(&t0, inv, p))
}

fn fp_derivative(a: &[u64], p: u64) -> Vec<u64> {
    fp_trim(
        a.iter()
            .enumerate()
            .skip(1)
            .map(|(i, &x)| x * (i as u64 % p) % p)
            .collect(),
    )
}

/// `base^e mod f`.
fn fp_powmod(base: &[u64], mut e: u64, f: &[u64], p: u64) -> Vec<u64> {
    let mut acc = vec![1u64];
    let mut b = fp_divrem(base, f, p).1;
    while e > 0 {
        if e & 1 == 1 {
            acc = fp_divrem(&fp_mul(&acc, &b, p), f, p).1;
        }
        e >>= 1;
        if e > 0 {
            b = fp_divrem(&fp_mul(&b, &b, p), f, p).1;
        }
    }
    acc
}

/// A basis of the null space of `a` over GF(p), by reduction to row echelon form.
fn nullspace_mod_p(mut a: Vec<Vec<u64>>, p: u64) -> Vec<Vec<u64>> {
    let rows = a.len();
    let cols = a[0].len();
    let mut pivot_cols = Vec::new();
    let mut r = 0;
    for c in 0..cols {
        if r == rows {
            break;
        }
        let Some(pr) = (r..rows).find(|&i| a[i][c] != 0) else {
            continue;
        };
        a.swap(r, pr);
        let inv = fp_inv(a[r][c], p);
        for x in a[r].iter_mut() {
            *x = *x * inv % p;
        }
        for i in 0..rows {
            if i != r && a[i][c] != 0 {
                let f = a[i][c];
                let pivot = a[r].clone();
                for (x, y) in a[i].iter_mut().zip(&pivot) {
                    *x = (*x + p - f * y % p) % p;
                }
            }
        }
        pivot_cols.push(c);
        r += 1;
    }
    (0..cols)
        .filter(|c| !pivot_cols.contains(c))
        .map(|fc| {
            let mut v = vec![0u64; cols];
            v[fc] = 1;
            for (ri, &pc) in pivot_cols.iter().enumerate() {
                v[pc] = (p - a[ri][fc]) % p;
            }
            v
        })
        .collect()
}

/// Berlekamp's algorithm: the monic irreducible factors of a monic squarefree `f` over GF(p).
/// The fixed space of the Frobenius `v ↦ vᵖ` on `GF(p)[x]/f` has dimension equal to the factor
/// count; gcds of `f` with `v − s` over its basis split `f` completely.
fn berlekamp(f: &[u64], p: u64) -> Vec<Vec<u64>> {
    let n = f.len() - 1;
    let xp = fp_powmod(&[0, 1], p, f, p);
    let mut rows = Vec::with_capacity(n);
    let mut cur = vec![1u64];
    for _ in 0..n {
        let mut row = cur.clone();
        row.resize(n, 0);
        rows.push(row);
        cur = fp_divrem(&fp_mul(&cur, &xp, p), f, p).1;
    }
    let mut a = vec![vec![0u64; n]; n];
    for (i, row) in rows.iter().enumerate() {
        for (j, &q) in row.iter().enumerate() {
            a[j][i] = if i == j { (q + p - 1) % p } else { q };
        }
    }
    let basis = nullspace_mod_p(a, p);
    let r = basis.len();
    let mut factors = vec![f.to_vec()];
    'split: for v in &basis {
        let v = fp_trim(v.clone());
        if v.len() <= 1 {
            continue;
        }
        for s in 0..p {
            let mut shifted = v.clone();
            shifted[0] = (shifted[0] + p - s) % p;
            let shifted = fp_trim(shifted);
            let mut next = Vec::with_capacity(factors.len() + 1);
            for g in factors.drain(..) {
                if g.len() <= 2 {
                    next.push(g);
                    continue;
                }
                let h = fp_gcd(&g, &shifted, p);
                if h.len() > 1 && h.len() < g.len() {
                    let rest = fp_monic(&fp_divrem(&g, &h, p).0, p);
                    next.push(h);
                    next.push(rest);
                } else {
                    next.push(g);
                }
            }
            factors = next;
            if factors.len() == r {
                break 'split;
            }
        }
    }
    factors.sort();
    factors
}

/// Lift `f ≡ g₀·h₀ (mod p)` (`g₀` monic, coprime to `h₀`) to `f ≡ g·h (mod pᵏ)`, one factor of
/// `p` per step: with `s·g₀ + t·h₀ ≡ 1`, the error `e = (f − g·h)/m` is absorbed by
/// `δg = t·e mod g₀` and `δh = s·e + (t·e div g₀)·h₀`, since `δg·h₀ + δh·g₀ ≡ e (mod p)`.
fn hensel_pair(
    f: &[BigInt],
    g0: &[u64],
    h0: &[u64],
    p: u64,
    k: u32,
) -> (Vec<BigInt>, Vec<BigInt>) {
    let (_, s, t) = fp_ext_gcd(g0, h0, p);
    let pb = BigInt::from_u64(p);
    let mut m = pb.clone();
    let mut g = from_fp(g0);
    let mut h = from_fp(h0);
    for _ in 1..k {
        let diff = z_sub(f, &z_mul(&g, &h));
        let e = fp_trim(
            diff.iter()
                .map(|c| {
                    let (q, r) = c.div_rem(&m).expect("m is nonzero");
                    debug_assert!(r.is_zero(), "f ≡ g·h (mod m) makes the error divisible by m");
                    modp(&q, p)
                })
                .collect(),
        );
        let te = fp_mul(&t, &e, p);
        let (q, dg) = fp_divrem(&te, g0, p);
        let dh = fp_add(&fp_mul(&s, &e, p), &fp_mul(&q, h0, p), p);
        let mp = m.mul(&pb);
        g = zm_reduce(&z_add(&g, &z_scale(&from_fp(&dg), &m)), &mp);
        h = zm_reduce(&z_add(&h, &z_scale(&from_fp(&dh), &m)), &mp);
        m = mp;
    }
    (g, h)
}

/// Lift every factor of `f ≡ lc(f)·∏ gᵢ (mod p)` to a monic factor modulo `pᵏ`, peeling one
/// factor at a time off the lifted cofactor.
fn hensel_multi(f: &[BigInt], mods: &[Vec<u64>], p: u64, k: u32, pk: &BigInt) -> Vec<Vec<BigInt>> {
    let mut out = Vec::with_capacity(mods.len());
    let mut target = f.to_vec();
    for g0 in &mods[..mods.len() - 1] {
        let (h0, rem) = fp_divrem(&to_fp(&target, p), g0, p);
        debug_assert!(rem.is_empty(), "every modular factor divides the target");
        let (g, h) = hensel_pair(&target, g0, &h0, p, k);
        out.push(g);
        target = h;
    }
    let inv = bigint_mod_inverse(target.last().expect("the cofactor is nonzero"), pk);
    out.push(zm_reduce(&z_scale(&target, &inv), pk));
    out
}

/// Advance `idx` (a strictly increasing `s`-subset of `0..n`) to the next subset in lexicographic
/// order; `false` when it was the last.
fn next_combination(idx: &mut [usize], n: usize) -> bool {
    let s = idx.len();
    for i in (0..s).rev() {
        if idx[i] < n - s + i {
            idx[i] += 1;
            for j in i + 1..s {
                idx[j] = idx[j - 1] + 1;
            }
            return true;
        }
    }
    false
}

/// Zassenhaus recombination: a true factor of `f` is, modulo `pᵏ`, `lc(f)` times a product of a
/// subset of the lifted factors; past the Mignotte bound its symmetric representative is the
/// factor itself (up to content), which trial division confirms.
fn recombine(mut f: Vec<BigInt>, mut lifted: Vec<Vec<BigInt>>, pk: &BigInt) -> Vec<Vec<BigInt>> {
    let mut found = Vec::new();
    let mut s = 1;
    while 2 * s <= lifted.len() {
        let n = lifted.len();
        let mut idx: Vec<usize> = (0..s).collect();
        let mut hit = None;
        loop {
            let lc = f.last().expect("f has positive degree").clone();
            let mut cand = vec![lc];
            for &i in &idx {
                cand = zm_reduce(&z_mul(&cand, &lifted[i]), pk);
            }
            let cand = z_primitive(&z_trim(cand.iter().map(|c| symmetric(c, pk)).collect()));
            if let Some(q) = z_div_exact(&f, &cand) {
                hit = Some((cand, q));
                break;
            }
            if !next_combination(&mut idx, n) {
                break;
            }
        }
        match hit {
            Some((g, q)) => {
                found.push(g);
                f = z_primitive(&q);
                for &i in idx.iter().rev() {
                    lifted.remove(i);
                }
            }
            None => s += 1,
        }
    }
    if f.len() > 1 {
        found.push(z_primitive(&f));
    }
    found
}

fn is_prime(n: u64) -> bool {
    n >= 2 && (2..).take_while(|d| d * d <= n).all(|d| !n.is_multiple_of(d))
}

/// Factor a squarefree primitive integer polynomial into its irreducible factors over ℤ.
fn factor_squarefree_primitive(f: Vec<BigInt>) -> Vec<Vec<BigInt>> {
    let n = f.len() - 1;
    if n <= 1 {
        return vec![f];
    }
    let lc = f.last().expect("positive degree").clone();
    let mut best: Option<(u64, Vec<Vec<u64>>)> = None;
    let mut admissible = 0;
    for p in (3u64..).step_by(2).filter(|&p| is_prime(p)) {
        if modp(&lc, p) == 0 {
            continue;
        }
        let fm = fp_monic(&to_fp(&f, p), p);
        if fp_gcd(&fm, &fp_derivative(&fm, p), p).len() != 1 {
            continue;
        }
        let factors = berlekamp(&fm, p);
        if factors.len() == 1 {
            return vec![f];
        }
        if best.as_ref().is_none_or(|(_, b)| factors.len() < b.len()) {
            best = Some((p, factors));
        }
        admissible += 1;
        if admissible == 5 {
            break;
        }
    }
    let (p, mods) = best.expect("a squarefree polynomial has admissible primes");
    let l1 = f.iter().fold(BigInt::zero(), |a, c| a.add(&c.abs()));
    let bound = lc.abs().mul(&l1).shl_bits(n as u64 + 1);
    let pb = BigInt::from_u64(p);
    let mut pk = pb.clone();
    let mut k = 1u32;
    while pk <= bound {
        pk = pk.mul(&pb);
        k += 1;
    }
    let lifted = hensel_multi(&f, &mods, p, k, &pk);
    recombine(f, lifted, &pk)
}

impl crate::exact_codec::ExactCodec for Poly {
    /// The coefficients, lowest power first.
    fn write(&self, out: &mut Vec<u8>) {
        crate::exact_codec::write_list(&self.c, out);
    }

    fn read(buf: &[u8], pos: &mut usize) -> Option<Self> {
        Some(Poly::from_coeffs(crate::exact_codec::read_list(buf, pos)?))
    }
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_base/poly/tests.rs"]
mod tests;
