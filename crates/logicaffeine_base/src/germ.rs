//! Germs at `dt = 0`: the hyperreals a loop of infinitely many infinitesimal steps leaves, known by
//! the series of their expansion in `dt` with exact computable-real coefficients.
//!
//! A [`Hyperreal`] is a rational function of `dt`; `(1 + dt)^(1/dt)`, the state after `⌈1/dt⌉`
//! Euler steps of `x′ = x`, is not. It is the germ of an analytic function at `dt = 0`, whose series
//! `e − (e/2)·dt + (11e/24)·dt² − …` has computable coefficients, and its standard part is the
//! first of them. A [`Germ`] holds such a value as the operations that built it — the series of a
//! rational function, a flow, sums, products, reciprocals — and computes coefficients on demand,
//! each once.
//!
//! **Flows.** `N` steps of an affine update `x ↦ A(dt)·x` with `A(0) = I`, `A(dt) = I + Σ_{k≥1}
//! dt^k·A_k`, and `N = T/dt + δ`, leave `exp(N·log A(dt))·x₀`. Writing `L(dt) = log A(dt) = Σ_{k≥1}
//! dt^k·L_k`, the exponent is `C_0 + Σ_{j≥1} dt^j·C_j` with `C_j = T·L_{j+1} + δ·L_j`, and `C_0 =
//! T·A_1`. The standard part is `e^{T·A_1}·x₀` always. When the `A_k` commute, so do the `C_j`,
//! and `exp(Σ_{j≥1} dt^j·C_j) = Σ_m dt^m·E_m` with `E_0 = I` and `E_m = (1/m)·Σ_{k=1}^m k·C_k·E_{m−k}`;
//! every coefficient is then exact. When they do not, the coefficients past the standard part are
//! refused by name rather than approximated.
//!
//! **Nonlinear flows.** A flow of a polynomial differential equation is known to its standard part,
//! the equation's solution at `T` enclosed by validated integration; its higher orders are refused
//! by name ([`Germ::standard`]). A solution that leaves every bound before `T` leaves a germ that is
//! not limited and has only a sign ([`Germ::unlimited`]); the sum of it and a limited germ, and its
//! product with one whose standard part is nonzero, are unlimited too.
//!
//! **Fractional orders.** A germ's orders are the multiples of `1/q` for its ramification `q`: order
//! `k` is the coefficient of `dt^(k/q)`. A value of the real closure ([`Puiseux`]) is read from its
//! branch on that branch's own grid; two germs meet on the least grid both lie on, each refined to it.
//!
//! **Functions.** `exp`, `ln` and the real `n`-th roots are germs of their arguments, each coefficient
//! from the ones before it: with `g = Σ g_k·t^k`, `t = dt^(1/q)`,
//!
//! - `e = exp(g)` for a limited `g`: `e_0 = e^{g_0}` and `k·e_k = Σ_{j=1..k} j·g_j·e_{k−j}`;
//! - `l = ln(g)` for an appreciable positive `g`: `l_0 = ln(g_0)` and
//!   `l_k = (g_k − (1/k)·Σ_{j=1..k−1} j·l_j·g_{k−j}) / g_0`;
//! - `w = ⁿ√g` for `g = c·t^v·(1 + …)`, refined until `n` divides `v`, `c > 0` for an even `n`: `w`
//!   starts at `ⁿ√c·t^(v/n)` and `w_{v/n+j} = (1/(j·c))·Σ_{i=1..j} ((1/n + 1)·i − j)·g_{v+i}·w_{v/n+j−i}`
//!   (Miller's recurrence for a power series raised to `1/n`).
//!
//! The exponential of an infinite value lies beyond every power of `dt`, and the logarithm of an
//! infinitesimal or an infinite one carries `ln(dt)`; neither is a germ, and both are refused by name.
//!
//! **Signs.** A germ's sign is the sign of its first nonzero coefficient. Coefficients compare
//! exactly on the exact forms; a general one that cannot be told from zero within the budget, or a
//! germ whose coefficients through `dt^`[`SIGN_ORDERS`] all vanish, is a loud error, never a guess.

use std::fmt;
use std::sync::{Arc, Mutex};

use crate::algebraic::negative_root;
use crate::exact_real::{ExactReal, RealError, DEFAULT_BUDGET};
use crate::hyperreal::Hyperreal;
use crate::matrix_exp::exp_matrix;
use crate::numeric::Rational;
use crate::puiseux::{expansion_term, remainder, Puiseux};

/// How many orders of `dt` a sign or a standard part looks through before it is refused.
pub const SIGN_ORDERS: i64 = 64;

/// Why a coefficient or a sign is not given.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GermError {
    /// The flow's update matrices do not commute, so its coefficients past the standard part are
    /// not computed exactly. The order is the exponent of `dt`.
    NotComputed { order: Rational },
    /// The value is known by its standard part alone, as a nonlinear flow is. The order is the
    /// exponent of `dt`.
    StandardOnly { order: Rational },
    /// A coefficient could not be told from zero.
    Inseparable(String),
    /// The coefficients through `dt^`[`SIGN_ORDERS`] vanish.
    Flat,
    /// The reciprocal of a germ that may be zero.
    DivisionByZero,
    /// An even root of a negative value.
    NegativeRoot(u32),
    /// The logarithm of a negative value.
    Nonpositive,
    /// The exponential of an infinite value, beyond every power of `dt`.
    BeyondEveryPower,
    /// The logarithm of a positive infinitesimal, which carries `ln(dt)`, or of an infinite value,
    /// which carries `ln(1/dt)`.
    CarriesLog { infinite: bool },
}

impl fmt::Display for GermError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GermError::NotComputed { order } => write!(
                f,
                "the {} term of this flow is not computed exactly: its update matrices do not commute",
                order_text(order)
            ),
            GermError::StandardOnly { order } => write!(
                f,
                "the {} term of this value is not known: a nonlinear flow is known by its standard part",
                order_text(order)
            ),
            GermError::Inseparable(why) => f.write_str(why),
            GermError::Flat => write!(
                f,
                "cannot tell this value from zero within its first {SIGN_ORDERS} orders in the infinitesimal"
            ),
            GermError::DivisionByZero => f.write_str("Division by zero"),
            GermError::NegativeRoot(n) => f.write_str(&negative_root(*n)),
            GermError::Nonpositive => f.write_str("logarithm of a nonpositive number"),
            GermError::BeyondEveryPower => {
                f.write_str("the exponential of this value is beyond every power of dt")
            }
            GermError::CarriesLog { infinite: false } => f.write_str("the logarithm of this value carries ln(dt)"),
            GermError::CarriesLog { infinite: true } => f.write_str("the logarithm of this value carries ln(1/dt)"),
        }
    }
}

impl From<RealError> for GermError {
    fn from(e: RealError) -> GermError {
        GermError::Inseparable(e.to_string())
    }
}

/// The term `dt^order` is named by: `dt^1`, `dt^2`, `dt^(3/2)`.
fn order_text(order: &Rational) -> String {
    if order.is_integer() {
        format!("dt^{order}")
    } else {
        format!("dt^({order})")
    }
}

/// `k/q` as an exponent of `dt`.
fn exponent(k: i64, q: u32) -> Rational {
    Rational::from_ratio_i64(k, i64::from(q)).expect("q is positive")
}

fn gcd(a: u64, b: u64) -> u64 {
    if b == 0 {
        a
    } else {
        gcd(b, a % b)
    }
}

fn lcm(a: u32, b: u32) -> u32 {
    let g = gcd(u64::from(a), u64::from(b));
    u32::try_from(u64::from(a) / g * u64::from(b)).expect("a germ's ramification fits")
}

/// `N` steps of an affine update, the shared part of each component's germ.
#[derive(Debug)]
pub struct Flow {
    /// The root of `dt` the update is a polynomial in: `A = I + Σ_{k≥1} u^k·A_k`, `u = dt^(1/grid)`.
    grid: u32,
    /// `A_k` for `k ≥ 1`, zero below `k = grid`, the term of `dt` itself.
    terms: Vec<Vec<Vec<Rational>>>,
    t: Rational,
    delta: Rational,
    start: Vec<Germ>,
    commuting: bool,
    exp_t: Vec<Vec<ExactReal>>,
    /// `E_m`, the coefficient of `u^m`, computed in order.
    expansion: Mutex<Vec<Vec<Vec<Rational>>>>,
    /// `C_j` for `j ≥ 1`, the coefficient of `u^j`, computed in order.
    exponent: Mutex<Vec<Vec<Vec<Rational>>>>,
    /// The least grid the update and the start lie on, every start refined to it: the state is a
    /// series in `dt^(1/q)`.
    q: u32,
    /// The lowest order of the start, and so of the state, in units of `dt^(1/q)`.
    floor: i64,
    /// The state's coefficients `s_m` computed so far.
    state: Mutex<std::collections::BTreeMap<i64, Vec<ExactReal>>>,
}

type Matrix = Vec<Vec<Rational>>;

fn zeros(n: usize) -> Matrix {
    vec![vec![Rational::zero(); n]; n]
}

fn identity(n: usize) -> Matrix {
    (0..n)
        .map(|i| (0..n).map(|j| if i == j { Rational::one() } else { Rational::zero() }).collect())
        .collect()
}

fn mat_mul(a: &Matrix, b: &Matrix) -> Matrix {
    let n = a.len();
    (0..n)
        .map(|i| {
            (0..n)
                .map(|j| (0..n).fold(Rational::zero(), |acc, k| acc.add(&a[i][k].mul(&b[k][j]))))
                .collect()
        })
        .collect()
}

fn mat_add(a: &Matrix, b: &Matrix) -> Matrix {
    a.iter()
        .zip(b)
        .map(|(x, y)| x.iter().zip(y).map(|(p, q)| p.add(q)).collect())
        .collect()
}

fn mat_scale(a: &Matrix, s: &Rational) -> Matrix {
    a.iter().map(|row| row.iter().map(|v| v.mul(s)).collect()).collect()
}

fn is_zero_matrix(a: &Matrix) -> bool {
    a.iter().all(|row| row.iter().all(Rational::is_zero))
}

impl Flow {
    /// The flow of `N = T/dt + δ` steps of `A(dt) = I + Σ_{k≥1} dt^k·terms[k − 1]` from `start`;
    /// `None` when the shapes disagree or there is no first-order term.
    pub fn new(terms: Vec<Matrix>, t: Rational, delta: Rational, start: Vec<Germ>) -> Option<Arc<Flow>> {
        Flow::on_grid(1, terms, t, delta, start)
    }

    /// The flow of `N = T/dt + δ` steps of `A = I + Σ_{k≥1} dt^(k/grid)·terms[k − 1]` from `start`;
    /// `None` when the shapes disagree, a term below `dt` does not vanish, or there is no term of
    /// `dt`.
    pub fn on_grid(grid: u32, terms: Vec<Matrix>, t: Rational, delta: Rational, start: Vec<Germ>) -> Option<Arc<Flow>> {
        let n = start.len();
        let g = grid as usize;
        if n == 0
            || grid == 0
            || terms.len() < g
            || terms.iter().any(|m| m.len() != n || m.iter().any(|r| r.len() != n))
            || !terms[..g - 1].iter().all(is_zero_matrix)
        {
            return None;
        }
        let commuting = terms.iter().enumerate().all(|(i, a)| {
            terms[i + 1..].iter().all(|b| mat_mul(a, b) == mat_mul(b, a))
        });
        let exp_t = exp_matrix(&terms[g - 1], &t)?;
        let q = start.iter().map(Germ::ramification).fold(grid, lcm);
        let start: Vec<Germ> = start.iter().map(|g| g.refined(q / g.q)).collect();
        let floor = start.iter().map(Germ::floor).min().unwrap_or(0);
        Some(Arc::new(Flow {
            grid,
            terms,
            t,
            delta,
            start,
            commuting,
            exp_t,
            expansion: Mutex::new(Vec::new()),
            exponent: Mutex::new(Vec::new()),
            q,
            floor,
            state: Mutex::new(std::collections::BTreeMap::new()),
        }))
    }

    fn width(&self) -> usize {
        self.start.len()
    }

    fn term(&self, k: usize) -> Matrix {
        self.terms.get(k - 1).cloned().unwrap_or_else(|| zeros(self.width()))
    }

    /// `L_k`, the coefficient of `u^k` in `log A = Σ_{n≥1} (−1)^{n+1}·X^n/n`, `X = Σ u^k·A_k`.
    fn log_term(&self, k: usize) -> Matrix {
        let n = self.width();
        let mut total = zeros(n);
        let mut power: Vec<Matrix> = (0..=k).map(|j| if j == 0 { identity(n) } else { zeros(n) }).collect();
        for count in 1..=k {
            let mut next: Vec<Matrix> = vec![zeros(n); k + 1];
            for (have, p) in power.iter().enumerate() {
                if p.iter().all(|r| r.iter().all(Rational::is_zero)) {
                    continue;
                }
                for step in 1..=(k - have) {
                    next[have + step] = mat_add(&next[have + step], &mat_mul(p, &self.term(step)));
                }
            }
            power = next;
            let sign = if count % 2 == 1 { 1 } else { -1 };
            let c = Rational::from_ratio_i64(sign, count as i64).expect("count ≥ 1");
            total = mat_add(&total, &mat_scale(&power[k], &c));
        }
        total
    }

    /// `C_j = T·L_{grid+j} + δ·L_j`: `N·log A = (T·u^(−grid) + δ)·Σ u^k·L_k` past its constant `T·A_grid`.
    fn exponent(&self, j: usize) -> Matrix {
        let mut cache = self.exponent.lock().expect("unpoisoned");
        while cache.len() < j {
            let i = cache.len() + 1;
            let c = mat_add(
                &mat_scale(&self.log_term(i + self.grid as usize), &self.t),
                &mat_scale(&self.log_term(i), &self.delta),
            );
            cache.push(c);
        }
        cache[j - 1].clone()
    }

    fn expansion(&self, m: usize) -> Matrix {
        let mut cache = self.expansion.lock().expect("unpoisoned");
        if cache.is_empty() {
            cache.push(identity(self.width()));
        }
        while cache.len() <= m {
            let mm = cache.len();
            let mut total = zeros(self.width());
            for k in 1..=mm {
                let c = self.exponent(k);
                if is_zero_matrix(&c) {
                    continue;
                }
                let term = mat_mul(&c, &cache[mm - k]);
                total = mat_add(&total, &mat_scale(&term, &Rational::from_i64(k as i64)));
            }
            cache.push(mat_scale(&total, &Rational::from_ratio_i64(1, mm as i64).expect("mm ≥ 1")));
        }
        cache[m].clone()
    }

    /// `s_m = e^{T·A_grid}·Σ_{a·r+b=m} E_a·x₀_b`, in units of `dt^(1/q)`, `u = dt^(r/q)`. Without
    /// commuting terms `e^{C_0 + Σ u^j·C_j}` is no product of exponentials, and only the orders below
    /// the first nonzero `C_j` are known, which need `E_0 = I` alone.
    fn state(&self, m: i64) -> Result<Vec<ExactReal>, GermError> {
        let r = i64::from(self.q / self.grid);
        let reach = (m - self.floor).max(0) / r;
        if !self.commuting && (1..=reach as usize).any(|j| !is_zero_matrix(&self.exponent(j))) {
            return Err(GermError::NotComputed { order: exponent(m, self.q) });
        }
        if let Some(s) = self.state.lock().expect("unpoisoned").get(&m) {
            return Ok(s.clone());
        }
        let n = self.width();
        let mut inner = vec![ExactReal::from_i64(0); n];
        for a in 0..=reach as usize {
            let e = self.expansion(a);
            if is_zero_matrix(&e) {
                continue;
            }
            let start: Vec<ExactReal> = self
                .start
                .iter()
                .map(|g| g.coefficient(m - a as i64 * r))
                .collect::<Result<_, _>>()?;
            for (i, slot) in inner.iter_mut().enumerate() {
                for (j, x) in start.iter().enumerate() {
                    if !e[i][j].is_zero() {
                        *slot = slot.add(&ExactReal::from_rational(e[i][j].clone()).mul(x));
                    }
                }
            }
        }
        let s: Vec<ExactReal> = (0..n)
            .map(|i| (0..n).fold(ExactReal::from_i64(0), |acc, j| acc.add(&self.exp_t[i][j].mul(&inner[j]))))
            .collect();
        self.state.lock().expect("unpoisoned").insert(m, s.clone());
        Ok(s)
    }
}

#[derive(Debug)]
enum Node {
    /// The series of a rational function of `dt`.
    Series(Hyperreal),
    /// A computable real, constant in `dt`.
    Constant(ExactReal),
    Add(Germ, Germ),
    Mul(Germ, Germ),
    Neg(Germ),
    /// `1/g` for a germ whose order and leading coefficient are known.
    Recip(Germ, i64, ExactReal),
    /// One component of a flow.
    Flow(Arc<Flow>, usize),
    /// A limited value whose standard part alone is known.
    Standard(ExactReal),
    /// A value beyond every bound, positive or negative, of which only the sign is known.
    Unlimited(i32),
    /// `g` on a grid `m` times finer: order `k` is `g`'s order `k/m`, and zero off `g`'s grid. Made
    /// only where two grids meet, and written as `g`, its reader meeting them again.
    Refine(Germ, u32),
    /// A value of the real closure, read from its branch.
    Puiseux(Puiseux),
    /// `exp(g)` for a limited `g`.
    Exp(Germ),
    /// `ln(g)` for `g` with a positive standard part, here `g_0`.
    Ln(Germ, ExactReal),
    /// The real `n`-th root of `g = c·t^v·(1 + …)`, `n` dividing `v`, with `ⁿ√c`.
    Root { g: Germ, n: u32, v: i64, c: ExactReal, lead: ExactReal },
}

/// A germ at `dt = 0`. See the module documentation.
#[derive(Clone, Debug)]
pub struct Germ {
    node: Arc<Node>,
    /// The lowest order that may be nonzero.
    floor: i64,
    /// The ramification: order `k` is the coefficient of `dt^(k/q)`.
    q: u32,
    cache: Arc<Mutex<Vec<ExactReal>>>,
}

impl Germ {
    /// Whether the germ can travel as data: one holding a real built from a function a program
    /// defines cannot, that function staying with the program.
    pub fn is_transmissible(&self) -> bool {
        match self.node.as_ref() {
            Node::Series(_) | Node::Unlimited(_) => true,
            Node::Constant(x) | Node::Standard(x) => x.is_transmissible(),
            Node::Add(a, b) | Node::Mul(a, b) => a.is_transmissible() && b.is_transmissible(),
            Node::Neg(a) | Node::Recip(a, _, _) | Node::Refine(a, _) | Node::Exp(a) | Node::Ln(a, _) => {
                a.is_transmissible()
            }
            Node::Root { g, .. } => g.is_transmissible(),
            Node::Puiseux(_) => true,
            Node::Flow(flow, _) => flow.start.iter().all(Germ::is_transmissible),
        }
    }

    fn of(node: Node, floor: i64, q: u32) -> Germ {
        Germ {
            node: Arc::new(node),
            floor,
            q,
            cache: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// The series of a hyperreal.
    pub fn from_hyperreal(h: &Hyperreal) -> Germ {
        let floor = h.valuation().unwrap_or(0).min(0);
        Germ::of(Node::Series(h.clone()), floor, 1)
    }

    /// The expansion of a value of the real closure, on its branch's grid.
    pub fn from_puiseux(p: &Puiseux) -> Germ {
        Germ::of(Node::Puiseux(p.clone()), p.order(), p.ramification())
    }

    pub fn from_i64(n: i64) -> Germ {
        Germ::from_hyperreal(&Hyperreal::from_i64(n))
    }

    /// A computable real, constant in `dt`.
    pub fn constant(x: &ExactReal) -> Germ {
        Germ::of(Node::Constant(x.clone()), 0, 1)
    }

    /// The rational function this germ is the series of, when it was built as one.
    pub fn as_hyperreal(&self) -> Option<&Hyperreal> {
        match self.node.as_ref() {
            Node::Series(h) => Some(h),
            _ => None,
        }
    }

    /// Component `i` of a flow's state.
    pub fn flow(flow: &Arc<Flow>, i: usize) -> Germ {
        Germ::of(Node::Flow(flow.clone(), i), flow.floor, flow.q)
    }

    /// A limited value known by its standard part alone; every higher order is
    /// [`GermError::StandardOnly`].
    pub fn standard(x: &ExactReal) -> Germ {
        Germ::of(Node::Standard(x.clone()), 0, 1)
    }

    /// A value beyond every bound with the sign of `sign`.
    pub fn unlimited(sign: i32) -> Germ {
        Germ::of(Node::Unlimited(sign.signum()), 0, 1)
    }

    /// The ramification `q`: order `k` is the coefficient of `dt^(k/q)`.
    pub fn ramification(&self) -> u32 {
        self.q
    }

    /// The same germ on a grid `m` times finer.
    fn refined(&self, m: u32) -> Germ {
        if m == 1 {
            return self.clone();
        }
        Germ::of(Node::Refine(self.clone(), m), self.floor * i64::from(m), self.q * m)
    }

    /// Both germs on the least grid both lie on.
    fn met(&self, o: &Germ) -> (Germ, Germ, u32) {
        let q = lcm(self.q, o.q);
        (self.refined(q / self.q), o.refined(q / o.q), q)
    }

    /// The orders a sign looks through: those of `dt` through [`SIGN_ORDERS`].
    fn sign_orders(&self) -> i64 {
        SIGN_ORDERS * i64::from(self.q)
    }

    /// The sign of an unlimited germ; `None` for a limited one and for one whose order is not known.
    pub fn unlimited_sign(&self) -> Option<i32> {
        match self.node.as_ref() {
            Node::Unlimited(sign) => Some(*sign),
            _ => None,
        }
    }

    /// Whether every coefficient below order zero is known to vanish: a limited value.
    fn limited_by_construction(&self) -> bool {
        self.floor >= 0 && self.unlimited_sign().is_none()
    }

    /// The lowest order whose coefficient may be nonzero, in units of `dt^(1/q)`.
    pub fn floor(&self) -> i64 {
        self.floor
    }

    /// The coefficient of `dt^(k/q)`, `q` the [`Germ::ramification`].
    pub fn coefficient(&self, k: i64) -> Result<ExactReal, GermError> {
        if k < self.floor {
            return Ok(ExactReal::from_i64(0));
        }
        let at = (k - self.floor) as usize;
        if let Some(c) = self.cache.lock().expect("unpoisoned").get(at) {
            return Ok(c.clone());
        }
        let c = match self.node.as_ref() {
            Node::Series(h) => ExactReal::from_algebraic(h.laurent_coefficient(k)),
            Node::Constant(x) => {
                if k == 0 {
                    x.clone()
                } else {
                    ExactReal::from_i64(0)
                }
            }
            Node::Add(a, b) => a.coefficient(k)?.add(&b.coefficient(k)?),
            Node::Neg(a) => a.coefficient(k)?.neg(),
            Node::Mul(a, b) => {
                let mut total = ExactReal::from_i64(0);
                for i in a.floor..=(k - b.floor) {
                    total = total.add(&a.coefficient(i)?.mul(&b.coefficient(k - i)?));
                }
                total
            }
            Node::Recip(g, order, lead) => {
                if k == -order {
                    lead.recip()?
                } else {
                    let mut total = ExactReal::from_i64(0);
                    for i in (-order)..k {
                        total = total.add(&self.coefficient(i)?.mul(&g.coefficient(order + k - i)?));
                    }
                    total.neg().div(lead)?
                }
            }
            Node::Flow(flow, i) => flow.state(k)?[*i].clone(),
            Node::Standard(x) => {
                if k == 0 {
                    x.clone()
                } else {
                    return Err(GermError::StandardOnly { order: Rational::from_i64(k) });
                }
            }
            Node::Unlimited(_) => return Err(GermError::StandardOnly { order: Rational::from_i64(k) }),
            Node::Refine(g, m) => {
                let m = i64::from(*m);
                if k.rem_euclid(m) == 0 {
                    g.coefficient(k.div_euclid(m))?
                } else {
                    ExactReal::from_i64(0)
                }
            }
            Node::Puiseux(p) => ExactReal::from_algebraic(p.coefficient(k)),
            Node::Exp(g) => {
                if k == 0 {
                    g.coefficient(0)?.exp()
                } else {
                    let mut total = ExactReal::from_i64(0);
                    for j in 1..=k {
                        let gj = g.coefficient(j)?;
                        if !gj.is_exactly_zero() {
                            total = total.add(&ExactReal::from_i64(j).mul(&gj).mul(&self.coefficient(k - j)?));
                        }
                    }
                    total.mul(&ExactReal::from_rational(Rational::from_ratio_i64(1, k).expect("k ≥ 1")))
                }
            }
            Node::Ln(g, g0) => {
                if k == 0 {
                    g0.log()?
                } else {
                    let mut inner = ExactReal::from_i64(0);
                    for j in 1..k {
                        let gk = g.coefficient(k - j)?;
                        if !gk.is_exactly_zero() {
                            inner = inner.add(&ExactReal::from_i64(j).mul(&self.coefficient(j)?).mul(&gk));
                        }
                    }
                    let inner = inner.mul(&ExactReal::from_rational(Rational::from_ratio_i64(1, k).expect("k ≥ 1")));
                    g.coefficient(k)?.sub(&inner).div(g0)?
                }
            }
            Node::Root { g, n, v, c, lead } => {
                let j = k - self.floor;
                if j == 0 {
                    lead.clone()
                } else {
                    let alpha = Rational::from_ratio_i64(1, i64::from(*n)).expect("n ≥ 1").add(&Rational::one());
                    let mut total = ExactReal::from_i64(0);
                    for i in 1..=j {
                        let gi = g.coefficient(v + i)?;
                        if !gi.is_exactly_zero() {
                            let weight = alpha.mul(&Rational::from_i64(i)).sub(&Rational::from_i64(j));
                            total = total.add(&ExactReal::from_rational(weight).mul(&gi).mul(&self.coefficient(k - i)?));
                        }
                    }
                    total.div(&ExactReal::from_i64(j).mul(c))?
                }
            }
        };
        let mut cache = self.cache.lock().expect("unpoisoned");
        if cache.len() == at {
            cache.push(c.clone());
        }
        Ok(c)
    }

    pub fn add(&self, o: &Germ) -> Germ {
        match (self.unlimited_sign(), o.unlimited_sign()) {
            (Some(s), None) if o.limited_by_construction() => return Germ::unlimited(s),
            (None, Some(s)) if self.limited_by_construction() => return Germ::unlimited(s),
            (Some(s), Some(t)) if s == t => return Germ::unlimited(s),
            _ => {}
        }
        let (a, b, q) = self.met(o);
        let floor = a.floor.min(b.floor);
        Germ::of(Node::Add(a, b), floor, q)
    }

    pub fn neg(&self) -> Germ {
        if let Some(s) = self.unlimited_sign() {
            return Germ::unlimited(-s);
        }
        Germ::of(Node::Neg(self.clone()), self.floor, self.q)
    }

    pub fn sub(&self, o: &Germ) -> Germ {
        self.add(&o.neg())
    }

    pub fn mul(&self, o: &Germ) -> Germ {
        let known = |g: &Germ| match g.unlimited_sign() {
            Some(s) => Some(s),
            None if g.limited_by_construction() => g
                .standard_part()
                .ok()
                .flatten()
                .and_then(|x| x.compare(&ExactReal::from_i64(0), DEFAULT_BUDGET).ok())
                .map(|o| o as i32),
            None => None,
        };
        if self.unlimited_sign().is_some() || o.unlimited_sign().is_some() {
            if let (Some(s), Some(t)) = (known(self), known(o)) {
                if s != 0 && t != 0 {
                    return Germ::unlimited(s * t);
                }
            }
        }
        let (a, b, q) = self.met(o);
        let floor = a.floor + b.floor;
        Germ::of(Node::Mul(a, b), floor, q)
    }

    /// The order, in units of `dt^(1/q)`, and the coefficient of the first nonzero term.
    pub fn leading(&self) -> Result<(i64, ExactReal), GermError> {
        for k in self.floor..self.floor + self.sign_orders() {
            let c = self.coefficient(k)?;
            if !c.is_exactly_zero() {
                let sign = c.compare(&ExactReal::from_i64(0), DEFAULT_BUDGET)?;
                if sign != std::cmp::Ordering::Equal {
                    return Ok((k, c));
                }
            }
        }
        Err(GermError::Flat)
    }

    pub fn recip(&self) -> Result<Germ, GermError> {
        let (order, lead) = match self.leading() {
            Ok(l) => l,
            Err(GermError::Flat) => return Err(GermError::DivisionByZero),
            Err(e) => return Err(e),
        };
        Ok(Germ::of(Node::Recip(self.clone(), order, lead), -order, self.q))
    }

    /// The real `n`-th root (`n ≥ 1`), the nonnegative one for an even `n`; refused for a negative
    /// germ and an even `n`.
    pub fn root(&self, n: u32) -> Result<Germ, GermError> {
        assert!(n >= 1, "a root has a positive index");
        if n == 1 {
            return Ok(self.clone());
        }
        if let Some(sign) = self.unlimited_sign() {
            return if sign < 0 && n % 2 == 0 {
                Err(GermError::NegativeRoot(n))
            } else {
                Ok(Germ::unlimited(sign))
            };
        }
        let (v, c) = self.leading()?;
        if n % 2 == 0 && c.compare(&ExactReal::from_i64(0), DEFAULT_BUDGET)? == std::cmp::Ordering::Less {
            return Err(GermError::NegativeRoot(n));
        }
        let m = (u64::from(n) / gcd(u64::from(n), v.unsigned_abs())) as u32;
        let g = self.refined(m);
        let v = v * i64::from(m);
        let lead = c.nth_root(n)?;
        let floor = v / i64::from(n);
        let q = g.q;
        Ok(Germ::of(Node::Root { g, n, v, c, lead }, floor, q))
    }

    /// `exp(self)`, refused for an infinite value, whose exponential lies beyond every power of `dt`.
    pub fn exp(&self) -> Result<Germ, GermError> {
        if self.unlimited_sign().is_some() || self.standard_part()?.is_none() {
            return Err(GermError::BeyondEveryPower);
        }
        Ok(Germ::of(Node::Exp(self.clone()), 0, self.q))
    }

    /// The natural logarithm of a value whose standard part is positive; one that is negative has
    /// none, and one that is a positive infinitesimal or infinite carries the logarithm of `dt`.
    pub fn ln(&self) -> Result<Germ, GermError> {
        if let Some(sign) = self.unlimited_sign() {
            return Err(if sign > 0 { GermError::CarriesLog { infinite: true } } else { GermError::Nonpositive });
        }
        let (order, lead) = self.leading()?;
        if lead.compare(&ExactReal::from_i64(0), DEFAULT_BUDGET)? == std::cmp::Ordering::Less {
            return Err(GermError::Nonpositive);
        }
        match order.cmp(&0) {
            std::cmp::Ordering::Less => Err(GermError::CarriesLog { infinite: true }),
            std::cmp::Ordering::Greater => Err(GermError::CarriesLog { infinite: false }),
            std::cmp::Ordering::Equal => Ok(Germ::of(Node::Ln(self.clone(), lead), 0, self.q)),
        }
    }

    pub fn div(&self, o: &Germ) -> Result<Germ, GermError> {
        Ok(self.mul(&o.recip()?))
    }

    /// `−1`, `0` (only for a germ that is exactly a zero rational function) or `1`.
    pub fn signum(&self) -> Result<i32, GermError> {
        match self.node.as_ref() {
            Node::Series(h) => return Ok(h.signum()),
            Node::Unlimited(sign) => return Ok(*sign),
            _ => {}
        }
        let (_, lead) = self.leading()?;
        Ok(match lead.compare(&ExactReal::from_i64(0), DEFAULT_BUDGET)? {
            std::cmp::Ordering::Less => -1,
            std::cmp::Ordering::Equal => 0,
            std::cmp::Ordering::Greater => 1,
        })
    }

    /// The standard part of a limited germ; `None` for an infinite one.
    pub fn standard_part(&self) -> Result<Option<ExactReal>, GermError> {
        if self.unlimited_sign().is_some() {
            return Ok(None);
        }
        for k in self.floor..0 {
            let c = self.coefficient(k)?;
            if !c.is_exactly_zero() && c.compare(&ExactReal::from_i64(0), DEFAULT_BUDGET)? != std::cmp::Ordering::Equal {
                return Ok(None);
            }
        }
        Ok(Some(self.coefficient(0)?))
    }

    /// The two leading orders and, from the order after the last one shown on the germ's grid, what
    /// is left, the infinitesimal written `name`: `e - (1/2)·e·dt + O(dt^2)`, `1 + √dt + O(dt)`.
    pub fn render(&self, name: &str) -> String {
        if let Some(sign) = self.unlimited_sign() {
            return if sign < 0 { "-unlimited".to_string() } else { "unlimited".to_string() };
        }
        let mut terms: Vec<String> = Vec::new();
        let mut k = self.floor;
        let mut last_shown = None;
        while terms.len() < 2 && k < self.floor + self.sign_orders() {
            match self.coefficient(k) {
                Ok(c) if c.is_exactly_zero() => {}
                Ok(c) => {
                    terms.push(expansion_term(name, &c.to_string(), &exponent(k, self.q)));
                    last_shown = Some(k);
                }
                Err(_) => break,
            }
            k += 1;
        }
        let k = last_shown.map_or(k, |order| order + 1);
        let rest = remainder(name, &exponent(k, self.q));
        let mut out = String::new();
        for (i, t) in terms.iter().enumerate() {
            match (i, t.strip_prefix('-')) {
                (0, _) => out.push_str(t),
                (_, Some(rest)) => {
                    out.push_str(" - ");
                    out.push_str(rest);
                }
                (_, None) => {
                    out.push_str(" + ");
                    out.push_str(t);
                }
            }
        }
        if out.is_empty() {
            rest
        } else {
            format!("{out} + {rest}")
        }
    }
}

impl crate::exact_codec::ExactCodec for Germ {
    /// A node tag, then the node: `0` the series of a hyperreal; `1` a sum and `2` a product of two
    /// germs; `3` a negation; `4` a reciprocal, whose leading term is found again on reading; `5` a
    /// component of a flow — its width `n`, the number of update terms and each as `n × n`
    /// rationals, `T`, `δ`, the `n` start germs and the component; `6` a computable real constant
    /// in `dt`; `7` a value known by its standard part; `8` an unlimited value by its sign; `10` a
    /// value of the real closure; `11` an exponential, `12` a logarithm and `13` an `n`-th root, `n`
    /// after the germ; `14` a component of a flow whose update is a polynomial in a root of `dt`, its
    /// grid and then the fields of `5`. A germ refined to a finer grid is written as itself: every node that meets two
    /// grids meets them again as it is read.
    fn write(&self, out: &mut Vec<u8>) {
        use crate::exact_codec::write_len;
        match self.node.as_ref() {
            Node::Refine(g, _) => g.write(out),
            Node::Puiseux(p) => {
                out.push(10);
                p.write(out);
            }
            Node::Exp(g) => {
                out.push(11);
                g.write(out);
            }
            Node::Ln(g, _) => {
                out.push(12);
                g.write(out);
            }
            Node::Root { g, n, .. } => {
                out.push(13);
                g.write(out);
                write_len(*n as usize, out);
            }
            Node::Series(h) => {
                out.push(0);
                h.write(out);
            }
            Node::Constant(x) => {
                out.push(6);
                x.write(out);
            }
            Node::Standard(x) => {
                out.push(7);
                x.write(out);
            }
            Node::Unlimited(sign) => {
                out.push(8);
                out.push(u8::from(*sign > 0));
            }
            Node::Add(a, b) => {
                out.push(1);
                a.write(out);
                b.write(out);
            }
            Node::Mul(a, b) => {
                out.push(2);
                a.write(out);
                b.write(out);
            }
            Node::Neg(a) => {
                out.push(3);
                a.write(out);
            }
            Node::Recip(a, _, _) => {
                out.push(4);
                a.write(out);
            }
            Node::Flow(flow, i) => {
                if flow.grid == 1 {
                    out.push(5);
                } else {
                    out.push(14);
                    write_len(flow.grid as usize, out);
                }
                let n = flow.width();
                write_len(n, out);
                write_len(flow.terms.len(), out);
                for m in &flow.terms {
                    for row in m {
                        for v in row {
                            v.write(out);
                        }
                    }
                }
                flow.t.write(out);
                flow.delta.write(out);
                for g in &flow.start {
                    g.write(out);
                }
                (*i as u64).write(out);
            }
        }
    }

    fn read(buf: &[u8], pos: &mut usize) -> Option<Self> {
        read_germ(buf, pos, 0)
    }
}

fn read_germ(buf: &[u8], pos: &mut usize, depth: usize) -> Option<Germ> {
    use crate::exact_codec::{read_byte, read_len, ExactCodec, MAX_WIRE_DEGREE, MAX_WIRE_DEPTH};
    if depth > MAX_WIRE_DEPTH {
        return None;
    }
    Some(match read_byte(buf, pos)? {
        0 => Germ::from_hyperreal(&Hyperreal::read(buf, pos)?),
        6 => Germ::constant(&ExactReal::read(buf, pos)?),
        7 => Germ::standard(&ExactReal::read(buf, pos)?),
        8 => match read_byte(buf, pos)? {
            0 => Germ::unlimited(-1),
            1 => Germ::unlimited(1),
            _ => return None,
        },
        1 => {
            let a = read_germ(buf, pos, depth + 1)?;
            a.add(&read_germ(buf, pos, depth + 1)?)
        }
        2 => {
            let a = read_germ(buf, pos, depth + 1)?;
            a.mul(&read_germ(buf, pos, depth + 1)?)
        }
        3 => read_germ(buf, pos, depth + 1)?.neg(),
        4 => read_germ(buf, pos, depth + 1)?.recip().ok()?,
        10 => Germ::from_puiseux(&Puiseux::read(buf, pos)?),
        11 => read_germ(buf, pos, depth + 1)?.exp().ok()?,
        12 => read_germ(buf, pos, depth + 1)?.ln().ok()?,
        13 => {
            let g = read_germ(buf, pos, depth + 1)?;
            let n = read_len(buf, pos, MAX_WIRE_DEGREE).filter(|n| *n >= 2)?;
            let n = u32::try_from(n).ok()?;
            g.root(n).ok()?
        }
        tag @ (5 | 14) => {
            let grid = if tag == 5 {
                1
            } else {
                u32::try_from(read_len(buf, pos, MAX_WIRE_DEGREE).filter(|g| *g >= 2)?).ok()?
            };
            let n = read_len(buf, pos, MAX_WIRE_DEGREE)?;
            let count = read_len(buf, pos, MAX_WIRE_DEGREE)?;
            let mut terms = Vec::with_capacity(count);
            for _ in 0..count {
                let mut m = Vec::with_capacity(n);
                for _ in 0..n {
                    let mut row = Vec::with_capacity(n);
                    for _ in 0..n {
                        row.push(Rational::read(buf, pos)?);
                    }
                    m.push(row);
                }
                terms.push(m);
            }
            let t = Rational::read(buf, pos)?;
            let delta = Rational::read(buf, pos)?;
            let mut start = Vec::with_capacity(n);
            for _ in 0..n {
                start.push(read_germ(buf, pos, depth + 1)?);
            }
            let i = u64::read(buf, pos)? as usize;
            if i >= n {
                return None;
            }
            Germ::flow(&Flow::on_grid(grid, terms, t, delta, start)?, i)
        }
        _ => return None,
    })
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_base/germ/tests.rs"]
mod tests;
