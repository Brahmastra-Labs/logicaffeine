//! Hybrid flows: loops of infinitesimal steps whose state flows between jumps — a bouncing ball, a
//! thermostat — run exactly, as the ideal hybrid system their steps follow to within an
//! infinitesimal.
//!
//! A loop stepping its clock by `dt` whose body is an Euler step `x ← x + dt·f(x)` of a polynomial
//! field and one or more jumps `If guard: …` stays, between jumps, within an infinitesimal of the
//! flow of `x' = f(x)`, and a jump whose guard the flow crosses fires within an infinitesimal of
//! the crossing. Every value the loop leaves is therefore infinitely close to the value the ideal
//! hybrid system leaves, and that value is what [`run`] computes, exactly, in real algebraic
//! numbers:
//!
//! - **flights**: a field whose Lie series terminates flows along polynomials in time, so a flight
//!   is a polynomial in the time since it began;
//! - **events**: a guard's atoms along a flight are polynomials in time. Their real roots cut the
//!   flight into intervals on which every atom keeps one sign, and the guard fires at the first
//!   root after which it holds. A guard that only touches its boundary, two guards that fire at
//!   once, or a jump in the loop's last step depend on which infinitesimal `dt` is, and are refused
//!   by name;
//! - **jumps**: the jump's statements run on the state at the crossing in three-valued logic. Two
//!   values infinitely close to each other are told apart only by what is known of their
//!   difference: that the run holds both exactly, or a sign the guard or a reset implies;
//! - **Zeno tails**: when one jump's events follow each other by a scaling — every quantity the
//!   guards read scaled by `s^wᵢ` and the time between events by `s^q`, under weights for which the
//!   field, the guards and the resets are homogeneous ([`check_scaling`]) — every later event is
//!   the scaled copy of the one before, every value at event `k` is a polynomial in `z = sᵏ` (plus
//!   a multiple of `k` for a count), and for `s < 1` the events accumulate at the sum of a
//!   geometric series. The values there are those polynomials at `z = 0`;
//! - **continuous firing**: a jump whose guard holds again within an infinitesimal of its firing
//!   fires infinitely often in every standard interval — the ball at rest after its Zeno time. The
//!   state it writes stays where the jump holds it, the rest flows, and a value the jump records
//!   takes its value at the last firing where its condition holds.
//!
//! Each value the run leaves says whether the loop's own value is exactly it or only infinitely
//! close to it.

use crate::algebraic::{resultant_in_y, Algebraic};
use crate::numeric::Rational;
use crate::poly::Poly;

/// The most firings a run follows one by one before the rest must be read off a scaling. Each
/// firing's values are exact numbers that grow as the firings go on, so the budget bounds the
/// work as well as the count.
pub const EVENT_BUDGET: usize = 1_000;

/// The highest Lie derivative a field may need before its flow is taken not to be a polynomial in
/// time.
pub const MAX_LIE_ORDER: usize = 32;

/// The largest denominator a weight `log r / log σ` is looked for with.
const MAX_WEIGHT_DENOMINATOR: i64 = 64;

/// The largest numerator a weight `log r / log σ` is looked for with.
const MAX_WEIGHT_NUMERATOR: i64 = 256;

fn alg(n: i64) -> Algebraic {
    Algebraic::from_i64(n)
}

fn quotient(a: &Algebraic, b: &Algebraic) -> Algebraic {
    a.div(b).expect("a divisor the run has checked is nonzero")
}

fn factorial(m: usize) -> Algebraic {
    alg((1..=m as i64).product::<i64>().max(1))
}

/// `base^e` for any integer `e`, `base` nonzero when `e < 0`.
fn power(base: &Algebraic, e: i64) -> Algebraic {
    let e = i32::try_from(e).expect("an exponent the run builds fits an i32");
    base.pow(e).expect("a power the run takes of zero is nonnegative")
}

// ── polynomials in one variable ──────────────────────────────────────────────────────────────

/// A polynomial in one variable with real algebraic coefficients, lowest power first, with no
/// trailing zero: a coordinate of a flight in the time since it began, an atom along a flight, a
/// value along a tail in `z`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UPoly(Vec<Algebraic>);

impl UPoly {
    pub fn zero() -> UPoly {
        UPoly(Vec::new())
    }

    pub fn from_coeffs(mut c: Vec<Algebraic>) -> UPoly {
        while c.last().is_some_and(Algebraic::is_zero) {
            c.pop();
        }
        UPoly(c)
    }

    pub fn constant(a: Algebraic) -> UPoly {
        UPoly::from_coeffs(vec![a])
    }

    /// `a·xᵏ`.
    pub fn monomial(a: Algebraic, k: usize) -> UPoly {
        let mut c = vec![alg(0); k + 1];
        c[k] = a;
        UPoly::from_coeffs(c)
    }

    pub fn coeffs(&self) -> &[Algebraic] {
        &self.0
    }

    pub fn coeff(&self, k: usize) -> Algebraic {
        self.0.get(k).cloned().unwrap_or_else(|| alg(0))
    }

    pub fn is_zero(&self) -> bool {
        self.0.is_empty()
    }

    pub fn degree(&self) -> Option<usize> {
        self.0.len().checked_sub(1)
    }

    /// The constant this polynomial is, when it is one.
    pub fn as_constant(&self) -> Option<Algebraic> {
        match self.0.len() {
            0 => Some(alg(0)),
            1 => Some(self.0[0].clone()),
            _ => None,
        }
    }

    pub fn add(&self, o: &UPoly) -> UPoly {
        let n = self.0.len().max(o.0.len());
        UPoly::from_coeffs((0..n).map(|k| self.coeff(k).add(&o.coeff(k))).collect())
    }

    pub fn neg(&self) -> UPoly {
        UPoly(self.0.iter().map(Algebraic::neg).collect())
    }

    pub fn sub(&self, o: &UPoly) -> UPoly {
        self.add(&o.neg())
    }

    pub fn mul(&self, o: &UPoly) -> UPoly {
        if self.is_zero() || o.is_zero() {
            return UPoly::zero();
        }
        let mut c = vec![alg(0); self.0.len() + o.0.len() - 1];
        for (i, a) in self.0.iter().enumerate() {
            for (j, b) in o.0.iter().enumerate() {
                c[i + j] = c[i + j].add(&a.mul(b));
            }
        }
        UPoly::from_coeffs(c)
    }

    pub fn scale(&self, k: &Algebraic) -> UPoly {
        UPoly::from_coeffs(self.0.iter().map(|a| a.mul(k)).collect())
    }

    pub fn eval(&self, x: &Algebraic) -> Algebraic {
        self.0.iter().rev().fold(alg(0), |acc, c| acc.mul(x).add(c))
    }

    /// The lowest power with a nonzero coefficient, and that coefficient.
    pub fn lowest(&self) -> Option<(usize, &Algebraic)> {
        self.0.iter().enumerate().find(|(_, c)| !c.is_zero())
    }

    /// `self(k·x)`.
    pub fn dilate(&self, k: &Algebraic) -> UPoly {
        let mut scale = alg(1);
        let mut c = Vec::with_capacity(self.0.len());
        for a in &self.0 {
            c.push(a.mul(&scale));
            scale = scale.mul(k);
        }
        UPoly::from_coeffs(c)
    }
}

// ── real roots ───────────────────────────────────────────────────────────────────────────────

/// The distinct real roots of `p`, ascending; none for a constant.
pub fn real_roots(p: &UPoly) -> Vec<Algebraic> {
    let Some((zeros, _)) = p.lowest() else {
        return Vec::new();
    };
    if zeros > 0 {
        let rest = UPoly::from_coeffs(p.0[zeros..].to_vec());
        let mut roots = real_roots(&rest);
        if let Err(at) = roots.binary_search(&alg(0)) {
            roots.insert(at, alg(0));
        }
        return roots;
    }
    let degree = match p.degree() {
        None | Some(0) => return Vec::new(),
        Some(d) => d,
    };
    match degree {
        1 => return vec![quotient(&p.coeff(0).neg(), &p.coeff(1))],
        2 => return quadratic_roots(&p.coeff(2), &p.coeff(1), &p.coeff(0)),
        _ => {}
    }
    if let Some(rational) = p.0.iter().map(Algebraic::to_rational).collect::<Option<Vec<Rational>>>() {
        return rational_roots(&Poly::from_coeffs(rational));
    }
    let mut roots: Vec<Algebraic> = rational_roots(&free_norm(p))
        .into_iter()
        .filter(|r| p.eval(r).is_zero())
        .collect();
    roots.dedup();
    roots
}

fn rational_roots(p: &Poly) -> Vec<Algebraic> {
    let q = p.squarefree_part();
    (1..=q.count_real_roots()).filter_map(|k| Algebraic::root_of(&q, k)).collect()
}

fn quadratic_roots(a: &Algebraic, b: &Algebraic, c: &Algebraic) -> Vec<Algebraic> {
    let disc = b.mul(b).sub(&alg(4).mul(a).mul(c));
    let two_a = alg(2).mul(a);
    match disc.signum() {
        -1 => Vec::new(),
        0 => vec![quotient(&b.neg(), &two_a)],
        _ => {
            let r = disc.sqrt().expect("a positive number has a square root");
            let mut roots = vec![quotient(&b.neg().sub(&r), &two_a), quotient(&b.neg().add(&r), &two_a)];
            roots.sort();
            roots
        }
    }
}

/// A nonzero rational polynomial vanishing at every root of `p`: the product of `Σ σⱼ(aⱼ)·xʲ` over
/// every choice of a conjugate `σⱼ(aⱼ)` for each nonzero coefficient. Its value at a point `ξ` is
/// the product of the roots of the polynomial whose roots are the sums `Σ σⱼ(aⱼ)·ξʲ`, which the
/// resultants that add algebraic numbers build; it is interpolated through `ξ = 1, 2, …`.
fn free_norm(p: &UPoly) -> Poly {
    let terms: Vec<(usize, &Algebraic)> = p.0.iter().enumerate().filter(|(_, a)| !a.is_zero()).collect();
    let n = p.degree().expect("a polynomial with roots is nonzero");
    let degree = n * terms.iter().map(|(_, a)| a.degree()).product::<usize>();
    let points: Vec<Rational> = (1..=degree as i64 + 1).map(Rational::from_i64).collect();
    let values: Vec<Rational> = points
        .iter()
        .map(|xi| {
            let mut sums: Option<Poly> = None;
            for (j, a) in &terms {
                let shrink = xi
                    .pow(*j as i32)
                    .and_then(|s| s.recip())
                    .expect("a positive point has nonzero powers");
                let scaled = a
                    .minimal_polynomial()
                    .compose(&Poly::from_coeffs(vec![Rational::zero(), shrink]));
                sums = Some(match sums {
                    None => scaled,
                    Some(s) => {
                        let d = s.degree().expect("nonzero") * scaled.degree().expect("nonzero");
                        resultant_in_y(
                            &s,
                            |z0| scaled.compose(&Poly::from_coeffs(vec![z0.clone(), Rational::from_i64(-1)])),
                            d,
                        )
                    }
                });
            }
            let s = sums.expect("a polynomial with roots has a nonzero coefficient");
            let d = s.degree().expect("a polynomial of sums is nonzero");
            let product = s.coeff(0).div(&s.coeff(d)).expect("a leading coefficient is nonzero");
            if d % 2 == 1 {
                product.negated()
            } else {
                product
            }
        })
        .collect();
    interpolate(&points, &values)
}

/// The polynomial through `(points[i], values[i])`, by Newton's divided differences.
fn interpolate(points: &[Rational], values: &[Rational]) -> Poly {
    let n = points.len();
    let mut d = values.to_vec();
    for j in 1..n {
        for i in (j..n).rev() {
            let num = d[i].sub(&d[i - 1]);
            let den = points[i].sub(&points[i - j]);
            d[i] = num.div(&den).expect("interpolation points are distinct");
        }
    }
    let mut poly = Poly::constant(d[n - 1].clone());
    for j in (0..n - 1).rev() {
        let factor = Poly::from_coeffs(vec![points[j].negated(), Rational::one()]);
        poly = poly.mul(&factor).add(&Poly::constant(d[j].clone()));
    }
    poly
}

/// A rational strictly between `a < b`.
fn rational_between(a: &Algebraic, b: &Algebraic) -> Rational {
    let mut bits = 8;
    loop {
        let (_, a_hi) = a.enclosure(bits);
        let (b_lo, _) = b.enclosure(bits);
        if a_hi < b_lo {
            return a_hi
                .add(&b_lo)
                .mul(&Rational::from_ratio_i64(1, 2).expect("2 is nonzero"));
        }
        bits = bits.saturating_mul(2);
    }
}

// ── polynomials over the slots ───────────────────────────────────────────────────────────────

/// What a polynomial over the slots evaluates in: algebraic numbers, polynomials in one variable,
/// polynomials over the slots, values along a tail.
pub trait Ring: Clone {
    fn lift(a: &Algebraic) -> Self;
    fn plus(&self, o: &Self) -> Result<Self, String>;
    fn times(&self, o: &Self) -> Result<Self, String>;
}

impl Ring for Algebraic {
    fn lift(a: &Algebraic) -> Self {
        a.clone()
    }
    fn plus(&self, o: &Self) -> Result<Self, String> {
        Ok(self.add(o))
    }
    fn times(&self, o: &Self) -> Result<Self, String> {
        Ok(self.mul(o))
    }
}

impl Ring for UPoly {
    fn lift(a: &Algebraic) -> Self {
        UPoly::constant(a.clone())
    }
    fn plus(&self, o: &Self) -> Result<Self, String> {
        Ok(self.add(o))
    }
    fn times(&self, o: &Self) -> Result<Self, String> {
        Ok(self.mul(o))
    }
}

/// A polynomial over the run's slots with real algebraic coefficients: each monomial by its
/// exponents, listed once, in order, none with a zero coefficient.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MPoly {
    width: usize,
    terms: Vec<(Vec<u32>, Algebraic)>,
}

impl MPoly {
    pub fn zero(width: usize) -> MPoly {
        MPoly { width, terms: Vec::new() }
    }

    pub fn constant(width: usize, a: Algebraic) -> MPoly {
        MPoly::from_terms(width, vec![(vec![0; width], a)])
    }

    /// The value of slot `i`.
    pub fn slot(width: usize, i: usize) -> MPoly {
        let mut e = vec![0; width];
        e[i] = 1;
        MPoly::from_terms(width, vec![(e, alg(1))])
    }

    pub fn from_terms(width: usize, mut terms: Vec<(Vec<u32>, Algebraic)>) -> MPoly {
        terms.sort_by(|a, b| a.0.cmp(&b.0));
        let mut merged: Vec<(Vec<u32>, Algebraic)> = Vec::with_capacity(terms.len());
        for (e, c) in terms {
            match merged.last_mut() {
                Some((last, sum)) if *last == e => *sum = sum.add(&c),
                _ => merged.push((e, c)),
            }
        }
        merged.retain(|(_, c)| !c.is_zero());
        MPoly { width, terms: merged }
    }

    pub fn width(&self) -> usize {
        self.width
    }

    pub fn terms(&self) -> &[(Vec<u32>, Algebraic)] {
        &self.terms
    }

    pub fn is_zero(&self) -> bool {
        self.terms.is_empty()
    }

    pub fn add(&self, o: &MPoly) -> MPoly {
        MPoly::from_terms(self.width, self.terms.iter().chain(&o.terms).cloned().collect())
    }

    pub fn neg(&self) -> MPoly {
        MPoly {
            width: self.width,
            terms: self.terms.iter().map(|(e, c)| (e.clone(), c.neg())).collect(),
        }
    }

    pub fn sub(&self, o: &MPoly) -> MPoly {
        self.add(&o.neg())
    }

    pub fn scale(&self, k: &Algebraic) -> MPoly {
        MPoly::from_terms(self.width, self.terms.iter().map(|(e, c)| (e.clone(), c.mul(k))).collect())
    }

    pub fn mul(&self, o: &MPoly) -> MPoly {
        let mut out = Vec::with_capacity(self.terms.len() * o.terms.len());
        for (e, c) in &self.terms {
            for (f, d) in &o.terms {
                out.push((e.iter().zip(f).map(|(a, b)| a + b).collect(), c.mul(d)));
            }
        }
        MPoly::from_terms(self.width, out)
    }

    /// `∂/∂xᵢ`.
    pub fn partial(&self, i: usize) -> MPoly {
        MPoly::from_terms(
            self.width,
            self.terms
                .iter()
                .filter(|(e, _)| e[i] > 0)
                .map(|(e, c)| {
                    let mut lowered = e.clone();
                    lowered[i] -= 1;
                    (lowered, c.mul(&alg(i64::from(e[i]))))
                })
                .collect(),
        )
    }

    /// The derivative along `field`: `Σᵢ ∂/∂xᵢ · fieldᵢ`.
    pub fn lie(&self, field: &[MPoly]) -> MPoly {
        let mut out = MPoly::zero(self.width);
        for (i, f) in field.iter().enumerate() {
            if !f.is_zero() && self.reads_slot(i) {
                out = out.add(&self.partial(i).mul(f));
            }
        }
        out
    }

    pub fn reads_slot(&self, i: usize) -> bool {
        self.terms.iter().any(|(e, _)| e[i] > 0)
    }

    /// The slots some monomial reads, ascending.
    pub fn reads(&self) -> Vec<usize> {
        (0..self.width).filter(|i| self.reads_slot(*i)).collect()
    }

    pub fn eval<R: Ring>(&self, x: &[R]) -> Result<R, String> {
        let mut total = R::lift(&alg(0));
        for (e, c) in &self.terms {
            let mut term = R::lift(c);
            for (i, k) in e.iter().enumerate() {
                for _ in 0..*k {
                    term = term.times(&x[i])?;
                }
            }
            total = total.plus(&term)?;
        }
        Ok(total)
    }

    /// The `k ≠ 0` with `self = k·o`.
    pub fn proportional_to(&self, o: &MPoly) -> Option<Algebraic> {
        if self.terms.len() != o.terms.len() || self.terms.is_empty() {
            return None;
        }
        let k = quotient(&self.terms[0].1, &o.terms[0].1);
        self.terms
            .iter()
            .zip(&o.terms)
            .all(|((e, c), (f, d))| e == f && *c == d.mul(&k))
            .then_some(k)
    }
}

/// `p` with each slot replaced by the polynomial over the slots in `x`.
fn substitute(p: &MPoly, x: &[MPoly]) -> MPoly {
    let mut total = MPoly::zero(p.width);
    for (e, c) in &p.terms {
        let mut term = MPoly::constant(p.width, c.clone());
        for (i, k) in e.iter().enumerate() {
            for _ in 0..*k {
                term = term.mul(&x[i]);
            }
        }
        total = total.add(&term);
    }
    total
}

// ── the model ────────────────────────────────────────────────────────────────────────────────

/// `left − right` compared with zero.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rel {
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Ne,
}

impl Rel {
    /// The relation a model names: `<`, `<=`, `>`, `>=`, `=`, `!=`.
    pub fn named(name: &str) -> Option<Rel> {
        Some(match name {
            "<" => Rel::Lt,
            "<=" => Rel::Le,
            ">" => Rel::Gt,
            ">=" => Rel::Ge,
            "=" => Rel::Eq,
            "!=" => Rel::Ne,
            _ => return None,
        })
    }

    fn allows(self) -> Signs {
        match self {
            Rel::Lt => Signs::NEG,
            Rel::Le => Signs::NEG.join(Signs::ZERO),
            Rel::Gt => Signs::POS,
            Rel::Ge => Signs::ZERO.join(Signs::POS),
            Rel::Eq => Signs::ZERO,
            Rel::Ne => Signs::NEG.join(Signs::POS),
        }
    }

    fn negated(self) -> Rel {
        match self {
            Rel::Lt => Rel::Ge,
            Rel::Le => Rel::Gt,
            Rel::Gt => Rel::Le,
            Rel::Ge => Rel::Lt,
            Rel::Eq => Rel::Ne,
            Rel::Ne => Rel::Eq,
        }
    }

    fn verdict(self, s: Signs) -> Tri {
        let allowed = self.allows();
        if s.0 == 0 {
            Tri::U
        } else if s.within(allowed) {
            Tri::T
        } else if s.meet(allowed).0 == 0 {
            Tri::F
        } else {
            Tri::U
        }
    }
}

/// A polynomial over the slots whose coefficients are the standard parts of the program's, with
/// whether they are the program's coefficients exactly.
#[derive(Clone, Debug)]
pub struct Term {
    pub poly: MPoly,
    pub exact: bool,
}

/// A condition of a guard or of a jump's statements.
#[derive(Clone, Debug)]
pub enum Cond {
    All(Vec<Cond>),
    Any(Vec<Cond>),
    Not(Box<Cond>),
    /// `poly rel 0`.
    Compare(Rel, Term),
    Infinitesimal(Term),
    Limited(Term),
    /// The truth value a slot holds.
    Truth(usize),
    Constant(bool),
}

/// What a jump assigns: a polynomial over the slots, or a value no polynomial reads.
#[derive(Clone, Debug)]
pub enum Value {
    Poly(Term),
    Fixed(Cell),
}

#[derive(Clone, Debug)]
pub enum Stmt {
    Set { slot: usize, value: Value },
    If { cond: Cond, then: Vec<Stmt>, otherwise: Vec<Stmt> },
}

/// `If guard: block`.
#[derive(Clone, Debug)]
pub struct Jump {
    pub guard: Cond,
    pub block: Vec<Stmt>,
}

/// One part of a loop's body, in its order: a run of flow statements — each slot's rate, the
/// coefficient of `dt` in its Euler step, and whether the run leaves it exactly unchanged — or the
/// check of a jump.
#[derive(Clone, Debug)]
pub enum Item {
    Flow { rates: Vec<MPoly>, preserved: Vec<bool> },
    Check(usize),
}

/// A loop's flow and jumps over its slots: its body in order, and the flow of a whole step — each
/// slot's rate, and whether a step leaves it exactly unchanged.
#[derive(Clone, Debug)]
pub struct Model {
    pub rates: Vec<MPoly>,
    pub preserved: Vec<bool>,
    pub jumps: Vec<Jump>,
    pub schedule: Vec<Item>,
}

impl Model {
    /// The model of a loop of `width` slots whose body is `schedule`.
    pub fn new(width: usize, schedule: Vec<Item>, jumps: Vec<Jump>) -> Model {
        let mut rates = vec![MPoly::zero(width); width];
        let mut preserved = vec![true; width];
        for item in &schedule {
            if let Item::Flow { rates: r, preserved: p } = item {
                for i in 0..width {
                    rates[i] = rates[i].add(&r[i]);
                    preserved[i] &= p[i];
                }
            }
        }
        Model {
            rates,
            preserved,
            jumps,
            schedule,
        }
    }
}

/// What a slot holds in the ideal run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Cell {
    /// A number's standard value, and whether the loop's value is exactly it.
    Num { value: Algebraic, exact: bool },
    Bool(bool),
    /// A value no polynomial reads, by its index among the values the caller holds.
    Other(usize),
}

impl Cell {
    fn number(&self) -> Option<&Algebraic> {
        match self {
            Cell::Num { value, .. } => Some(value),
            _ => None,
        }
    }

    fn is_exact(&self) -> bool {
        !matches!(self, Cell::Num { exact: false, .. })
    }

    fn inexact(&self) -> Cell {
        match self {
            Cell::Num { value, .. } => Cell::Num { value: value.clone(), exact: false },
            other => other.clone(),
        }
    }
}

fn ideal(cells: &[Cell]) -> Vec<Algebraic> {
    cells.iter().map(|c| c.number().cloned().unwrap_or_else(|| alg(0))).collect()
}

// ── three-valued logic and signs ─────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tri {
    F,
    T,
    U,
}

impl Tri {
    fn not(self) -> Tri {
        match self {
            Tri::F => Tri::T,
            Tri::T => Tri::F,
            Tri::U => Tri::U,
        }
    }

    fn and(self, o: Tri) -> Tri {
        match (self, o) {
            (Tri::F, _) | (_, Tri::F) => Tri::F,
            (Tri::T, Tri::T) => Tri::T,
            _ => Tri::U,
        }
    }

    fn or(self, o: Tri) -> Tri {
        self.not().and(o.not()).not()
    }
}

/// The signs a value may have: a set of negative, zero, positive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Signs(u8);

impl Signs {
    const NEG: Signs = Signs(1);
    const ZERO: Signs = Signs(2);
    const POS: Signs = Signs(4);
    const ALL: Signs = Signs(7);

    fn of(sign: i32) -> Signs {
        match sign.signum() {
            -1 => Signs::NEG,
            0 => Signs::ZERO,
            _ => Signs::POS,
        }
    }

    fn meet(self, o: Signs) -> Signs {
        Signs(self.0 & o.0)
    }

    fn join(self, o: Signs) -> Signs {
        Signs(self.0 | o.0)
    }

    fn within(self, o: Signs) -> bool {
        self.0 & !o.0 == 0
    }

    fn members(self) -> impl Iterator<Item = i32> {
        [(-1, Signs::NEG), (0, Signs::ZERO), (1, Signs::POS)]
            .into_iter()
            .filter(move |(_, s)| self.0 & s.0 != 0)
            .map(|(v, _)| v)
    }

    fn times(self, o: Signs) -> Signs {
        let mut out = Signs(0);
        for a in self.members() {
            for b in o.members() {
                out = out.join(Signs::of(a * b));
            }
        }
        out
    }

    fn negated(self) -> Signs {
        self.times(Signs::NEG)
    }

    fn power(self, k: u32) -> Signs {
        (0..k).fold(Signs::POS, |acc, _| acc.times(self))
    }
}

/// What is known of the signs of values infinitely close to zero: each fact a polynomial over the
/// slots and the signs its value may have.
#[derive(Clone, Debug, Default)]
struct Facts(Vec<(MPoly, Signs)>);

impl Facts {
    fn signs_of(&self, p: &MPoly) -> Signs {
        let mut acc = Signs::ALL;
        for (q, s) in &self.0 {
            if let Some(k) = p.proportional_to(q) {
                acc = acc.meet(if k.signum() > 0 { *s } else { s.negated() });
            }
        }
        if acc.0 == 0 {
            Signs::ALL
        } else {
            acc
        }
    }

    fn forget(&mut self, slot: usize) {
        self.0.retain(|(p, _)| !p.reads_slot(slot));
    }

    fn add(&mut self, p: MPoly, s: Signs) {
        if s != Signs::ALL {
            self.0.push((p, s));
        }
    }
}

/// The facts a guard that holds gives: each comparison among its conjuncts.
fn guard_facts(cond: &Cond, facts: &mut Facts) {
    match cond {
        Cond::All(cs) => cs.iter().for_each(|c| guard_facts(c, facts)),
        Cond::Compare(rel, t) => facts.add(t.poly.clone(), rel.allows()),
        Cond::Not(inner) => match inner.as_ref() {
            Cond::Compare(rel, t) => facts.add(t.poly.clone(), rel.negated().allows()),
            Cond::Any(cs) => cs
                .iter()
                .for_each(|c| guard_facts(&Cond::Not(Box::new(c.clone())), facts)),
            Cond::Not(c) => guard_facts(c, facts),
            _ => {}
        },
        _ => {}
    }
}

/// An atom of a condition.
enum Atom<'c> {
    Compare(Rel, &'c Term),
    Infinitesimal(&'c Term),
}

/// `cond` decided atom by atom by `atom`, the truth values a slot holds read from `cells`.
fn decide(cond: &Cond, cells: &[Cell], atom: &mut dyn FnMut(Atom<'_>) -> Result<Tri, String>) -> Result<Tri, String> {
    Ok(match cond {
        Cond::All(cs) => {
            let mut acc = Tri::T;
            for c in cs {
                acc = acc.and(decide(c, cells, atom)?);
            }
            acc
        }
        Cond::Any(cs) => {
            let mut acc = Tri::F;
            for c in cs {
                acc = acc.or(decide(c, cells, atom)?);
            }
            acc
        }
        Cond::Not(c) => decide(c, cells, atom)?.not(),
        Cond::Compare(rel, t) => atom(Atom::Compare(*rel, t))?,
        Cond::Infinitesimal(t) => atom(Atom::Infinitesimal(t))?,
        Cond::Limited(_) => Tri::T,
        Cond::Truth(slot) => match &cells[*slot] {
            Cell::Bool(b) => {
                if *b {
                    Tri::T
                } else {
                    Tri::F
                }
            }
            _ => return Err("a condition of a hybrid loop reads a value that is not a truth value".to_string()),
        },
        Cond::Constant(b) => {
            if *b {
                Tri::T
            } else {
                Tri::F
            }
        }
    })
}

/// Every term a condition compares or asks about.
fn atom_terms(cond: &Cond, out: &mut Vec<Term>) {
    match cond {
        Cond::All(cs) | Cond::Any(cs) => cs.iter().for_each(|c| atom_terms(c, out)),
        Cond::Not(c) => atom_terms(c, out),
        Cond::Compare(_, t) | Cond::Infinitesimal(t) => out.push(t.clone()),
        Cond::Limited(_) | Cond::Truth(_) | Cond::Constant(_) => {}
    }
}

/// Every slot a condition reads.
fn cond_reads(cond: &Cond, out: &mut Vec<usize>) {
    match cond {
        Cond::All(cs) | Cond::Any(cs) => cs.iter().for_each(|c| cond_reads(c, out)),
        Cond::Not(c) => cond_reads(c, out),
        Cond::Compare(_, t) | Cond::Infinitesimal(t) | Cond::Limited(t) => out.extend(t.poly.reads()),
        Cond::Truth(slot) => out.push(*slot),
        Cond::Constant(_) => {}
    }
}

/// Whether a term's value is known exactly from `cells`: exact coefficients over exact slots.
fn term_exact(term: &Term, cells: &[Cell]) -> bool {
    term.exact && term.poly.reads().into_iter().all(|i| cells[i].is_exact())
}

/// The signs a term may have at a state: its standard value's sign, zero when the run holds it
/// exactly, else what the facts know.
fn signs_at(term: &Term, cells: &[Cell], facts: &Facts) -> Result<Signs, String> {
    let value = term.poly.eval(&ideal(cells))?;
    Ok(if !value.is_zero() {
        Signs::of(value.signum())
    } else if term_exact(term, cells) {
        Signs::ZERO
    } else {
        facts.signs_of(&term.poly)
    })
}

fn point_atom(atom: Atom<'_>, cells: &[Cell], facts: &Facts) -> Result<Tri, String> {
    Ok(match atom {
        Atom::Compare(rel, t) => rel.verdict(signs_at(t, cells, facts)?),
        Atom::Infinitesimal(t) => {
            if t.poly.eval(&ideal(cells))?.is_zero() {
                Tri::T
            } else {
                Tri::F
            }
        }
    })
}

// ── the run ──────────────────────────────────────────────────────────────────────────────────

/// `Lᵐ xᵢ` for `m = 0, 1, …` up to the first that vanishes: the Lie series of every slot.
fn lie_series(rates: &[MPoly]) -> Result<Vec<Vec<MPoly>>, String> {
    let width = rates.len();
    (0..width)
        .map(|i| {
            let mut series = vec![MPoly::slot(width, i)];
            loop {
                let next = series.last().expect("nonempty").lie(rates);
                if next.is_zero() {
                    return Ok(series);
                }
                if series.len() > MAX_LIE_ORDER {
                    return Err(
                        "the flow between the loop's jumps is not a polynomial in time — its field's derivatives never vanish — so the times its jumps fire at are not found exactly"
                            .to_string(),
                    );
                }
                series.push(next);
            }
        })
        .collect()
}

/// The flight from `cells`: each slot as a polynomial in the time since the flight began.
fn flight(series: &[Vec<MPoly>], cells: &[Cell]) -> Result<Vec<UPoly>, String> {
    let x = ideal(cells);
    series
        .iter()
        .map(|derivatives| {
            let mut c = Vec::with_capacity(derivatives.len());
            for (m, d) in derivatives.iter().enumerate() {
                c.push(quotient(&d.eval(&x)?, &factorial(m)));
            }
            Ok(UPoly::from_coeffs(c))
        })
        .collect()
}

/// A firing: the state after the jump, which branch each condition it met took, and what is known
/// of the signs of its values infinitely close to zero.
#[derive(Clone, Debug)]
struct Firing {
    post: Vec<Cell>,
    path: Vec<bool>,
    facts: Facts,
    /// Each condition it met, atom by atom, as decided.
    conditions: Vec<Vec<Tri>>,
}

/// A firing at a crossing, in the record a tail is read from.
#[derive(Clone, Debug)]
struct Event {
    at: Algebraic,
    jump: usize,
    firing: Firing,
}

enum Next {
    /// A jump fires now, on the state as it is.
    FireNow(usize),
    /// A jump fires within an infinitesimal time, on the state flowed that far.
    FireSoon(usize),
    /// A jump that just fired fires again and again.
    Slide(usize),
    Flight(Vec<UPoly>),
}

enum SlideEnd {
    Bound,
    LiftOff,
    Fires(usize),
}

enum TailMove {
    None,
    Moved,
    Zeno(usize),
}

/// The values a loop of infinitesimal steps leaves after `span` of its time, starting from
/// `start`: each infinitely close to what the loop leaves, and exactly it where the cell says so.
pub fn run(model: &Model, start: Vec<Cell>, span: &Algebraic) -> Result<Vec<Cell>, String> {
    let width = model.rates.len();
    if start.len() != width || model.preserved.len() != width {
        return Err("a hybrid loop's model and state disagree in width".to_string());
    }
    if span.signum() < 0 {
        return Err("a hybrid loop runs forward a nonnegative time".to_string());
    }
    check_numbers(model, &start)?;
    let series = lie_series(&model.rates)?;
    let mut r = Run {
        model,
        width,
        series,
        cells: start,
        elapsed: alg(0),
        span: span.clone(),
        facts: Facts::default(),
        history: Vec::new(),
        fired: 0,
    };
    r.go()?;
    Ok(r.cells)
}

/// Every slot a polynomial of the model reads: a rate, a guard, a jump's statement or condition.
fn numeric_slots(model: &Model) -> Vec<bool> {
    let width = model.rates.len();
    let mut out = vec![false; width];
    let mut mark = |p: &MPoly| p.reads().into_iter().for_each(|i| out[i] = true);
    model.rates.iter().for_each(&mut mark);
    fn cond_polys(c: &Cond, mark: &mut dyn FnMut(&MPoly)) {
        let mut terms = Vec::new();
        atom_terms(c, &mut terms);
        if let Cond::Limited(t) = c {
            terms.push(t.clone());
        }
        terms.iter().for_each(|t| mark(&t.poly));
    }
    fn block_polys(stmts: &[Stmt], mark: &mut dyn FnMut(&MPoly)) {
        for s in stmts {
            match s {
                Stmt::Set { value: Value::Poly(t), .. } => mark(&t.poly),
                Stmt::Set { .. } => {}
                Stmt::If { cond, then, otherwise } => {
                    cond_polys(cond, mark);
                    block_polys(then, mark);
                    block_polys(otherwise, mark);
                }
            }
        }
    }
    for jump in &model.jumps {
        cond_polys(&jump.guard, &mut mark);
        block_polys(&jump.block, &mut mark);
    }
    out
}

/// A slot a polynomial reads holds a number from the start, and no jump sets it to anything else.
fn check_numbers(model: &Model, start: &[Cell]) -> Result<(), String> {
    let numeric = numeric_slots(model);
    if (0..start.len()).any(|i| numeric[i] && start[i].number().is_none()) {
        return Err("a hybrid loop computes with a value that is not a number".to_string());
    }
    fn sets_other(stmts: &[Stmt], numeric: &[bool]) -> bool {
        stmts.iter().any(|s| match s {
            Stmt::Set { slot, value: Value::Fixed(cell) } => numeric[*slot] && cell.number().is_none(),
            Stmt::Set { .. } => false,
            Stmt::If { then, otherwise, .. } => sets_other(then, numeric) || sets_other(otherwise, numeric),
        })
    }
    if model.jumps.iter().any(|j| sets_other(&j.block, &numeric)) {
        return Err("a jump of a hybrid loop sets a value the loop computes with to something that is not a number".to_string());
    }
    Ok(())
}

struct Run<'m> {
    model: &'m Model,
    width: usize,
    series: Vec<Vec<MPoly>>,
    cells: Vec<Cell>,
    elapsed: Algebraic,
    span: Algebraic,
    facts: Facts,
    history: Vec<Event>,
    fired: usize,
}

impl<'m> Run<'m> {
    fn remaining(&self) -> Algebraic {
        self.span.sub(&self.elapsed)
    }

    fn count(&mut self) -> Result<(), String> {
        self.fired += 1;
        if self.fired > EVENT_BUDGET {
            let gaps: Vec<Algebraic> = self
                .history
                .windows(2)
                .rev()
                .take(4)
                .map(|w| w[1].at.sub(&w[0].at))
                .collect();
            let quickening = gaps.len() == 4 && gaps.windows(2).all(|g| g[0] < g[1]);
            return Err(format!(
                "the loop's jumps fire more than {} times before its end{} and follow no scaling that settles the rest, so its end state is not found exactly",
                thousands(EVENT_BUDGET),
                if quickening { ", each time sooner after the last," } else { "" }
            ));
        }
        Ok(())
    }

    fn go(&mut self) -> Result<(), String> {
        let mut last: Option<usize> = None;
        let mut after_zeno: Option<usize> = None;
        loop {
            if self.elapsed == self.span {
                return Ok(());
            }
            match self.next(last, after_zeno)? {
                Next::FireNow(k) => {
                    self.fire_here(k, self.cells.clone())?;
                    last = Some(k);
                    after_zeno = None;
                    self.history.clear();
                }
                Next::FireSoon(k) => {
                    let pre = self.flowed_an_instant();
                    self.fire_here(k, pre)?;
                    last = Some(k);
                    after_zeno = None;
                    self.history.clear();
                }
                Next::Slide(k) => {
                    self.history.clear();
                    after_zeno = None;
                    match self.slide(k)? {
                        SlideEnd::Bound => return Ok(()),
                        SlideEnd::LiftOff => last = Some(k),
                        SlideEnd::Fires(other) => {
                            self.fire_here(other, self.cells.clone())?;
                            last = Some(other);
                        }
                    }
                }
                Next::Flight(flight) => {
                    let remaining = self.remaining();
                    match self.crossing(&flight, &remaining)? {
                        None => {
                            self.advance(&flight, &remaining);
                            return Ok(());
                        }
                        Some((at, k)) => {
                            self.advance(&flight, &at);
                            let firing = self.fire(k, self.cells.clone())?;
                            self.count()?;
                            self.cells = firing.post.clone();
                            self.facts = firing.facts.clone();
                            self.history.push(Event {
                                at: self.elapsed.clone(),
                                jump: k,
                                firing,
                            });
                            last = Some(k);
                            after_zeno = None;
                            match self.tail()? {
                                TailMove::None => {}
                                TailMove::Moved => {}
                                TailMove::Zeno(j) => {
                                    after_zeno = Some(j);
                                    last = None;
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    /// The state an infinitesimal time later: every slot that flows known only to within an
    /// infinitesimal.
    fn flowed_an_instant(&self) -> Vec<Cell> {
        self.cells
            .iter()
            .enumerate()
            .map(|(i, c)| if self.model.preserved[i] { c.clone() } else { c.inexact() })
            .collect()
    }

    fn advance(&mut self, flight: &[UPoly], by: &Algebraic) {
        if by.is_zero() {
            return;
        }
        for i in 0..self.width {
            if let Cell::Num { exact, .. } = &self.cells[i] {
                let exact = *exact && self.model.preserved[i];
                self.cells[i] = Cell::Num {
                    value: flight[i].eval(by),
                    exact,
                };
            }
        }
        self.elapsed = self.elapsed.add(by);
        self.facts = Facts::default();
    }

    fn fire_here(&mut self, k: usize, pre: Vec<Cell>) -> Result<(), String> {
        let firing = self.fire(k, pre)?;
        self.count()?;
        self.cells = firing.post;
        self.facts = firing.facts;
        Ok(())
    }

    /// Jump `k` fired on `pre`: its statements run on the state, the guard's conjuncts known to
    /// hold.
    fn fire(&self, k: usize, pre: Vec<Cell>) -> Result<Firing, String> {
        let jump = &self.model.jumps[k];
        let mut facts = Facts::default();
        guard_facts(&jump.guard, &mut facts);
        let mut cells = pre;
        let mut path = Vec::new();
        let mut conditions = Vec::new();
        self.exec(&jump.block, &mut cells, &mut facts, &mut path, &mut conditions)?;
        Ok(Firing {
            post: cells,
            path,
            facts,
            conditions,
        })
    }

    fn exec(
        &self,
        stmts: &[Stmt],
        cells: &mut Vec<Cell>,
        facts: &mut Facts,
        path: &mut Vec<bool>,
        conditions: &mut Vec<Vec<Tri>>,
    ) -> Result<(), String> {
        for s in stmts {
            match s {
                Stmt::Set { slot, value } => match value {
                    Value::Poly(term) => {
                        let value = term.poly.eval(&ideal(cells))?;
                        let exact = term_exact(term, cells);
                        let derived = if value.is_zero() && !exact {
                            derived_signs(term, cells, facts)?
                        } else {
                            Signs::ALL
                        };
                        cells[*slot] = Cell::Num { value, exact };
                        facts.forget(*slot);
                        facts.add(MPoly::slot(self.width, *slot), derived);
                    }
                    Value::Fixed(cell) => {
                        cells[*slot] = cell.clone();
                        facts.forget(*slot);
                    }
                },
                Stmt::If { cond, then, otherwise } => {
                    let mut atoms = Vec::new();
                    let verdict = decide(cond, cells, &mut |a| {
                        let v = point_atom(a, cells, facts)?;
                        atoms.push(v);
                        Ok(v)
                    })?;
                    conditions.push(atoms);
                    match verdict {
                        Tri::T => {
                            path.push(true);
                            self.exec(then, cells, facts, path, conditions)?;
                        }
                        Tri::F => {
                            path.push(false);
                            self.exec(otherwise, cells, facts, path, conditions)?;
                        }
                        Tri::U => {
                            return Err(format!(
                                "whether a condition in the loop's jump holds when it fires {} into the loop depends on which infinitesimal dt is: it compares values infinitely close to each other",
                                self.elapsed
                            ))
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// Each jump's next check after `last` fired, or after the loop began, in the order the body
    /// checks them: the rates of the flow statements that run before it and whether they leave each
    /// slot exactly unchanged, `None` when none run.
    fn first_checks(&self, last: Option<usize>) -> Vec<(usize, Option<(Vec<MPoly>, Vec<bool>)>)> {
        let schedule = &self.model.schedule;
        let start = match last {
            Some(l) => schedule
                .iter()
                .position(|item| matches!(item, Item::Check(k) if *k == l))
                .map_or(0, |p| p + 1),
            None => 0,
        };
        let mut rates: Vec<MPoly> = vec![MPoly::zero(self.width); self.width];
        let mut preserved = vec![true; self.width];
        let mut flowed = false;
        let mut out = Vec::with_capacity(self.model.jumps.len());
        for n in 0..schedule.len() {
            match &schedule[(start + n) % schedule.len()] {
                Item::Flow { rates: r, preserved: p } => {
                    flowed = true;
                    for i in 0..self.width {
                        rates[i] = rates[i].add(&r[i]);
                        preserved[i] &= p[i];
                    }
                }
                Item::Check(k) => out.push((*k, flowed.then(|| (rates.clone(), preserved.clone())))),
            }
        }
        out
    }

    /// What happens next: a jump that fires at its next check, or within an infinitesimal time
    /// because its guard holds just after now — again and again when it is the jump that just
    /// fired — or a flight.
    fn next(&self, last: Option<usize>, after_zeno: Option<usize>) -> Result<Next, String> {
        let flight = flight(&self.series, &self.cells)?;
        let mut open = Vec::with_capacity(self.model.jumps.len());
        for jump in &self.model.jumps {
            open.push(decide(&jump.guard, &self.cells, &mut |a| self.open_atom(a, &flight))?);
        }
        let again = |k: usize| last == Some(k) || after_zeno == Some(k);
        for (k, flowed) in self.first_checks(last) {
            let guard = &self.model.jumps[k].guard;
            let verdict = match &flowed {
                None => decide(guard, &self.cells, &mut |a| point_atom(a, &self.cells, &self.facts))?,
                Some((rates, preserved)) => decide(guard, &self.cells, &mut |a| self.window_atom(a, rates, preserved))?,
            };
            let fires = match verdict {
                Tri::T => true,
                Tri::U if open[k] == Tri::T => true,
                Tri::U => {
                    return Err(format!(
                        "whether a jump of the loop fires {} into it depends on which infinitesimal dt is: its guard compares values infinitely close to each other",
                        self.elapsed
                    ))
                }
                Tri::F => false,
            };
            if fires {
                return Ok(if again(k) {
                    Next::Slide(k)
                } else if verdict == Tri::T && flowed.is_none() {
                    Next::FireNow(k)
                } else {
                    Next::FireSoon(k)
                });
            }
        }
        if open.contains(&Tri::U) {
            return Err(format!(
                "the loop's flow runs along a jump's boundary {} into it, so whether the jump fires depends on which infinitesimal dt is",
                self.elapsed
            ));
        }
        let firing: Vec<usize> = (0..open.len()).filter(|k| open[*k] == Tri::T).collect();
        match firing.as_slice() {
            [] => {}
            [k] => return Ok(if again(*k) { Next::Slide(*k) } else { Next::FireSoon(*k) }),
            _ => {
                return Err(format!(
                    "two of the loop's jumps fire at once {} into it, so which fires first depends on which infinitesimal dt is",
                    self.elapsed
                ))
            }
        }
        for jump in &self.model.jumps {
            let verdict = decide(&jump.guard, &self.cells, &mut |a| {
                self.window_atom(a, &self.model.rates, &self.model.preserved)
            })?;
            if verdict != Tri::F {
                return Err(format!(
                    "a jump of the loop may fire again within an infinitesimal time of {} into it, so its end state depends on which infinitesimal dt is",
                    self.elapsed
                ));
            }
        }
        Ok(Next::Flight(flight))
    }

    /// An atom on the open interval just after now, along the flight.
    fn open_atom(&self, atom: Atom<'_>, flight: &[UPoly]) -> Result<Tri, String> {
        let (term, rel) = match atom {
            Atom::Compare(rel, t) => (t, Some(rel)),
            Atom::Infinitesimal(t) => (t, None),
        };
        let along = term.poly.eval(flight)?;
        let signs = match along.lowest() {
            Some((_, c)) => Signs::of(c.signum()),
            None if self.constant_exactly(term) => Signs::ZERO,
            None => Signs::ALL,
        };
        Ok(match rel {
            Some(rel) => rel.verdict(signs),
            None => {
                if along.is_zero() {
                    Tri::T
                } else {
                    Tri::F
                }
            }
        })
    }

    /// Whether a term keeps its value exactly while the loop flows: exact coefficients over exact
    /// slots the flow leaves unchanged.
    fn constant_exactly(&self, term: &Term) -> bool {
        term.exact
            && term
                .poly
                .reads()
                .into_iter()
                .all(|i| self.cells[i].is_exact() && self.model.preserved[i])
    }

    /// An atom over the steps an infinitesimal time after now, each having run flow statements of
    /// `rates`: a value at zero moves first by its rate, from where the facts put it, and a value
    /// they leave unchanged stays as it is.
    fn window_atom(&self, atom: Atom<'_>, rates: &[MPoly], preserved: &[bool]) -> Result<Tri, String> {
        let (term, rel) = match atom {
            Atom::Compare(rel, t) => (t, rel),
            Atom::Infinitesimal(t) => {
                return Ok(if t.poly.eval(&ideal(&self.cells))?.is_zero() {
                    Tri::T
                } else {
                    Tri::F
                })
            }
        };
        let x = ideal(&self.cells);
        let value = term.poly.eval(&x)?;
        if !value.is_zero() {
            return Ok(rel.verdict(Signs::of(value.signum())));
        }
        let start = if term_exact(term, &self.cells) {
            Signs::ZERO
        } else {
            self.facts.signs_of(&term.poly)
        };
        if term.poly.reads().into_iter().all(|i| preserved[i]) {
            return Ok(rel.verdict(start));
        }
        let rate = term.poly.lie(rates).eval(&x)?;
        if rate.is_zero() {
            return Ok(rel.verdict(Signs::ALL));
        }
        let moving = Signs::of(rate.signum());
        Ok(rel.verdict(if start.within(Signs::ZERO.join(moving)) {
            moving
        } else {
            Signs::ALL
        }))
    }

    /// The first time in `(0, remaining]` a jump's guard crosses into holding along `flight`, and
    /// which jump; `None` when none does before the loop ends.
    fn crossing(&self, flight: &[UPoly], remaining: &Algebraic) -> Result<Option<(Algebraic, usize)>, String> {
        let atoms = self.guard_atoms(flight)?;
        let zero = alg(0);
        let mut points: Vec<Algebraic> = atoms
            .iter()
            .flatten()
            .flat_map(real_roots)
            .filter(|r| r > &zero && r <= remaining)
            .collect();
        points.sort();
        points.dedup();
        if points.last() != Some(remaining) {
            points.push(remaining.clone());
        }
        let mut lo = zero;
        for (n, at) in points.iter().enumerate() {
            let inside = rational_between(&lo, at);
            let before = self.verdicts(flight, &Algebraic::from_rational(inside))?;
            if before.iter().any(|v| *v != Tri::F) {
                return Err(format!(
                    "the loop's flow runs along a jump's boundary {} into it, so whether the jump fires depends on which infinitesimal dt is",
                    self.elapsed.add(&lo)
                ));
            }
            let here = self.verdicts(flight, at)?;
            if at == remaining {
                if here.iter().any(|v| *v != Tri::F) {
                    return Err(format!(
                        "a jump of the loop meets its guard's boundary at the loop's last step, {} into it, so whether it fires before the loop ends depends on which infinitesimal dt is",
                        self.elapsed.add(at)
                    ));
                }
                return Ok(None);
            }
            let next = &points[n + 1];
            let after = self.verdicts(flight, &Algebraic::from_rational(rational_between(at, next)))?;
            let mut crossing = Vec::new();
            for k in 0..here.len() {
                match (here[k], after[k]) {
                    (_, Tri::T) => crossing.push(k),
                    (_, Tri::U) => {
                        return Err(format!(
                            "the loop's flow runs along a jump's boundary {} into it, so whether the jump fires depends on which infinitesimal dt is",
                            self.elapsed.add(at)
                        ))
                    }
                    (Tri::F, Tri::F) => {}
                    (_, Tri::F) => {
                        return Err(format!(
                            "the loop's flow touches a jump's guard without crossing it {} into the loop, so whether the jump fires depends on which infinitesimal dt is",
                            self.elapsed.add(at)
                        ))
                    }
                }
            }
            match crossing.as_slice() {
                [] => {}
                [k] => return Ok(Some((at.clone(), *k))),
                _ => {
                    return Err(format!(
                        "two of the loop's jumps fire at once {} into it, so which fires first depends on which infinitesimal dt is",
                        self.elapsed.add(at)
                    ))
                }
            }
            lo = at.clone();
        }
        Ok(None)
    }

    /// Every guard atom along a flight, jump by jump.
    fn guard_atoms(&self, flight: &[UPoly]) -> Result<Vec<Vec<UPoly>>, String> {
        self.model
            .jumps
            .iter()
            .map(|jump| {
                let mut terms = Vec::new();
                atom_terms(&jump.guard, &mut terms);
                terms.iter().map(|t| t.poly.eval(flight)).collect()
            })
            .collect()
    }

    /// Each jump's guard at time `at` of a flight: an atom that vanishes there is decided only if it
    /// keeps its value exactly.
    fn verdicts(&self, flight: &[UPoly], at: &Algebraic) -> Result<Vec<Tri>, String> {
        let mut out = Vec::with_capacity(self.model.jumps.len());
        for jump in &self.model.jumps {
            out.push(decide(&jump.guard, &self.cells, &mut |a| {
                let (term, rel) = match a {
                    Atom::Compare(rel, t) => (t, Some(rel)),
                    Atom::Infinitesimal(t) => (t, None),
                };
                let value = term.poly.eval(flight)?.eval(at);
                let signs = if !value.is_zero() {
                    Signs::of(value.signum())
                } else if self.constant_exactly(term) {
                    Signs::ZERO
                } else {
                    Signs::ALL
                };
                Ok(match rel {
                    Some(rel) => rel.verdict(signs),
                    None => {
                        if value.is_zero() {
                            Tri::T
                        } else {
                            Tri::F
                        }
                    }
                })
            })?);
        }
        Ok(out)
    }
}

/// The signs a reset leaves a value it sets to an infinitesimal: a single monomial's sign is the
/// product of its coefficient's and its factors'.
fn derived_signs(term: &Term, cells: &[Cell], facts: &Facts) -> Result<Signs, String> {
    let [(exponents, coefficient)] = term.poly.terms() else {
        return Ok(Signs::ALL);
    };
    let width = term.poly.width();
    let mut signs = Signs::of(coefficient.signum());
    for (i, k) in exponents.iter().enumerate() {
        if *k == 0 {
            continue;
        }
        let factor = Term {
            poly: MPoly::slot(width, i),
            exact: true,
        };
        signs = signs.times(signs_at(&factor, cells, facts)?.power(*k));
    }
    Ok(signs)
}

fn thousands(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

fn magnitude(a: &Algebraic) -> Algebraic {
    if a.signum() < 0 {
        a.neg()
    } else {
        a.clone()
    }
}

// ── continuous firing ────────────────────────────────────────────────────────────────────────

/// A firing's statements in the order it ran them: the slot each sets, what it sets, and the
/// conditions it ran under with the branch each took.
type PathWrite<'b> = (usize, &'b Value, Vec<(&'b Cond, bool)>);

fn path_writes<'b>(block: &'b [Stmt], path: &[bool]) -> Vec<PathWrite<'b>> {
    fn walk<'b>(
        stmts: &'b [Stmt],
        bits: &mut std::slice::Iter<'_, bool>,
        conds: &mut Vec<(&'b Cond, bool)>,
        out: &mut Vec<PathWrite<'b>>,
    ) {
        for s in stmts {
            match s {
                Stmt::Set { slot, value } => out.push((*slot, value, conds.clone())),
                Stmt::If { cond, then, otherwise } => {
                    let taken = bits.next().copied().unwrap_or(false);
                    conds.push((cond, taken));
                    walk(if taken { then } else { otherwise }, bits, conds, out);
                    conds.pop();
                }
            }
        }
    }
    let mut out = Vec::new();
    walk(block, &mut path.iter(), &mut Vec::new(), &mut out);
    out
}

/// A truth over the open interval `(0, end)` of a regime: one value on each interval between
/// `points`, and one at each point.
#[derive(Clone, Debug)]
struct Piecewise {
    points: Vec<Algebraic>,
    between: Vec<Tri>,
    at: Vec<Tri>,
}

impl Piecewise {
    fn value_at(&self, x: &Algebraic) -> Tri {
        match self.points.binary_search(x) {
            Ok(k) => self.at[k],
            Err(k) => self.between[k],
        }
    }

    /// The conjunction of `parts`, each taken as it is or negated.
    fn all(parts: &[(&Piecewise, bool)], end: &Algebraic) -> Piecewise {
        let mut points: Vec<Algebraic> = parts.iter().flat_map(|(p, _)| p.points.iter().cloned()).collect();
        points.sort();
        points.dedup();
        let eval = |x: &Algebraic| {
            parts.iter().fold(Tri::T, |acc, (p, polarity)| {
                let v = p.value_at(x);
                acc.and(if *polarity { v } else { v.not() })
            })
        };
        let mut between = Vec::with_capacity(points.len() + 1);
        let mut lo = alg(0);
        for p in points.iter().chain(std::iter::once(end)) {
            between.push(eval(&Algebraic::from_rational(rational_between(&lo, p))));
            lo = p.clone();
        }
        let at = points.iter().map(eval).collect();
        Piecewise { points, between, at }
    }

    fn holds_throughout(&self) -> bool {
        self.between.iter().all(|v| *v == Tri::T)
    }

    fn never_holds(&self) -> bool {
        self.between.iter().chain(&self.at).all(|v| *v == Tri::F)
    }

    /// The end of the last interval it holds on — the time of the last firing under it — or
    /// `None` when it holds on none. A truth that holds only at isolated instants, or is unknown
    /// on an interval, has no such time.
    fn last_hold(&self, end: &Algebraic) -> Result<Option<Algebraic>, String> {
        if self.between.contains(&Tri::U) {
            return Err(
                "whether a condition of the jump holds while it fires again and again depends on which infinitesimal dt is"
                    .to_string(),
            );
        }
        for (k, v) in self.at.iter().enumerate() {
            if *v != Tri::F && self.between[k] != Tri::T && self.between[k + 1] != Tri::T {
                return Err(
                    "a condition of the jump holds only at an instant while it fires again and again, so whether a firing meets it depends on which infinitesimal dt is"
                        .to_string(),
                );
            }
        }
        Ok(self
            .between
            .iter()
            .rposition(|v| *v == Tri::T)
            .map(|k| self.points.get(k).cloned().unwrap_or_else(|| end.clone())))
    }
}

/// The state a firing in a regime sees, as polynomials in the time since the regime began.
#[derive(Clone, Debug)]
struct SymState {
    values: Vec<UPoly>,
    exact: Vec<bool>,
    cells: Vec<Cell>,
    /// The conditions each slot was last set under, by their place in the jump; a read under
    /// conditions that do not extend them sees a value it cannot know.
    scope: Vec<Vec<usize>>,
}

/// A statement a regime's firings run, with the truth of the conditions it runs under.
struct RegimeWrite {
    slot: usize,
    value: Option<UPoly>,
    fixed: Option<Cell>,
    exact: bool,
    path: Piecewise,
    /// Whether it adds to the value it sets rather than replacing it.
    accumulates: bool,
    /// Whether it leaves the value it sets exactly as it was.
    unchanged: bool,
}

const UNREADABLE: usize = usize::MAX;

impl<'m> Run<'m> {
    /// Jump `k` fires again and again: the slots it sets that flow stay where it holds them, the
    /// rest flow, and the values it records take their value at the last firing under their
    /// conditions. The regime lasts until the jump's guard no longer holds an instant after a
    /// firing, another jump fires, or the loop ends.
    fn slide(&mut self, k: usize) -> Result<SlideEnd, String> {
        let width = self.width;
        let jump = &self.model.jumps[k];
        let pre = self.flowed_an_instant();
        let firing = self.fire(k, pre)?;
        self.count()?;
        let writes = path_writes(&jump.block, &firing.path);
        let mut effect: Vec<MPoly> = (0..width).map(|i| MPoly::slot(width, i)).collect();
        for (slot, value, _) in &writes {
            if let Value::Poly(t) = value {
                effect[*slot] = substitute(&t.poly, &effect);
            }
        }
        let x = ideal(&self.cells);
        let mut pinned: Vec<usize> = writes
            .iter()
            .filter(|(slot, value, _)| matches!(value, Value::Poly(_)) && !self.model.preserved[*slot])
            .map(|(slot, _, _)| *slot)
            .filter(|slot| effect[*slot] != MPoly::slot(width, *slot))
            .collect();
        pinned.sort_unstable();
        pinned.dedup();
        for p in &pinned {
            if effect[*p].eval(&x)? != x[*p] {
                return Err(format!(
                    "a jump of the loop fires again at once {} into it but moves the state it fires on, so its firings have no standard rhythm",
                    self.elapsed
                ));
            }
        }
        let mut pull = alg(0);
        for p in &pinned {
            let mut row = alg(0);
            for q in &pinned {
                row = row.add(&magnitude(&effect[*p].partial(*q).eval(&x)?));
            }
            pull = pull.max(row);
        }
        if pull >= alg(1) {
            return Err(format!(
                "a jump of the loop holds its state where it fires again and again {} into it, but does not pull the state back as it does, so the infinitesimal motion between firings may grow",
                self.elapsed
            ));
        }
        let mut rates = self.model.rates.clone();
        for p in &pinned {
            rates[*p] = MPoly::zero(width);
        }
        let sliding = lie_series(&rates)?;
        let y = flight(&sliding, &self.cells)?;
        let remaining = self.remaining();
        let (end, kind) = self.regime_end(k, &y, &remaining)?;
        let mut state = SymState {
            values: y.clone(),
            exact: (0..width)
                .map(|i| self.cells[i].is_exact() && self.model.preserved[i] && !pinned.contains(&i))
                .collect(),
            cells: self.cells.clone(),
            scope: vec![Vec::new(); width],
        };
        let mut facts = Facts::default();
        guard_facts(&jump.guard, &mut facts);
        let mut regime = Vec::new();
        let mut scope = Vec::new();
        let mut path: Vec<(Piecewise, bool)> = Vec::new();
        let mut counter = 0;
        regime_walk(
            &jump.block,
            &mut state,
            &mut facts,
            &mut path,
            &mut scope,
            &mut counter,
            &mut regime,
            &end,
        )?;
        let read_by_flow_or_guard = |slot: usize| {
            self.model.rates.iter().any(|r| r.reads_slot(slot))
                || self.model.jumps.iter().any(|j| {
                    let mut reads = Vec::new();
                    cond_reads(&j.guard, &mut reads);
                    reads.contains(&slot)
                })
        };
        let mut last: Vec<Option<(Algebraic, usize)>> = vec![None; width];
        for (order, w) in regime.iter().enumerate() {
            let slot = w.slot;
            if !self.model.preserved[slot] && w.value.is_some() {
                if pinned.contains(&slot) {
                    let held = w.value.as_ref().and_then(UPoly::as_constant);
                    if !w.path.holds_throughout() || held.as_ref() != Some(&x[slot]) {
                        return Err(format!(
                            "a jump of the loop holds its state where it fires again and again {} into it only under a condition that changes, so its firings have no standard rhythm",
                            self.elapsed
                        ));
                    }
                } else if !w.unchanged && !w.path.never_holds() {
                    return Err(format!(
                        "a jump of the loop would move a flowing value while it fires again and again {} into it, so its firings have no standard rhythm",
                        self.elapsed
                    ));
                }
                continue;
            }
            if w.accumulates && !w.path.never_holds() {
                return Err(format!(
                    "a jump of the loop fires infinitely often {} into it and adds to a value at each firing, so that value is not known to any standard precision",
                    self.elapsed
                ));
            }
            if read_by_flow_or_guard(slot) && !w.path.never_holds() {
                let unchanged = match (&w.value, &w.fixed) {
                    (Some(v), _) => v.as_constant().as_ref() == Some(&x[slot]) && w.exact == self.cells[slot].is_exact(),
                    (None, Some(c)) => *c == self.cells[slot],
                    _ => false,
                };
                if !unchanged {
                    return Err(format!(
                        "a jump of the loop fires infinitely often {} into it and changes a value its flow or a guard reads, so its firings have no standard rhythm",
                        self.elapsed
                    ));
                }
            }
            if let Some(at) = w.path.last_hold(&end)? {
                let later = match &last[slot] {
                    Some((t, _)) => at >= *t,
                    None => true,
                };
                if later {
                    last[slot] = Some((at, order));
                }
            }
        }
        for i in 0..width {
            if pinned.contains(&i) {
                self.cells[i] = self.cells[i].inexact();
            } else if let (Cell::Num { .. }, false) = (&self.cells[i], self.model.preserved[i]) {
                self.cells[i] = Cell::Num {
                    value: y[i].eval(&end),
                    exact: false,
                };
            }
        }
        for (slot, entry) in last.iter().enumerate() {
            let Some((at, order)) = entry else { continue };
            let w = &regime[*order];
            self.cells[slot] = match (&w.value, &w.fixed) {
                (Some(v), _) => Cell::Num {
                    value: v.eval(at),
                    exact: w.exact && v.as_constant().is_some(),
                },
                (None, Some(c)) => c.clone(),
                _ => unreachable!("a write sets a polynomial or a fixed value"),
            };
        }
        self.elapsed = self.elapsed.add(&end);
        self.facts = firing.facts;
        Ok(kind)
    }

    /// Where a regime of jump `k` along `y` ends: when its guard no longer holds an instant after
    /// a firing, when another jump's guard crosses into holding, or at the loop's end.
    fn regime_end(&self, k: usize, y: &[UPoly], remaining: &Algebraic) -> Result<(Algebraic, SlideEnd), String> {
        let mut own_terms = Vec::new();
        atom_terms(&self.model.jumps[k].guard, &mut own_terms);
        let mut micro: Vec<Vec<UPoly>> = Vec::with_capacity(own_terms.len());
        for t in &own_terms {
            let mut derivatives = Vec::new();
            let mut d = t.poly.clone();
            while !d.is_zero() {
                if derivatives.len() > MAX_LIE_ORDER {
                    return Err(
                        "the flow between the loop's jumps is not a polynomial in time, so the times its jumps fire at are not found exactly"
                            .to_string(),
                    );
                }
                derivatives.push(d.eval(y)?);
                d = d.lie(&self.model.rates);
            }
            micro.push(derivatives);
        }
        let others: Vec<usize> = (0..self.model.jumps.len()).filter(|j| *j != k).collect();
        let other_atoms = self.guard_atoms(y)?;
        let zero = alg(0);
        let mut points: Vec<Algebraic> = micro
            .iter()
            .flatten()
            .chain(others.iter().flat_map(|j| other_atoms[*j].iter()))
            .flat_map(real_roots)
            .filter(|r| r > &zero && r < remaining)
            .collect();
        points.sort();
        points.dedup();
        points.push(remaining.clone());
        let micro_at = |x: &Algebraic| -> Result<Tri, String> {
            let mut n = 0;
            decide(&self.model.jumps[k].guard, &self.cells, &mut |a| {
                let derivatives = &micro[n];
                n += 1;
                let first = derivatives.iter().map(|d| d.eval(x)).find(|v| !v.is_zero());
                Ok(match a {
                    Atom::Compare(rel, t) => rel.verdict(match first {
                        Some(v) => Signs::of(v.signum()),
                        None if self.constant_exactly(t) => Signs::ZERO,
                        None => Signs::ALL,
                    }),
                    Atom::Infinitesimal(_) => {
                        if derivatives.first().is_none_or(|d| d.eval(x).is_zero()) {
                            Tri::T
                        } else {
                            Tri::F
                        }
                    }
                })
            })
        };
        let mut lo = zero;
        for (n, at) in points.iter().enumerate() {
            let inside = Algebraic::from_rational(rational_between(&lo, at));
            let own = micro_at(&inside)?;
            let rest = self.verdicts(y, &inside)?;
            if own != Tri::T || others.iter().any(|j| rest[*j] != Tri::F) {
                return Err(format!(
                    "a jump of the loop that fires again and again {} into it meets a boundary the regime cannot follow",
                    self.elapsed.add(&lo)
                ));
            }
            if at == remaining {
                return Ok((at.clone(), SlideEnd::Bound));
            }
            let next = Algebraic::from_rational(rational_between(at, &points[n + 1]));
            let own_here = micro_at(at)?;
            let own_after = micro_at(&next)?;
            let here = self.verdicts(y, at)?;
            let after = self.verdicts(y, &next)?;
            let crossing: Vec<usize> = others.iter().copied().filter(|j| after[*j] == Tri::T).collect();
            for j in &others {
                if after[*j] == Tri::U || (after[*j] == Tri::F && here[*j] != Tri::F) {
                    return Err(format!(
                        "another jump of the loop meets its guard's boundary without crossing it {} into the loop, so whether it fires depends on which infinitesimal dt is",
                        self.elapsed.add(at)
                    ));
                }
            }
            match crossing.as_slice() {
                [] => {}
                [j] => return Ok((at.clone(), SlideEnd::Fires(*j))),
                _ => {
                    return Err(format!(
                        "two of the loop's jumps fire at once {} into it, so which fires first depends on which infinitesimal dt is",
                        self.elapsed.add(at)
                    ))
                }
            }
            match own_after {
                Tri::F => return Ok((at.clone(), SlideEnd::LiftOff)),
                Tri::U => {
                    return Err(format!(
                        "whether the jump that fires again and again keeps firing {} into the loop depends on which infinitesimal dt is",
                        self.elapsed.add(at)
                    ))
                }
                Tri::T => {}
            }
            if own_here != Tri::T {
                return Err(format!(
                    "the jump that fires again and again lets go for an instant {} into the loop, so whether it fires then depends on which infinitesimal dt is",
                    self.elapsed.add(at)
                ));
            }
            lo = at.clone();
        }
        unreachable!("the loop's end is the last point")
    }
}

/// A condition's truth over `(0, end)` from the state its statement sees.
fn piecewise(cond: &Cond, state: &SymState, facts: &Facts, end: &Algebraic) -> Result<Piecewise, String> {
    let mut terms = Vec::new();
    atom_terms(cond, &mut terms);
    let along: Vec<UPoly> = terms.iter().map(|t| t.poly.eval(&state.values)).collect::<Result<_, _>>()?;
    let structural: Vec<Signs> = terms
        .iter()
        .map(|t| {
            if t.exact && t.poly.reads().into_iter().all(|i| state.exact[i]) {
                Signs::ZERO
            } else {
                facts.signs_of(&t.poly)
            }
        })
        .collect();
    let zero = alg(0);
    let mut points: Vec<Algebraic> = along
        .iter()
        .flat_map(real_roots)
        .filter(|r| r > &zero && r < end)
        .collect();
    points.sort();
    points.dedup();
    let eval = |x: &Algebraic| -> Result<Tri, String> {
        let mut n = 0;
        decide(cond, &state.cells, &mut |a| {
            let (p, signs) = (&along[n], structural[n]);
            n += 1;
            let value = p.eval(x);
            Ok(match a {
                Atom::Compare(rel, _) => rel.verdict(if !value.is_zero() {
                    Signs::of(value.signum())
                } else if p.is_zero() {
                    signs
                } else {
                    Signs::ALL
                }),
                Atom::Infinitesimal(_) => {
                    if value.is_zero() {
                        Tri::T
                    } else {
                        Tri::F
                    }
                }
            })
        })
    };
    let mut between = Vec::with_capacity(points.len() + 1);
    let mut lo = zero;
    for p in points.iter().chain(std::iter::once(end)) {
        between.push(eval(&Algebraic::from_rational(rational_between(&lo, p)))?);
        lo = p.clone();
    }
    let at = points.iter().map(eval).collect::<Result<Vec<Tri>, String>>()?;
    Ok(Piecewise { points, between, at })
}

fn check_scope(reads: &[usize], state: &SymState, scope: &[usize]) -> Result<(), String> {
    for r in reads {
        let written = &state.scope[*r];
        if written.len() > scope.len() || written.iter().zip(scope).any(|(a, b)| a != b) {
            return Err(
                "a statement of a jump that fires again and again reads a value the jump sets only under another condition, so which value it reads depends on the firing"
                    .to_string(),
            );
        }
    }
    Ok(())
}

/// The statements of a jump that fires again and again, run on the regime's state: both branches
/// of each condition, each statement recorded with the truth of the conditions it runs under.
#[allow(clippy::too_many_arguments)]
fn regime_walk(
    stmts: &[Stmt],
    state: &mut SymState,
    facts: &mut Facts,
    path: &mut Vec<(Piecewise, bool)>,
    scope: &mut Vec<usize>,
    counter: &mut usize,
    out: &mut Vec<RegimeWrite>,
    end: &Algebraic,
) -> Result<(), String> {
    let width = state.values.len();
    for s in stmts {
        match s {
            Stmt::Set { slot, value } => {
                let parts: Vec<(&Piecewise, bool)> = path.iter().map(|(p, b)| (p, *b)).collect();
                let truth = Piecewise::all(&parts, end);
                match value {
                    Value::Poly(t) => {
                        check_scope(&t.poly.reads(), state, scope)?;
                        let v = t.poly.eval(&state.values)?;
                        let exact = t.exact && t.poly.reads().into_iter().all(|i| state.exact[i]);
                        let unchanged = v == state.values[*slot] && exact == state.exact[*slot];
                        let accumulates = t.poly.reads_slot(*slot) && !unchanged;
                        let derived = if v.is_zero() && !exact {
                            regime_signs(t, state, facts)
                        } else {
                            Signs::ALL
                        };
                        out.push(RegimeWrite {
                            slot: *slot,
                            value: Some(v.clone()),
                            fixed: None,
                            exact,
                            path: truth,
                            accumulates,
                            unchanged,
                        });
                        state.values[*slot] = v;
                        state.exact[*slot] = exact;
                        facts.forget(*slot);
                        facts.add(MPoly::slot(width, *slot), derived);
                    }
                    Value::Fixed(c) => {
                        out.push(RegimeWrite {
                            slot: *slot,
                            value: None,
                            fixed: Some(c.clone()),
                            exact: true,
                            path: truth,
                            accumulates: false,
                            unchanged: state.cells[*slot] == *c,
                        });
                        state.cells[*slot] = c.clone();
                        facts.forget(*slot);
                    }
                }
                state.scope[*slot] = scope.clone();
            }
            Stmt::If { cond, then, otherwise } => {
                let mut reads = Vec::new();
                cond_reads(cond, &mut reads);
                check_scope(&reads, state, scope)?;
                let truth = piecewise(cond, state, facts, end)?;
                let id = *counter;
                *counter += 1;
                let saved = state.clone();
                let saved_facts = facts.clone();
                let mut written = vec![false; width];
                for (branch, block) in [(true, then), (false, otherwise)] {
                    let mut st = saved.clone();
                    let mut fa = saved_facts.clone();
                    path.push((truth.clone(), branch));
                    scope.push(id * 2 + usize::from(branch));
                    regime_walk(block, &mut st, &mut fa, path, scope, counter, out, end)?;
                    scope.pop();
                    path.pop();
                    for i in 0..width {
                        if st.scope[i] != saved.scope[i] {
                            written[i] = true;
                        }
                    }
                }
                *state = saved;
                *facts = saved_facts;
                for (i, w) in written.iter().enumerate() {
                    if *w {
                        state.scope[i] = vec![UNREADABLE];
                        facts.forget(i);
                    }
                }
            }
        }
    }
    Ok(())
}

/// The signs a regime's reset leaves a value it sets to an infinitesimal, as at a single firing.
fn regime_signs(term: &Term, state: &SymState, facts: &Facts) -> Signs {
    let [(exponents, coefficient)] = term.poly.terms() else {
        return Signs::ALL;
    };
    let width = term.poly.width();
    let mut signs = Signs::of(coefficient.signum());
    for (i, k) in exponents.iter().enumerate() {
        if *k == 0 {
            continue;
        }
        let v = &state.values[i];
        let factor = match v.as_constant() {
            Some(c) if !c.is_zero() => Signs::of(c.signum()),
            Some(_) if state.exact[i] => Signs::ZERO,
            Some(_) => facts.signs_of(&MPoly::slot(width, i)),
            None => Signs::ALL,
        };
        signs = signs.times(factor.power(*k));
    }
    signs
}

// ── self-similar tails ───────────────────────────────────────────────────────────────────────

/// The longest cycle of firings a tail is looked for in.
const MAX_CYCLE: usize = 8;

/// One firing of a cycle: which jump fires, and the branch each condition it meets takes.
pub type Step = (usize, Vec<bool>);

/// A scaling certificate: from one round of the firings in `cycle` to the next, every slot in
/// `slots` is multiplied by `s^w` (its weight `w`), and each time between firings by `s^q`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Scaling {
    pub cycle: Vec<Step>,
    pub slots: Vec<usize>,
    pub weights: Vec<i64>,
    pub q: i64,
    pub s: Algebraic,
}

impl Scaling {
    fn weight(&self, slot: usize) -> Option<i64> {
        self.slots.iter().position(|s| *s == slot).map(|k| self.weights[k])
    }

    /// Whether `after` is the scaling's image of `before` on its slots.
    fn image(&self, before: &[Cell], after: &[Cell]) -> bool {
        self.slots.iter().zip(&self.weights).all(|(i, w)| match (&before[*i], &after[*i]) {
            (Cell::Num { value: x, .. }, Cell::Num { value: y, .. }) => *y == power(&self.s, *w).mul(x),
            (b, a) => b == a,
        })
    }
}

/// Each firing's statements of a cycle, in order.
fn cycle_writes<'m>(model: &'m Model, cycle: &[Step]) -> Vec<Vec<PathWrite<'m>>> {
    cycle
        .iter()
        .map(|(j, path)| path_writes(&model.jumps[*j].block, path))
        .collect()
}

/// The slots a scaling of a cycle covers: every slot a guard reads, closed under what feeds them
/// — the slots their rates read, and the slots a statement of the cycle setting one of them, or a
/// condition it runs under, reads.
pub fn scaled_slots(model: &Model, cycle: &[Step]) -> Vec<usize> {
    let width = model.rates.len();
    let mut set = vec![false; width];
    for j in &model.jumps {
        let mut reads = Vec::new();
        cond_reads(&j.guard, &mut reads);
        reads.into_iter().for_each(|r| set[r] = true);
    }
    let writes = cycle_writes(model, cycle);
    loop {
        let mut grown = false;
        let mut mark = |r: usize, set: &mut Vec<bool>| {
            if !set[r] {
                set[r] = true;
                grown = true;
            }
        };
        for i in 0..width {
            if set[i] {
                for r in model.rates[i].reads() {
                    mark(r, &mut set);
                }
            }
        }
        for (slot, value, conds) in writes.iter().flatten() {
            if !set[*slot] {
                continue;
            }
            if let Value::Poly(t) = value {
                for r in t.poly.reads() {
                    mark(r, &mut set);
                }
            }
            for (cond, _) in conds {
                let mut reads = Vec::new();
                cond_reads(cond, &mut reads);
                for r in reads {
                    mark(r, &mut set);
                }
            }
        }
        if !grown {
            return (0..width).filter(|i| set[*i]).collect();
        }
    }
}

/// The equations a scaling's weights satisfy, each `Σ cᵢ·wᵢ + c_q·q = 0`: every rate of a scaled
/// slot is homogeneous of its weight less `q`, every guard atom and every condition a scaled
/// slot's statement runs under is homogeneous, and every statement setting a scaled slot is
/// homogeneous of its weight.
fn homogeneity(model: &Model, cycle: &[Step], slots: &[usize]) -> Vec<(Vec<i64>, i64)> {
    let monomial = |e: &[u32]| -> Vec<i64> { e.iter().map(|k| i64::from(*k)).collect() };
    let mut out = Vec::new();
    let same = |p: &MPoly, out: &mut Vec<(Vec<i64>, i64)>| {
        for pair in p.terms().windows(2) {
            let (a, b) = (monomial(&pair[0].0), monomial(&pair[1].0));
            out.push((a.iter().zip(&b).map(|(x, y)| x - y).collect(), 0));
        }
    };
    for &i in slots {
        for (e, _) in model.rates[i].terms() {
            let mut c = monomial(e);
            c[i] -= 1;
            out.push((c, 1));
        }
    }
    for j in &model.jumps {
        let mut terms = Vec::new();
        atom_terms(&j.guard, &mut terms);
        terms.iter().for_each(|t| same(&t.poly, &mut out));
    }
    for (slot, value, conds) in cycle_writes(model, cycle).iter().flatten() {
        if !slots.contains(slot) {
            continue;
        }
        if let Value::Poly(t) = value {
            for (e, _) in t.poly.terms() {
                let mut c = monomial(e);
                c[*slot] -= 1;
                out.push((c, 0));
            }
        }
        for (cond, _) in conds {
            let mut terms = Vec::new();
            atom_terms(cond, &mut terms);
            terms.iter().for_each(|t| same(&t.poly, &mut out));
        }
    }
    out
}

/// The certificate a tail rests on, checked on its own: the slots are closed under what feeds
/// them, the field, the guards and the cycle's statements setting them are homogeneous for the
/// weights, `after` is the scaling's image of `before`, and each flight of the round after is
/// `s^q` times the same flight of the round before. A run whose state the scaling maps to its state
/// a round later is then, round after round, the scaled copy of the round before. A scaling by one
/// is the identity, which every model is homogeneous for: its certificate is that the state
/// repeats.
pub fn check_scaling(
    model: &Model,
    scaling: &Scaling,
    before: &[Cell],
    after: &[Cell],
    flights_before: &[Algebraic],
    flights_after: &[Algebraic],
) -> Result<(), String> {
    if scaling.cycle.is_empty() {
        return Err("a scaling maps a round of at least one firing".to_string());
    }
    if scaling.q < 1 {
        return Err("a scaling's time weight is a positive whole number".to_string());
    }
    if scaling.s.signum() <= 0 {
        return Err("a scaling's factor is positive".to_string());
    }
    if scaling.weights.len() != scaling.slots.len() {
        return Err("a scaling weighs every slot it covers".to_string());
    }
    if scaled_slots(model, &scaling.cycle) != scaling.slots {
        return Err("a scaling covers exactly the slots the guards read and what feeds them".to_string());
    }
    let identity = scaling.s == alg(1);
    for (c, cq) in homogeneity(model, &scaling.cycle, &scaling.slots).into_iter().filter(|_| !identity) {
        let mut total = cq * scaling.q;
        for (i, ci) in c.iter().enumerate() {
            if *ci == 0 {
                continue;
            }
            match scaling.weight(i) {
                Some(w) => total += ci * w,
                None => return Err("a scaled slot is fed by a slot the scaling does not cover".to_string()),
            }
        }
        if total != 0 {
            return Err("the field, a guard or a reset is not homogeneous for the scaling's weights".to_string());
        }
    }
    if !scaling.image(before, after) {
        return Err("the state after is not the scaling's image of the state before".to_string());
    }
    if flights_before.len() != scaling.cycle.len() || flights_after.len() != scaling.cycle.len() {
        return Err("a scaling maps one flight before each firing of its round".to_string());
    }
    let step = power(&scaling.s, scaling.q);
    if flights_before.iter().zip(flights_after).any(|(b, a)| *a != step.mul(b)) {
        return Err("a flight after is not the scaling's image of the flight before".to_string());
    }
    Ok(())
}

/// `p/q` with `r = σ^(p/q)` exactly, `q` at most [`MAX_WEIGHT_DENOMINATOR`] and `|p|` at most
/// [`MAX_WEIGHT_NUMERATOR`].
fn exponent_ratio(r: &Algebraic, sigma: &Algebraic) -> Option<Rational> {
    let target = r.to_f64().ln() / sigma.to_f64().ln();
    if !target.is_finite() {
        return None;
    }
    let (mut h0, mut h1, mut k0, mut k1) = (0i64, 1i64, 1i64, 0i64);
    let mut x = target;
    for _ in 0..40 {
        let a = x.floor();
        if a.abs() > 1e9 {
            return None;
        }
        let a = a as i64;
        let h = a.checked_mul(h1)?.checked_add(h0)?;
        let k = a.checked_mul(k1)?.checked_add(k0)?;
        if k > MAX_WEIGHT_DENOMINATOR || h.abs() > MAX_WEIGHT_NUMERATOR {
            return None;
        }
        if k > 0 && power(r, k) == power(sigma, h) {
            return Rational::from_ratio_i64(h, k);
        }
        (h0, h1, k0, k1) = (h1, h, k1, k);
        let frac = x - a as f64;
        if frac.abs() < 1e-12 {
            return None;
        }
        x = 1.0 / frac;
    }
    None
}

fn lcm(a: i64, b: i64) -> i64 {
    let (mut x, mut y) = (a, b);
    while y != 0 {
        (x, y) = (y, x % y);
    }
    a / x * b
}

/// The scaling that maps `before` to `after` over a round whose length is multiplied by `sigma`,
/// its weights read off the ratios of the values and, for a value zero at both, off the
/// homogeneity the scaling must have; `None` when no scaling does.
fn find_scaling(
    model: &Model,
    cycle: &[Step],
    slots: &[usize],
    before: &[Cell],
    after: &[Cell],
    sigma: &Algebraic,
) -> Option<Scaling> {
    let one = alg(1);
    if *sigma == one {
        let same = slots.iter().all(|i| match (&before[*i], &after[*i]) {
            (Cell::Num { value: x, .. }, Cell::Num { value: y, .. }) => x == y,
            (b, a) => b == a,
        });
        return same.then(|| Scaling {
            cycle: cycle.to_vec(),
            slots: slots.to_vec(),
            weights: vec![0; slots.len()],
            q: 1,
            s: one,
        });
    }
    let mut ratio: Vec<Option<Rational>> = Vec::with_capacity(slots.len());
    for i in slots {
        match (&before[*i], &after[*i]) {
            (Cell::Num { value: x, .. }, Cell::Num { value: y, .. }) => {
                if x.is_zero() {
                    if !y.is_zero() {
                        return None;
                    }
                    ratio.push(None);
                } else {
                    let r = quotient(y, x);
                    if r.signum() <= 0 {
                        return None;
                    }
                    ratio.push(Some(exponent_ratio(&r, sigma)?));
                }
            }
            (b, a) => {
                if b != a {
                    return None;
                }
                ratio.push(Some(Rational::zero()));
            }
        }
    }
    let constraints = homogeneity(model, cycle, slots);
    let at = |slot: usize| slots.iter().position(|s| *s == slot);
    loop {
        let mut solved = false;
        for (c, cq) in &constraints {
            let unknown: Vec<usize> = (0..c.len())
                .filter(|i| c[*i] != 0 && at(*i).is_some_and(|k| ratio[k].is_none()))
                .collect();
            if let [u] = unknown.as_slice() {
                let mut sum = Rational::from_i64(*cq);
                for (i, ci) in c.iter().enumerate() {
                    if *ci != 0 && i != *u {
                        sum = sum.add(&Rational::from_i64(*ci).mul(ratio[at(i)?].as_ref()?));
                    }
                }
                ratio[at(*u)?] = sum.negated().div(&Rational::from_i64(c[*u]));
                solved = true;
            }
        }
        if !solved {
            break;
        }
    }
    let ratio: Vec<Rational> = ratio.into_iter().map(|r| r.unwrap_or_else(Rational::zero)).collect();
    let mut q = 1i64;
    for r in &ratio {
        q = lcm(q, r.denominator().to_i64()?);
    }
    let weights = ratio
        .iter()
        .map(|r| r.mul(&Rational::from_i64(q)).to_bigint().and_then(|b| b.to_i64()))
        .collect::<Option<Vec<i64>>>()?;
    let s = sigma.nth_root(u32::try_from(q).ok()?)?;
    Some(Scaling {
        cycle: cycle.to_vec(),
        slots: slots.to_vec(),
        weights,
        q,
        s,
    })
}

/// A value along a tail: a polynomial in `z = sᵏ` plus a multiple of the round index `k`.
#[derive(Clone, Debug, PartialEq, Eq)]
struct TailVal {
    p: UPoly,
    lin: Algebraic,
}

impl TailVal {
    fn constant(a: Algebraic) -> TailVal {
        TailVal { p: UPoly::constant(a), lin: alg(0) }
    }

    fn is_zero(&self) -> bool {
        self.p.is_zero() && self.lin.is_zero()
    }

    fn is_constant(&self) -> bool {
        self.p.as_constant().is_some() && self.lin.is_zero()
    }

    fn at(&self, s: &Algebraic, k: usize) -> Algebraic {
        self.p.eval(&power(s, k as i64)).add(&self.lin.mul(&alg(k as i64)))
    }

    /// A value computed from the state at the end of one round, as a value of the round it is
    /// computed in.
    fn next_round(&self, s: &Algebraic) -> TailVal {
        TailVal {
            p: self
                .p
                .dilate(&s.recip().expect("a scaling's factor is positive"))
                .sub(&UPoly::constant(self.lin.clone())),
            lin: self.lin.clone(),
        }
    }
}

impl Ring for TailVal {
    fn lift(a: &Algebraic) -> Self {
        TailVal::constant(a.clone())
    }
    fn plus(&self, o: &Self) -> Result<Self, String> {
        Ok(TailVal {
            p: self.p.add(&o.p),
            lin: self.lin.add(&o.lin),
        })
    }
    fn times(&self, o: &Self) -> Result<Self, String> {
        if self.lin.is_zero() && o.lin.is_zero() {
            return Ok(TailVal {
                p: self.p.mul(&o.p),
                lin: alg(0),
            });
        }
        let (c, v) = match (self.p.as_constant(), o.p.as_constant()) {
            (Some(c), _) if self.lin.is_zero() => (c, o),
            (_, Some(c)) if o.lin.is_zero() => (c, self),
            _ => {
                return Err(
                    "a count of the jumps multiplies a value that changes with them, so the values along the jumps are not polynomials in their scaling"
                        .to_string(),
                )
            }
        };
        Ok(TailVal {
            p: v.p.scale(&c),
            lin: v.lin.mul(&c),
        })
    }
}

/// A polynomial in one unknown value with coefficients along a tail.
#[derive(Clone, Debug)]
struct YPoly(Vec<TailVal>);

impl YPoly {
    fn trimmed(mut self) -> YPoly {
        while self.0.last().is_some_and(TailVal::is_zero) {
            self.0.pop();
        }
        self
    }
}

impl Ring for YPoly {
    fn lift(a: &Algebraic) -> Self {
        YPoly(vec![TailVal::constant(a.clone())]).trimmed()
    }
    fn plus(&self, o: &Self) -> Result<Self, String> {
        let n = self.0.len().max(o.0.len());
        let zero = TailVal::constant(alg(0));
        let mut c = Vec::with_capacity(n);
        for k in 0..n {
            c.push(self.0.get(k).unwrap_or(&zero).plus(o.0.get(k).unwrap_or(&zero))?);
        }
        Ok(YPoly(c).trimmed())
    }
    fn times(&self, o: &Self) -> Result<Self, String> {
        if self.0.is_empty() || o.0.is_empty() {
            return Ok(YPoly(Vec::new()));
        }
        let mut c = vec![TailVal::constant(alg(0)); self.0.len() + o.0.len() - 1];
        for (i, a) in self.0.iter().enumerate() {
            for (j, b) in o.0.iter().enumerate() {
                c[i + j] = c[i + j].plus(&a.times(b)?)?;
            }
        }
        Ok(YPoly(c).trimmed())
    }
}

/// A slot's value along a tail, from the round it holds from.
#[derive(Clone, Debug)]
struct TailSlot {
    val: TailVal,
    from: usize,
}

enum TailSolve {
    Values(Vec<Option<TailSlot>>),
    /// The values hold only from a round the run has not reached yet.
    Wait,
    Unsupported(String),
}

fn union(sets: impl Iterator<Item = Vec<usize>>) -> Vec<usize> {
    let mut out: Vec<usize> = sets.flatten().collect();
    out.sort_unstable();
    out.dedup();
    out
}

/// The slots each slot's value at the end of a round depends on, at the round's start.
fn round_dependencies(series: &[Vec<MPoly>], cycle: &[Vec<PathWrite<'_>>]) -> Vec<Vec<usize>> {
    let width = series.len();
    let flown: Vec<Vec<usize>> = series
        .iter()
        .map(|derivatives| union(derivatives.iter().map(MPoly::reads)))
        .collect();
    let mut deps: Vec<Vec<usize>> = (0..width).map(|i| vec![i]).collect();
    for writes in cycle {
        deps = (0..width)
            .map(|i| union(flown[i].iter().map(|r| deps[*r].clone())))
            .collect();
        for (slot, value, _) in writes {
            deps[*slot] = match value {
                Value::Poly(t) => union(t.poly.reads().into_iter().map(|r| deps[r].clone())),
                Value::Fixed(_) => Vec::new(),
            };
        }
    }
    deps
}

/// `x` flown for `tau`.
fn fly<R: Ring>(series: &[Vec<MPoly>], x: &[R], tau: &R) -> Result<Vec<R>, String> {
    let mut out = Vec::with_capacity(series.len());
    for derivatives in series {
        let mut total = R::lift(&alg(0));
        let mut t_power = R::lift(&alg(1));
        for (m, d) in derivatives.iter().enumerate() {
            let term = d
                .eval(x)?
                .times(&t_power)?
                .times(&R::lift(&quotient(&alg(1), &factorial(m))))?;
            total = total.plus(&term)?;
            t_power = t_power.times(tau)?;
        }
        out.push(total);
    }
    Ok(out)
}

/// The values at the end of a round from the values at its start, `x`, each firing after a
/// flight of `taus[r]`.
fn round_map<R: Ring>(series: &[Vec<MPoly>], cycle: &[Vec<PathWrite<'_>>], x: &[R], taus: &[R]) -> Result<Vec<R>, String> {
    let mut cur = x.to_vec();
    for (writes, tau) in cycle.iter().zip(taus) {
        cur = fly(series, &cur, tau)?;
        for (slot, value, _) in writes {
            if let Value::Poly(t) = value {
                cur[*slot] = t.poly.eval(&cur)?;
            }
        }
    }
    Ok(cur)
}

/// The value of every numeric slot at the end of every round of a tail, as a polynomial in
/// `z = sᵏ` plus a multiple of `k`, rounds counted from the first of `known`.
fn tail_values(
    model: &Model,
    series: &[Vec<MPoly>],
    scaling: &Scaling,
    known: &[Vec<Cell>],
    flights0: &[Algebraic],
) -> TailSolve {
    let width = model.rates.len();
    let m = known.len() - 1;
    let base = &known[0];
    let s = &scaling.s;
    let one = alg(1);
    let mut out: Vec<Option<TailSlot>> = vec![None; width];
    for (i, w) in scaling.slots.iter().zip(&scaling.weights) {
        if let Cell::Num { value, .. } = &base[*i] {
            if value.is_zero() {
                out[*i] = Some(TailSlot { val: TailVal::constant(alg(0)), from: 0 });
            } else if *w < 0 {
                return TailSolve::Unsupported("and a value the guards read grows without bound as they do".to_string());
            } else {
                out[*i] = Some(TailSlot {
                    val: TailVal {
                        p: UPoly::monomial(value.clone(), *w as usize),
                        lin: alg(0),
                    },
                    from: 0,
                });
            }
        }
    }
    let writes = cycle_writes(model, &scaling.cycle);
    let deps = round_dependencies(series, &writes);
    let mut pending: Vec<usize> = (0..width)
        .filter(|i| matches!(base[*i], Cell::Num { .. }) && out[*i].is_none())
        .collect();
    let taus: Vec<YPoly> = flights0
        .iter()
        .map(|f| {
            YPoly(vec![TailVal {
                p: UPoly::monomial(f.clone(), scaling.q as usize),
                lin: alg(0),
            }])
        })
        .collect();
    while !pending.is_empty() {
        let Some(at) = pending
            .iter()
            .position(|k| deps[*k].iter().all(|d| *d == *k || !pending.contains(d)))
        else {
            return TailSolve::Unsupported(
                "and values that feed each other through the jumps change along them in a way the engine does not sum".to_string(),
            );
        };
        let k = pending.remove(at);
        let x: Vec<YPoly> = (0..width)
            .map(|i| {
                if i == k {
                    YPoly(vec![TailVal::constant(alg(0)), TailVal::constant(alg(1))])
                } else {
                    match &out[i] {
                        Some(t) => YPoly(vec![t.val.clone()]).trimmed(),
                        None => YPoly(Vec::new()),
                    }
                }
            })
            .collect();
        let next = match round_map(series, &writes, &x, &taus) {
            Ok(v) => v[k].clone(),
            Err(why) => return TailSolve::Unsupported(format!("and {why}")),
        };
        let inputs_from = deps[k]
            .iter()
            .filter(|d| **d != k)
            .filter_map(|d| out[*d].as_ref().map(|t| t.from))
            .max()
            .unwrap_or(0);
        let zero = TailVal::constant(alg(0));
        let c0 = next.0.first().cloned().unwrap_or_else(|| zero.clone());
        let c1 = next.0.get(1).cloned().unwrap_or_else(|| zero.clone());
        if next.0.len() > 2 {
            return TailSolve::Unsupported("and a value the jumps raise to a power changes with them".to_string());
        }
        if !c1.is_constant() {
            return TailSolve::Unsupported(
                "and a value the jumps multiply by a changing factor changes with them".to_string(),
            );
        }
        let a = c1.p.coeff(0);
        let anchor = |i0: usize| -> Option<Algebraic> {
            (i0 <= m).then(|| known[i0][k].number().cloned().unwrap_or_else(|| alg(0)))
        };
        let solved = if a.is_zero() {
            TailSlot {
                val: c0.next_round(s),
                from: inputs_from + 1,
            }
        } else if a == one {
            if !c0.lin.is_zero() {
                return TailSolve::Unsupported("and a value that adds up a count of the jumps grows without bound".to_string());
            }
            let Some(start) = anchor(inputs_from) else { return TailSolve::Wait };
            let i0 = inputs_from;
            if *s == one {
                let step = c0.p.eval(&one);
                TailSlot {
                    val: TailVal {
                        p: UPoly::constant(start.sub(&step.mul(&alg(i0 as i64)))),
                        lin: step,
                    },
                    from: i0,
                }
            } else {
                let lin = c0.p.coeff(0);
                let mut coeffs = vec![alg(0)];
                let z0 = power(s, i0 as i64);
                let mut kappa = start.sub(&lin.mul(&alg(i0 as i64)));
                for d in 1..=c0.p.degree().unwrap_or(0) {
                    let c = quotient(&c0.p.coeff(d), &power(s, d as i64).sub(&one));
                    kappa = kappa.sub(&c.mul(&power(&z0, d as i64)));
                    coeffs.push(c);
                }
                coeffs[0] = kappa;
                TailSlot {
                    val: TailVal {
                        p: UPoly::from_coeffs(coeffs),
                        lin,
                    },
                    from: i0,
                }
            }
        } else {
            let Some(mu) = scale_exponent(&a, s) else {
                return TailSolve::Unsupported(
                    "and a value the jumps multiply by a factor their scaling does not produce changes with them".to_string(),
                );
            };
            if !c0.lin.is_zero() || !c0.p.coeff(mu).is_zero() {
                return TailSolve::Unsupported(
                    "and a value the jumps multiply by their own scaling is driven at that same rate, so it is not a polynomial in the scaling".to_string(),
                );
            }
            let Some(start) = anchor(inputs_from) else { return TailSolve::Wait };
            let i0 = inputs_from;
            let s_mu = power(s, mu as i64);
            let mut coeffs: Vec<Algebraic> = (0..=c0.p.degree().unwrap_or(0).max(mu))
                .map(|d| {
                    if d == mu {
                        alg(0)
                    } else {
                        quotient(&c0.p.coeff(d), &power(s, d as i64).sub(&s_mu))
                    }
                })
                .collect();
            let z0 = power(s, i0 as i64);
            let particular = UPoly::from_coeffs(coeffs.clone()).eval(&z0);
            coeffs[mu] = quotient(&start.sub(&particular), &power(&z0, mu as i64));
            TailSlot {
                val: TailVal {
                    p: UPoly::from_coeffs(coeffs),
                    lin: alg(0),
                },
                from: i0,
            }
        };
        out[k] = Some(solved);
    }
    if out.iter().flatten().any(|t| t.from > m + 1) {
        return TailSolve::Wait;
    }
    TailSolve::Values(out)
}

/// The `μ ≥ 1` with `sᵘ = a`.
fn scale_exponent(a: &Algebraic, s: &Algebraic) -> Option<usize> {
    if a.signum() <= 0 || *s == alg(1) {
        return None;
    }
    let estimate = (a.to_f64().ln() / s.to_f64().ln()).round();
    if !estimate.is_finite() || !(1.0..=4096.0).contains(&estimate) {
        return None;
    }
    let mu = estimate as usize;
    (mu.saturating_sub(1)..=mu + 1)
        .filter(|m| *m >= 1)
        .find(|m| power(s, *m as i64) == *a)
}

/// The conditions a firing along `path` meets, in order, each with the branch it takes.
fn path_conditions<'b>(block: &'b [Stmt], path: &[bool]) -> Vec<(&'b Cond, bool)> {
    fn walk<'b>(stmts: &'b [Stmt], bits: &mut std::slice::Iter<'_, bool>, out: &mut Vec<(&'b Cond, bool)>) {
        for s in stmts {
            if let Stmt::If { cond, then, otherwise } = s {
                let taken = bits.next().copied().unwrap_or(false);
                out.push((cond, taken));
                walk(if taken { then } else { otherwise }, bits, out);
            }
        }
    }
    let mut out = Vec::new();
    walk(block, &mut path.iter(), &mut out);
    out
}

/// For each firing of a round, each condition it meets, each atom's value along the tail as of
/// the round it is met in.
fn tail_conditions(
    model: &Model,
    series: &[Vec<MPoly>],
    scaling: &Scaling,
    values: &[Option<TailSlot>],
    flights0: &[Algebraic],
) -> Result<Vec<Vec<Vec<TailVal>>>, String> {
    let width = model.rates.len();
    let mut cur: Vec<TailVal> = (0..width)
        .map(|i| values[i].as_ref().map(|t| t.val.clone()).unwrap_or_else(|| TailVal::constant(alg(0))))
        .collect();
    fn walk(
        stmts: &[Stmt],
        bits: &mut std::slice::Iter<'_, bool>,
        cur: &mut Vec<TailVal>,
        out: &mut Vec<Vec<TailVal>>,
        s: &Algebraic,
    ) -> Result<(), String> {
        for st in stmts {
            match st {
                Stmt::Set { slot, value } => {
                    if let Value::Poly(t) = value {
                        cur[*slot] = t.poly.eval(cur)?;
                    }
                }
                Stmt::If { cond, then, otherwise } => {
                    let mut terms = Vec::new();
                    atom_terms(cond, &mut terms);
                    out.push(
                        terms
                            .iter()
                            .map(|t| t.poly.eval(cur).map(|v| v.next_round(s)))
                            .collect::<Result<Vec<TailVal>, String>>()?,
                    );
                    let taken = bits.next().copied().unwrap_or(false);
                    walk(if taken { then } else { otherwise }, bits, cur, out, s)?;
                }
            }
        }
        Ok(())
    }
    let mut rounds = Vec::with_capacity(scaling.cycle.len());
    for ((j, path), flight) in scaling.cycle.iter().zip(flights0) {
        let tau = TailVal {
            p: UPoly::monomial(flight.clone(), scaling.q as usize),
            lin: alg(0),
        };
        cur = fly(series, &cur, &tau)?;
        let mut out = Vec::new();
        walk(&model.jumps[*j].block, &mut path.iter(), &mut cur, &mut out, &scaling.s)?;
        rounds.push(out);
    }
    Ok(rounds)
}

/// The first `k ≥ from` with `pred(sᵏ)`, for a `pred` that, once it holds, holds for every later
/// `k`; `None` when it does not hold by an index the run can reach.
fn first_index(s: &Algebraic, from: usize, pred: &dyn Fn(&Algebraic) -> bool) -> Option<usize> {
    if pred(&power(s, from as i64)) {
        return Some(from);
    }
    let mut lo = from;
    let mut hi = from.max(1);
    loop {
        hi = hi.checked_mul(2)?;
        if hi > 1 << 22 {
            return None;
        }
        if pred(&power(s, hi as i64)) {
            break;
        }
        lo = hi;
    }
    while hi - lo > 1 {
        let mid = lo + (hi - lo) / 2;
        if pred(&power(s, mid as i64)) {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    Some(hi)
}

/// The indices `k ≥ from` at which the sign of `f(k)` may differ from its sign at `k − 1`; `None`
/// when they are not bounded within the run's reach.
fn sign_changes(f: &TailVal, s: &Algebraic, from: usize) -> Option<Vec<usize>> {
    if f.is_zero() {
        return Some(Vec::new());
    }
    let one = alg(1);
    let around = |k0: &Algebraic| -> Vec<usize> {
        let floor = k0.floor().to_i64().unwrap_or(i64::MAX).max(0);
        (floor..=floor.saturating_add(2))
            .filter_map(|k| usize::try_from(k).ok())
            .filter(|k| *k >= from)
            .collect()
    };
    if *s == one {
        if f.lin.is_zero() {
            return Some(Vec::new());
        }
        return Some(around(&quotient(&f.p.eval(&one).neg(), &f.lin)));
    }
    if f.lin.is_zero() {
        let mut out = Vec::new();
        for r in real_roots(&f.p) {
            if r.signum() <= 0 {
                continue;
            }
            let k = if *s < one {
                first_index(s, from, &|z| *z <= r)?
            } else {
                first_index(s, from, &|z| *z >= r)?
            };
            out.extend([k, k + 1]);
        }
        return Some(out);
    }
    let bound = if *s < one {
        let b = f.p.coeffs().iter().fold(alg(0), |acc, c| acc.add(&magnitude(c)));
        quotient(&b, &magnitude(&f.lin)).floor().to_i64()? + 1
    } else {
        let d = f.p.degree().unwrap_or(0);
        if d == 0 {
            return Some(around(&quotient(&f.p.coeff(0).neg(), &f.lin)));
        }
        let lead = magnitude(&f.p.coeff(d));
        let lower = (0..d).fold(alg(0), |acc, k| acc.add(&magnitude(&f.p.coeff(k))));
        let drift = quotient(&magnitude(&f.lin), &s.sub(&one));
        let threshold = if d >= 2 {
            quotient(&lower.add(&drift), &lead)
        } else {
            let margin = lead.sub(&drift);
            if margin.signum() <= 0 {
                return None;
            }
            quotient(&lower, &margin)
        };
        let threshold = threshold.max(one);
        first_index(s, from, &|z| *z > threshold)? as i64
    };
    let bound = usize::try_from(bound).ok()?;
    if bound.saturating_sub(from) > EVENT_BUDGET {
        return None;
    }
    Some((from..=bound.max(from)).collect())
}

impl<'m> Run<'m> {
    /// After a firing: when the last firings repeat a cycle of one to [`MAX_CYCLE`] firings and a
    /// scaling maps each round to the next, the rest of the run along it is read off the tail —
    /// to the round where a condition of a firing turns, to the loop's last round, or to the time
    /// the firings accumulate at.
    fn tail(&mut self) -> Result<TailMove, String> {
        let n = self.history.len();
        for period in 1..=MAX_CYCLE {
            if n < 2 * period + 1 {
                break;
            }
            let cycle: Vec<Step> = self.history[n - period..]
                .iter()
                .map(|e| (e.jump, e.firing.path.clone()))
                .collect();
            let repeats = (0..period).all(|r| {
                let e = &self.history[n - 2 * period + r];
                e.jump == cycle[r].0 && e.firing.path == cycle[r].1
            });
            if !repeats {
                continue;
            }
            if let Some(found) = self.cycle_tail(period, cycle)? {
                return Ok(found);
            }
        }
        Ok(TailMove::None)
    }

    /// The tail of a cycle of `period` firings, when a scaling certifies it.
    fn cycle_tail(&mut self, period: usize, cycle: Vec<Step>) -> Result<Option<TailMove>, String> {
        let n = self.history.len();
        let (ia, ib, ic) = (n - 1 - 2 * period, n - 1 - period, n - 1);
        let flights = |from: usize| -> Vec<Algebraic> {
            (from..from + period)
                .map(|i| self.history[i + 1].at.sub(&self.history[i].at))
                .collect()
        };
        let total = |f: &[Algebraic]| f.iter().fold(alg(0), |acc, x| acc.add(x));
        let (fa, fb) = (flights(ia), flights(ib));
        let sigma = quotient(&total(&fb), &total(&fa));
        let slots = scaled_slots(self.model, &cycle);
        let (a, b, c) = (
            &self.history[ia].firing.post,
            &self.history[ib].firing.post,
            &self.history[ic].firing.post,
        );
        let Some(scaling) = find_scaling(self.model, &cycle, &slots, a, b, &sigma) else {
            return Ok(None);
        };
        if check_scaling(self.model, &scaling, a, b, &fa, &fb).is_err() || !scaling.image(b, c) {
            return Ok(None);
        }
        let step = power(&scaling.s, scaling.q);
        let mut base = ia;
        while base >= period {
            let prev = base - period;
            let same_steps = (0..period).all(|r| {
                let e = &self.history[prev + 1 + r];
                e.jump == cycle[r].0 && e.firing.path == cycle[r].1
            });
            let fits = same_steps
                && scaling.image(&self.history[prev].firing.post, &self.history[base].firing.post)
                && flights(prev).iter().zip(&flights(base)).all(|(x, y)| *y == step.mul(x));
            if !fits {
                break;
            }
            base = prev;
        }
        let known: Vec<Vec<Cell>> = (base..n).step_by(period).map(|i| self.history[i].firing.post.clone()).collect();
        let conditions: Vec<Vec<Vec<Tri>>> = (n - period..n).map(|i| self.history[i].firing.conditions.clone()).collect();
        let facts = self.history[n - 1].firing.facts.clone();
        let at0 = self.history[base].at.clone();
        let flights0 = flights(base);
        self.resolve_tail(&scaling, &known, &conditions, facts, &at0, &flights0).map(Some)
    }

    fn resolve_tail(
        &mut self,
        scaling: &Scaling,
        known: &[Vec<Cell>],
        conditions: &[Vec<Vec<Tri>>],
        facts: Facts,
        at0: &Algebraic,
        flights0: &[Algebraic],
    ) -> Result<TailMove, String> {
        let m = known.len() - 1;
        let one = alg(1);
        let round = flights0.iter().fold(alg(0), |acc, x| acc.add(x));
        let sigma = power(&scaling.s, scaling.q);
        let zeno = (sigma < one).then(|| at0.add(&quotient(&round, &one.sub(&sigma))));
        let accumulates_first = zeno.as_ref().is_some_and(|z| *z <= self.span);
        let values = match tail_values(self.model, &self.series, scaling, known, flights0) {
            TailSolve::Values(v) => v,
            TailSolve::Wait => return Ok(TailMove::None),
            TailSolve::Unsupported(why) => {
                return match &zeno {
                    Some(z) if accumulates_first => Err(format!(
                        "the loop's jumps fire infinitely often, accumulating {z} into it, {why}"
                    )),
                    _ => Ok(TailMove::None),
                }
            }
        };
        let round_at = |k: usize| -> Algebraic {
            if sigma == one {
                at0.add(&round.mul(&alg(k as i64)))
            } else {
                at0.add(&quotient(&round.mul(&one.sub(&power(&sigma, k as i64))), &one.sub(&sigma)))
            }
        };
        let Some(deviation) = self.first_deviation(scaling, &values, conditions, flights0, m)? else {
            return match &zeno {
                Some(z) if accumulates_first => Err(format!(
                    "the loop's jumps fire infinitely often, accumulating {z} into it, and whether a condition of a jump holds along them is not settled exactly"
                )),
                _ => Ok(TailMove::None),
            };
        };
        let last = if accumulates_first {
            None
        } else {
            match self.last_before_end(&sigma, at0, &round)? {
                Some(j) => Some(j),
                None => return Ok(TailMove::None),
            }
        };
        let target = match (deviation, last) {
            (Some(d), Some(j)) => Some((d - 1).min(j)),
            (Some(d), None) => Some(d - 1),
            (None, Some(j)) => Some(j),
            (None, None) => None,
        };
        match target {
            Some(k) => {
                if k <= m {
                    return Ok(TailMove::None);
                }
                self.cells = (0..self.width)
                    .map(|i| match (&known[m][i], &values[i]) {
                        (Cell::Num { exact, .. }, Some(t)) => Cell::Num {
                            value: t.val.at(&scaling.s, k),
                            exact: *exact,
                        },
                        (cell, _) => cell.clone(),
                    })
                    .collect();
                self.elapsed = round_at(k);
                self.facts = facts;
                self.history.clear();
                Ok(TailMove::Moved)
            }
            None => {
                let z = zeno.expect("only firings that accumulate leave no last round");
                let mut cells = Vec::with_capacity(self.width);
                for i in 0..self.width {
                    cells.push(match (&known[m][i], &values[i]) {
                        (Cell::Num { exact, .. }, Some(t)) => {
                            if !t.val.lin.is_zero() {
                                return Err(format!(
                                    "the loop's jumps fire infinitely often, accumulating {z} into it, and a value that counts them grows without bound"
                                ));
                            }
                            Cell::Num {
                                value: t.val.p.coeff(0),
                                exact: *exact && t.val.is_constant(),
                            }
                        }
                        (cell, _) => cell.clone(),
                    });
                }
                self.cells = cells;
                self.elapsed = z;
                self.facts = Facts::default();
                self.history.clear();
                Ok(if self.elapsed == self.span {
                    TailMove::Moved
                } else {
                    TailMove::Zeno(scaling.cycle.last().expect("a cycle has a firing").0)
                })
            }
        }
    }

    /// The first round after the run's last at which a condition of a firing no longer takes the
    /// branch the tail takes: `Some(None)` when none ever does, `None` when that is not settled
    /// within the run's reach.
    fn first_deviation(
        &self,
        scaling: &Scaling,
        values: &[Option<TailSlot>],
        conditions: &[Vec<Vec<Tri>>],
        flights0: &[Algebraic],
        m: usize,
    ) -> Result<Option<Option<usize>>, String> {
        let Ok(atoms) = tail_conditions(self.model, &self.series, scaling, values, flights0) else {
            return Ok(None);
        };
        let from = m + 1;
        let mut first: Option<usize> = None;
        for (r, (j, path)) in scaling.cycle.iter().enumerate() {
            let conds = path_conditions(&self.model.jumps[*j].block, path);
            for (c, ((cond, taken), along)) in conds.iter().zip(&atoms[r]).enumerate() {
                let mut candidates = vec![from];
                for f in along {
                    match sign_changes(f, &scaling.s, from) {
                        Some(ks) => candidates.extend(ks),
                        None => return Ok(None),
                    }
                }
                candidates.sort_unstable();
                candidates.dedup();
                let recorded = conditions
                    .get(r)
                    .and_then(|rc| rc.get(c))
                    .cloned()
                    .unwrap_or_default();
                for k in candidates {
                    if first.is_some_and(|f| k >= f) {
                        break;
                    }
                    let mut n = 0;
                    let verdict = decide(cond, &self.cells, &mut |a| {
                        let f = &along[n];
                        let structural = recorded.get(n).copied().unwrap_or(Tri::U);
                        n += 1;
                        let value = f.at(&scaling.s, k);
                        Ok(match a {
                            Atom::Compare(rel, _) => {
                                if f.is_zero() {
                                    structural
                                } else if value.is_zero() {
                                    Tri::U
                                } else {
                                    rel.verdict(Signs::of(value.signum()))
                                }
                            }
                            Atom::Infinitesimal(_) => {
                                if value.is_zero() {
                                    Tri::T
                                } else {
                                    Tri::F
                                }
                            }
                        })
                    })?;
                    let expected = if *taken { Tri::T } else { Tri::F };
                    if verdict != expected {
                        first = Some(k);
                        break;
                    }
                }
            }
        }
        Ok(Some(first))
    }

    /// The index of the tail's last round to end before the loop does, counted from its first;
    /// `None` when it lies beyond the run's reach. A round ending exactly at the loop's end is
    /// refused.
    fn last_before_end(&self, sigma: &Algebraic, at0: &Algebraic, round: &Algebraic) -> Result<Option<usize>, String> {
        let one = alg(1);
        let left = self.span.sub(at0);
        let k = if *sigma == one {
            match quotient(&left, round).ceil().to_i64().and_then(|c| usize::try_from(c).ok()) {
                Some(c) => c,
                None => return Ok(None),
            }
        } else {
            let threshold = one.sub(&quotient(&left.mul(&one.sub(sigma)), round));
            let found = if *sigma < one {
                first_index(sigma, 0, &|p| *p <= threshold)
            } else {
                first_index(sigma, 0, &|p| *p >= threshold)
            };
            match found {
                Some(k) => k,
                None => return Ok(None),
            }
        };
        let at_k = if *sigma == one {
            at0.add(&round.mul(&alg(k as i64)))
        } else {
            at0.add(&quotient(&round.mul(&one.sub(&power(sigma, k as i64))), &one.sub(sigma)))
        };
        if at_k == self.span {
            return Err(format!(
                "a jump of the loop fires at its last step, {} into it, so whether it fires before the loop ends depends on which infinitesimal dt is",
                self.span
            ));
        }
        Ok(Some(k.saturating_sub(1)))
    }
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_base/hybrid/tests.rs"]
mod tests;
