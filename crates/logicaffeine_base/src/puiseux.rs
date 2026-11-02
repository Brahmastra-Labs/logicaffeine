//! The real closure of the hyperreals: real algebraic Puiseux series in `dt`, exactly.
//!
//! The hyperreals a program builds with the field operations are the rational functions of `dt` over
//! the real algebraic numbers ([`Hyperreal`]); the real closure of that ordered field adds a root to
//! every polynomial that changes sign. Its elements are the real Puiseux series algebraic over it —
//! `√dt = dt^(1/2)`, `√(1 + dt) = 1 + dt/2 − dt²/8 + …` — and every one is held exactly, as a
//! [`Puiseux`]: a squarefree polynomial it is a root of, with coefficients rational functions of
//! `s = dt^(1/q)`, and the branch of that polynomial's real roots it is.
//!
//! **Branches.** The real roots of a polynomial over `K_s` are found by Newton's polygon: an edge of
//! the lower convex hull of the points `(i, v(p_i))` of slope `−γ` holds the roots of valuation `γ`,
//! and the real roots `c` of the edge's characteristic polynomial their leading coefficients. A
//! fractional `γ` refines `s` to a root of itself; a simple `c` separates its root from every other,
//! and a repeated one is followed into the polynomial `s^(-v)·P(s^γ·(c + z))` until its roots
//! separate. A separated branch has one root of positive valuation left, so each further term is
//! `−lc(p₀)/lc(p₁)·s^(v₀ − v₁)`, computed only as far as a comparison, a selection or a printed digit
//! asks.
//!
//! **Operations.** `α + β` is a root of `Res_z(P(z), Q(y − z))`, `α·β` of `Res_z(P(z), z^n·Q(y/z))`,
//! `α^k` of `Res_z(P(z), y − z^k)` and `ⁿ√α` of `P(y^n)`, each resultant interpolated in `y` at
//! integer points from resultants over `K_s`. The result is the one root whose expansion agrees with
//! the operation applied to the operands' expansions through the order at which that polynomial's
//! roots all differ — so every decision is exact, and two values are equal exactly when their
//! difference is the zero root of its polynomial.
//!
//! **One form for a value.** A value that is a rational function of a root of `dt` is held as that
//! function ([`Closed::Field`] when it is one of `dt` itself), found by Padé reconstruction from its
//! expansion with the degree bounds Gauss's lemma gives and checked exactly. It prints as that
//! function; a value with a power that is one prints as a radical, `√(1 + dt)`, `2·∛(1 + dt)`; any
//! other prints by its two leading terms and the order of the rest, `2√2 - (√2/16)·dt^2 + O(dt^3)`.

use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use crate::algebraic::{negative_root, Algebraic};
use crate::hyperreal::{Hyperreal, PowerError};
use crate::numeric::Rational;

fn zero() -> Hyperreal {
    Hyperreal::from_i64(0)
}

fn constant(c: &Algebraic) -> Hyperreal {
    Hyperreal::from_algebraic(c.clone())
}

fn power_of_s(k: i64) -> Hyperreal {
    Hyperreal::monomial(Algebraic::from_i64(1), k)
}

fn gcd_i64(a: i64, b: i64) -> i64 {
    if b == 0 {
        a.abs()
    } else {
        gcd_i64(b, a % b)
    }
}

fn lcm_u32(a: u32, b: u32) -> u32 {
    a / gcd_i64(i64::from(a), i64::from(b)) as u32 * b
}

// ── polynomials over K_s ─────────────────────────────────────────────────────────────────────

/// A polynomial in `y` whose coefficients are rational functions of `s`, lowest power first, with
/// no trailing zero.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct KPoly {
    c: Vec<Hyperreal>,
}

impl KPoly {
    fn from(c: Vec<Hyperreal>) -> KPoly {
        let mut c = c;
        while c.last().is_some_and(Hyperreal::is_zero) {
            c.pop();
        }
        KPoly { c }
    }

    fn is_zero(&self) -> bool {
        self.c.is_empty()
    }

    fn degree(&self) -> Option<usize> {
        self.c.len().checked_sub(1)
    }

    fn coeff(&self, i: usize) -> Hyperreal {
        self.c.get(i).cloned().unwrap_or_else(zero)
    }

    /// `y − k`.
    fn linear(k: &Hyperreal) -> KPoly {
        KPoly::from(vec![k.neg(), Hyperreal::from_i64(1)])
    }

    fn add(&self, o: &KPoly) -> KPoly {
        let n = self.c.len().max(o.c.len());
        KPoly::from((0..n).map(|i| self.coeff(i).add(&o.coeff(i))).collect())
    }

    fn mul(&self, o: &KPoly) -> KPoly {
        if self.is_zero() || o.is_zero() {
            return KPoly { c: Vec::new() };
        }
        let mut out = vec![zero(); self.c.len() + o.c.len() - 1];
        for (i, a) in self.c.iter().enumerate() {
            if a.is_zero() {
                continue;
            }
            for (j, b) in o.c.iter().enumerate() {
                if !b.is_zero() {
                    out[i + j] = out[i + j].add(&a.mul(b));
                }
            }
        }
        KPoly::from(out)
    }

    fn scale(&self, k: &Hyperreal) -> KPoly {
        KPoly::from(self.c.iter().map(|a| a.mul(k)).collect())
    }

    fn monic(&self) -> KPoly {
        match self.c.last() {
            None => self.clone(),
            Some(lc) => self.scale(&lc.recip().expect("a leading coefficient is nonzero")),
        }
    }

    fn div_rem(&self, d: &KPoly) -> (KPoly, KPoly) {
        let dd = d.degree().expect("the divisor is nonzero");
        let inv = d.c[dd].recip().expect("a leading coefficient is nonzero");
        if self.c.len() <= dd {
            return (KPoly { c: Vec::new() }, self.clone());
        }
        let mut r = self.c.clone();
        let mut q = vec![zero(); r.len() - dd];
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
        (KPoly::from(q), KPoly::from(r))
    }

    fn gcd(&self, o: &KPoly) -> KPoly {
        let (mut a, mut b) = (self.monic(), o.monic());
        while !b.is_zero() {
            let r = a.div_rem(&b).1;
            a = b;
            b = r.monic();
        }
        a
    }

    fn derivative(&self) -> KPoly {
        KPoly::from(
            self.c
                .iter()
                .enumerate()
                .skip(1)
                .map(|(i, a)| a.mul(&Hyperreal::from_i64(i as i64)))
                .collect(),
        )
    }

    /// The polynomial with each repeated root once, monic.
    fn squarefree(&self) -> KPoly {
        let g = self.gcd(&self.derivative());
        if g.degree().unwrap_or(0) == 0 {
            return self.monic();
        }
        self.div_rem(&g).0.monic()
    }

    fn eval(&self, y: &Hyperreal) -> Hyperreal {
        self.c.iter().rev().fold(zero(), |acc, a| acc.mul(y).add(a))
    }

    /// `P(y + a)`.
    fn shift(&self, a: &Hyperreal) -> KPoly {
        let mut out = KPoly { c: Vec::new() };
        let step = KPoly::from(vec![a.clone(), Hyperreal::from_i64(1)]);
        for c in self.c.iter().rev() {
            out = out.mul(&step).add(&KPoly::from(vec![c.clone()]));
        }
        out
    }

    /// `P(k·y)`.
    fn stretch(&self, k: &Hyperreal) -> KPoly {
        let mut power = Hyperreal::from_i64(1);
        let mut out = Vec::with_capacity(self.c.len());
        for c in &self.c {
            out.push(c.mul(&power));
            power = power.mul(k);
        }
        KPoly::from(out)
    }

    /// `P(−y)`.
    fn neg_var(&self) -> KPoly {
        KPoly::from(
            self.c
                .iter()
                .enumerate()
                .map(|(i, c)| if i % 2 == 1 { c.neg() } else { c.clone() })
                .collect(),
        )
    }

    /// `P(y^n)`.
    fn spread(&self, n: u32) -> KPoly {
        let mut out = vec![zero(); self.c.len().saturating_sub(1) * n as usize + 1];
        for (i, c) in self.c.iter().enumerate() {
            out[i * n as usize] = c.clone();
        }
        KPoly::from(out)
    }

    /// The same polynomial over `K_{s'}`, `s = s'^m`.
    fn ramify(&self, m: u32) -> KPoly {
        KPoly {
            c: self.c.iter().map(|c| c.ramify(m)).collect(),
        }
    }

    /// The polynomial with its root `0` divided out.
    fn without_zero_root(&self) -> KPoly {
        let k = self.c.iter().position(|c| !c.is_zero()).unwrap_or(0);
        KPoly::from(self.c[k..].to_vec())
    }

    /// The resultant `lc^{deg o}·Π o(α)` over the roots `α` of `self`, by Euclid's algorithm over
    /// the field `K_s`.
    fn resultant(&self, o: &KPoly) -> Hyperreal {
        let (Some(m), Some(n)) = (self.degree(), o.degree()) else {
            return zero();
        };
        let power = |h: &Hyperreal, k: usize| h.pow(k as i32).expect("a resultant's power is small");
        if n == 0 {
            return power(&o.c[0], m);
        }
        if m == 0 {
            return power(&self.c[0], n);
        }
        let r = self.div_rem(o).1;
        let Some(dr) = r.degree() else {
            return zero();
        };
        let v = power(&o.c[n], m - dr).mul(&o.resultant(&r));
        if (m * n) % 2 == 1 {
            v.neg()
        } else {
            v
        }
    }

    /// The polynomial in `y` of degree at most `degree` whose value at each integer `j ≤ degree` is
    /// `at(j)`, by Newton's divided differences over `K_s`.
    fn interpolate(degree: usize, at: impl Fn(i64) -> Hyperreal) -> KPoly {
        let nodes: Vec<i64> = (0..=degree as i64).collect();
        let mut values: Vec<Hyperreal> = nodes.iter().map(|j| at(*j)).collect();
        let n = nodes.len();
        for j in 1..n {
            for i in (j..n).rev() {
                let gap = Hyperreal::from_i64(nodes[i] - nodes[i - j]);
                values[i] = values[i].sub(&values[i - 1]).div(&gap).expect("the nodes are distinct");
            }
        }
        let mut out = KPoly::from(vec![values[n - 1].clone()]);
        for i in (0..n - 1).rev() {
            let factor = KPoly::from(vec![Hyperreal::from_i64(-nodes[i]), Hyperreal::from_i64(1)]);
            out = out.mul(&factor).add(&KPoly::from(vec![values[i].clone()]));
        }
        out
    }

    /// `Res_z(P(z), Q(y − z))`: its roots are the sums of a root of each.
    fn sum_poly(p: &KPoly, q: &KPoly) -> KPoly {
        let degree = p.degree().unwrap_or(0) * q.degree().unwrap_or(0);
        let reflected = q.neg_var();
        KPoly::interpolate(degree, |j| p.resultant(&reflected.shift(&Hyperreal::from_i64(-j))))
    }

    /// `Res_z(P(z), z^n·Q(y/z))` for `P` and `Q` without the root `0`: its roots are the products of
    /// a root of each.
    fn product_poly(p: &KPoly, q: &KPoly) -> KPoly {
        let n = q.degree().unwrap_or(0);
        let degree = p.degree().unwrap_or(0) * n;
        KPoly::interpolate(degree, |j| {
            let j = Hyperreal::from_i64(j);
            let mut c = vec![zero(); n + 1];
            let mut power = Hyperreal::from_i64(1);
            for i in 0..=n {
                c[n - i] = q.coeff(i).mul(&power);
                power = power.mul(&j);
            }
            p.resultant(&KPoly::from(c))
        })
    }

    /// `Res_z(P(z), y − z^k)`: its roots are the `k`-th powers of the roots of `P`.
    fn power_poly(p: &KPoly, k: u32) -> KPoly {
        let degree = p.degree().unwrap_or(0);
        KPoly::interpolate(degree, |j| {
            let mut c = vec![zero(); k as usize + 1];
            c[0] = Hyperreal::from_i64(j);
            c[k as usize] = Hyperreal::from_i64(-1);
            p.resultant(&KPoly::from(c))
        })
    }
}

// ── branches: the real roots by Newton's polygon ────────────────────────────────────────────

/// One real root of a polynomial over `K_s`: `y = Σ terms + s^at·z`, where `z` is the one root of
/// positive valuation of `rest` the branch follows — or `y` is exactly the sum of its terms.
#[derive(Clone, Debug)]
struct Branch {
    /// `s^q = dt` for the branch's own `s`.
    q: u32,
    /// `(exponent in s, coefficient)`, increasing exponents, every coefficient nonzero.
    terms: Vec<(i64, Algebraic)>,
    at: i64,
    rest: KPoly,
    /// Whether the root is exactly the sum of its terms.
    exact: bool,
}

impl Branch {
    /// The same branch over `s'`, `s = s'^m`.
    fn ramified(&self, m: u32) -> Branch {
        Branch {
            q: self.q * m,
            terms: self.terms.iter().map(|(e, c)| (e * i64::from(m), c.clone())).collect(),
            at: self.at * i64::from(m),
            rest: self.rest.ramify(m),
            exact: self.exact,
        }
    }

    /// The exponent of a term in `dt`'s own units.
    fn in_dt(&self, e: i64) -> Rational {
        Rational::from_ratio_i64(e, i64::from(self.q)).expect("q is positive")
    }

    /// One more term of a separated branch.
    fn extend(&mut self) {
        if self.exact {
            return;
        }
        let (c0, c1) = (self.rest.coeff(0), self.rest.coeff(1));
        if c0.is_zero() {
            self.exact = true;
            return;
        }
        let v0 = c0.valuation().expect("nonzero");
        let v1 = c1.valuation().expect("a separated branch's linear coefficient is nonzero");
        let gamma = v0 - v1;
        assert!(gamma > 0, "a separated branch's next term lies past its last");
        let lead = c0
            .leading_coefficient()
            .expect("nonzero")
            .neg()
            .div(&c1.leading_coefficient().expect("nonzero"))
            .expect("nonzero");
        self.rest = descend(&self.rest, gamma, &lead);
        self.at += gamma;
        self.terms.push((self.at, lead));
    }

    /// Every term of exponent at most `e`, in `dt`'s units.
    fn through(&mut self, e: &Rational) -> Series {
        while !self.exact && self.in_dt(self.at) < *e {
            self.extend();
        }
        self.terms
            .iter()
            .map(|(k, c)| (self.in_dt(*k), c.clone()))
            .filter(|(k, _)| k <= e)
            .collect()
    }

    /// The exponent of the last term found, in `dt`'s units; `None` for the zero root.
    fn last(&self) -> Option<Rational> {
        self.terms.last().map(|(k, _)| self.in_dt(*k))
    }

    /// The coefficient of `s^k`: every term through `s^at` is known, the rest of the root lying past it.
    fn coefficient(&mut self, k: i64) -> Algebraic {
        while !self.exact && self.at < k {
            self.extend();
        }
        self.terms
            .iter()
            .find(|(e, _)| *e == k)
            .map_or_else(|| Algebraic::from_i64(0), |(_, c)| c.clone())
    }
}

/// `s^(-v)·P(s^γ·(c + z))`, `v` the least valuation of the coefficients of `P(s^γ·y)`.
fn descend(p: &KPoly, gamma: i64, c: &Algebraic) -> KPoly {
    let stretched = p.stretch(&power_of_s(gamma));
    let v = stretched
        .c
        .iter()
        .filter_map(Hyperreal::valuation)
        .min()
        .expect("a nonzero polynomial");
    stretched.shift(&constant(c)).scale(&power_of_s(-v))
}

/// The lower convex hull of points of increasing first coordinate.
fn lower_hull(points: &[(i64, i64)]) -> Vec<(i64, i64)> {
    let mut hull: Vec<(i64, i64)> = Vec::new();
    for &p in points {
        while hull.len() >= 2 {
            let (o, a) = (hull[hull.len() - 2], hull[hull.len() - 1]);
            let cross = (a.0 - o.0) * (p.1 - o.1) - (a.1 - o.1) * (p.0 - o.0);
            if cross <= 0 {
                hull.pop();
            } else {
                break;
            }
        }
        hull.push(p);
    }
    hull
}

/// The real roots of the squarefree `p` over `K_s`, `s^q = dt`, each followed until it separates.
fn branches(p: &KPoly, q: u32) -> Vec<Branch> {
    let root = Branch {
        q,
        terms: Vec::new(),
        at: 0,
        rest: p.clone(),
        exact: false,
    };
    split(root, true)
}

/// The roots `z` of `b.rest` — of any valuation at the `top`, of positive valuation below it — each
/// as a branch of `b`.
fn split(b: Branch, top: bool) -> Vec<Branch> {
    let mut out = Vec::new();
    let mut rest = b.rest.clone();
    if rest.coeff(0).is_zero() {
        out.push(Branch {
            exact: true,
            ..b.clone()
        });
        rest = rest.without_zero_root();
    }
    let points: Vec<(i64, i64)> = rest
        .c
        .iter()
        .enumerate()
        .filter_map(|(i, c)| c.valuation().map(|v| (i as i64, v)))
        .collect();
    for edge in lower_hull(&points).windows(2) {
        let ((i0, v0), (i1, v1)) = (edge[0], edge[1]);
        let (num, den) = (v0 - v1, i1 - i0);
        if !top && num <= 0 {
            continue;
        }
        let g = gcd_i64(num, den).max(1);
        let (a, d) = (num / g, den / g);
        let phi: Vec<Algebraic> = (i0..=i1)
            .map(|i| match points.iter().find(|(j, _)| *j == i) {
                Some(&(_, v)) if (v - v0) * den == -num * (i - i0) => {
                    rest.coeff(i as usize).leading_coefficient().expect("nonzero")
                }
                _ => Algebraic::from_i64(0),
            })
            .collect();
        for (c, multiplicity) in Algebraic::real_roots(&phi) {
            if c.is_zero() {
                continue;
            }
            let mut child = Branch {
                rest: rest.clone(),
                exact: false,
                ..b.clone()
            };
            if d > 1 {
                child = child.ramified(d as u32);
            }
            child.rest = descend(&child.rest, a, &c);
            child.at += a;
            child.terms.push((child.at, c));
            if multiplicity == 1 {
                out.push(child);
            } else {
                out.extend(split(child, false));
            }
        }
    }
    out
}

// ── series with rational exponents ───────────────────────────────────────────────────────────

/// A truncated Puiseux series: `(exponent in dt, coefficient)`, increasing exponents, every
/// coefficient nonzero.
type Series = Vec<(Rational, Algebraic)>;

fn series_add(a: &Series, b: &Series) -> Series {
    let mut out: Series = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < a.len() || j < b.len() {
        let take_a = j >= b.len() || (i < a.len() && a[i].0 < b[j].0);
        let take_b = i >= a.len() || (j < b.len() && b[j].0 < a[i].0);
        if take_a {
            out.push(a[i].clone());
            i += 1;
        } else if take_b {
            out.push(b[j].clone());
            j += 1;
        } else {
            let c = a[i].1.add(&b[j].1);
            if !c.is_zero() {
                out.push((a[i].0.clone(), c));
            }
            i += 1;
            j += 1;
        }
    }
    out
}

fn series_neg(a: &Series) -> Series {
    a.iter().map(|(e, c)| (e.clone(), c.neg())).collect()
}

/// The product's terms of exponent at most `e`.
fn series_mul(a: &Series, b: &Series, e: &Rational) -> Series {
    let mut out: Series = Vec::new();
    for (ea, ca) in a {
        for (eb, cb) in b {
            let k = ea.add(eb);
            if k > *e {
                continue;
            }
            out = series_add(&out, &vec![(k, ca.mul(cb))]);
        }
    }
    out
}

/// `Σ_k w_k·u^k` through exponent `e`, for `u` of positive exponents and `w_k` the weights `weight`
/// gives in turn.
fn series_compose(u: &Series, e: &Rational, mut weight: impl FnMut(usize) -> Algebraic) -> Series {
    let mut out: Series = Vec::new();
    let mut power: Series = vec![(Rational::zero(), Algebraic::from_i64(1))];
    let mut k = 0;
    while !power.is_empty() {
        let w = weight(k);
        if !w.is_zero() {
            out = series_add(&out, &power.iter().map(|(x, c)| (x.clone(), c.mul(&w))).collect());
        }
        power = series_mul(&power, u, e);
        k += 1;
    }
    out
}

/// `a = c·dt^v·(1 + u)`: the leading term and `u`, every term `u` needs for `a` through exponent `e`.
fn series_split(a: &Series) -> (Rational, Algebraic, Series) {
    let (v, c) = a[0].clone();
    let inv = c.recip().expect("a leading coefficient is nonzero");
    let u = a[1..].iter().map(|(x, k)| (x.sub(&v), k.mul(&inv))).collect();
    (v, c, u)
}

fn shift_series(a: &Series, by: &Rational, scale: &Algebraic) -> Series {
    a.iter().map(|(x, c)| (x.add(by), c.mul(scale))).collect()
}

/// `1/a` through exponent `e`, `a` nonzero and known through `e + 2·v(a)`.
fn series_recip(a: &Series, e: &Rational) -> Series {
    let (v, c, u) = series_split(a);
    let reach = e.add(&v);
    let minus_u = series_neg(&u);
    let body = series_compose(&minus_u, &reach, |_| Algebraic::from_i64(1));
    shift_series(&body, &v.negated(), &c.recip().expect("nonzero"))
}

/// The real `n`-th root of `a` through exponent `e` — the positive one for even `n` — or the reason
/// there is none.
fn series_root(a: &Series, n: u32, e: &Rational) -> Result<Series, String> {
    let (v, c, u) = series_split(a);
    let lead = c.nth_root(n).ok_or_else(|| negative_root(n))?;
    let order = v.div(&Rational::from_i64(i64::from(n))).expect("n is positive");
    let reach = e.sub(&order);
    let alpha = Rational::from_ratio_i64(1, i64::from(n)).expect("n is positive");
    let mut binomial = Rational::one();
    let body = series_compose(&u, &reach, |k| {
        if k > 0 {
            let k = Rational::from_i64(k as i64);
            binomial = binomial.mul(&alpha.sub(&k.sub(&Rational::one()))).div(&k).expect("k ≥ 1");
        }
        Algebraic::from_rational(binomial.clone())
    });
    Ok(shift_series(&body, &order, &lead))
}

// ── the values ───────────────────────────────────────────────────────────────────────────────

/// A real algebraic Puiseux series in `dt`: a root of `poly` (squarefree, over `K_s`, `s^q = dt`),
/// picked by its branch. See the module documentation.
#[derive(Clone)]
pub struct Puiseux {
    poly: KPoly,
    q: u32,
    /// The rational function of `s` the value is, when `poly` has degree one.
    function: Option<Hyperreal>,
    /// The branch the value follows. A rational function `h` of a root of `dt` is the one root of
    /// `y − h` and answers its terms from `h`, so its branch is followed only when an operation with a
    /// value that is no such function asks for it.
    picked: Arc<OnceLock<Picked>>,
}

/// A branch and the order at which every real root of its polynomial differs from every other, in
/// `dt`'s units.
struct Picked {
    branch: Mutex<Branch>,
    separation: Rational,
}

/// Why a power of a value of the real closure is refused: the hyperreal's own refusal, its degree
/// counted in the root `dt^(1/q)` the value is a rational function of.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PowerRefusal {
    pub error: PowerError,
    pub q: u32,
}

impl PowerRefusal {
    /// The refusal, the infinitesimal written `name`: `the power would have degree 5000 in √dt; …`.
    pub fn describe(&self, name: &str) -> String {
        self.error.describe(&power_text(name, &ratio(1, self.q)))
    }
}

/// A value of the real closure in its one form: a hyperreal when it is a rational function of `dt`,
/// otherwise a Puiseux series that is no such function.
#[derive(Clone, Debug)]
pub enum Closed {
    Field(Hyperreal),
    Series(Puiseux),
}

/// How far a polynomial's branches must be followed to tell them all apart.
fn separation_of(bs: &[Branch]) -> Rational {
    bs.iter().filter_map(Branch::last).max().unwrap_or_else(Rational::zero)
}

impl Puiseux {
    /// The root of `poly` over `K_s` whose expansion agrees with `target` through the order that
    /// tells `poly`'s real roots apart.
    fn select(poly: KPoly, q: u32, target: &dyn Fn(&Rational) -> Series) -> Puiseux {
        let poly = poly.squarefree();
        let bs = branches(&poly, q);
        let separation = separation_of(&bs);
        let want = target(&separation);
        let chosen = bs
            .into_iter()
            .find_map(|mut b| (b.through(&separation) == want).then_some(b))
            .expect("the operation's value is a real root of its polynomial");
        Puiseux::picked_as(poly, q, chosen, separation)
    }

    fn picked_as(poly: KPoly, q: u32, branch: Branch, separation: Rational) -> Puiseux {
        let function = (poly.degree() == Some(1))
            .then(|| poly.coeff(0).neg().div(&poly.coeff(1)).expect("degree one"));
        Puiseux {
            poly,
            q,
            function,
            picked: Arc::new(OnceLock::from(Picked {
                branch: Mutex::new(branch),
                separation,
            })),
        }
    }

    /// The hyperreal `h` (a rational function of `s`, `s^q = dt`) as the root of `y − h`.
    fn of_field(h: &Hyperreal, q: u32) -> Puiseux {
        Puiseux {
            poly: KPoly::linear(h),
            q,
            function: Some(h.clone()),
            picked: Arc::new(OnceLock::new()),
        }
    }

    /// The branch, followed now when the value was made as a rational function of a root of `dt`.
    fn picked(&self) -> &Picked {
        self.picked.get_or_init(|| {
            assert_eq!(self.poly.degree(), Some(1), "only the root of `y − h` is picked when first asked for");
            let bs = branches(&self.poly, self.q);
            let separation = separation_of(&bs);
            let branch = bs.into_iter().next().expect("y − h has its root");
            Picked {
                branch: Mutex::new(branch),
                separation,
            }
        })
    }

    fn branch(&self) -> MutexGuard<'_, Branch> {
        self.picked().branch.lock().expect("unpoisoned")
    }

    /// The order at which every real root of `poly` differs from every other: for the root of
    /// `y − h`, the order of its leading term.
    fn separation(&self) -> Rational {
        match self.function {
            Some(_) => self.leading().map_or_else(Rational::zero, |(v, _)| v),
            None => self.picked().separation.clone(),
        }
    }

    /// Every term of exponent at most `e`, in `dt`'s units.
    fn through(&self, e: &Rational) -> Series {
        let (Some(h), q) = (&self.function, self.q) else {
            return self.branch().through(e);
        };
        let Some(v) = h.valuation() else {
            return Vec::new();
        };
        let top = e.mul(&Rational::from_i64(i64::from(q))).floor().to_i64().expect("a small order");
        (v..=top)
            .filter_map(|k| {
                let c = h.laurent_coefficient(k);
                (!c.is_zero()).then(|| (Rational::from_ratio_i64(k, i64::from(q)).expect("q is positive"), c))
            })
            .collect()
    }

    /// The `q` whose root of `dt` the expansion is a series in: every exponent is a multiple of `1/q`.
    /// The root of `y − h` follows `h`'s own root of `dt`; a fractional slope of Newton's polygon
    /// refines it only for a polynomial of higher degree.
    pub fn ramification(&self) -> u32 {
        match self.function {
            Some(_) => self.q,
            None => self.branch().q,
        }
    }

    /// The coefficient of `dt^(k/q)`, `q` the [`Puiseux::ramification`].
    pub fn coefficient(&self, k: i64) -> Algebraic {
        match &self.function {
            Some(h) => h.laurent_coefficient(k),
            None => self.branch().coefficient(k),
        }
    }

    /// The polynomial the value is a root of, its coefficients rational functions of `dt^(1/q)`, with
    /// that `q`.
    pub(crate) fn annihilator(&self) -> (Vec<Hyperreal>, u32) {
        (self.poly.c.clone(), self.q)
    }

    /// The rational function of `dt^(1/q)` the value is, with that `q`, when it is one: a value in
    /// its one form ([`Closed`]) that is one is held as the root of `y − h`.
    pub fn rational_function(&self) -> Option<(Hyperreal, u32)> {
        self.function.clone().map(|h| (h, self.q))
    }

    /// The exponent of the leading term in units of `dt^(1/q)`, `q` the [`Puiseux::ramification`].
    pub fn order(&self) -> i64 {
        match &self.function {
            Some(h) => h.valuation().expect("a series value is nonzero"),
            None => self.branch().terms.first().expect("a series value is nonzero").0,
        }
    }

    /// The first term; `None` for zero.
    fn leading(&self) -> Option<(Rational, Algebraic)> {
        if let Some(h) = &self.function {
            let v = h.valuation()?;
            let c = h.leading_coefficient()?;
            return Some((Rational::from_ratio_i64(v, i64::from(self.q)).expect("q is positive"), c));
        }
        let b = self.branch();
        b.terms.first().map(|(k, c)| (b.in_dt(*k), c.clone()))
    }

    fn signum(&self) -> i32 {
        self.leading().map_or(0, |(_, c)| c.signum())
    }

    /// The polynomial over `K_{s'}`, `s' = dt^(1/q')` for `q'` a multiple of `q`.
    fn poly_over(&self, q: u32) -> KPoly {
        self.poly.ramify(q / self.q)
    }

    fn plus(&self, o: &Puiseux) -> Puiseux {
        let q = lcm_u32(self.q, o.q);
        let poly = KPoly::sum_poly(&self.poly_over(q), &o.poly_over(q));
        Puiseux::select(poly, q, &|e| series_add(&self.through(e), &o.through(e)))
    }

    /// `−α` is a root of `P(−y)` whose branch is `α`'s with every sign turned.
    fn negated(&self) -> Puiseux {
        if let Some(h) = &self.function {
            return Puiseux::of_field(&h.neg(), self.q);
        }
        let b = self.branch().clone();
        let branch = Branch {
            terms: b.terms.iter().map(|(k, c)| (*k, c.neg())).collect(),
            rest: b.rest.neg_var(),
            ..b
        };
        Puiseux::picked_as(self.poly.neg_var(), self.q, branch, self.separation())
    }

    /// The product of two nonzero values.
    fn times(&self, o: &Puiseux) -> Puiseux {
        let q = lcm_u32(self.q, o.q);
        let (va, _) = self.leading().expect("nonzero");
        let (vb, _) = o.leading().expect("nonzero");
        let poly = KPoly::product_poly(&self.poly_over(q).without_zero_root(), &o.poly_over(q).without_zero_root());
        Puiseux::select(poly, q, &|e| series_mul(&self.through(&e.sub(&vb)), &o.through(&e.sub(&va)), e))
    }

    /// The reciprocal of a nonzero value: a root of `P` reversed.
    fn inverse(&self) -> Puiseux {
        let (v, _) = self.leading().expect("nonzero");
        let mut reversed = self.poly.without_zero_root().c;
        reversed.reverse();
        let target = |e: &Rational| series_recip(&self.through(&e.add(&v).add(&v)), e);
        Puiseux::select(KPoly::from(reversed), self.q, &target)
    }

    /// `α^k` for a nonzero `α` and `k ≥ 1`. Each partial product is cut at the order the factors still
    /// to come could lower it to, `v` apiece, so a value of negative valuation loses no term it needs.
    fn power(&self, k: u32) -> Puiseux {
        let (v, _) = self.leading().expect("nonzero");
        let target = |e: &Rational| {
            let base = self.through(&e.sub(&v.mul(&Rational::from_i64(i64::from(k) - 1))));
            let mut acc: Series = vec![(Rational::zero(), Algebraic::from_i64(1))];
            for done in 1..=k {
                let to_come = Rational::from_i64(i64::from(k - done));
                acc = series_mul(&acc, &base, &e.sub(&v.mul(&to_come)));
            }
            acc
        };
        Puiseux::select(KPoly::power_poly(&self.poly, k), self.q, &target)
    }

    /// The real `n`-th root of a nonzero `α` — the positive one for even `n` — or the reason there
    /// is none.
    fn nth_root(&self, n: u32) -> Result<Puiseux, String> {
        let (v, c) = self.leading().expect("nonzero");
        if n % 2 == 0 && c.signum() < 0 {
            return Err(negative_root(n));
        }
        let order = v.div(&Rational::from_i64(i64::from(n))).expect("n is positive");
        let target = |e: &Rational| {
            series_root(&self.through(&e.sub(&order).add(&v)), n, e).expect("the sign was checked")
        };
        Ok(Puiseux::select(self.poly.spread(n), self.q, &target))
    }

    /// The value as the rational function of a root of `dt` it is, when it is one, with that root's
    /// `q`. A branch that ends is the sum of its terms; otherwise the candidate is the Padé
    /// approximant of the expansion in the branch's own root of `dt`, within the degrees Gauss's
    /// lemma bounds a root of the monic `poly` by — its denominator dividing the least common multiple
    /// `L` of the coefficients' denominators, its numerator `L·p₀` — kept only when it is a root of
    /// `poly` that agrees with the value through `separation`.
    fn field_value(&self) -> Option<(Hyperreal, u32)> {
        if let Some(h) = &self.function {
            return Some((h.clone(), self.q));
        }
        let (qb, exact, terms) = {
            let b = self.branch();
            (b.q, b.exact, b.terms.clone())
        };
        if exact {
            let sum = terms.iter().fold(zero(), |acc, (k, c)| acc.add(&Hyperreal::monomial(c.clone(), *k)));
            return Some((sum, qb));
        }
        let poly = self.poly_over(qb).monic();
        let degrees: Vec<(usize, usize)> = poly.c.iter().map(Hyperreal::degrees).collect();
        let lcm_bound: usize = degrees.iter().map(|(_, d)| d).sum();
        let (n0, d0) = degrees[0];
        let num_bound = (lcm_bound + n0).checked_sub(d0)?;
        let q = Rational::from_i64(i64::from(qb));
        let (v, _) = self.leading()?;
        let v_s = v.mul(&q);
        let v_s = v_s.to_bigint().filter(|_| v_s.is_integer())?.to_i64()?;
        let (l, m) = if v_s >= 0 {
            (num_bound.checked_sub(usize::try_from(v_s).ok()?)?, lcm_bound)
        } else {
            (num_bound, lcm_bound.checked_sub(usize::try_from(-v_s).ok()?)?)
        };
        let reach = Rational::from_i64(v_s + (l + m) as i64).div(&q).expect("q is positive");
        let mut series = vec![Algebraic::from_i64(0); l + m + 1];
        for (e, c) in self.through(&reach) {
            let e_s = e.mul(&q);
            if !e_s.is_integer() {
                return None;
            }
            let k = e_s.to_bigint()?.to_i64()? - v_s;
            series[usize::try_from(k).ok()?] = c;
        }
        let candidate = Hyperreal::pade(&series, l, m)?.mul(&power_of_s(v_s));
        if !poly.eval(&candidate).is_zero() {
            return None;
        }
        let check = Puiseux::of_field(&candidate, qb);
        let separation = self.separation();
        (check.through(&separation) == self.through(&separation)).then_some((candidate, qb))
    }

    /// The value in its one form.
    pub(crate) fn canonical(self) -> Closed {
        match self.field_value() {
            Some((h, q)) => Closed::of_field(h, q),
            None => Closed::Series(self),
        }
    }

    /// The rational function of `s` with `s`'s powers written in `dt`, a radical, or the leading
    /// terms. See the module documentation.
    fn render(&self, name: &str) -> String {
        let q = self.q;
        if let Some(h) = &self.function {
            return h.render_powers(&|k| power_text(name, &ratio(k, q)));
        }
        if let Some(text) = self.radical(name) {
            return text;
        }
        let mut b = self.branch();
        while b.terms.len() < 3 && !b.exact {
            b.extend();
        }
        let terms: Vec<(Rational, Algebraic)> = b.terms.iter().map(|(k, c)| (b.in_dt(*k), c.clone())).collect();
        drop(b);
        let mut out = String::new();
        for (i, (e, c)) in terms.iter().take(2).enumerate() {
            let t = term_text(name, c, e);
            match (i, t.strip_prefix('-')) {
                (0, _) => out.push_str(&t),
                (_, Some(rest)) => {
                    out.push_str(" - ");
                    out.push_str(rest);
                }
                (_, None) => {
                    out.push_str(" + ");
                    out.push_str(&t);
                }
            }
        }
        if let Some((next, _)) = terms.get(2) {
            out.push_str(&format!(" + {}", remainder(name, &remainder_order(&terms[..2], next))));
        }
        out
    }

    /// `F·ⁿ√κ'` for the least `n` whose power of the value is a rational function `κ` of a root of
    /// `dt`: the `n`-th powers of `κ`'s polynomial factors and a constant that is an `n`-th power taken
    /// out into `F` ([`Hyperreal::root_split`]). A value whose power has a constant numerator prints as
    /// one over the radical of the reciprocal: `1/∛(3 + dt)`.
    fn radical(&self, name: &str) -> Option<String> {
        let degree = self.poly.degree().unwrap_or(0) as u32;
        let negative = self.signum() < 0;
        for n in 2..=degree.max(2) {
            let Some((kappa, q)) = self.power(n).field_value() else {
                continue;
            };
            let powers = |k: usize| power_text(name, &ratio(k, q));
            let (kappa, inverted) = if kappa.numerator_is_constant() && kappa.degrees().1 > 0 {
                (kappa.recip()?, true)
            } else {
                (kappa, false)
            };
            let (outside, inside) = kappa.root_split(n)?;
            let outside = if n % 2 == 0 && negative { outside.neg() } else { outside };
            let radicand = inside.render_powers(&powers);
            let radicand = if radicand.contains([' ', '/', '·', '^']) {
                format!("({radicand})")
            } else {
                radicand
            };
            let sign = radical_sign(n);
            let factor = outside.render_powers(&powers);
            let (minus, factor) = match factor.strip_prefix('-') {
                Some(rest) if !rest.contains(' ') => ("-", rest.to_string()),
                _ => ("", factor),
            };
            let factor = if factor.contains([' ', '/']) { format!("({factor})") } else { factor };
            let root = if factor == "1" {
                format!("{sign}{radicand}")
            } else {
                format!("{factor}·{sign}{radicand}")
            };
            return Some(if inverted {
                let root = if root.contains('·') { format!("({root})") } else { root };
                format!("{minus}1/{root}")
            } else {
                format!("{minus}{root}")
            });
        }
        None
    }
}

/// The order the rest of an expansion is written `O(·)` of, after the terms `shown`: the order after
/// the last one on the grid their exponents lie on, as a germ's rest is written — or the next term's
/// own order when that comes sooner, the expansion running on a finer grid. Both are the value's own,
/// so the bound reads the same whatever route reached the value, and it always holds.
fn remainder_order(shown: &[(Rational, Algebraic)], next: &Rational) -> Rational {
    let exponents: Vec<Rational> = shown.iter().map(|(e, _)| e.clone()).collect();
    remainder_after(&exponents, next)
}

/// [`remainder_order`] for the exponents of the terms shown, whatever their coefficients.
pub(crate) fn remainder_after(shown: &[Rational], next: &Rational) -> Rational {
    let grid = shown
        .iter()
        .map(|e| e.denominator().to_i64().expect("a small denominator"))
        .fold(1i64, |l, d| l / gcd_i64(l, d) * d);
    let after = shown
        .last()
        .map(|e| e.add(&Rational::from_ratio_i64(1, grid).expect("grid > 0")))
        .expect("terms are shown");
    if *next < after {
        next.clone()
    } else {
        after
    }
}

/// `k/q` as a rational.
fn ratio(k: usize, q: u32) -> Rational {
    Rational::from_ratio_i64(k as i64, i64::from(q)).expect("q is positive")
}

impl Closed {
    /// A hyperreal of the rational functions of `dt`.
    pub fn from_hyperreal(h: Hyperreal) -> Closed {
        Closed::Field(h)
    }

    /// The rational function `h` of `s = dt^(1/q)` in its one form: of the least root of `dt` it is
    /// a function of, and a hyperreal when that is `dt` itself.
    pub(crate) fn of_field(h: Hyperreal, q: u32) -> Closed {
        let mut h = h;
        let mut q = q;
        for p in 2..=q {
            while q % p == 0 {
                match h.unramify(p) {
                    Some(r) => {
                        h = r;
                        q /= p;
                    }
                    None => break,
                }
            }
        }
        if q == 1 {
            Closed::Field(h)
        } else {
            Closed::Series(Puiseux::of_field(&h, q))
        }
    }

    /// The standard number the value is, when it has no infinitesimal part.
    pub fn as_standard(&self) -> Option<Algebraic> {
        match self {
            Closed::Field(h) => h.as_standard(),
            Closed::Series(_) => None,
        }
    }

    /// The exponent of the leading term in `dt` and its coefficient; `None` for zero.
    pub fn leading(&self) -> Option<(Rational, Algebraic)> {
        match self {
            Closed::Field(h) => Some((Rational::from_i64(h.valuation()?), h.leading_coefficient()?)),
            Closed::Series(p) => p.leading(),
        }
    }

    /// The value as the rational function of `dt^(1/q)` it is, with that `q`, when it is one.
    pub(crate) fn field(&self) -> Option<(Hyperreal, u32)> {
        match self {
            Closed::Field(h) => Some((h.clone(), 1)),
            Closed::Series(p) => p.rational_function(),
        }
    }

    /// Both values as rational functions of the least root of `dt` both are functions of, when both
    /// are such functions: their arithmetic is then that field's own, with no resultant.
    fn on_one_grid(&self, o: &Closed) -> Option<(Hyperreal, Hyperreal, u32)> {
        let (a, qa) = self.field()?;
        let (b, qb) = o.field()?;
        let q = lcm_u32(qa, qb);
        Some((a.ramify(q / qa), b.ramify(q / qb), q))
    }

    /// The value as a series over `K_s`.
    fn series(&self) -> Puiseux {
        match self {
            Closed::Field(h) => Puiseux::of_field(h, 1),
            Closed::Series(p) => p.clone(),
        }
    }

    pub fn is_zero(&self) -> bool {
        matches!(self, Closed::Field(h) if h.is_zero())
    }

    pub fn add(&self, o: &Closed) -> Closed {
        if let Some((a, b, q)) = self.on_one_grid(o) {
            return Closed::of_field(a.add(&b), q);
        }
        self.series().plus(&o.series()).canonical()
    }

    pub fn neg(&self) -> Closed {
        match self {
            Closed::Field(h) => Closed::Field(h.neg()),
            Closed::Series(p) => Closed::Series(p.negated()),
        }
    }

    pub fn sub(&self, o: &Closed) -> Closed {
        self.add(&o.neg())
    }

    pub fn mul(&self, o: &Closed) -> Closed {
        if let Some((a, b, q)) = self.on_one_grid(o) {
            return Closed::of_field(a.mul(&b), q);
        }
        if self.is_zero() || o.is_zero() {
            return Closed::Field(zero());
        }
        self.series().times(&o.series()).canonical()
    }

    /// The reciprocal; `None` for zero. The reciprocal of a value that is no rational function of a
    /// root of `dt` is none either.
    pub fn recip(&self) -> Option<Closed> {
        match self {
            Closed::Field(h) => h.recip().map(Closed::Field),
            Closed::Series(p) if p.poly.degree() == Some(1) => p.clone().canonical_recip(),
            Closed::Series(p) => Some(Closed::Series(p.inverse())),
        }
    }

    pub fn div(&self, o: &Closed) -> Option<Closed> {
        Some(self.mul(&o.recip()?))
    }

    /// `self^k`; refused for zero to a negative power and, on a rational function of a root of
    /// `dt`, for a power past [`crate::hyperreal::MAX_POWER_DEGREE`] in that root.
    pub fn pow(&self, k: i32) -> Result<Closed, PowerRefusal> {
        if k < 0 {
            let inverse = self.recip().ok_or(PowerRefusal {
                error: PowerError::ZeroToNegative,
                q: 1,
            })?;
            return inverse.pow(-k);
        }
        if k == 0 {
            return Ok(Closed::Field(Hyperreal::from_i64(1)));
        }
        if let Some((h, q)) = self.field() {
            let power = h.pow(k).map_err(|error| PowerRefusal { error, q })?;
            return Ok(Closed::of_field(power, q));
        }
        match self {
            Closed::Field(_) => unreachable!("a hyperreal is a rational function of dt"),
            Closed::Series(p) => Ok(p.power(k as u32).canonical()),
        }
    }

    /// The real `n`-th root — the positive one for even `n` — or the reason there is none.
    pub fn root(&self, n: u32) -> Result<Closed, String> {
        if n == 1 || self.is_zero() {
            return Ok(self.clone());
        }
        Ok(self.series().nth_root(n)?.canonical())
    }

    /// `−1`, `0` or `1`: the sign of the leading coefficient.
    pub fn signum(&self) -> i32 {
        match self {
            Closed::Field(h) => h.signum(),
            Closed::Series(p) => p.signum(),
        }
    }

    /// The order. The terms both expansions hold through the orders that separate their own
    /// polynomials' roots decide it at the first place they differ; expansions that agree that far are
    /// ordered by the sign of the difference, which is the zero root of its polynomial exactly when the
    /// two are equal.
    pub fn compare(&self, o: &Closed) -> Ordering {
        if let Some((a, b, _)) = self.on_one_grid(o) {
            return a.cmp(&b);
        }
        let (a, b) = (self.series(), o.series());
        let depth = a.separation().max(b.separation());
        let differ = series_add(&a.through(&depth), &series_neg(&b.through(&depth)));
        if let Some((_, c)) = differ.first() {
            return if c.signum() < 0 { Ordering::Less } else { Ordering::Greater };
        }
        match a.plus(&b.negated()).signum() {
            -1 => Ordering::Less,
            0 => Ordering::Equal,
            _ => Ordering::Greater,
        }
    }

    /// The exponent of the leading term in `dt`; `None` for zero.
    pub fn valuation(&self) -> Option<Rational> {
        match self {
            Closed::Field(h) => h.valuation().map(Rational::from_i64),
            Closed::Series(p) => p.leading().map(|(v, _)| v),
        }
    }

    pub fn is_infinitesimal(&self) -> bool {
        self.valuation().is_none_or(|v| v.is_positive())
    }

    pub fn is_limited(&self) -> bool {
        self.valuation().is_none_or(|v| !v.is_negative())
    }

    /// The standard real infinitely close to a limited value; `None` for an infinite one.
    pub fn standard_part(&self) -> Option<Algebraic> {
        match self {
            Closed::Field(h) => h.standard_part(),
            Closed::Series(p) => {
                let (v, c) = p.leading().expect("a series value is nonzero");
                if v.is_positive() {
                    Some(Algebraic::from_i64(0))
                } else if v.is_zero() {
                    Some(c)
                } else {
                    None
                }
            }
        }
    }

    /// The value with the infinitesimal written `name`.
    pub fn render(&self, name: &str) -> String {
        match self {
            Closed::Field(h) => h.render(name),
            Closed::Series(p) => p.render(name),
        }
    }
}

impl Puiseux {
    /// The reciprocal of a rational function of a root of `dt`, in its one form.
    fn canonical_recip(self) -> Option<Closed> {
        let (h, q) = self.field_value()?;
        Some(Closed::of_field(h.recip()?, q))
    }
}

fn radical_sign(n: u32) -> String {
    match n {
        2 => "√".to_string(),
        3 => "∛".to_string(),
        4 => "∜".to_string(),
        n => {
            let digits: String = n
                .to_string()
                .chars()
                .map(|d| ['⁰', '¹', '²', '³', '⁴', '⁵', '⁶', '⁷', '⁸', '⁹'][d.to_digit(10).expect("a digit") as usize])
                .collect();
            format!("{digits}√")
        }
    }
}

/// `dt^e` written: `dt`, `dt^2`, `√dt`, `∛dt`, `dt^(3/2)`.
pub(crate) fn power_text(name: &str, e: &Rational) -> String {
    let one = Rational::one();
    if *e == one {
        return name.to_string();
    }
    if e.is_integer() {
        return format!("{name}^{e}");
    }
    let half = Rational::from_ratio_i64(1, 2).expect("2 > 0");
    let third = Rational::from_ratio_i64(1, 3).expect("3 > 0");
    if *e == half {
        return format!("√{name}");
    }
    if *e == third {
        return format!("∛{name}");
    }
    format!("{name}^({e})")
}

/// What is left of an expansion from `dt^e` on: `O(dt^2)`, `O(√dt)`, and `O(1)` from the standard order.
pub(crate) fn remainder(name: &str, e: &Rational) -> String {
    if e.is_zero() {
        "O(1)".to_string()
    } else {
        format!("O({})", power_text(name, e))
    }
}

fn term_text(name: &str, c: &Algebraic, e: &Rational) -> String {
    expansion_term(name, &c.to_string(), e)
}

/// One term `c·dt^e` of an expansion, its coefficient printed `text`: `dt`, `2·√dt`, `-(1/2)·dt^2`,
/// `e^2·dt`, `3/dt`. A coefficient that is a sum or a quotient is parenthesized, and the sign of one
/// that is not a sum stands outside.
pub(crate) fn expansion_term(name: &str, text: &str, e: &Rational) -> String {
    if e.is_zero() {
        return text.to_string();
    }
    let (sign, magnitude) = match text.strip_prefix('-') {
        Some(m) if !top_level(m, &[' ']) => ("-", m),
        _ => ("", text),
    };
    let magnitude = if top_level(magnitude, &[' ', '/']) {
        format!("({magnitude})")
    } else {
        magnitude.to_string()
    };
    if e.is_negative() {
        return format!("{sign}{magnitude}/{}", power_text(name, &e.negated()));
    }
    let power = power_text(name, e);
    if magnitude == "1" {
        format!("{sign}{power}")
    } else {
        format!("{sign}{magnitude}·{power}")
    }
}

/// Whether `text` holds one of `marks` outside every parenthesis.
pub(crate) fn top_level(text: &str, marks: &[char]) -> bool {
    let mut depth = 0i32;
    for ch in text.chars() {
        match ch {
            '(' => depth += 1,
            ')' => depth -= 1,
            ch if depth == 0 && marks.contains(&ch) => return true,
            _ => {}
        }
    }
    false
}

impl fmt::Debug for Puiseux {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Puiseux {{ q: {}, degree: {:?}, leading: {:?} }}", self.q, self.poly.degree(), self.leading())
    }
}

impl PartialEq for Closed {
    fn eq(&self, o: &Self) -> bool {
        match (self, o) {
            (Closed::Field(a), Closed::Field(b)) => a == b,
            (Closed::Series(_), Closed::Series(_)) => self.compare(o) == Ordering::Equal,
            _ => false,
        }
    }
}

impl Eq for Closed {}

impl Hash for Closed {
    /// Equal values have one form, and two equal series one expansion: the field value itself, or a
    /// series by its leading term.
    fn hash<H: Hasher>(&self, state: &mut H) {
        match self {
            Closed::Field(h) => {
                0u8.hash(state);
                h.hash(state);
            }
            Closed::Series(p) => {
                1u8.hash(state);
                let (e, c) = p.leading().expect("a series value is nonzero");
                e.hash(state);
                c.hash(state);
            }
        }
    }
}

impl crate::exact_codec::ExactCodec for Puiseux {
    /// `q`, the polynomial's coefficients lowest power first, and the root's terms through the order
    /// that tells the polynomial's roots apart, from which a reader picks the same root.
    fn write(&self, out: &mut Vec<u8>) {
        (u64::from(self.q)).write(out);
        crate::exact_codec::write_list(&self.poly.c, out);
        let terms = self.through(&self.separation());
        crate::exact_codec::write_len(terms.len(), out);
        for (e, c) in &terms {
            e.write(out);
            c.write(out);
        }
    }

    fn read(buf: &[u8], pos: &mut usize) -> Option<Self> {
        use crate::exact_codec::{read_len, MAX_WIRE_DEGREE};
        let q = u32::try_from(u64::read(buf, pos)?).ok().filter(|q| *q > 0)?;
        let poly = KPoly::from(crate::exact_codec::read_list(buf, pos)?);
        if poly.degree().unwrap_or(0) == 0 || poly.squarefree().degree() != poly.degree() {
            return None;
        }
        let count = read_len(buf, pos, MAX_WIRE_DEGREE)?;
        let mut want: Series = Vec::with_capacity(count);
        for _ in 0..count {
            want.push((Rational::read(buf, pos)?, Algebraic::read(buf, pos)?));
        }
        let bs = branches(&poly, q);
        let separation = separation_of(&bs);
        let chosen = bs.into_iter().find_map(|mut b| (b.through(&separation) == want).then_some(b))?;
        Some(Puiseux::picked_as(poly, q, chosen, separation))
    }
}

impl Closed {
    /// The bytes of the value's one form.
    pub fn encode(&self, out: &mut Vec<u8>) {
        use crate::exact_codec::ExactCodec;
        match self {
            Closed::Field(h) => {
                out.push(0);
                h.write(out);
            }
            Closed::Series(p) => {
                out.push(1);
                p.write(out);
            }
        }
    }

    pub fn decode(buf: &[u8], pos: &mut usize) -> Option<Closed> {
        use crate::exact_codec::{read_byte, ExactCodec};
        match read_byte(buf, pos)? {
            0 => Some(Closed::Field(Hyperreal::read(buf, pos)?)),
            1 => Some(Puiseux::read(buf, pos)?.canonical()),
            _ => None,
        }
    }
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_base/puiseux/tests.rs"]
mod tests;
