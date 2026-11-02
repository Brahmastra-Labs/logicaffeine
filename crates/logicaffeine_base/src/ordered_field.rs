//! Polynomials over an ordered field with exact arithmetic, the coefficient rings of the hyperreals.
//!
//! The hyperreals a program's infinitesimal generates are rational functions of it over the real
//! algebraic numbers; a second infinitesimal smaller than every power of the first generates rational
//! functions of itself over those, and so on. Each level is one variable over the field below, so the
//! arithmetic every level needs (sums, products, Euclid's division and gcd, Yun's square-free
//! decomposition) is written once here, over any [`OrderedField`].

use std::fmt::Debug;
use std::hash::Hash;

use crate::algebraic::Algebraic;

/// A field with exact arithmetic and a decided order, read from the sign of an element.
pub trait OrderedField: Clone + PartialEq + Eq + Hash + Debug {
    fn zero() -> Self;
    fn one() -> Self;
    fn from_i64(n: i64) -> Self;
    fn is_zero(&self) -> bool;
    fn add(&self, o: &Self) -> Self;
    fn neg(&self) -> Self;
    fn mul(&self, o: &Self) -> Self;
    /// `None` for zero.
    fn recip(&self) -> Option<Self>;
    /// `−1`, `0` or `1`.
    fn signum(&self) -> i32;

    fn sub(&self, o: &Self) -> Self {
        self.add(&o.neg())
    }
}

impl OrderedField for Algebraic {
    fn zero() -> Self {
        Algebraic::from_i64(0)
    }
    fn one() -> Self {
        Algebraic::from_i64(1)
    }
    fn from_i64(n: i64) -> Self {
        Algebraic::from_i64(n)
    }
    fn is_zero(&self) -> bool {
        Algebraic::is_zero(self)
    }
    fn add(&self, o: &Self) -> Self {
        Algebraic::add(self, o)
    }
    fn neg(&self) -> Self {
        Algebraic::neg(self)
    }
    fn mul(&self, o: &Self) -> Self {
        Algebraic::mul(self, o)
    }
    fn recip(&self) -> Option<Self> {
        Algebraic::recip(self)
    }
    fn signum(&self) -> i32 {
        Algebraic::signum(self)
    }
    fn sub(&self, o: &Self) -> Self {
        Algebraic::sub(self, o)
    }
}

/// A polynomial in one variable over `K`, little-endian, no trailing zero.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PolyOver<K> {
    pub(crate) c: Vec<K>,
}

impl<K: OrderedField> PolyOver<K> {
    pub fn from(c: Vec<K>) -> PolyOver<K> {
        let mut c = c;
        while c.last().is_some_and(K::is_zero) {
            c.pop();
        }
        PolyOver { c }
    }

    pub fn constant(a: K) -> PolyOver<K> {
        PolyOver::from(vec![a])
    }

    pub fn zero() -> PolyOver<K> {
        PolyOver { c: Vec::new() }
    }

    pub fn one() -> PolyOver<K> {
        PolyOver::constant(K::one())
    }

    /// The variable itself.
    pub fn variable() -> PolyOver<K> {
        PolyOver::from(vec![K::zero(), K::one()])
    }

    pub fn is_zero(&self) -> bool {
        self.c.is_empty()
    }

    pub fn degree(&self) -> Option<usize> {
        self.c.len().checked_sub(1)
    }

    /// The index of the lowest-order nonzero coefficient.
    pub fn order(&self) -> Option<usize> {
        self.c.iter().position(|a| !a.is_zero())
    }

    pub fn coeff(&self, i: usize) -> K {
        self.c.get(i).cloned().unwrap_or_else(K::zero)
    }

    pub fn add(&self, o: &PolyOver<K>) -> PolyOver<K> {
        let n = self.c.len().max(o.c.len());
        PolyOver::from((0..n).map(|i| self.coeff(i).add(&o.coeff(i))).collect())
    }

    pub fn neg(&self) -> PolyOver<K> {
        PolyOver {
            c: self.c.iter().map(K::neg).collect(),
        }
    }

    pub fn mul(&self, o: &PolyOver<K>) -> PolyOver<K> {
        if self.is_zero() || o.is_zero() {
            return PolyOver::zero();
        }
        let mut out = vec![K::zero(); self.c.len() + o.c.len() - 1];
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
        PolyOver::from(out)
    }

    pub fn scale(&self, k: &K) -> PolyOver<K> {
        PolyOver::from(self.c.iter().map(|a| a.mul(k)).collect())
    }

    /// Euclid's division by a nonzero `d`: `(q, r)` with `self = q·d + r` and `deg r < deg d`.
    pub fn div_rem(&self, d: &PolyOver<K>) -> (PolyOver<K>, PolyOver<K>) {
        let dd = d.degree().expect("the divisor is nonzero");
        let inv = d.c[dd].recip().expect("the leading coefficient is nonzero");
        if self.c.len() <= dd {
            return (PolyOver::zero(), self.clone());
        }
        let mut r = self.c.clone();
        let mut q = vec![K::zero(); r.len() - dd];
        for k in (0..q.len()).rev() {
            let coef = r[k + dd].mul(&inv);
            if !coef.is_zero() {
                for (j, dj) in d.c.iter().enumerate() {
                    r[k + j] = r[k + j].sub(&coef.mul(dj));
                }
            }
            q[k] = coef;
        }
        r.truncate(dd);
        (PolyOver::from(q), PolyOver::from(r))
    }

    pub fn monic(&self) -> PolyOver<K> {
        match self.c.last() {
            None => PolyOver::zero(),
            Some(lc) => self.scale(&lc.recip().expect("nonzero")),
        }
    }

    /// The monic greatest common divisor.
    pub fn gcd(&self, o: &PolyOver<K>) -> PolyOver<K> {
        let (mut a, mut b) = (self.monic(), o.monic());
        while !b.is_zero() {
            let r = a.div_rem(&b).1;
            a = b;
            b = r.monic();
        }
        a
    }

    pub fn derivative(&self) -> PolyOver<K> {
        PolyOver::from(
            self.c
                .iter()
                .enumerate()
                .skip(1)
                .map(|(i, a)| a.mul(&K::from_i64(i as i64)))
                .collect(),
        )
    }

    /// `lc · Π aᵢ^i` with each `aᵢ` monic and squarefree and the `aᵢ` coprime, by Yun's algorithm:
    /// the leading coefficient and each `(aᵢ, i)` of positive degree.
    pub fn squarefree_parts(&self) -> (K, Vec<(PolyOver<K>, usize)>) {
        let lc = self.c.last().cloned().unwrap_or_else(K::zero);
        if self.degree().unwrap_or(0) == 0 {
            return (lc, Vec::new());
        }
        let f = self.monic();
        let g = f.gcd(&f.derivative());
        let mut b = f.div_rem(&g).0;
        let c = f.derivative().div_rem(&g).0;
        let mut d = c.add(&b.derivative().neg());
        let mut parts = Vec::new();
        let mut i = 1;
        while b.degree().unwrap_or(0) > 0 {
            let a = b.gcd(&d);
            b = b.div_rem(&a).0;
            let c = d.div_rem(&a).0;
            d = c.add(&b.derivative().neg());
            if a.degree().unwrap_or(0) > 0 {
                parts.push((a, i));
            }
            i += 1;
        }
        (lc, parts)
    }

    pub fn eval(&self, t: &K) -> K {
        self.c.iter().rev().fold(K::zero(), |acc, a| acc.mul(t).add(a))
    }

    /// `self(x^m)`.
    pub fn spread(&self, m: usize) -> PolyOver<K> {
        let mut c = vec![K::zero(); self.c.len().saturating_sub(1) * m + 1];
        for (i, a) in self.c.iter().enumerate() {
            c[i * m] = a.clone();
        }
        PolyOver::from(c)
    }

    /// The squarefree part, monic: `self / gcd(self, self′)`.
    pub fn squarefree(&self) -> PolyOver<K> {
        if self.degree().unwrap_or(0) == 0 {
            return self.monic();
        }
        let g = self.gcd(&self.derivative());
        self.div_rem(&g).0.monic()
    }

    /// `self(x + a)`, by Horner's rule.
    pub fn shift(&self, a: &K) -> PolyOver<K> {
        let linear = PolyOver::from(vec![a.clone(), K::one()]);
        self.c
            .iter()
            .rev()
            .fold(PolyOver::zero(), |acc, c| acc.mul(&linear).add(&PolyOver::constant(c.clone())))
    }

    /// `self(−x)`.
    pub fn neg_var(&self) -> PolyOver<K> {
        PolyOver::from(
            self.c
                .iter()
                .enumerate()
                .map(|(i, a)| if i % 2 == 1 { a.neg() } else { a.clone() })
                .collect(),
        )
    }

    /// `self / x^k` for the largest `k` that divides it: the polynomial without its root `0`.
    pub fn without_zero_root(&self) -> PolyOver<K> {
        match self.order() {
            Some(k) => PolyOver::from(self.c[k..].to_vec()),
            None => PolyOver::zero(),
        }
    }

    /// `x^n·self(1/x)` for `n` the degree: the polynomial whose roots are the reciprocals of the
    /// nonzero roots.
    pub fn reversed(&self) -> PolyOver<K> {
        let mut c = self.without_zero_root().c;
        c.reverse();
        PolyOver::from(c)
    }

    /// The resultant `lc^{deg o}·Π o(α)` over the roots `α` of `self`, by Euclid's algorithm.
    pub fn resultant(&self, o: &PolyOver<K>) -> K {
        let (Some(m), Some(n)) = (self.degree(), o.degree()) else {
            return K::zero();
        };
        let power = |h: &K, k: usize| (0..k).fold(K::one(), |acc, _| acc.mul(h));
        if n == 0 {
            return power(&o.c[0], m);
        }
        if m == 0 {
            return power(&self.c[0], n);
        }
        let r = self.div_rem(o).1;
        let Some(dr) = r.degree() else {
            return K::zero();
        };
        let v = power(&o.c[n], m - dr).mul(&o.resultant(&r));
        if (m * n) % 2 == 1 {
            v.neg()
        } else {
            v
        }
    }

    /// The polynomial of degree at most `degree` whose value at each integer `j ≤ degree` is
    /// `at(j)`, by Newton's divided differences.
    pub fn interpolate(degree: usize, at: impl Fn(i64) -> K) -> PolyOver<K> {
        let nodes: Vec<i64> = (0..=degree as i64).collect();
        let mut values: Vec<K> = nodes.iter().map(|j| at(*j)).collect();
        let n = nodes.len();
        for j in 1..n {
            for i in (j..n).rev() {
                let gap = K::from_i64(nodes[i] - nodes[i - j]);
                values[i] = values[i]
                    .sub(&values[i - 1])
                    .mul(&gap.recip().expect("the nodes are distinct"));
            }
        }
        let mut out = PolyOver::constant(values[n - 1].clone());
        for i in (0..n - 1).rev() {
            let factor = PolyOver::from(vec![K::from_i64(-nodes[i]), K::one()]);
            out = out.mul(&factor).add(&PolyOver::constant(values[i].clone()));
        }
        out
    }

    /// The monic polynomial whose roots are the sums `rᵢ + sⱼ` of a root of each, every pair once —
    /// `Res_z(P(z), Q(y − z))` up to its leading coefficient — through power sums: the sums' `m`-th power
    /// sum is `Σ_k C(m, k)·p_k(P)·p_(m−k)(Q)`.
    pub fn sum_poly(p: &PolyOver<K>, q: &PolyOver<K>) -> PolyOver<K> {
        let (Some(dp), Some(dq)) = (p.degree(), q.degree()) else {
            return PolyOver::one();
        };
        let degree = dp * dq;
        let (sp, sq) = (p.power_sums(degree), q.power_sums(degree));
        let mut sums = vec![K::zero(); degree + 1];
        for (m, sum) in sums.iter_mut().enumerate().skip(1) {
            let mut choose = K::one();
            for k in 0..=m {
                if k > 0 {
                    choose = choose
                        .mul(&K::from_i64((m - k + 1) as i64))
                        .mul(&K::from_i64(k as i64).recip().expect("a positive integer is nonzero"));
                }
                *sum = sum.add(&choose.mul(&sp[k]).mul(&sq[m - k]));
            }
        }
        PolyOver::from_power_sums(&sums, degree)
    }

    /// The monic polynomial whose roots are the products `rᵢ·sⱼ` of a root of each, every pair once —
    /// `Res_z(P(z), z^n·Q(y/z))` up to its leading coefficient — through power sums: the products'
    /// `m`-th power sum is `p_m(P)·p_m(Q)`.
    pub fn product_poly(p: &PolyOver<K>, q: &PolyOver<K>) -> PolyOver<K> {
        let (Some(dp), Some(dq)) = (p.degree(), q.degree()) else {
            return PolyOver::one();
        };
        let degree = dp * dq;
        let (sp, sq) = (p.power_sums(degree), q.power_sums(degree));
        let sums: Vec<K> = (0..=degree).map(|m| sp[m].mul(&sq[m])).collect();
        PolyOver::from_power_sums(&sums, degree)
    }

    /// The power sums `s_0 … s_n` of the roots of the polynomial, `s_0` its degree, by Newton's
    /// identities from its coefficients.
    fn power_sums(&self, n: usize) -> Vec<K> {
        let d = self.degree().unwrap_or(0);
        let mut sums = vec![K::zero(); n + 1];
        sums[0] = K::from_i64(d as i64);
        if d == 0 {
            return sums;
        }
        let lead = self.coeff(d).recip().expect("a leading coefficient is nonzero");
        let a: Vec<K> = (0..=d).map(|i| if i == 0 { K::one() } else { self.coeff(d - i).mul(&lead) }).collect();
        for m in 1..=n {
            let mut acc = if m <= d { a[m].mul(&K::from_i64(m as i64)) } else { K::zero() };
            for i in 1..m.min(d + 1) {
                acc = acc.add(&a[i].mul(&sums[m - i]));
            }
            sums[m] = acc.neg();
        }
        sums
    }

    /// The monic polynomial of degree `d` whose roots have the power sums `sums[1] … sums[d]`.
    fn from_power_sums(sums: &[K], d: usize) -> PolyOver<K> {
        let mut b = vec![K::one(); d + 1];
        for j in 1..=d {
            let mut acc = sums[j].clone();
            for i in 1..j {
                acc = acc.add(&b[i].mul(&sums[j - i]));
            }
            b[j] = acc.mul(&K::from_i64(j as i64).recip().expect("a positive integer is nonzero")).neg();
        }
        PolyOver::from((0..=d).map(|i| b[d - i].clone()).collect())
    }

    /// The rational function `num/den` of numerator degree at most `l` and denominator degree at most
    /// `m`, `den(0) ≠ 0`, whose expansion at 0 begins with `series` (its first `l + m + 1`
    /// coefficients), by the extended Euclidean algorithm on `x^(l+m+1)` and the truncated series;
    /// `None` when no such function exists.
    pub fn pade(series: &[K], l: usize, m: usize) -> Option<(PolyOver<K>, PolyOver<K>)> {
        let n = l + m + 1;
        let mut top = vec![K::zero(); n + 1];
        top[n] = K::one();
        let truncated: Vec<K> = (0..n).map(|i| series.get(i).cloned().unwrap_or_else(K::zero)).collect();
        let (mut r0, mut r1) = (PolyOver::from(top), PolyOver::from(truncated));
        let (mut t0, mut t1) = (PolyOver::zero(), PolyOver::one());
        while r1.degree().is_some_and(|d| d > l) {
            let (q, r) = r0.div_rem(&r1);
            let t = t0.add(&q.mul(&t1).neg());
            r0 = std::mem::replace(&mut r1, r);
            t0 = std::mem::replace(&mut t1, t);
        }
        if t1.degree().unwrap_or(0) > m || t1.coeff(0).is_zero() {
            return None;
        }
        Some((r1, t1))
    }

    /// The monic polynomial whose roots are the `k`-th powers of the roots of `P`, each as often as in
    /// `P` — `Res_z(P(z), y − z^k)` up to its leading coefficient. A binomial `P = a·zⁿ + b`, whose roots
    /// are the `n`-th roots of `c = −b/a`, has their `k`-th powers the roots of `y^(n/g) − c^(k/g)` for
    /// `g = gcd(n, k)`, read off at once. Any other is read through the power sums of its roots: by
    /// Newton's identities the coefficients give `s₁ … s_{dk}`, the `k`-th powers have the power sums
    /// `s_k, s_{2k}, …, s_{dk}`, and the same identities give their coefficients back — `O(d²k)` field
    /// operations, no determinant.
    pub fn power_poly(p: &PolyOver<K>, k: u32) -> PolyOver<K> {
        let Some(d) = p.degree().filter(|d| *d >= 1) else {
            return p.clone();
        };
        if (1..d).all(|i| p.coeff(i).is_zero()) {
            let c = p.coeff(0).neg().mul(&p.coeff(d).recip().expect("a leading coefficient is nonzero"));
            let (mut a, mut b) = (d as u64, u64::from(k));
            while b != 0 {
                (a, b) = (b, a % b);
            }
            let (degree, exponent) = (d / a as usize, u64::from(k) / a);
            let power = (0..exponent).fold(K::one(), |acc, _| acc.mul(&c));
            let mut out = vec![K::zero(); degree + 1];
            out[0] = power.neg();
            out[degree] = K::one();
            return PolyOver::from(out);
        }
        let k = k as usize;
        let sums = p.power_sums(d * k);
        let powers: Vec<K> = (0..=d).map(|j| sums[j * k].clone()).collect();
        PolyOver::from_power_sums(&powers, d)
    }
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_base/ordered_field/tests.rs"]
mod tests;
