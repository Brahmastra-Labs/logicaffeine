//! The real closure of the values of several infinitesimals: `√(dt + dx)`, `∛(h + dt)`, every root the
//! program's arithmetic reaches among the values of more than one infinitesimal, held exactly.
//!
//! A value of `dx`, declared much smaller than `dt`, that is no rational function of a root of `dx`
//! over the values of the larger infinitesimals is an [`AlgValue`]: the recipe that built it, a
//! squarefree polynomial over those rational functions with the value among its roots, and the value's
//! expansion in `dx` with exact coefficients ([`crate::expansion`]). The polynomial composes as the
//! value does — `α + β` is a root of `Res_z(P(z), Q(y − z))`, `α·β` of `Res_z(P(z), z^n·Q(y/z))`, `1/α`
//! of `P` reversed, `ⁿ√α` of `P(y^n)`, `α^k` of `Res_z(P(z), y − z^k)` — and decides what the expansion
//! alone cannot. The orders of `P`'s nonzero roots are the slopes of its Newton polygon, so a value
//! whose expansion vanishes through the largest of them is the root `0`, and a nonzero value's leading
//! term lies at or below it: every comparison is a bounded scan of exact coefficients.
//!
//! **One form for a value.** A value that is a rational function of a root of `dx` is held as that
//! function ([`Multi::Over`]): its candidate is the Padé approximant of its expansion within the
//! degrees Gauss's lemma bounds a root of the monic polynomial by, kept only when it is a root of the
//! polynomial and its difference from the value is the root `0`. A value prints as that function, as a
//! radical when a power of it is one (`√(dt + dx)`, `2·√(dt + dx)`, `1/√(dt + dx)`), or by its two
//! leading terms and the order of the rest (`1/(2·√dt) - (1/(8·dt^(3/2)))·dx + O(dx^2)`).

use std::sync::Arc;

use crate::algebraic::Algebraic;
use crate::expansion::{Expansion, ExpansionError, Source};
use crate::multi::Multi;
use crate::numeric::{BigInt, Rational};
use crate::ordered_field::{OrderedField, PolyOver};
use crate::puiseux::{expansion_term, remainder, remainder_after, Closed, Puiseux};
use crate::tower::Scale;

/// How an [`AlgValue`] was built, which is what it is written as.
#[derive(Clone, Debug)]
pub(crate) enum Recipe {
    /// A rational function of a root of the infinitesimal, or a value of the larger ones.
    Field(Multi),
    /// A value of the real closure of the infinitesimal alone.
    Closed(Puiseux),
    Add(Arc<AlgValue>, Arc<AlgValue>),
    Neg(Arc<AlgValue>),
    Mul(Arc<AlgValue>, Arc<AlgValue>),
    Recip(Arc<AlgValue>),
    Root(Arc<AlgValue>, u32),
    Pow(Arc<AlgValue>, u32),
}

/// A value of the real closure of several infinitesimals at its smallest. See the module
/// documentation.
#[derive(Clone, Debug)]
pub struct AlgValue {
    pub(crate) scale: Arc<Scale>,
    pub(crate) recipe: Recipe,
    /// Squarefree and monic, over the values of `scale` and the larger infinitesimals.
    pub(crate) poly: PolyOver<Multi>,
    pub(crate) expansion: Expansion<Multi>,
}

/// A value of one infinitesimal's real closure read as a value of several: its coefficients.
#[derive(Debug)]
struct ClosedSource(Puiseux);

impl Source<Multi> for ClosedSource {
    fn coefficient(&self, k: i64) -> Result<Multi, ExpansionError> {
        Ok(Multi::Standard(self.0.coefficient(k)))
    }
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

fn lcm_i64(a: i64, b: i64) -> i64 {
    a / gcd_u64(a.unsigned_abs(), b.unsigned_abs()) as i64 * b
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

/// The order in `scale` of a nonzero value of `scale` and the larger infinitesimals, in `scale`'s
/// own units: the exponent of its leading term, and `0` for a value of the larger ones alone.
fn valuation(a: &Multi, scale: &Arc<Scale>) -> Rational {
    match a.top() {
        Some(t) if t.same_infinitesimal(scale) => a.leading().expect("a nonzero value has a leading term").0,
        _ => Rational::zero(),
    }
}

/// How far a scan of an expansion reads for the terms a printed value shows past its first.
const SHOWN_REACH: i64 = 64;

/// The orders of two values' expansions read for a difference before their difference's polynomial is.
const EQUALS_ORDERS: i64 = 8;

/// The orders past the ones a Padé approximant `num/den` is built from at which `den·S − num` must vanish
/// before the candidate is made and checked exactly: an approximant that is no field value is rejected
/// by its series alone.
const PADE_CHECK_ORDERS: usize = 4;

impl AlgValue {
    /// A rational function of a root of `scale`, or a value of the larger infinitesimals, as the one
    /// root of `y − x`.
    pub(crate) fn field(x: &Multi, scale: &Arc<Scale>) -> AlgValue {
        let poly = PolyOver::from(vec![x.neg(), Multi::one()]);
        let expansion = match x.top() {
            Some(t) if t.same_infinitesimal(scale) => {
                let (q, num, den) = x.parts_at(scale);
                Expansion::fraction(num.c, den.c, q).expect("a value's denominator is nonzero")
            }
            _ => Expansion::constant(x.clone()),
        };
        AlgValue {
            scale: scale.clone(),
            recipe: Recipe::Field(x.clone()),
            poly,
            expansion,
        }
    }

    /// A value of one infinitesimal's real closure as a value of several: the polynomial it is a root
    /// of, its coefficients rational functions of a root of `scale`, and its own expansion.
    pub(crate) fn closed(p: &Puiseux, scale: &Arc<Scale>) -> AlgValue {
        let (coefficients, q) = p.annihilator();
        let poly = PolyOver::from(
            coefficients
                .into_iter()
                .map(|h| Multi::of_closed(Closed::of_field(h, q), scale))
                .collect(),
        )
        .monic();
        let expansion = Expansion::source(Arc::new(ClosedSource(p.clone())), p.order(), p.ramification());
        AlgValue {
            scale: scale.clone(),
            recipe: Recipe::Closed(p.clone()),
            poly,
            expansion,
        }
    }

    /// `m` as a value of the real closure at `scale`, its smallest infinitesimal or one smaller.
    pub(crate) fn at(m: &Multi, scale: &Arc<Scale>) -> AlgValue {
        match m {
            Multi::Alg(a) if a.scale.same_infinitesimal(scale) => (**a).clone(),
            Multi::One(Closed::Series(p), s) if s.same_infinitesimal(scale) && p.rational_function().is_none() => {
                AlgValue::closed(p, scale)
            }
            _ => AlgValue::field(m, scale),
        }
    }

    /// The largest order, on the expansion's grid, a nonzero root of the polynomial lies at; `None`
    /// when it has none. Each edge of the Newton polygon of the points `(i, v(a_i))` holds roots of
    /// the order its slope names.
    fn reach(&self) -> Option<i64> {
        let p = self.poly.without_zero_root();
        if p.degree().unwrap_or(0) == 0 {
            return None;
        }
        let points: Vec<(i64, Rational)> = p
            .c
            .iter()
            .enumerate()
            .filter(|(_, a)| !a.is_zero())
            .map(|(i, a)| (i as i64, valuation(a, &self.scale)))
            .collect();
        let common = points
            .iter()
            .map(|(_, v)| v.denominator().to_i64().expect("a small denominator"))
            .fold(1i64, lcm_i64);
        let scaled: Vec<(i64, i64)> = points
            .iter()
            .map(|(i, v)| {
                let w = v.mul(&Rational::from_i64(common));
                (*i, w.to_bigint().and_then(|b| b.to_i64()).expect("a small order"))
            })
            .collect();
        let hull = lower_hull(&scaled);
        let largest = hull
            .windows(2)
            .map(|e| Rational::from_ratio_i64(e[0].1 - e[1].1, (e[1].0 - e[0].0) * common).expect("a positive run"))
            .max()?;
        let on_grid = largest.mul(&Rational::from_i64(i64::from(self.expansion.ramification())));
        Some(on_grid.ceil().to_i64().expect("a small order"))
    }

    /// Whether the value is `0`: a root `0` of its polynomial whose expansion vanishes through the
    /// largest order of the polynomial's nonzero roots.
    pub(crate) fn is_zero(&self) -> bool {
        if !self.poly.coeff(0).is_zero() {
            return false;
        }
        match self.reach() {
            None => true,
            Some(reach) => self
                .expansion
                .leading_through(reach)
                .expect("exact coefficients")
                .is_none(),
        }
    }

    /// The leading term of a nonzero value: its order on the expansion's grid and its coefficient.
    pub(crate) fn leading(&self) -> (i64, Multi) {
        let reach = self.reach().expect("a nonzero value is a nonzero root of its polynomial");
        self.expansion
            .leading_through(reach)
            .expect("exact coefficients")
            .expect("a nonzero root's leading term lies at or below the largest order of the polynomial's roots")
    }

    /// The leading order in the infinitesimal's own units.
    pub(crate) fn order(&self) -> Rational {
        let (k, _) = self.leading();
        Rational::from_ratio_i64(k, i64::from(self.expansion.ramification())).expect("q is positive")
    }

    pub(crate) fn sign(&self) -> i32 {
        OrderedField::signum(&self.leading().1)
    }

    fn of(&self, recipe: Recipe, poly: PolyOver<Multi>, expansion: Expansion<Multi>) -> AlgValue {
        AlgValue {
            scale: self.scale.clone(),
            recipe,
            poly: poly.squarefree(),
            expansion,
        }
    }

    pub(crate) fn add(a: &Arc<AlgValue>, b: &Arc<AlgValue>) -> Multi {
        let poly = PolyOver::sum_poly(&a.poly, &b.poly);
        a.of(Recipe::Add(a.clone(), b.clone()), poly, a.expansion.add(&b.expansion)).canonical()
    }

    pub(crate) fn neg(a: &Arc<AlgValue>) -> Multi {
        a.of(Recipe::Neg(a.clone()), a.poly.neg_var(), a.expansion.neg()).canonical()
    }

    /// The product of two nonzero values.
    pub(crate) fn mul(a: &Arc<AlgValue>, b: &Arc<AlgValue>) -> Multi {
        let poly = PolyOver::product_poly(&a.poly.without_zero_root(), &b.poly.without_zero_root());
        a.of(Recipe::Mul(a.clone(), b.clone()), poly, a.expansion.mul(&b.expansion)).canonical()
    }

    /// The reciprocal of a nonzero value.
    pub(crate) fn recip(a: &Arc<AlgValue>) -> Multi {
        let (k, c) = a.leading();
        a.of(Recipe::Recip(a.clone()), a.poly.reversed(), a.expansion.recip(k, c)).canonical()
    }

    /// The real `n`-th root of a nonzero value, the positive one for an even `n`.
    pub(crate) fn root(a: &Arc<AlgValue>, n: u32) -> Result<Multi, String> {
        let (k, c) = a.leading();
        let expansion = a.expansion.root(n, k, c).map_err(|e| e.to_string())?;
        Ok(a.of(Recipe::Root(a.clone(), n), a.poly.spread(n as usize), expansion).canonical())
    }

    /// `a^k` for `k ≥ 1`: the polynomial of the `k`-th powers of its roots, and the product of `k`
    /// expansions by repeated squaring.
    pub(crate) fn pow(a: &Arc<AlgValue>, k: u32) -> Multi {
        let poly = PolyOver::power_poly(&a.poly, k);
        let mut base = a.expansion.clone();
        let mut acc: Option<Expansion<Multi>> = None;
        let mut e = k;
        while e > 0 {
            if e & 1 == 1 {
                acc = Some(match acc {
                    None => base.clone(),
                    Some(x) => x.mul(&base),
                });
            }
            e >>= 1;
            if e > 0 {
                base = base.mul(&base);
            }
        }
        a.of(Recipe::Pow(a.clone(), k), poly, acc.expect("k ≥ 1")).canonical()
    }

    /// The value in its one form: `0`, the rational function it is, or itself.
    fn canonical(self) -> Multi {
        if self.is_zero() {
            return <Multi as OrderedField>::zero();
        }
        if self.poly.degree() == Some(1) {
            return self.poly.coeff(0).neg().mul(&self.poly.coeff(1).recip().expect("a leading coefficient is nonzero"));
        }
        if let Some(x) = self.field_value() {
            return x;
        }
        Multi::Alg(Arc::new(self))
    }

    /// The rational function of a root of the infinitesimal the value is, when it is one. See the
    /// module documentation.
    fn field_value(&self) -> Option<Multi> {
        let scale = &self.scale;
        let monic = self.poly.monic();
        let parts: Vec<(u32, PolyOver<Multi>, PolyOver<Multi>)> = monic
            .c
            .iter()
            .map(|a| match a {
                Multi::Alg(_) => None,
                Multi::One(c, s) if s.same_infinitesimal(scale) && c.field().is_none() => None,
                _ => Some(a.parts_at(scale)),
            })
            .collect::<Option<_>>()?;
        let qe = self.expansion.ramification();
        let q = parts.iter().fold(qe, |l, (qa, _, _)| lcm_u32(l, *qa));
        let degrees: Vec<(usize, usize)> = parts
            .iter()
            .map(|(qa, n, d)| {
                let m = (q / qa) as usize;
                (n.degree().unwrap_or(0) * m, d.degree().unwrap_or(0) * m)
            })
            .collect();
        let lcm_bound: usize = degrees.iter().map(|(_, d)| d).sum();
        let (n0, d0) = degrees[0];
        let num_bound = (lcm_bound + n0).checked_sub(d0)?;
        let refined = self.expansion.refined(q / qe);
        let (k, _) = self.leading();
        let v = k * i64::from(q / qe);
        let (l, m) = if v >= 0 {
            (num_bound.checked_sub(usize::try_from(v).ok()?)?, lcm_bound)
        } else {
            (num_bound, lcm_bound.checked_sub(usize::try_from(-v).ok()?)?)
        };
        let series: Vec<Multi> = (0..=(l + m) as i64)
            .map(|i| refined.coefficient(v + i))
            .collect::<Result<_, _>>()
            .ok()?;
        let (num, den) = PolyOver::pade(&series, l, m)?;
        'past: for k in (l + m + 1)..=(l + m + PADE_CHECK_ORDERS) {
            let mut past = OrderedField::neg(&num.coeff(k));
            for (j, d) in den.c.iter().enumerate().take(k + 1) {
                let Ok(term) = refined.coefficient(v + (k - j) as i64) else {
                    break 'past;
                };
                past = OrderedField::add(&past, &OrderedField::mul(d, &term));
            }
            if !OrderedField::is_zero(&past) {
                return None;
            }
        }
        let shift = |p: &PolyOver<Multi>, by: usize| {
            let mut c = vec![<Multi as OrderedField>::zero(); by];
            c.extend(p.c.iter().cloned());
            PolyOver::from(c)
        };
        let (num, den) = if v >= 0 {
            (shift(&num, v as usize), den)
        } else {
            (num, shift(&den, (-v) as usize))
        };
        let candidate = Multi::make(scale, q, num, den);
        if !OrderedField::is_zero(&monic.eval(&candidate)) {
            return None;
        }
        let at_candidate = AlgValue::field(&candidate, scale);
        let difference = AlgValue {
            scale: scale.clone(),
            recipe: Recipe::Field(candidate.clone()),
            poly: monic.shift(&candidate),
            expansion: self.expansion.sub(&at_candidate.expansion),
        };
        difference.is_zero().then_some(candidate)
    }

    /// Whether the two are built the same way from the same values, and so are one value.
    fn same_recipe(a: &Arc<AlgValue>, b: &Arc<AlgValue>) -> bool {
        if Arc::ptr_eq(a, b) {
            return true;
        }
        if !a.scale.same_infinitesimal(&b.scale) {
            return false;
        }
        let same = AlgValue::same_recipe;
        match (&a.recipe, &b.recipe) {
            (Recipe::Field(x), Recipe::Field(y)) => x == y,
            (Recipe::Closed(x), Recipe::Closed(y)) => Closed::Series(x.clone()) == Closed::Series(y.clone()),
            (Recipe::Add(a1, a2), Recipe::Add(b1, b2)) | (Recipe::Mul(a1, a2), Recipe::Mul(b1, b2)) => {
                same(a1, b1) && same(a2, b2)
            }
            (Recipe::Neg(x), Recipe::Neg(y)) | (Recipe::Recip(x), Recipe::Recip(y)) => same(x, y),
            (Recipe::Root(x, n), Recipe::Root(y, m)) | (Recipe::Pow(x, n), Recipe::Pow(y, m)) => n == m && same(x, y),
            _ => false,
        }
    }

    /// Whether the two are one value: their difference is `0`. Two values built the same way are; two
    /// values that differ differ in their expansions, so a difference among the first
    /// [`EQUALS_ORDERS`] orders both read answers at once; only values that agree that far are told
    /// apart by their difference's polynomial.
    pub(crate) fn equals(a: &Arc<AlgValue>, b: &Arc<AlgValue>) -> bool {
        if !a.scale.same_infinitesimal(&b.scale) {
            return false;
        }
        if AlgValue::same_recipe(a, b) {
            return true;
        }
        let difference = a.expansion.sub(&b.expansion);
        let through = difference.floor() + EQUALS_ORDERS * i64::from(difference.ramification());
        if let Ok(Some(_)) = difference.leading_through(through) {
            return false;
        }
        OrderedField::is_zero(&OrderedField::sub(&Multi::Alg(a.clone()), &Multi::Alg(b.clone())))
    }

    // ── printing ──────────────────────────────────────────────────────────────────────────────

    /// A radical when a power of the value is a rational function of a root of its infinitesimal,
    /// otherwise its two leading terms and the order of the rest. A value of degree two — its
    /// polynomial, a zero root aside, `z² + p·z + q`, its minimal one since the value is no field
    /// value — is a radical exactly when `p = 0`: were `y² ∈ K` with `p ≠ 0`, the conjugate
    /// `y₂ = −p − y` would have `y₂² ∈ K` too, and `y = (−p − (y² − y₂²)/p)/2` would lie in `K`.
    pub(crate) fn render(a: &Arc<AlgValue>) -> String {
        if let Some(text) = AlgValue::radical(a) {
            return text;
        }
        a.leading_terms()
    }

    /// `F·ⁿ√κ` for the least `n` whose power of the value is a rational function `κ` of a root of the
    /// infinitesimal, an integer factor of `κ` that is an `n`-th power taken out into `F`; a value whose
    /// power has a constant numerator prints as one over the radical of the reciprocal.
    fn radical(a: &Arc<AlgValue>) -> Option<String> {
        let degree = a.poly.degree().unwrap_or(0) as u32;
        let own = a.poly.without_zero_root();
        if own.degree() == Some(2) && !OrderedField::is_zero(&own.coeff(1)) {
            return None;
        }
        let negative = a.sign() < 0;
        for n in 2..=degree.max(2) {
            let kappa = AlgValue::pow(a, n);
            if matches!(kappa, Multi::Alg(_)) {
                continue;
            }
            let (num, den) = kappa.fraction_parts();
            let inverted = num.top().is_none() && den.top().is_some();
            let kappa = if inverted { kappa.recip()? } else { kappa };
            let (num, den) = kappa.fraction_parts();
            let (cn, num) = integer_content(&num);
            let (cd, den) = integer_content(&den);
            let constant = Rational::new(cn, cd).expect("a content is positive");
            let root = Algebraic::from_rational(constant.clone())
                .nth_root(n)
                .and_then(|r| r.to_rational());
            let (outside, inside) = match root {
                Some(r) => (r, num.mul(&den.recip().expect("a denominator is nonzero"))),
                None => (
                    Rational::one(),
                    num.mul(&den.recip().expect("a denominator is nonzero"))
                        .mul(&Multi::Standard(Algebraic::from_rational(constant))),
                ),
            };
            let outside = if n % 2 == 0 && negative { outside.negated() } else { outside };
            let radicand = inside.render();
            let radicand = if radicand.contains([' ', '/', '·', '^']) { format!("({radicand})") } else { radicand };
            let sign = radical_sign(n);
            let factor = outside.to_string();
            let (minus, factor) = match factor.strip_prefix('-') {
                Some(rest) => ("-", rest.to_string()),
                None => ("", factor),
            };
            let factor = if factor.contains('/') { format!("({factor})") } else { factor };
            let root = if factor == "1" { format!("{sign}{radicand}") } else { format!("{factor}·{sign}{radicand}") };
            return Some(if inverted {
                let root = if root.contains('·') { format!("({root})") } else { root };
                format!("{minus}1/{root}")
            } else {
                format!("{minus}{root}")
            });
        }
        None
    }

    /// The two leading terms and, from the order after the last one shown, what is left.
    fn leading_terms(&self) -> String {
        let name = &self.scale.name;
        let q = i64::from(self.expansion.ramification());
        let (k0, c0) = self.leading();
        let next_after = |from: i64| -> Option<(i64, Multi)> {
            for k in from + 1..=from + SHOWN_REACH * q {
                let c = self.expansion.coefficient(k).expect("exact coefficients");
                if !OrderedField::is_zero(&c) {
                    return Some((k, c));
                }
            }
            None
        };
        let exponent = |k: i64| Rational::from_ratio_i64(k, q).expect("q is positive");
        let mut terms = vec![(k0, c0)];
        if let Some(second) = next_after(k0) {
            terms.push(second);
        }
        let mut out = String::new();
        for (i, (k, c)) in terms.iter().enumerate() {
            let t = expansion_term(name, &c.render(), &exponent(*k));
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
        let shown: Vec<Rational> = terms.iter().map(|(k, _)| exponent(*k)).collect();
        let last = terms.last().expect("a term is shown").0;
        let next = next_after(last).map(|(k, _)| exponent(k)).unwrap_or_else(|| {
            exponent(last + SHOWN_REACH * q)
        });
        format!("{out} + {}", remainder(name, &remainder_after(&shown, &next)))
    }
}

/// `√`, `∛`, `∜`, or the index written small before `√`.
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

/// A polynomial in the infinitesimals as its positive integer content and the rest: the gcd of its
/// standard integer coefficients, `1` when any is not one.
fn integer_content(p: &Multi) -> (BigInt, Multi) {
    let mut leaves: Vec<Rational> = Vec::new();
    if !collect_leaves(p, &mut leaves) {
        return (BigInt::from_i64(1), p.clone());
    }
    let g = leaves
        .iter()
        .filter(|r| !r.is_zero())
        .fold(BigInt::from_i64(0), |g, r| g.gcd(r.numerator()));
    if g.is_zero() || g == BigInt::from_i64(1) {
        return (BigInt::from_i64(1), p.clone());
    }
    let inv = Multi::Standard(Algebraic::from_rational(Rational::new(BigInt::from_i64(1), g.clone()).expect("g ≠ 0")));
    (g, p.mul(&inv))
}

/// The standard coefficients of a polynomial in the infinitesimals, when every one is an integer.
fn collect_leaves(p: &Multi, out: &mut Vec<Rational>) -> bool {
    match p {
        Multi::Standard(a) => match a.to_rational().filter(Rational::is_integer) {
            Some(r) => {
                out.push(r);
                true
            }
            None => false,
        },
        Multi::One(c, _) => match c.field() {
            Some((h, _)) => {
                let (num, den) = h.parts();
                if *den != PolyOver::one() {
                    return false;
                }
                num.c.iter().all(|a| match a.to_rational().filter(Rational::is_integer) {
                    Some(r) => {
                        out.push(r);
                        true
                    }
                    None => false,
                })
            }
            None => false,
        },
        Multi::Over { num, den, .. } => {
            *den == PolyOver::one() && num.c.iter().all(|c| collect_leaves(c, out))
        }
        Multi::Alg(_) => false,
    }
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_base/closure/tests.rs"]
mod tests;
