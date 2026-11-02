//! Expansions of the values of several infinitesimals in their smallest one.
//!
//! A value of `dx`, declared much smaller than `dt`, that is no rational function of a root of `dx`
//! over the values of `dt` — `√(dt + dx)`, the state a loop stepped by `dx` leaves — is the sum of its
//! expansion `Σ c_k·dx^(k/q)`, each coefficient a value of `dt` and of the infinitesimals larger than
//! it. An [`Expansion`] holds such a value as the operations that built it and computes coefficients on
//! demand, each once: a rational function of a root of `dx` by power-series division; sums and
//! products term by term; a reciprocal and a real root by the recurrences [`crate::germ`] reads one
//! infinitesimal's germs by. Every recurrence runs over any [`Coefficient`] ring, so the same
//! expansion serves exact coefficients and coefficients that are themselves expansions.
//!
//! Orders are integers in units of `s^(1/q)`, `q` the expansion's ramification; two expansions meet on
//! the least grid both lie on, each refined to it.

use std::fmt;
use std::sync::{Arc, Mutex};

use crate::algebraic::Algebraic;
use crate::multi::Multi;
use crate::numeric::Rational;
use crate::ordered_field::OrderedField;

/// Why a coefficient or an operation of an expansion is not given.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExpansionError {
    /// The reciprocal of zero.
    DivisionByZero,
    /// An even root of a negative value.
    NegativeRoot(u32),
    /// A coefficient's own refusal, in its own words.
    Coefficient(String),
}

impl fmt::Display for ExpansionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ExpansionError::DivisionByZero => f.write_str("Division by zero"),
            ExpansionError::NegativeRoot(n) => f.write_str(&crate::algebraic::negative_root(*n)),
            ExpansionError::Coefficient(why) => f.write_str(why),
        }
    }
}

/// The values an expansion's coefficients are: a field with the operations the recurrences ask for,
/// each refused in the coefficient's own words when it cannot be done.
pub trait Coefficient: Clone + fmt::Debug + Send + Sync + 'static {
    fn zero() -> Self;
    fn one() -> Self;
    fn of_rational(r: Rational) -> Self;
    fn add(&self, o: &Self) -> Result<Self, ExpansionError>;
    fn sub(&self, o: &Self) -> Result<Self, ExpansionError>;
    fn neg(&self) -> Result<Self, ExpansionError>;
    fn mul(&self, o: &Self) -> Result<Self, ExpansionError>;
    /// Whether the coefficient is zero; refused when that cannot be told.
    fn is_zero(&self) -> Result<bool, ExpansionError>;
    fn recip(&self) -> Result<Self, ExpansionError>;
    /// `−1`, `0` or `1`; refused when the sign cannot be told.
    fn sign(&self) -> Result<i32, ExpansionError>;
    /// The real `n`-th root, the nonnegative one for an even `n`.
    fn nth_root(&self, n: u32) -> Result<Self, ExpansionError>;
    /// `exp`, for an expansion's exponential.
    fn exp(&self) -> Result<Self, ExpansionError>;
    /// The natural logarithm of a positive coefficient, for an expansion's logarithm.
    fn ln(&self) -> Result<Self, ExpansionError>;
}

/// A source of coefficients defined outside the operations here: a value of one infinitesimal read
/// as a value of several, a flow's state.
pub trait Source<C>: fmt::Debug + Send + Sync {
    /// The coefficient of order `k`, `k` at least the floor the expansion was built with.
    fn coefficient(&self, k: i64) -> Result<C, ExpansionError>;
}

/// A value of several infinitesimals is the coefficient of an expansion in a smaller one, exactly.
impl Coefficient for Multi {
    fn zero() -> Self {
        <Multi as OrderedField>::zero()
    }
    fn one() -> Self {
        <Multi as OrderedField>::one()
    }
    fn of_rational(r: Rational) -> Self {
        Multi::Standard(Algebraic::from_rational(r))
    }
    fn add(&self, o: &Self) -> Result<Self, ExpansionError> {
        Ok(OrderedField::add(self, o))
    }
    fn sub(&self, o: &Self) -> Result<Self, ExpansionError> {
        Ok(OrderedField::sub(self, o))
    }
    fn neg(&self) -> Result<Self, ExpansionError> {
        Ok(OrderedField::neg(self))
    }
    fn mul(&self, o: &Self) -> Result<Self, ExpansionError> {
        Ok(OrderedField::mul(self, o))
    }
    fn is_zero(&self) -> Result<bool, ExpansionError> {
        Ok(OrderedField::is_zero(self))
    }
    fn recip(&self) -> Result<Self, ExpansionError> {
        OrderedField::recip(self).ok_or(ExpansionError::DivisionByZero)
    }
    fn sign(&self) -> Result<i32, ExpansionError> {
        Ok(OrderedField::signum(self))
    }
    fn nth_root(&self, n: u32) -> Result<Self, ExpansionError> {
        self.root(n).map_err(ExpansionError::Coefficient)
    }
    fn exp(&self) -> Result<Self, ExpansionError> {
        Err(ExpansionError::Coefficient(
            "the exponential of a value of several infinitesimals is a germ, not an exact value".to_string(),
        ))
    }
    fn ln(&self) -> Result<Self, ExpansionError> {
        Err(ExpansionError::Coefficient(
            "the logarithm of a value of several infinitesimals is a germ, not an exact value".to_string(),
        ))
    }
}

#[derive(Debug)]
enum Node<C: Coefficient> {
    /// A value constant in `s`.
    Constant(C),
    /// `num/den`, polynomials in `t = s^(1/q)` lowest power first, with `den = t^d·u` and `u(0) ≠ 0`:
    /// the power series of `num/u` shifted down by `d`.
    Fraction { num: Vec<C>, unit: Vec<C>, d: i64 },
    Add(Expansion<C>, Expansion<C>),
    Sub(Expansion<C>, Expansion<C>),
    Neg(Expansion<C>),
    Mul(Expansion<C>, Expansion<C>),
    /// `1/g` for `g` whose order and leading coefficient are known.
    Recip(Expansion<C>, i64, C),
    /// `g` on a grid `m` times finer.
    Refine(Expansion<C>, u32),
    /// The real `n`-th root of `g = c·t^v·(1 + …)`, `n` dividing `v`, with `ⁿ√c`.
    Root { g: Expansion<C>, n: u32, v: i64, c: C, lead: C },
    /// `exp(g)` for a limited `g`: `k·e_k = Σ_{j=1..k} j·g_j·e_{k−j}` from `e_0 = exp(g_0)`.
    Exp(Expansion<C>),
    /// `ln(g)` for `g` with no infinitesimal order and a positive leading coefficient `g_0`.
    Ln(Expansion<C>, C),
    Source(Arc<dyn Source<C>>),
}

/// An expansion in the smallest infinitesimal of a value. See the module documentation.
#[derive(Clone, Debug)]
pub struct Expansion<C: Coefficient> {
    node: Arc<Node<C>>,
    /// The lowest order that may be nonzero.
    floor: i64,
    /// Order `k` is the coefficient of `s^(k/q)`.
    q: u32,
    cache: Arc<Mutex<Vec<C>>>,
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
    u32::try_from(u64::from(a) / g * u64::from(b)).expect("an expansion's ramification fits")
}

impl<C: Coefficient> Expansion<C> {
    fn of(node: Node<C>, floor: i64, q: u32) -> Expansion<C> {
        Expansion {
            node: Arc::new(node),
            floor,
            q,
            cache: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// The expansion of `num/den`, polynomials in `s^(1/q)` with `den` nonzero.
    pub fn fraction(num: Vec<C>, den: Vec<C>, q: u32) -> Result<Expansion<C>, ExpansionError> {
        let mut d = 0usize;
        while d < den.len() && den[d].is_zero()? {
            d += 1;
        }
        if d == den.len() {
            return Err(ExpansionError::DivisionByZero);
        }
        let unit = den[d..].to_vec();
        let mut n0 = 0usize;
        while n0 < num.len() && num[n0].is_zero()? {
            n0 += 1;
        }
        let floor = n0 as i64 - d as i64;
        Ok(Expansion::of(Node::Fraction { num, unit, d: d as i64 }, floor, q))
    }

    /// A value constant in `s`: the coefficient itself at order `0`.
    pub fn constant(c: C) -> Expansion<C> {
        Expansion::of(Node::Constant(c), 0, 1)
    }

    /// The expansion a [`Source`] gives, lowest order `floor`, on the grid `q`.
    pub fn source(source: Arc<dyn Source<C>>, floor: i64, q: u32) -> Expansion<C> {
        Expansion::of(Node::Source(source), floor, q)
    }

    /// The ramification `q`: order `k` is the coefficient of `s^(k/q)`.
    pub fn ramification(&self) -> u32 {
        self.q
    }

    /// The lowest order that may be nonzero.
    pub fn floor(&self) -> i64 {
        self.floor
    }

    /// The same expansion on a grid `m` times finer.
    pub fn refined(&self, m: u32) -> Expansion<C> {
        if m == 1 {
            return self.clone();
        }
        Expansion::of(Node::Refine(self.clone(), m), self.floor * i64::from(m), self.q * m)
    }

    /// Both expansions on the least grid both lie on.
    fn met(&self, o: &Expansion<C>) -> (Expansion<C>, Expansion<C>, u32) {
        let q = lcm(self.q, o.q);
        (self.refined(q / self.q), o.refined(q / o.q), q)
    }

    /// The coefficient of `s^(k/q)`.
    pub fn coefficient(&self, k: i64) -> Result<C, ExpansionError> {
        if k < self.floor {
            return Ok(C::zero());
        }
        let at = (k - self.floor) as usize;
        if let Some(c) = self.cache.lock().expect("unpoisoned").get(at) {
            return Ok(c.clone());
        }
        let c = match self.node.as_ref() {
            Node::Constant(c) => {
                if k == 0 {
                    c.clone()
                } else {
                    C::zero()
                }
            }
            Node::Fraction { num, unit, d } => {
                let j = k + d;
                if j < 0 {
                    C::zero()
                } else {
                    let mut total = num.get(j as usize).cloned().unwrap_or_else(C::zero);
                    for (i, u) in unit.iter().enumerate().skip(1) {
                        let earlier = k - i as i64;
                        if earlier < self.floor {
                            break;
                        }
                        if !u.is_zero()? {
                            total = total.sub(&u.mul(&self.coefficient(earlier)?)?)?;
                        }
                    }
                    total.mul(&unit[0].recip()?)?
                }
            }
            Node::Add(a, b) => a.coefficient(k)?.add(&b.coefficient(k)?)?,
            Node::Sub(a, b) => a.coefficient(k)?.sub(&b.coefficient(k)?)?,
            Node::Neg(a) => a.coefficient(k)?.neg()?,
            Node::Mul(a, b) => {
                let mut total = C::zero();
                for i in a.floor..=(k - b.floor) {
                    let x = a.coefficient(i)?;
                    if x.is_zero()? {
                        continue;
                    }
                    total = total.add(&x.mul(&b.coefficient(k - i)?)?)?;
                }
                total
            }
            Node::Recip(g, order, lead) => {
                if k == -order {
                    lead.recip()?
                } else {
                    let mut total = C::zero();
                    for i in (-order)..k {
                        let gi = g.coefficient(order + k - i)?;
                        if gi.is_zero()? {
                            continue;
                        }
                        total = total.add(&self.coefficient(i)?.mul(&gi)?)?;
                    }
                    total.neg()?.mul(&lead.recip()?)?
                }
            }
            Node::Refine(g, m) => {
                let m = i64::from(*m);
                if k.rem_euclid(m) == 0 {
                    g.coefficient(k.div_euclid(m))?
                } else {
                    C::zero()
                }
            }
            Node::Root { g, n, v, c, lead } => {
                let j = k - self.floor;
                if j == 0 {
                    lead.clone()
                } else {
                    let alpha = Rational::from_ratio_i64(1, i64::from(*n)).expect("n ≥ 1").add(&Rational::one());
                    let mut total = C::zero();
                    for i in 1..=j {
                        let gi = g.coefficient(v + i)?;
                        if gi.is_zero()? {
                            continue;
                        }
                        let weight = alpha.mul(&Rational::from_i64(i)).sub(&Rational::from_i64(j));
                        total = total.add(&C::of_rational(weight).mul(&gi)?.mul(&self.coefficient(k - i)?)?)?;
                    }
                    total.mul(&C::of_rational(Rational::from_i64(j)).mul(c)?.recip()?)?
                }
            }
            Node::Exp(g) => {
                if k == 0 {
                    g.coefficient(0)?.exp()?
                } else {
                    let mut total = C::zero();
                    for j in 1..=k {
                        let gj = g.coefficient(j)?;
                        if gj.is_zero()? {
                            continue;
                        }
                        total = total.add(&C::of_rational(Rational::from_i64(j)).mul(&gj)?.mul(&self.coefficient(k - j)?)?)?;
                    }
                    total.mul(&C::of_rational(Rational::from_ratio_i64(1, k).expect("k ≥ 1")))?
                }
            }
            Node::Ln(g, g0) => {
                if k == 0 {
                    g0.ln()?
                } else {
                    let mut inner = C::zero();
                    for j in 1..k {
                        let gk = g.coefficient(k - j)?;
                        if gk.is_zero()? {
                            continue;
                        }
                        inner = inner.add(&C::of_rational(Rational::from_i64(j)).mul(&self.coefficient(j)?)?.mul(&gk)?)?;
                    }
                    let inner = inner.mul(&C::of_rational(Rational::from_ratio_i64(1, k).expect("k ≥ 1")))?;
                    g.coefficient(k)?.sub(&inner)?.mul(&g0.recip()?)?
                }
            }
            Node::Source(source) => source.coefficient(k)?,
        };
        let mut cache = self.cache.lock().expect("unpoisoned");
        if cache.len() == at {
            cache.push(c.clone());
        }
        Ok(c)
    }

    pub fn add(&self, o: &Expansion<C>) -> Expansion<C> {
        let (a, b, q) = self.met(o);
        let floor = a.floor.min(b.floor);
        Expansion::of(Node::Add(a, b), floor, q)
    }

    pub fn neg(&self) -> Expansion<C> {
        Expansion::of(Node::Neg(self.clone()), self.floor, self.q)
    }

    /// The difference coefficient by coefficient, so a term both sides share cancels in the
    /// coefficient's own arithmetic.
    pub fn sub(&self, o: &Expansion<C>) -> Expansion<C> {
        let (a, b, q) = self.met(o);
        let floor = a.floor.min(b.floor);
        Expansion::of(Node::Sub(a, b), floor, q)
    }

    pub fn mul(&self, o: &Expansion<C>) -> Expansion<C> {
        let (a, b, q) = self.met(o);
        let floor = a.floor + b.floor;
        Expansion::of(Node::Mul(a, b), floor, q)
    }

    /// The first nonzero term at or below the order `through`: its order and coefficient; `None`
    /// when every coefficient through it is zero.
    pub fn leading_through(&self, through: i64) -> Result<Option<(i64, C)>, ExpansionError> {
        for k in self.floor..=through {
            let c = self.coefficient(k)?;
            if !c.is_zero()? {
                return Ok(Some((k, c)));
            }
        }
        Ok(None)
    }

    /// `1/self` for a value whose leading term is `lead·s^(order/q)`.
    pub fn recip(&self, order: i64, lead: C) -> Expansion<C> {
        Expansion::of(Node::Recip(self.clone(), order, lead), -order, self.q)
    }

    /// `exp(self)` for a value with no negative order.
    pub fn exp(&self) -> Expansion<C> {
        Expansion::of(Node::Exp(self.clone()), 0, self.q)
    }

    /// `ln(self)` for a value with no infinitesimal order whose coefficient of order `0` is `g0`.
    pub fn ln(&self, g0: C) -> Expansion<C> {
        Expansion::of(Node::Ln(self.clone(), g0), 0, self.q)
    }

    /// The real `n`-th root (`n ≥ 2`) of a value whose leading term is `c·s^(v/q)`, the nonnegative
    /// one for an even `n`; refused for a negative value and an even `n`. The grid is refined until `n`
    /// divides the leading order.
    pub fn root(&self, n: u32, v: i64, c: C) -> Result<Expansion<C>, ExpansionError> {
        assert!(n >= 2, "a root of index 2 or more");
        if n % 2 == 0 && c.sign()? < 0 {
            return Err(ExpansionError::NegativeRoot(n));
        }
        let m = (u64::from(n) / gcd(u64::from(n), v.unsigned_abs())) as u32;
        let g = self.refined(m);
        let v = v * i64::from(m);
        let lead = c.nth_root(n)?;
        let floor = v / i64::from(n);
        let q = g.q;
        Ok(Expansion::of(Node::Root { g, n, v, c, lead }, floor, q))
    }
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_base/expansion/tests.rs"]
mod tests;
