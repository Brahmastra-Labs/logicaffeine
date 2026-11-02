//! Values of several ordered infinitesimals: `dt`, `dx` declared much smaller than `dt`, and so on.
//!
//! An infinitesimal declared much smaller than another is smaller than every positive number made from
//! it ([`Scale`]), so the values a program makes from the two are the rational functions of the smaller
//! over the values of the larger, `R(dt)(dx)`, ordered by the transfer principle one infinitesimal at a
//! time, the smallest first: a rational function of `dx` has, for every small enough `dx` beside a fixed
//! `dt`, the sign of its lowest-order coefficient in `dx`, and that coefficient's sign is decided among
//! the values of `dt`. So `dx < dt^n` for every standard `n`, and `dx/dt^1000` is infinitesimal.
//!
//! A [`Multi`] holds a value at the smallest infinitesimal it involves, as a rational function of a
//! root `dx^(1/q)` whose coefficients involve only larger infinitesimals: reduced, its denominator
//! comonic, on the least root that holds it. A value of one infinitesimal is held as its real closure's
//! own ([`Closed`]) and a standard number as an algebraic number, so a value has one form whatever route
//! made it.
//!
//! A value prints as an exact fraction of polynomials in the infinitesimals, in ascending powers of the
//! smallest and each coefficient in the larger ones: `dt + dx`, `dx/dt`, `1/(dt + dx)`, `(1 + dt)·dx`.

use std::cmp::Ordering;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use crate::algebraic::{negative_root, Algebraic};
use crate::closure::{AlgValue, Recipe};
use crate::exact_codec::{read_byte, read_len, write_len, ExactCodec, MAX_WIRE_DEPTH, MAX_WIRE_LEN};
use crate::hyperreal::{Hyperreal, PowerError, MAX_POWER_DEGREE};
use crate::numeric::{BigInt, Rational};
use crate::ordered_field::{OrderedField, PolyOver};
use crate::puiseux::{power_text, top_level, Closed, Puiseux};
use crate::tower::Scale;

/// A value of several ordered infinitesimals. See the module documentation.
#[derive(Clone, Debug)]
pub enum Multi {
    /// A standard number.
    Standard(Algebraic),
    /// A value of one infinitesimal, not standard.
    One(Closed, Arc<Scale>),
    /// `num/den` in `scale^(1/q)`, every coefficient a value of larger infinitesimals and at least one
    /// not standard.
    Over {
        scale: Arc<Scale>,
        q: u32,
        num: PolyOver<Multi>,
        den: PolyOver<Multi>,
    },
    /// A value of the real closure at its smallest infinitesimal that is no rational function of a
    /// root of it ([`crate::closure`]).
    Alg(Arc<AlgValue>),
}

/// Why a power of a value of several infinitesimals is refused, with the infinitesimal and the root
/// of it its degree is counted in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MultiPowerError {
    pub error: PowerError,
    pub name: String,
    pub q: u32,
}

impl MultiPowerError {
    /// The refusal: `the power would have degree 5000 in dx; …`.
    pub fn describe(&self) -> String {
        self.error.describe(&power_text(&self.name, &ratio(1, self.q)))
    }
}

fn ratio(k: usize, q: u32) -> Rational {
    Rational::from_ratio_i64(k as i64, i64::from(q)).expect("q is positive")
}

fn gcd_u64(a: u64, b: u64) -> u64 {
    if b == 0 {
        a
    } else {
        gcd_u64(b, a % b)
    }
}

fn lcm_u32(a: u32, b: u32) -> u32 {
    let g = gcd_u64(u64::from(a), u64::from(b));
    u32::try_from(u64::from(a) / g * u64::from(b)).expect("a root of an infinitesimal of moderate index")
}

/// The smaller of two infinitesimals a value combines; the tower has ordered every pair first.
fn smaller<'a>(a: &'a Arc<Scale>, b: &'a Arc<Scale>) -> &'a Arc<Scale> {
    match a.order(b).expect("every infinitesimal a value combines is ordered with the others") {
        Ordering::Greater => b,
        Ordering::Less | Ordering::Equal => a,
    }
}

/// The smallest infinitesimal either value involves; when both involve it, as both values know it — a
/// value read on a residue class of its reciprocal, with the facts its class adds, combined with any
/// other is read on that class ([`met`]).
fn smallest(a: Option<&Arc<Scale>>, b: Option<&Arc<Scale>>) -> Option<Arc<Scale>> {
    match (a, b) {
        (Some(x), Some(y)) if x.same_infinitesimal(y) => Some(met(x, y)),
        (Some(x), Some(y)) => Some(smaller(x, y).clone()),
        (x, None) => x.cloned(),
        (None, y) => y.cloned(),
    }
}

/// One infinitesimal as two values know it: with the facts about its reciprocal both state. Each value
/// holds on the classes its facts admit, so their combination holds where both do.
fn met(a: &Arc<Scale>, b: &Arc<Scale>) -> Arc<Scale> {
    match a.residues.and(&b.residues) {
        Ok(both) if both == a.residues => a.clone(),
        Ok(both) if both == b.residues => b.clone(),
        Ok(both) => Arc::new(a.with_residues(both)),
        Err(_) => a.clone(),
    }
}

/// `s` among the infinitesimals `out` lists, one copy each: an infinitesimal listed already is held with
/// the facts both copies state ([`met`]), for a value lives where all of its parts do.
fn push_scale(out: &mut Vec<Arc<Scale>>, s: &Arc<Scale>) {
    match out.iter_mut().find(|t| t.same_infinitesimal(s)) {
        Some(t) => *t = met(t, s),
        None => out.push(s.clone()),
    }
}

impl Multi {
    /// The infinitesimal itself.
    pub fn generator(scale: &Arc<Scale>) -> Multi {
        Multi::One(Closed::from_hyperreal(Hyperreal::dt()), scale.clone())
    }

    /// A value of one infinitesimal, in its one form.
    pub fn of_closed(c: Closed, scale: &Arc<Scale>) -> Multi {
        match c.as_standard() {
            Some(a) => Multi::Standard(a),
            None => Multi::One(c, scale.clone()),
        }
    }

    /// The smallest infinitesimal the value involves; `None` for a standard number.
    pub fn top(&self) -> Option<&Arc<Scale>> {
        match self {
            Multi::Standard(_) => None,
            Multi::One(_, s) | Multi::Over { scale: s, .. } => Some(s),
            Multi::Alg(a) => Some(&a.scale),
        }
    }

    /// Every infinitesimal the value involves, each once, with the facts all of its parts state.
    pub fn scales(&self) -> Vec<Arc<Scale>> {
        let mut out = Vec::new();
        self.collect_scales(&mut out);
        out
    }

    fn collect_scales(&self, out: &mut Vec<Arc<Scale>>) {
        match self {
            Multi::Standard(_) => {}
            Multi::One(_, s) => push_scale(out, s),
            Multi::Over { scale, num, den, .. } => {
                push_scale(out, scale);
                for c in num.c.iter().chain(den.c.iter()) {
                    c.collect_scales(out);
                }
            }
            Multi::Alg(a) => {
                push_scale(out, &a.scale);
                recipe_scales(&a.recipe, out);
            }
        }
    }

    /// The value as `(q, num, den)`, a fraction of polynomials in `scale^(1/q)`, for `scale` its own
    /// smallest infinitesimal or one smaller, and the value no root that is none.
    pub(crate) fn parts_at(&self, scale: &Arc<Scale>) -> (u32, PolyOver<Multi>, PolyOver<Multi>) {
        match self {
            Multi::Alg(a) if a.scale.same_infinitesimal(scale) => {
                unreachable!("a root that is no rational function of a root of its infinitesimal has no fraction there")
            }
            Multi::Over { scale: s, q, num, den } if s.same_infinitesimal(scale) => (*q, num.clone(), den.clone()),
            Multi::One(c, s) if s.same_infinitesimal(scale) => {
                let (h, q) = c
                    .field()
                    .expect("a root that is no rational function of a root of its infinitesimal meets no larger one");
                let (num, den) = h.parts();
                let lift = |p: &PolyOver<Algebraic>| PolyOver::from(p.c.iter().cloned().map(Multi::Standard).collect());
                (q, lift(num), lift(den))
            }
            _ => (1, PolyOver::constant(self.clone()), PolyOver::one()),
        }
    }

    /// Both values over the smallest infinitesimal either involves, on the least root of it both lie
    /// on: that infinitesimal, the root's `q`, and each value's numerator and denominator.
    #[allow(clippy::type_complexity)]
    fn over_one(
        &self,
        o: &Multi,
    ) -> (Arc<Scale>, u32, (PolyOver<Multi>, PolyOver<Multi>), (PolyOver<Multi>, PolyOver<Multi>)) {
        let scale = smallest(self.top(), o.top()).expect("a value of an infinitesimal is on one side");
        let (qa, na, da) = self.parts_at(&scale);
        let (qb, nb, db) = o.parts_at(&scale);
        let q = lcm_u32(qa, qb);
        let (ma, mb) = ((q / qa) as usize, (q / qb) as usize);
        (scale, q, (na.spread(ma), da.spread(ma)), (nb.spread(mb), db.spread(mb)))
    }

    /// Both values as values of one infinitesimal's real closure, when neither involves another.
    fn closed_pair(&self, o: &Multi) -> Option<(Closed, Closed, Arc<Scale>)> {
        let closed = |m: &Multi| match m {
            Multi::Standard(a) => Some(Closed::from_hyperreal(Hyperreal::from_algebraic(a.clone()))),
            Multi::One(c, _) => Some(c.clone()),
            Multi::Over { .. } | Multi::Alg(_) => None,
        };
        let scale = match (self.top(), o.top()) {
            (Some(a), Some(b)) if a.same_infinitesimal(b) => met(a, b),
            (Some(a), None) | (None, Some(a)) => a.clone(),
            _ => return None,
        };
        Some((closed(self)?, closed(o)?, scale))
    }

    /// Both values in the real closure at the smallest infinitesimal either involves, when one of them
    /// is a root there that is no rational function of a root of it.
    fn closure_pair(&self, o: &Multi) -> Option<(Arc<AlgValue>, Arc<AlgValue>)> {
        let scale = &smallest(self.top(), o.top())?;
        let root_there = |m: &Multi| match m {
            Multi::Alg(a) => a.scale.same_infinitesimal(scale),
            Multi::One(c, s) => s.same_infinitesimal(scale) && c.field().is_none(),
            _ => false,
        };
        if !(root_there(self) || root_there(o)) {
            return None;
        }
        Some((Arc::new(AlgValue::at(self, scale)), Arc::new(AlgValue::at(o, scale))))
    }

    /// `num/den` in `scale^(1/q)` in its one form: reduced, the denominator comonic, on the least root
    /// of `scale` that holds it; a value with no power of `scale` left is its coefficient, and one whose
    /// every coefficient is standard is a value of `scale` alone. A denominator that is one power of the
    /// root shares with the numerator only the power both hold, so it reduces without a gcd.
    pub(crate) fn make(scale: &Arc<Scale>, q: u32, num: PolyOver<Multi>, den: PolyOver<Multi>) -> Multi {
        if num.is_zero() {
            return Multi::zero();
        }
        let monomial = den.order().filter(|&d| den.c.iter().enumerate().all(|(i, c)| i == d || c.is_zero()));
        let (num, den) = if den.degree() == Some(0) {
            let inv = den.coeff(0).recip().expect("the denominator is nonzero");
            (num.scale(&inv), PolyOver::one())
        } else if let Some(d) = monomial {
            let shift = d.min(num.order().expect("the numerator is nonzero"));
            let inv = den.coeff(d).recip().expect("the denominator is nonzero");
            let mut power = vec![Multi::zero(); d - shift + 1];
            power[d - shift] = Multi::Standard(Algebraic::from_i64(1));
            (PolyOver::from(num.c[shift..].to_vec()).scale(&inv), PolyOver::from(power))
        } else {
            let g = num.gcd(&den);
            let (num, den) = (num.div_rem(&g).0, den.div_rem(&g).0);
            let inv = den.coeff(den.order().expect("the denominator is nonzero")).recip().expect("nonzero");
            (num.scale(&inv), den.scale(&inv))
        };
        let (num, den, q) = least_root(num, den, q);
        if num.degree() == Some(0) && den.degree() == Some(0) {
            return num.coeff(0);
        }
        let standard = |p: &PolyOver<Multi>| -> Option<PolyOver<Algebraic>> {
            p.c.iter()
                .map(|c| match c {
                    Multi::Standard(a) => Some(a.clone()),
                    _ => None,
                })
                .collect::<Option<Vec<_>>>()
                .map(PolyOver::from)
        };
        if let (Some(n), Some(d)) = (standard(&num), standard(&den)) {
            return Multi::of_closed(Closed::of_field(Hyperreal::from_parts(n, d), q), scale);
        }
        Multi::Over {
            scale: scale.clone(),
            q,
            num,
            den,
        }
    }

    /// The exponent of the leading term in the value's smallest infinitesimal and its coefficient, a
    /// value of the larger ones; `None` for a standard number.
    pub(crate) fn leading(&self) -> Option<(Rational, Multi)> {
        match self {
            Multi::Standard(_) => None,
            Multi::Alg(a) => Some((a.order(), a.leading().1)),
            Multi::One(c, _) => c.leading().map(|(v, a)| (v, Multi::Standard(a))),
            Multi::Over { q, num, den, .. } => {
                let n = num.order().expect("a value is nonzero");
                let d = den.order().expect("the denominator is nonzero");
                let v = Rational::from_ratio_i64(n as i64 - d as i64, i64::from(*q)).expect("q is positive");
                Some((v, num.coeff(n)))
            }
        }
    }

    /// What the value is infinitely close to among the values of `at` and the infinitesimals larger than
    /// it, when it is limited from `at`'s standpoint: every infinitesimal smaller than `at` read at zero,
    /// the inner limit with `at` held fixed. With no `at`, the standard part. `None` for a value infinite
    /// from that standpoint. Every infinitesimal of the value is ordered with `at`.
    pub fn standard_part_at(&self, at: Option<&Arc<Scale>>) -> Option<Multi> {
        let Some(top) = self.top() else {
            return Some(self.clone());
        };
        if let Some(at) = at {
            if top.order(at).expect("the standpoint is ordered with the value's infinitesimals") != Ordering::Less {
                return Some(self.clone());
            }
        }
        let (v, c) = self.leading().expect("a value of an infinitesimal is nonzero");
        match v.cmp(&Rational::zero()) {
            Ordering::Greater => Some(Multi::zero()),
            Ordering::Less => None,
            Ordering::Equal => c.standard_part_at(at),
        }
    }

    /// The standard real infinitely close to a limited value; `None` for an infinite one.
    pub fn standard_part(&self) -> Option<Algebraic> {
        match self.standard_part_at(None)? {
            Multi::Standard(a) => Some(a),
            _ => unreachable!("the standard part of a value is standard"),
        }
    }

    /// Zero, or smaller than every positive value of `at` and the infinitesimals larger than it (every
    /// positive standard number, with no `at`).
    pub fn is_infinitesimal_at(&self, at: Option<&Arc<Scale>>) -> bool {
        self.standard_part_at(at).is_some_and(|s| s.is_zero())
    }

    /// Bounded by a value of `at` and the infinitesimals larger than it (a standard number, with no
    /// `at`).
    pub fn is_limited_at(&self, at: Option<&Arc<Scale>>) -> bool {
        self.standard_part_at(at).is_some()
    }

    pub fn is_infinitesimal(&self) -> bool {
        self.is_infinitesimal_at(None)
    }

    pub fn is_limited(&self) -> bool {
        self.is_limited_at(None)
    }

    /// The largest degree any of the value's infinitesimals reaches in it, with that infinitesimal and
    /// the root of it the degree counts.
    fn widest(&self) -> Option<(usize, Arc<Scale>, u32)> {
        match self {
            Multi::Standard(_) | Multi::Alg(_) => None,
            Multi::One(c, s) => c.field().map(|(h, q)| (h.degree(), s.clone(), q)),
            Multi::Over { scale, q, num, den } => {
                let here = (num.degree().unwrap_or(0).max(den.degree().unwrap_or(0)), scale.clone(), *q);
                num.c
                    .iter()
                    .chain(den.c.iter())
                    .filter_map(Multi::widest)
                    .chain(std::iter::once(here))
                    .max_by_key(|(d, _, _)| *d)
            }
        }
    }

    /// `self^k`; refused for zero to a negative power and for a degree past
    /// [`crate::hyperreal::MAX_POWER_DEGREE`] in any of the value's infinitesimals.
    pub fn pow(&self, k: i32) -> Result<Multi, MultiPowerError> {
        let zero_to_negative = |name: &str| MultiPowerError {
            error: PowerError::ZeroToNegative,
            name: name.to_string(),
            q: 1,
        };
        match self {
            Multi::Standard(a) => a.pow(k).map(Multi::Standard).ok_or_else(|| zero_to_negative("")),
            Multi::One(c, s) => c.pow(k).map(|r| Multi::of_closed(r, s)).map_err(|e| MultiPowerError {
                error: e.error,
                name: s.name.clone(),
                q: e.q,
            }),
            Multi::Alg(a) => {
                let reach = u128::from(k.unsigned_abs());
                if reach > u128::from(MAX_POWER_DEGREE) {
                    return Err(MultiPowerError {
                        error: PowerError::DegreeTooHigh { degree: reach },
                        name: a.scale.name.clone(),
                        q: a.expansion.ramification(),
                    });
                }
                Ok(match k.cmp(&0) {
                    Ordering::Equal => Multi::one(),
                    Ordering::Greater => AlgValue::pow(a, k.unsigned_abs()),
                    Ordering::Less => match AlgValue::recip(a) {
                        Multi::Alg(r) => AlgValue::pow(&r, k.unsigned_abs()),
                        r => r.pow(k.saturating_neg())?,
                    },
                })
            }
            Multi::Over { .. } => {
                if let Some((degree, scale, q)) = self.widest() {
                    let reach = degree as u128 * u128::from(k.unsigned_abs());
                    if reach > u128::from(MAX_POWER_DEGREE) {
                        return Err(MultiPowerError {
                            error: PowerError::DegreeTooHigh { degree: reach },
                            name: scale.name.clone(),
                            q,
                        });
                    }
                }
                let mut base = if k < 0 {
                    self.recip().expect("a value of several infinitesimals is nonzero")
                } else {
                    self.clone()
                };
                let mut acc = Multi::one();
                let mut e = k.unsigned_abs();
                while e > 0 {
                    if e & 1 == 1 {
                        acc = acc.mul(&base);
                    }
                    e >>= 1;
                    if e > 0 {
                        base = base.mul(&base);
                    }
                }
                Ok(acc)
            }
        }
    }

    /// The real `n`-th root, the nonnegative one for an even `n`: of a value of one infinitesimal in its
    /// real closure, and of a product of a power of the smallest infinitesimal and a value of the larger
    /// ones as the product of their roots.
    pub fn root(&self, n: u32) -> Result<Multi, String> {
        if n == 1 || self.is_zero() {
            return Ok(self.clone());
        }
        match self {
            Multi::Standard(a) => a.nth_root(n).map(Multi::Standard).ok_or_else(|| negative_root(n)),
            Multi::One(c, s) => Ok(Multi::of_closed(c.root(n)?, s)),
            Multi::Alg(a) => AlgValue::root(a, n),
            Multi::Over { scale, q, num, den } => {
                let single = |p: &PolyOver<Multi>| -> Option<usize> {
                    let mut terms = p.c.iter().enumerate().filter(|(_, c)| !c.is_zero());
                    let (i, _) = terms.next()?;
                    terms.next().is_none().then_some(i)
                };
                let (Some(a), Some(b)) = (single(num), single(den)) else {
                    return AlgValue::root(&Arc::new(AlgValue::field(self, scale)), n);
                };
                let root = num.coeff(a).root(n)?;
                let e = a as i64 - b as i64;
                let mut power = vec![Multi::zero(); e.unsigned_abs() as usize + 1];
                power[e.unsigned_abs() as usize] = Multi::one();
                let power = PolyOver::from(power);
                let (top, bottom) = if e >= 0 {
                    (power.scale(&root), PolyOver::one())
                } else {
                    (PolyOver::constant(root), power)
                };
                Ok(Multi::make(scale, q * n, top, bottom))
            }
        }
    }
}

/// `num/den` on the least root of their infinitesimal: `t = s^(1/q)` replaced by `t^m` for the largest
/// `m` dividing `q` and every power the two hold.
fn least_root(num: PolyOver<Multi>, den: PolyOver<Multi>, q: u32) -> (PolyOver<Multi>, PolyOver<Multi>, u32) {
    let powers = num
        .c
        .iter()
        .enumerate()
        .chain(den.c.iter().enumerate())
        .filter(|(_, c)| !c.is_zero())
        .map(|(i, _)| i as u64);
    let m = powers.fold(u64::from(q), gcd_u64) as usize;
    if m <= 1 {
        return (num, den, q);
    }
    let gather = |p: &PolyOver<Multi>| PolyOver::from(p.c.iter().step_by(m).cloned().collect());
    (gather(&num), gather(&den), q / m as u32)
}

impl OrderedField for Multi {
    fn zero() -> Self {
        Multi::Standard(Algebraic::from_i64(0))
    }
    fn one() -> Self {
        Multi::Standard(Algebraic::from_i64(1))
    }
    fn from_i64(n: i64) -> Self {
        Multi::Standard(Algebraic::from_i64(n))
    }
    fn is_zero(&self) -> bool {
        matches!(self, Multi::Standard(a) if a.is_zero())
    }
    fn add(&self, o: &Self) -> Self {
        if let (Multi::Standard(a), Multi::Standard(b)) = (self, o) {
            return Multi::Standard(a.add(b));
        }
        if let Some((a, b, s)) = self.closed_pair(o) {
            return Multi::of_closed(a.add(&b), &s);
        }
        if let Some((a, b)) = self.closure_pair(o) {
            return AlgValue::add(&a, &b);
        }
        let (scale, q, (na, da), (nb, db)) = self.over_one(o);
        if da == db {
            return Multi::make(&scale, q, na.add(&nb), da);
        }
        Multi::make(&scale, q, na.mul(&db).add(&nb.mul(&da)), da.mul(&db))
    }
    fn neg(&self) -> Self {
        match self {
            Multi::Standard(a) => Multi::Standard(a.neg()),
            Multi::One(c, s) => Multi::One(c.neg(), s.clone()),
            Multi::Over { scale, q, num, den } => Multi::Over {
                scale: scale.clone(),
                q: *q,
                num: num.neg(),
                den: den.clone(),
            },
            Multi::Alg(a) => AlgValue::neg(a),
        }
    }
    fn mul(&self, o: &Self) -> Self {
        if let (Multi::Standard(a), Multi::Standard(b)) = (self, o) {
            return Multi::Standard(a.mul(b));
        }
        if self.is_zero() || o.is_zero() {
            return Multi::zero();
        }
        if let Some((a, b, s)) = self.closed_pair(o) {
            return Multi::of_closed(a.mul(&b), &s);
        }
        if let Some((a, b)) = self.closure_pair(o) {
            return AlgValue::mul(&a, &b);
        }
        let (scale, q, (na, da), (nb, db)) = self.over_one(o);
        Multi::make(&scale, q, na.mul(&nb), da.mul(&db))
    }
    fn recip(&self) -> Option<Self> {
        match self {
            Multi::Standard(a) => a.recip().map(Multi::Standard),
            Multi::One(c, s) => c.recip().map(|r| Multi::of_closed(r, s)),
            Multi::Over { scale, q, num, den } => Some(Multi::make(scale, *q, den.clone(), num.clone())),
            Multi::Alg(a) => Some(AlgValue::recip(a)),
        }
    }
    fn signum(&self) -> i32 {
        match self {
            Multi::Standard(a) => a.signum(),
            Multi::One(c, _) => c.signum(),
            Multi::Over { num, .. } => num.coeff(num.order().expect("a value is nonzero")).signum(),
            Multi::Alg(a) => a.sign(),
        }
    }
}

impl PartialEq for Multi {
    fn eq(&self, o: &Self) -> bool {
        match (self, o) {
            (Multi::Standard(a), Multi::Standard(b)) => a == b,
            (Multi::One(a, s), Multi::One(b, t)) => s.same_infinitesimal(t) && a == b,
            (
                Multi::Over { scale: s, q: qa, num: na, den: da },
                Multi::Over { scale: t, q: qb, num: nb, den: db },
            ) => s.same_infinitesimal(t) && qa == qb && na == nb && da == db,
            (Multi::Alg(a), Multi::Alg(b)) => AlgValue::equals(a, b),
            _ => false,
        }
    }
}

impl Eq for Multi {}

impl Hash for Multi {
    /// Equal values have one form, and one infinitesimal one name.
    fn hash<H: Hasher>(&self, state: &mut H) {
        match self {
            Multi::Standard(a) => {
                0u8.hash(state);
                a.hash(state);
            }
            Multi::One(c, s) => {
                1u8.hash(state);
                s.name.hash(state);
                c.hash(state);
            }
            Multi::Over { scale, q, num, den } => {
                2u8.hash(state);
                scale.name.hash(state);
                q.hash(state);
                num.hash(state);
                den.hash(state);
            }
            Multi::Alg(a) => {
                3u8.hash(state);
                a.scale.name.hash(state);
                let (order, lead) = a.leading();
                order.hash(state);
                a.expansion.ramification().hash(state);
                lead.hash(state);
            }
        }
    }
}

// ── printing ──────────────────────────────────────────────────────────────────────────────────

impl Multi {
    /// The value as an exact fraction of polynomials in its infinitesimals, as a hyperreal prints: a
    /// polynomial with its coefficients as they are, `(1/2)·dt + dx`, and any other value as a fraction
    /// of polynomials with coprime integer coefficients where every coefficient is rational,
    /// `dx/(2·dt)`. An infinite integer prints as one of one infinitesimal does, a sum of reciprocals
    /// (`Multi::render_reciprocals`), wherever it is printed — alone, under a radical, as a
    /// coefficient.
    pub fn render(&self) -> String {
        if let Multi::Alg(a) = self {
            return AlgValue::render(a);
        }
        if self.is_integral() {
            if let Some(text) = self.render_reciprocals() {
                return text;
            }
        }
        let (n, d) = self.fraction();
        if d.top().is_none() {
            return render_polynomial(self);
        }
        let numerator = render_polynomial(&n);
        let denominator = render_polynomial(&d);
        let numerator = if top_level(&numerator, &[' ']) { format!("({numerator})") } else { numerator };
        let denominator = if top_level(&denominator, &[' ', '·']) { format!("({denominator})") } else { denominator };
        format!("{numerator}/{denominator}")
    }

    /// Whether the value is an integer on every residue class its infinitesimals' facts admit — the facts
    /// its scales carry, a class's own among them, an infinitesimal its terms know apart read with the
    /// facts all of them state ([`met`]): a polynomial in their reciprocals with rational
    /// coefficients whose fraction vanishes on each. Modulo the coefficients' common denominator `D` the
    /// fraction depends only on each reciprocal's class, so it is read on one member of each admitted
    /// class; past [`INTEGRAL_CLASSES`] joint classes — counted before any is read — the value is not
    /// called one, for no program holds a residue that fine.
    pub(crate) fn is_integral(&self) -> bool {
        let mut terms = Vec::new();
        if reciprocal_terms(self, &mut Vec::new(), &mut terms).is_none() {
            return false;
        }
        let denominator = terms.iter().fold(BigInt::from_i64(1), |d, (_, c)| {
            let g = d.gcd(c.denominator());
            d.div_rem(&g).expect("a nonzero divisor").0.mul(c.denominator())
        });
        let Some(d) = denominator.to_i64().and_then(|d| u64::try_from(d).ok()) else {
            return false;
        };
        if d == 1 {
            return true;
        }
        let mut scales: Vec<Arc<Scale>> = Vec::new();
        for (monomial, _) in &terms {
            for (s, _) in monomial {
                match scales.iter_mut().find(|t| t.same_infinitesimal(s)) {
                    Some(t) => *t = met(t, s),
                    None => scales.push(s.clone()),
                }
            }
        }
        let mut moduli: Vec<u64> = Vec::with_capacity(scales.len());
        let mut joint: u64 = 1;
        for s in &scales {
            let ty = &s.residues;
            let g = gcd_u64(d, ty.modulus());
            let Some(m) = (d / g).checked_mul(ty.modulus()) else {
                return false;
            };
            let per_period = (0..ty.modulus()).filter(|h| ty.admits(*h)).count() as u64;
            joint = match joint.checked_mul((m / ty.modulus()).saturating_mul(per_period)) {
                Some(j) if j <= INTEGRAL_CLASSES => j,
                _ => return false,
            };
            moduli.push(m);
        }
        let members: Vec<Vec<u64>> = scales
            .iter()
            .zip(&moduli)
            .map(|(s, m)| (0..*m).filter(|h| s.residues.admits(*h)).collect())
            .collect();
        let mut at = vec![0usize; scales.len()];
        loop {
            let value = terms.iter().fold(Rational::zero(), |acc, (monomial, c)| {
                let product = monomial.iter().fold(c.clone(), |p, (s, k)| {
                    let i = scales.iter().position(|t| t.same_infinitesimal(s)).expect("every scale is listed");
                    let h = Rational::from_i64(members[i][at[i]] as i64);
                    p.mul(&h.pow(*k as i32).expect("a natural power"))
                });
                acc.add(&product)
            });
            if !value.is_integer() {
                return false;
            }
            let mut i = 0;
            loop {
                if i == at.len() {
                    return true;
                }
                at[i] += 1;
                if at[i] < members[i].len() {
                    break;
                }
                at[i] = 0;
                i += 1;
            }
        }
    }

    /// The value as a sum of terms `c/(dt^i·dx^j)`, the infinitesimals of each term written larger
    /// first and the terms most infinite in the smallest infinitesimal first, when it is a polynomial in
    /// the reciprocals of its infinitesimals with rational coefficients: the form an infinite integer of
    /// several prints in, as one of one infinitesimal does (`1/(3·dt^3) - 1/(2·dt^2) + 1/(6·dt)`).
    /// `None` for any other value.
    pub(crate) fn render_reciprocals(&self) -> Option<String> {
        let mut terms = Vec::new();
        reciprocal_terms(self, &mut Vec::new(), &mut terms)?;
        let mut scales: Vec<Arc<Scale>> = Vec::new();
        for (monomial, _) in &terms {
            for (s, _) in monomial {
                if !scales.iter().any(|t| t.same_infinitesimal(s)) {
                    scales.push(s.clone());
                }
            }
        }
        scales.sort_by(|a, b| a.order(b).unwrap_or(Ordering::Equal));
        let powers = |monomial: &[(Arc<Scale>, u32)]| -> Vec<u32> {
            scales
                .iter()
                .map(|s| monomial.iter().find(|(t, _)| t.same_infinitesimal(s)).map_or(0, |(_, k)| *k))
                .collect()
        };
        terms.sort_by(|(a, _), (b, _)| powers(b).cmp(&powers(a)));
        let mut out = String::new();
        for (i, (monomial, c)) in terms.iter().enumerate() {
            let negative = c.is_negative();
            let m = c.abs();
            let body = if monomial.is_empty() {
                m.to_string()
            } else {
                let factors: Vec<String> = scales
                    .iter()
                    .rev()
                    .filter_map(|s| {
                        let k = monomial.iter().find(|(t, _)| t.same_infinitesimal(s))?.1;
                        Some(if k == 1 { s.name.clone() } else { format!("{}^{k}", s.name) })
                    })
                    .collect();
                let product = factors.join("·");
                let (num, den) = (m.numerator().clone(), m.denominator().clone());
                if den == BigInt::from_i64(1) && factors.len() == 1 {
                    format!("{num}/{product}")
                } else if den == BigInt::from_i64(1) {
                    format!("{num}/({product})")
                } else {
                    format!("{num}/({den}·{product})")
                }
            };
            out.push_str(&match (i, negative) {
                (0, true) => format!("-{body}"),
                (0, false) => body,
                (_, true) => format!(" - {body}"),
                (_, false) => format!(" + {body}"),
            });
        }
        Some(if out.is_empty() { "0".to_string() } else { out })
    }

    /// The numerator and denominator the value prints as. See [`Multi::fraction`].
    pub(crate) fn fraction_parts(&self) -> (Multi, Multi) {
        self.fraction()
    }

    /// `self = n/d` for polynomials `n` and `d` in the infinitesimals with no common factor, `d`'s
    /// leading term positive, and integer coefficients when every coefficient is rational.
    fn fraction(&self) -> (Multi, Multi) {
        let (n, d) = self.raw_fraction();
        if d.signum() < 0 {
            (n.neg(), d.neg())
        } else {
            (n, d)
        }
    }

    fn raw_fraction(&self) -> (Multi, Multi) {
        match self {
            Multi::Standard(a) => match a.to_rational() {
                Some(r) => (
                    Multi::Standard(Algebraic::from_rational(Rational::from_bigint(r.numerator().clone()))),
                    Multi::Standard(Algebraic::from_rational(Rational::from_bigint(r.denominator().clone()))),
                ),
                None => (self.clone(), Multi::one()),
            },
            Multi::One(c, s) => match c.field() {
                Some((h, q)) => {
                    let (n, d) = h.display_fraction();
                    let poly = |p: PolyOver<Algebraic>| {
                        Multi::of_closed(Closed::of_field(Hyperreal::from_parts(p, PolyOver::one()), q), s)
                    };
                    (poly(n), poly(d))
                }
                None => (self.clone(), Multi::one()),
            },
            Multi::Alg(_) => (self.clone(), Multi::one()),
            Multi::Over { scale, q, num, den } => {
                let l = num
                    .c
                    .iter()
                    .chain(den.c.iter())
                    .filter(|c| !c.is_zero())
                    .fold(Multi::one(), |acc, c| polynomial_lcm(&acc, &c.raw_fraction().1));
                let lift = |p: &PolyOver<Multi>| PolyOver::from(p.c.iter().map(|c| c.mul(&l)).collect());
                let (n, d) = (lift(num), lift(den));
                let g = polynomial_gcd(&content(&n), &content(&d));
                let g_inv = g.recip().expect("a gcd of nonzero polynomials is nonzero");
                let reduce = |p: PolyOver<Multi>| Multi::make(scale, *q, p.scale(&g_inv), PolyOver::one());
                (reduce(n), reduce(d))
            }
        }
    }
}

/// The greatest common divisor of the coefficients of `p`, polynomials in larger infinitesimals.
fn content(p: &PolyOver<Multi>) -> Multi {
    p.c.iter().filter(|c| !c.is_zero()).fold(Multi::zero(), |acc, c| polynomial_gcd(&acc, c))
}

fn polynomial_lcm(a: &Multi, b: &Multi) -> Multi {
    let g = polynomial_gcd(a, b);
    a.mul(b).mul(&g.recip().expect("a gcd of nonzero polynomials is nonzero"))
}

/// Whether the value is a root that is no rational function of a root of its infinitesimal: a
/// polynomial's coefficient that no denominator is cleared from and no factor shared with.
fn is_root_atom(m: &Multi) -> bool {
    matches!(m, Multi::One(c, _) if c.field().is_none()) || matches!(m, Multi::Alg(_))
}

/// The smallest infinitesimal a polynomial is a polynomial in; a root atom is a coefficient, in none.
fn polynomial_top(m: &Multi) -> Option<&Arc<Scale>> {
    if is_root_atom(m) {
        None
    } else {
        m.top()
    }
}

/// A polynomial as `(q, coefficients)` in `scale^(1/q)`, a root atom a constant.
fn polynomial_at(m: &Multi, scale: &Arc<Scale>) -> (u32, PolyOver<Multi>) {
    if is_root_atom(m) {
        return (1, PolyOver::constant(m.clone()));
    }
    let (q, p, _) = m.parts_at(scale);
    (q, p)
}

/// The greatest common divisor of two polynomials in the infinitesimals, its leading term positive:
/// the gcd of their contents in the smallest infinitesimal either involves, times the primitive gcd
/// of what is left, found over the field of the larger infinitesimals' values and cleared of its
/// denominators. Two standard integers meet in their integer gcd, any other two constants — standard
/// numbers and roots alike — in 1.
fn polynomial_gcd(a: &Multi, b: &Multi) -> Multi {
    let positive = |m: &Multi| if m.signum() < 0 { m.neg() } else { m.clone() };
    if a.is_zero() {
        return positive(b);
    }
    if b.is_zero() {
        return positive(a);
    }
    let Some(scale) = smallest(polynomial_top(a), polynomial_top(b)) else {
        let integer = |m: &Multi| match m {
            Multi::Standard(x) => x.to_rational().filter(Rational::is_integer).and_then(|r| r.to_bigint()),
            _ => None,
        };
        return match (integer(a), integer(b)) {
            (Some(x), Some(y)) => Multi::Standard(Algebraic::from_rational(Rational::from_bigint(x.gcd(&y)))),
            _ => Multi::one(),
        };
    };
    let (qa, pa) = polynomial_at(a, &scale);
    let (qb, pb) = polynomial_at(b, &scale);
    let q = lcm_u32(qa, qb);
    let (pa, pb) = (pa.spread((q / qa) as usize), pb.spread((q / qb) as usize));
    let (ca, cb) = (content(&pa), content(&pb));
    let c = polynomial_gcd(&ca, &cb);
    let primitive = |p: &PolyOver<Multi>, k: &Multi| p.scale(&k.recip().expect("a content is nonzero"));
    let (pa, pb) = (primitive(&pa, &ca), primitive(&pb, &cb));
    if pa.degree() == Some(0) || pb.degree() == Some(0) {
        return c;
    }
    let g = Multi::make(&scale, q, pa.gcd(&pb), PolyOver::one());
    let (g, _) = g.raw_fraction();
    let g_content = match polynomial_top(&g) {
        Some(t) if t.same_infinitesimal(&scale) => content(&polynomial_at(&g, &scale).1),
        _ => g.clone(),
    };
    let g = g.mul(&g_content.recip().expect("a content is nonzero"));
    c.mul(&positive(&g))
}

/// A polynomial in the infinitesimals, in ascending powers of its smallest, each coefficient written in
/// the larger ones: the constant term with its own signs, every other term's sign between the terms.
fn render_polynomial(p: &Multi) -> String {
    match p {
        Multi::Standard(a) => a.to_string(),
        Multi::One(c, s) => c.render(&s.name),
        Multi::Alg(a) => AlgValue::render(a),
        Multi::Over { scale, q, num, .. } => {
            let mut out = String::new();
            for (k, c) in num.c.iter().enumerate() {
                if c.is_zero() {
                    continue;
                }
                if k == 0 {
                    out.push_str(&render_polynomial(c));
                    continue;
                }
                let power = power_text(&scale.name, &ratio(k, *q));
                let negative = c.signum() < 0;
                let magnitude = render_polynomial(&if negative { c.neg() } else { c.clone() });
                let body = if magnitude == "1" {
                    power
                } else if top_level(&magnitude, &[' ', '/']) {
                    format!("({magnitude})·{power}")
                } else {
                    format!("{magnitude}·{power}")
                };
                match (out.is_empty(), negative) {
                    (true, true) => out.push_str(&format!("-{body}")),
                    (true, false) => out.push_str(&body),
                    (false, true) => out.push_str(&format!(" - {body}")),
                    (false, false) => out.push_str(&format!(" + {body}")),
                }
            }
            out
        }
    }
}

// ── bytes ─────────────────────────────────────────────────────────────────────────────────────

impl Multi {
    /// The value's canonical bytes: each infinitesimal it involves once, by its name, its declaration
    /// ([`Scale::to_bytes`]) and the facts about its reciprocal the value holds it with — a residue
    /// class's, on which an infinite integer of several is one — in the order a walk of the value first
    /// meets them; then the value, each infinitesimal written by its place among them.
    pub fn to_bytes(&self) -> Vec<u8> {
        let table = self.scales();
        let mut out = Vec::new();
        write_len(table.len(), &mut out);
        for s in &table {
            write_text(&s.name, &mut out);
            let bytes = s.to_bytes();
            write_len(bytes.len(), &mut out);
            out.extend_from_slice(&bytes);
            s.residues.write(&mut out);
        }
        self.write_value(&table, &mut out);
        out
    }

    fn write_value(&self, table: &[Arc<Scale>], out: &mut Vec<u8>) {
        let place = |s: &Arc<Scale>| {
            table.iter().position(|t| t.same_infinitesimal(s)).expect("the table holds every infinitesimal") as u64
        };
        match self {
            Multi::Standard(a) => {
                out.push(0);
                a.write(out);
            }
            Multi::One(c, s) => {
                out.push(1);
                place(s).write(out);
                c.encode(out);
            }
            Multi::Over { scale, q, num, den } => {
                out.push(2);
                place(scale).write(out);
                u64::from(*q).write(out);
                for p in [num, den] {
                    write_len(p.c.len(), out);
                    for c in &p.c {
                        c.write_value(table, out);
                    }
                }
            }
            Multi::Alg(a) => {
                out.push(3);
                place(&a.scale).write(out);
                write_recipe(&a.recipe, table, out);
            }
        }
    }

    /// The value [`Multi::to_bytes`] wrote; `None` for a forged or non-canonical encoding.
    pub fn from_bytes(buf: &[u8]) -> Option<Multi> {
        let mut pos = 0;
        let count = read_len(buf, &mut pos, MAX_WIRE_LEN)?;
        let mut table = Vec::with_capacity(count.min(64));
        for _ in 0..count {
            let name = read_text(buf, &mut pos)?;
            let n = read_len(buf, &mut pos, MAX_WIRE_LEN)?;
            let bytes = buf.get(pos..pos.checked_add(n)?)?;
            pos += n;
            let declared = Scale::from_bytes(&name, bytes)?;
            let residues = crate::limit_value::ResidueType::read(buf, &mut pos)?;
            table.push(if residues == declared.residues {
                declared
            } else if declared.residues.and(&residues).ok()? == residues {
                Arc::new(declared.with_residues(residues))
            } else {
                return None;
            });
        }
        for (i, a) in table.iter().enumerate() {
            if table[i + 1..].iter().any(|b| a.order(b).is_none_or(|o| o == Ordering::Equal)) {
                return None;
            }
        }
        let value = Multi::read_value(buf, &mut pos, &table, 0)?;
        (pos == buf.len() && value.to_bytes() == buf).then_some(value)
    }

    fn read_value(buf: &[u8], pos: &mut usize, table: &[Arc<Scale>], depth: usize) -> Option<Multi> {
        if depth > MAX_WIRE_DEPTH {
            return None;
        }
        Some(match read_byte(buf, pos)? {
            0 => Multi::Standard(Algebraic::read(buf, pos)?),
            1 => {
                let s = table.get(usize::try_from(u64::read(buf, pos)?).ok()?)?;
                Multi::of_closed(Closed::decode(buf, pos)?, s)
            }
            2 => {
                let s = table.get(usize::try_from(u64::read(buf, pos)?).ok()?)?.clone();
                let q = u32::try_from(u64::read(buf, pos)?).ok().filter(|q| *q > 0)?;
                let mut polys = Vec::with_capacity(2);
                for _ in 0..2 {
                    let n = read_len(buf, pos, MAX_WIRE_LEN)?;
                    let mut c = Vec::with_capacity(n.min(64));
                    for _ in 0..n {
                        c.push(Multi::read_value(buf, pos, table, depth + 1)?);
                    }
                    polys.push(PolyOver::from(c));
                }
                let den = polys.pop().expect("two polynomials");
                let num = polys.pop().expect("two polynomials");
                if den.is_zero() {
                    return None;
                }
                let larger = num
                    .c
                    .iter()
                    .chain(den.c.iter())
                    .all(|c| c.scales().iter().all(|t| t.order(&s) == Some(Ordering::Greater)));
                if !larger {
                    return None;
                }
                Multi::make(&s, q, num, den)
            }
            3 => {
                let s = table.get(usize::try_from(u64::read(buf, pos)?).ok()?)?.clone();
                read_recipe(buf, pos, table, &s, depth + 1)?
            }
            _ => return None,
        })
    }
}

/// Every infinitesimal the leaves of a recipe involve.
fn recipe_scales(recipe: &Recipe, out: &mut Vec<Arc<Scale>>) {
    match recipe {
        Recipe::Field(x) => x.collect_scales(out),
        Recipe::Closed(_) => {}
        Recipe::Add(a, b) | Recipe::Mul(a, b) => {
            recipe_scales(&a.recipe, out);
            recipe_scales(&b.recipe, out);
        }
        Recipe::Neg(a) | Recipe::Recip(a) | Recipe::Root(a, _) | Recipe::Pow(a, _) => recipe_scales(&a.recipe, out),
    }
}

/// A recipe: a tag, then its parts — `0` a rational function and `1` a root of one infinitesimal at
/// the leaves, `2` a sum, `3` a negation, `4` a product, `5` a reciprocal, `6` a root and `7` a power,
/// the index or exponent after the value.
fn write_recipe(recipe: &Recipe, table: &[Arc<Scale>], out: &mut Vec<u8>) {
    match recipe {
        Recipe::Field(x) => {
            out.push(0);
            x.write_value(table, out);
        }
        Recipe::Closed(p) => {
            out.push(1);
            p.write(out);
        }
        Recipe::Add(a, b) => {
            out.push(2);
            write_recipe(&a.recipe, table, out);
            write_recipe(&b.recipe, table, out);
        }
        Recipe::Neg(a) => {
            out.push(3);
            write_recipe(&a.recipe, table, out);
        }
        Recipe::Mul(a, b) => {
            out.push(4);
            write_recipe(&a.recipe, table, out);
            write_recipe(&b.recipe, table, out);
        }
        Recipe::Recip(a) => {
            out.push(5);
            write_recipe(&a.recipe, table, out);
        }
        Recipe::Root(a, n) => {
            out.push(6);
            write_recipe(&a.recipe, table, out);
            u64::from(*n).write(out);
        }
        Recipe::Pow(a, k) => {
            out.push(7);
            write_recipe(&a.recipe, table, out);
            u64::from(*k).write(out);
        }
    }
}

/// The value a recipe [`write_recipe`] wrote builds, at the infinitesimal `scale`.
fn read_recipe(buf: &[u8], pos: &mut usize, table: &[Arc<Scale>], scale: &Arc<Scale>, depth: usize) -> Option<Multi> {
    use crate::exact_codec::MAX_WIRE_DEGREE;
    if depth > MAX_WIRE_DEPTH {
        return None;
    }
    let operand = |m: Multi| Arc::new(AlgValue::at(&m, scale));
    Some(match read_byte(buf, pos)? {
        0 => Multi::read_value(buf, pos, table, depth + 1)?,
        1 => Multi::of_closed(Puiseux::read(buf, pos)?.canonical(), scale),
        2 => {
            let a = operand(read_recipe(buf, pos, table, scale, depth + 1)?);
            let b = operand(read_recipe(buf, pos, table, scale, depth + 1)?);
            AlgValue::add(&a, &b)
        }
        3 => AlgValue::neg(&operand(read_recipe(buf, pos, table, scale, depth + 1)?)),
        4 => {
            let a = read_recipe(buf, pos, table, scale, depth + 1)?;
            let b = read_recipe(buf, pos, table, scale, depth + 1)?;
            if a.is_zero() || b.is_zero() {
                return None;
            }
            AlgValue::mul(&operand(a), &operand(b))
        }
        5 => {
            let a = read_recipe(buf, pos, table, scale, depth + 1)?;
            if a.is_zero() {
                return None;
            }
            AlgValue::recip(&operand(a))
        }
        6 => {
            let a = read_recipe(buf, pos, table, scale, depth + 1)?;
            let n = u32::try_from(u64::read(buf, pos)?).ok().filter(|n| (2..=MAX_WIRE_DEGREE as u32).contains(n))?;
            if a.is_zero() {
                return None;
            }
            AlgValue::root(&operand(a), n).ok()?
        }
        7 => {
            let a = read_recipe(buf, pos, table, scale, depth + 1)?;
            let k = u32::try_from(u64::read(buf, pos)?).ok().filter(|k| (2..=MAX_POWER_DEGREE as u32).contains(k))?;
            if a.is_zero() {
                return None;
            }
            AlgValue::pow(&operand(a), k)
        }
        _ => return None,
    })
}

fn write_text(text: &str, out: &mut Vec<u8>) {
    write_len(text.len(), out);
    out.extend_from_slice(text.as_bytes());
}

fn read_text(buf: &[u8], pos: &mut usize) -> Option<String> {
    let n = read_len(buf, pos, MAX_WIRE_LEN)?;
    let bytes = buf.get(*pos..pos.checked_add(n)?)?;
    *pos += n;
    String::from_utf8(bytes.to_vec()).ok()
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_base/multi/tests.rs"]
mod tests;

/// The terms of a polynomial in the reciprocals of infinitesimals with rational coefficients, each its
/// monomial — the infinitesimals and the powers of their reciprocals, `monomial` the part already
/// read — and its coefficient; `None` for a value that is no such polynomial.
fn reciprocal_terms(m: &Multi, monomial: &mut Vec<(Arc<Scale>, u32)>, out: &mut Vec<(Vec<(Arc<Scale>, u32)>, Rational)>) -> Option<()> {
    match m {
        Multi::Standard(a) => {
            let r = a.to_rational()?;
            if !r.is_zero() {
                out.push((monomial.clone(), r));
            }
        }
        Multi::One(c, s) => {
            let (h, q) = c.field()?;
            if q != 1 {
                return None;
            }
            for (j, r) in h.as_reciprocal_powers()?.into_iter().enumerate() {
                if r.is_zero() {
                    continue;
                }
                let mut term = monomial.clone();
                if j > 0 {
                    term.push((s.clone(), j as u32));
                }
                out.push((term, r));
            }
        }
        Multi::Over { scale, q, num, den } => {
            let d = den.order()?;
            if den.c.iter().enumerate().any(|(i, c)| i != d && !c.is_zero()) {
                return None;
            }
            for (i, c) in num.c.iter().enumerate() {
                if c.is_zero() {
                    continue;
                }
                if i > d || (d - i) % *q as usize != 0 {
                    return None;
                }
                let j = ((d - i) / *q as usize) as u32;
                if j > 0 {
                    monomial.push((scale.clone(), j));
                }
                reciprocal_terms(&c.mul(&den.coeff(d).recip()?), monomial, out)?;
                if j > 0 {
                    monomial.pop();
                }
            }
        }
        Multi::Alg(_) => return None,
    }
    Some(())
}

/// The most joint residue classes [`Multi::is_integral`] reads.
const INTEGRAL_CLASSES: u64 = 1 << 16;
