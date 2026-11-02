//! Validated integration of a polynomial differential equation `x' = f(x)`.
//!
//! Every enclosure this module returns contains the exact solution: nothing in it is an
//! approximation that is merely believed accurate.
//!
//! **One step** of length `h` from an interval box `X₀` of start points runs in two halves:
//!
//! - *An a priori box.* A box `B` with `X₀ + [0, h]·f(B) ⊆ B` holds every solution from `X₀` over
//!   the whole step (Picard–Lindelöf through Schauder's fixed point theorem). It is found by
//!   iterating `B ← X₀ + [0, h]·f(B)` from `X₀`, inflating each round.
//! - *A Taylor model.* The solution's Taylor coefficients satisfy `(j + 1)·x_[j+1] = f(x)_[j]`.
//!   Each coefficient of a monomial of `f` is a Cauchy product of the coefficients already known,
//!   so `K` orders cost `O(K²)` products per factor. Lagrange's remainder gives
//!   `x(h) ∈ Σ_{j<K} x_[j](X₀)·h^j + x_[K](B)·[0, h^K]`. The coefficient at `B` encloses
//!   `x^{(K)}(ξ)/K!` for every `ξ` in the step, because the solution stays in `B` there.
//!
//! **The driver** walks the steps at a working precision `q`. It halves the step when no a priori
//! box closes. A solution that leaves every bound, and a step pushed below `2^-cap`, ends the walk
//! with [`FlowError::Escapes`].
//!
//! **A scalar field that blows up** is decided apart from the walk. When `x' = p(x)` with
//! `p > 0` on `[x₀, ∞)` and `deg p ≥ 2`, the solution is increasing and leaves every bound at
//! `T* = ∫_{x₀}^∞ dx/p(x)`. [`escape_time_bound`] bounds `T*` from above by a rational number:
//! a positive interval lower bound of `p` on each piece of a grid over `[x₀, b]`, and past
//! `b ≥ 1`, where every lower term is at most `S·x^{d−1}` for `S` the sum of their magnitudes,
//! `p(x) ≥ (lc − S/b)·x^d`.

use crate::interval::{Dyadic, Iv};
use crate::numeric::Rational;
use crate::poly::Poly;

/// `c·∏ xᵢ^{eᵢ}`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Monomial {
    pub coefficient: Rational,
    pub powers: Vec<u32>,
}

/// A polynomial vector field: component `i` of `f(x)` is the sum of `components[i]`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Field {
    pub components: Vec<Vec<Monomial>>,
}

/// Why a walk stopped before its time.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FlowError {
    /// No a priori box closed above the smallest step: the solution may leave every bound.
    Escapes,
    /// The field's shape does not match the state's.
    Shape,
}

impl Field {
    pub fn dimension(&self) -> usize {
        self.components.len()
    }

    fn well_formed(&self) -> bool {
        let n = self.dimension();
        n > 0
            && self
                .components
                .iter()
                .all(|c| c.iter().all(|m| m.powers.len() == n))
    }

    /// The interval value of the field over a box.
    fn eval(&self, x: &[Iv], q: i64) -> Vec<Iv> {
        self.components
            .iter()
            .map(|monomials| {
                let mut total = Iv::point(Dyadic::zero());
                for m in monomials {
                    let mut term = rational_iv(&m.coefficient, q);
                    for (i, &e) in m.powers.iter().enumerate() {
                        for _ in 0..e {
                            term = term.mul(&x[i]).round_out(q);
                        }
                    }
                    total = total.add(&term).round_out(q);
                }
                total
            })
            .collect()
    }
}

/// The tightest dyadic interval at precision `q` around a rational.
fn rational_iv(r: &Rational, q: i64) -> Iv {
    Iv {
        lo: Dyadic::from_rational_floor(r, q),
        hi: Dyadic::from_rational_ceil(r, q),
    }
}

/// `x / k` for a positive integer `k`, rounded outward to precision `q`.
fn div_int(x: &Iv, k: i64, q: i64) -> Iv {
    let k = Rational::from_i64(k);
    let lo = x.lo.to_rational().div(&k).expect("k > 0");
    let hi = x.hi.to_rational().div(&k).expect("k > 0");
    Iv {
        lo: Dyadic::from_rational_floor(&lo, q),
        hi: Dyadic::from_rational_ceil(&hi, q),
    }
}

fn subset(inner: &Iv, outer: &Iv) -> bool {
    inner.lo.cmp(&outer.lo) != std::cmp::Ordering::Less && inner.hi.cmp(&outer.hi) != std::cmp::Ordering::Greater
}

/// The Taylor coefficients of the solution from a box, one order at a time.
struct Series<'f> {
    field: &'f Field,
    /// Each monomial's factors, a variable index per unit of degree.
    factors: Vec<Vec<Vec<usize>>>,
    coefficients: Vec<Vec<Iv>>,
    /// `x[i][j]`, the coefficient of `t^j` in component `i`.
    x: Vec<Vec<Iv>>,
    /// The coefficients of each monomial's partial products, `P_m = ∏_{k ≤ m} factor_k`.
    partial: Vec<Vec<Vec<Vec<Iv>>>>,
    q: i64,
}

impl<'f> Series<'f> {
    fn new(field: &'f Field, start: &[Iv], q: i64) -> Series<'f> {
        let factors: Vec<Vec<Vec<usize>>> = field
            .components
            .iter()
            .map(|monomials| {
                monomials
                    .iter()
                    .map(|m| {
                        m.powers
                            .iter()
                            .enumerate()
                            .flat_map(|(i, &e)| std::iter::repeat(i).take(e as usize))
                            .collect()
                    })
                    .collect()
            })
            .collect();
        let coefficients = field
            .components
            .iter()
            .map(|monomials| monomials.iter().map(|m| rational_iv(&m.coefficient, q)).collect())
            .collect();
        let partial = factors
            .iter()
            .map(|monomials| monomials.iter().map(|f| vec![Vec::new(); f.len()]).collect())
            .collect();
        Series {
            field,
            factors,
            coefficients,
            x: start.iter().map(|s| vec![s.clone()]).collect(),
            partial,
            q,
        }
    }

    /// Extend every component by one order: `x_[j+1] = f(x)_[j] / (j + 1)`.
    fn extend(&mut self) {
        let j = self.x[0].len() - 1;
        let q = self.q;
        let mut next = Vec::with_capacity(self.field.dimension());
        for c in 0..self.field.dimension() {
            let mut total = Iv::point(Dyadic::zero());
            for (k, factors) in self.factors[c].iter().enumerate() {
                let value = if factors.is_empty() {
                    if j == 0 {
                        self.coefficients[c][k].clone()
                    } else {
                        Iv::point(Dyadic::zero())
                    }
                } else {
                    for m in 0..factors.len() {
                        let coefficient = if m == 0 {
                            self.x[factors[0]][j].clone()
                        } else {
                            let mut sum = Iv::point(Dyadic::zero());
                            for i in 0..=j {
                                let product = self.partial[c][k][m - 1][i].mul(&self.x[factors[m]][j - i]);
                                sum = sum.add(&product).round_out(q);
                            }
                            sum
                        };
                        self.partial[c][k][m].push(coefficient);
                    }
                    let last = factors.len() - 1;
                    self.coefficients[c][k].mul(&self.partial[c][k][last][j]).round_out(q)
                };
                total = total.add(&value).round_out(q);
            }
            next.push(div_int(&total, (j + 1) as i64, q));
        }
        for (c, v) in next.into_iter().enumerate() {
            self.x[c].push(v);
        }
    }

    fn coefficient(&mut self, i: usize, order: usize) -> Iv {
        while self.x[i].len() <= order {
            self.extend();
        }
        self.x[i][order].clone()
    }
}

/// A box holding every solution from `x0` for `t ∈ [0, h]`, if one closes.
fn a_priori(field: &Field, x0: &[Iv], h: &Iv, q: i64) -> Option<Vec<Iv>> {
    let time = Iv {
        lo: Dyadic::zero(),
        hi: h.hi.clone(),
    };
    let mut b: Vec<Iv> = x0.to_vec();
    for _ in 0..16 {
        let fb = field.eval(&b, q);
        let image: Vec<Iv> = x0
            .iter()
            .zip(&fb)
            .map(|(x, f)| x.add(&time.mul(f)).round_out(q))
            .collect();
        if image.iter().zip(&b).all(|(i, o)| subset(i, o)) {
            return Some(image);
        }
        b = image
            .iter()
            .map(|i| {
                let slack = i.width().shift(-2).add(&Dyadic::new(crate::numeric::BigInt::from_i64(1), -q));
                i.widen(&slack)
            })
            .collect();
        if b.iter().any(|i| i.mag_bits() > 64) {
            return None;
        }
    }
    None
}

/// One step of length `h` (a point or a thin interval) from the box `x0`.
fn step(field: &Field, x0: &[Iv], h: &Iv, order: usize, q: i64) -> Option<Vec<Iv>> {
    let b = a_priori(field, x0, h, q)?;
    let mut inner = Series::new(field, x0, q);
    let mut outer = Series::new(field, &b, q);
    let mut out = Vec::with_capacity(x0.len());
    let h_power_k = (0..order).fold(Iv::point(Dyadic::int(1)), |acc, _| acc.mul(h).round_out(q));
    let remainder_time = Iv {
        lo: Dyadic::zero().min(h_power_k.lo.clone()),
        hi: h_power_k.hi.clone(),
    };
    for i in 0..x0.len() {
        let mut sum = Iv::point(Dyadic::zero());
        for j in (0..order).rev() {
            sum = sum.mul(h).add(&inner.coefficient(i, j)).round_out(q);
        }
        let remainder = outer.coefficient(i, order).mul(&remainder_time).round_out(q);
        out.push(sum.add(&remainder).round_out(q));
    }
    Some(out)
}

/// An enclosure of the solution at time `t ≥ 0` from every start in `start`, at working
/// precision `q`; `cap` bounds how small a step may become.
pub fn flow(field: &Field, start: &[Iv], t: &Rational, q: i64, cap: i64) -> Result<Vec<Iv>, FlowError> {
    if !field.well_formed() || start.len() != field.dimension() {
        return Err(FlowError::Shape);
    }
    let mut x = start.to_vec();
    let mut remaining = t.clone();
    let mut s: i64 = 3;
    while remaining.is_positive() {
        let full = Rational::from_ratio_i64(1, 1i64 << s.min(62)).expect("2^s > 0");
        let last = remaining.cmp(&full) != std::cmp::Ordering::Greater;
        let length = if last { remaining.clone() } else { full };
        let h = rational_iv(&length, q);
        let order = ((q / s) as usize + 4).clamp(8, 400);
        match step(field, &x, &h, order, q) {
            Some(next) => {
                x = next;
                remaining = remaining.sub(&length);
                if x.iter().any(|i| i.mag_bits() > 64) {
                    return Err(FlowError::Escapes);
                }
            }
            None => {
                s += 1;
                if s > cap {
                    return Err(FlowError::Escapes);
                }
            }
        }
    }
    Ok(x)
}

/// A lower bound of `p` on `[lo, hi]` by interval Horner evaluation.
fn lower_bound(p: &Poly, lo: &Rational, hi: &Rational) -> Rational {
    let mut acc = (Rational::zero(), Rational::zero());
    for c in p.coeffs().iter().rev() {
        let products = [acc.0.mul(lo), acc.0.mul(hi), acc.1.mul(lo), acc.1.mul(hi)];
        let least = products.iter().min().expect("four products").clone();
        let most = products.iter().max().expect("four products").clone();
        acc = (least.add(c), most.add(c));
    }
    acc.0
}

/// For `x' = p(x)` with `p > 0` on `[x₀, ∞)` and `deg p ≥ 2`, a rational number at least the time
/// the solution takes to leave every bound; `None` when `p` has a root there, grows too slowly to
/// leave every bound in finite time, or no grid finer than `2^cap` pieces bounds it.
pub fn escape_time_bound(p: &Poly, x0: &Rational, cap: u32) -> Option<Rational> {
    let d = p.degree()?;
    if d < 2 {
        return None;
    }
    let lc = p.coeff(d);
    if !lc.is_positive() || !p.eval(x0).is_positive() || p.count_real_roots_in(x0, &p.root_bound()) != 0 {
        return None;
    }
    let spread = (0..d).fold(Rational::zero(), |acc, i| acc.add(&p.coeff(i).abs()));
    let b = [x0.clone(), Rational::one(), Rational::from_i64(2).mul(&spread).div(&lc).expect("lc > 0")]
        .into_iter()
        .max()
        .expect("three candidates");
    let slope = lc.sub(&spread.div(&b).expect("b ≥ 1"));
    let tail = Rational::one()
        .div(&slope.mul(&Rational::from_i64(d as i64 - 1)).mul(&b.pow((d - 1) as i32).expect("b ≥ 1")))
        .expect("positive");
    if b == *x0 {
        return Some(tail);
    }
    let span = b.sub(x0);
    for k in 0..=cap {
        let pieces = 1i64 << k.min(40);
        let width = span.div(&Rational::from_i64(pieces)).expect("pieces > 0");
        let mut head = Rational::zero();
        let mut positive = true;
        for i in 0..pieces {
            let lo = x0.add(&width.mul(&Rational::from_i64(i)));
            let hi = lo.add(&width);
            let least = lower_bound(p, &lo, &hi);
            if !least.is_positive() {
                positive = false;
                break;
            }
            head = head.add(&width.div(&least).expect("least > 0"));
        }
        if positive {
            return Some(head.add(&tail));
        }
    }
    None
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_base/ode/tests.rs"]
mod tests;
