//! Germs of several infinitesimals: a value of `dx`, declared much smaller than `dt`, known by its
//! expansion in `dx` whose coefficients are values of `dt` and of the infinitesimals larger than it,
//! not all of them exact — `exp(dt + dx) = Σ (e^dt/k!)·dx^k`, `ln(1 + dt + dx)`, the state a loop
//! stepped by `dx` leaves when its rate is a value of `dt`.
//!
//! A [`NestedGerm`] holds such a value as the recipe that built it and its [`Expansion`] over
//! [`Number`] coefficients, each computed once as a comparison or a printed term asks. The functions
//! are the recurrences one infinitesimal's germs follow ([`crate::germ`]), one level down: `exp(g)` for
//! a `g` with no negative order starts at `e^{g_0}`, a coefficient's own exponential; `ln(g)` for a `g`
//! of order `0` with a positive leading coefficient starts at `ln(g_0)`. What is no germ is refused by
//! name: the exponential of a value infinite in the smallest infinitesimal lies beyond every power of
//! it, and the logarithm of a value of nonzero order carries the logarithm of that infinitesimal.
//!
//! Signs are read from the first nonzero coefficient, through [`crate::germ::SIGN_ORDERS`] orders: a
//! coefficient that cannot be told from zero, or a germ whose first orders all vanish, is a loud
//! error, never a guess. Two germs whose recipes are the same are the same value, so a term both sides
//! of a difference share cancels exactly.

use std::sync::Arc;

use crate::closure::AlgValue;
use crate::expansion::{Coefficient, Expansion, ExpansionError, Source};
use crate::germ::{Germ, SIGN_ORDERS};
use crate::multi::Multi;
use crate::nested_flow::{FlowSpec, MatrixExpSpec, PowerSpec};
use crate::number::Number;
use crate::numeric::Rational;
use crate::puiseux::{expansion_term, remainder, remainder_after};
use crate::tower::{self, ExactRef, HyperForm, Scale, TowerValue};

/// How a [`NestedGerm`] was built.
#[derive(Clone, Debug)]
pub(crate) enum GermRecipe {
    /// An exact value of the infinitesimal and the larger ones, read by its expansion.
    Exact(Multi),
    /// A germ of the infinitesimal alone, its coefficients computable reals.
    Germ(Germ),
    /// A value of the larger infinitesimals, or a computable real: constant in this one.
    Constant(Number),
    Add(Arc<NestedGerm>, Arc<NestedGerm>),
    Sub(Arc<NestedGerm>, Arc<NestedGerm>),
    Neg(Arc<NestedGerm>),
    Mul(Arc<NestedGerm>, Arc<NestedGerm>),
    Recip(Arc<NestedGerm>),
    Root(Arc<NestedGerm>, u32),
    Exp(Arc<NestedGerm>),
    Ln(Arc<NestedGerm>),
    /// An entry of the exponential of a matrix of numbers ([`crate::nested_flow::matrix_exp`]).
    MatrixExp(Arc<MatrixExpSpec>, usize, usize),
    /// A component of a flow whose rates are values of larger infinitesimals.
    Flow(Arc<FlowSpec>, usize),
    /// A component of a power whose rates are values of this, a smaller infinitesimal.
    Power(Arc<PowerSpec>, usize),
}

/// A germ of several infinitesimals at its smallest. See the module documentation.
#[derive(Clone, Debug)]
pub struct NestedGerm {
    pub(crate) scale: Arc<Scale>,
    pub(crate) recipe: GermRecipe,
    pub(crate) expansion: Expansion<Number>,
}

/// An exact value's expansion read with [`Number`] coefficients.
#[derive(Debug)]
struct ExactSource(Expansion<Multi>);

impl Source<Number> for ExactSource {
    fn coefficient(&self, k: i64) -> Result<Number, ExpansionError> {
        Ok(tower::from_multi(self.0.coefficient(k)?))
    }
}

/// A germ of one infinitesimal read with [`Number`] coefficients.
#[derive(Debug)]
struct GermSource(Germ);

impl Source<Number> for GermSource {
    fn coefficient(&self, k: i64) -> Result<Number, ExpansionError> {
        self.0
            .coefficient(k)
            .map(tower::from_exact_real)
            .map_err(|e| ExpansionError::Coefficient(e.to_string()))
    }
}

/// The exponent of a nonzero number's leading term in its smallest infinitesimal, with that
/// infinitesimal; `None` for a standard number. Refused when the term cannot be found.
pub(crate) fn leading_order(n: &Number) -> Result<Option<(Arc<Scale>, Rational)>, String> {
    let Some(ExactRef::Hyper(h)) = n.exact() else {
        return Ok(None);
    };
    let order = match &h.value {
        HyperForm::Rational(r) => Rational::from_i64(r.valuation().expect("a hyperreal held as one is nonzero")),
        HyperForm::Puiseux(p) => {
            Rational::from_ratio_i64(p.order(), i64::from(p.ramification())).expect("q is positive")
        }
        HyperForm::Germ(g) => {
            let (k, _) = g.leading().map_err(|e| e.to_string())?;
            Rational::from_ratio_i64(k, i64::from(g.ramification())).expect("q is positive")
        }
        HyperForm::Multi(m) | HyperForm::Integer(m) => m.leading().expect("a value of several infinitesimals is nonzero").0,
        HyperForm::Nested(g) => g.order()?,
    };
    Ok(Some((h.scale.clone(), order)))
}

/// The coefficient of a nonzero number's leading term in its smallest infinitesimal: a value of the
/// larger ones.
fn leading_coefficient(n: &Number) -> Result<Number, String> {
    let Some(ExactRef::Hyper(h)) = n.exact() else {
        return Ok(n.clone());
    };
    Ok(match &h.value {
        HyperForm::Rational(r) => tower::from_algebraic(r.leading_coefficient().expect("nonzero")),
        HyperForm::Puiseux(p) => {
            tower::from_algebraic(p.coefficient(p.order()))
        }
        HyperForm::Germ(g) => tower::from_exact_real(g.leading().map_err(|e| e.to_string())?.1),
        HyperForm::Multi(m) | HyperForm::Integer(m) => tower::from_multi(m.leading().expect("nonzero").1),
        HyperForm::Nested(g) => g.leading()?.1,
    })
}

/// Every infinitesimal a number involves.
fn number_scales(n: &Number) -> Vec<Arc<Scale>> {
    match n.exact() {
        Some(ExactRef::Hyper(h)) => match &h.value {
            HyperForm::Multi(m) | HyperForm::Integer(m) => m.scales(),
            HyperForm::Nested(g) => g.scales(),
            _ => vec![h.scale.clone()],
        },
        Some(ExactRef::Natural(m)) => vec![m.scale.clone()],
        _ => Vec::new(),
    }
}

/// The infinitesimal `scale` itself, as a number.
fn generator(scale: &Arc<Scale>) -> Number {
    Number::hyper(tower::HyperValue {
        value: HyperForm::Rational(crate::hyperreal::Hyperreal::dt()),
        scale: scale.clone(),
    })
}

impl NestedGerm {
    /// Every infinitesimal the value involves: its own and those of its recipe's leaves.
    pub(crate) fn scales(&self) -> Vec<Arc<Scale>> {
        let mut out = vec![self.scale.clone()];
        self.recipe_scales(&mut out);
        out
    }

    fn recipe_scales(&self, out: &mut Vec<Arc<Scale>>) {
        let push = |s: Arc<Scale>, out: &mut Vec<Arc<Scale>>| {
            if !out.iter().any(|t| t.same_infinitesimal(&s)) {
                out.push(s);
            }
        };
        match &self.recipe {
            GermRecipe::Exact(m) => m.scales().into_iter().for_each(|s| push(s, out)),
            GermRecipe::Germ(_) => {}
            GermRecipe::Constant(n) => number_scales(n).into_iter().for_each(|s| push(s, out)),
            GermRecipe::Add(a, b) | GermRecipe::Sub(a, b) | GermRecipe::Mul(a, b) => {
                a.recipe_scales(out);
                b.recipe_scales(out);
            }
            GermRecipe::Neg(a)
            | GermRecipe::Recip(a)
            | GermRecipe::Root(a, _)
            | GermRecipe::Exp(a)
            | GermRecipe::Ln(a) => a.recipe_scales(out),
            GermRecipe::MatrixExp(spec, _, _) => {
                spec.a.iter().flatten().flat_map(number_scales).for_each(|s| push(s, out))
            }
            GermRecipe::Flow(spec, _) => spec
                .a
                .iter()
                .flatten()
                .chain(spec.start.iter())
                .flat_map(number_scales)
                .for_each(|s| push(s, out)),
            GermRecipe::Power(spec, _) => spec
                .a
                .iter()
                .flatten()
                .chain(spec.start.iter())
                .chain(std::iter::once(&spec.count))
                .flat_map(number_scales)
                .for_each(|s| push(s, out)),
        }
    }

    /// Whether two germs were built by the same recipe, which makes them the same value.
    pub(crate) fn same_recipe(a: &NestedGerm, b: &NestedGerm) -> bool {
        if !a.scale.same_infinitesimal(&b.scale) {
            return false;
        }
        match (&a.recipe, &b.recipe) {
            (GermRecipe::Exact(x), GermRecipe::Exact(y)) => x == y,
            (GermRecipe::Germ(x), GermRecipe::Germ(y)) => {
                use crate::exact_codec::ExactCodec;
                x.is_transmissible() && y.is_transmissible() && x.to_bytes() == y.to_bytes()
            }
            (GermRecipe::Constant(x), GermRecipe::Constant(y)) => x.values_equal(y),
            (GermRecipe::Add(p, q), GermRecipe::Add(r, s))
            | (GermRecipe::Sub(p, q), GermRecipe::Sub(r, s))
            | (GermRecipe::Mul(p, q), GermRecipe::Mul(r, s)) => {
                NestedGerm::same_recipe(p, r) && NestedGerm::same_recipe(q, s)
            }
            (GermRecipe::Neg(p), GermRecipe::Neg(r))
            | (GermRecipe::Recip(p), GermRecipe::Recip(r))
            | (GermRecipe::Exp(p), GermRecipe::Exp(r))
            | (GermRecipe::Ln(p), GermRecipe::Ln(r)) => NestedGerm::same_recipe(p, r),
            (GermRecipe::Root(p, n), GermRecipe::Root(r, m)) => n == m && NestedGerm::same_recipe(p, r),
            (GermRecipe::MatrixExp(p, i, j), GermRecipe::MatrixExp(r, k, l)) => Arc::ptr_eq(p, r) && (i, j) == (k, l),
            (GermRecipe::Flow(p, i), GermRecipe::Flow(r, k)) => Arc::ptr_eq(p, r) && i == k,
            (GermRecipe::Power(p, i), GermRecipe::Power(r, k)) => Arc::ptr_eq(p, r) && i == k,
            _ => false,
        }
    }

    /// A germ whose expansion a source gives.
    pub(crate) fn from_source(scale: &Arc<Scale>, recipe: GermRecipe, expansion: Expansion<Number>) -> NestedGerm {
        NestedGerm::of(scale, recipe, expansion)
    }

    fn of(scale: &Arc<Scale>, recipe: GermRecipe, expansion: Expansion<Number>) -> NestedGerm {
        NestedGerm {
            scale: scale.clone(),
            recipe,
            expansion,
        }
    }

    /// An exact value of `scale` and the larger infinitesimals, read as a germ at `scale`.
    pub(crate) fn exact(m: &Multi, scale: &Arc<Scale>) -> NestedGerm {
        match m.top() {
            Some(t) if t.same_infinitesimal(scale) => {
                let expansion = AlgValue::at(m, scale).expansion;
                let (floor, q) = (expansion.floor(), expansion.ramification());
                NestedGerm::of(scale, GermRecipe::Exact(m.clone()), Expansion::source(Arc::new(ExactSource(expansion)), floor, q))
            }
            _ => NestedGerm::constant(tower::from_multi(m.clone()), scale),
        }
    }

    /// A germ of `scale` alone, read with [`Number`] coefficients.
    pub(crate) fn germ(g: &Germ, scale: &Arc<Scale>) -> NestedGerm {
        let expansion = Expansion::source(Arc::new(GermSource(g.clone())), g.floor(), g.ramification());
        NestedGerm::of(scale, GermRecipe::Germ(g.clone()), expansion)
    }

    /// A number constant in `scale`.
    pub(crate) fn constant(n: Number, scale: &Arc<Scale>) -> NestedGerm {
        NestedGerm::of(scale, GermRecipe::Constant(n.clone()), Expansion::constant(n))
    }

    fn arc(self) -> Arc<NestedGerm> {
        Arc::new(self)
    }

    pub(crate) fn add(a: &Arc<NestedGerm>, b: &Arc<NestedGerm>) -> NestedGerm {
        NestedGerm::of(&a.scale, GermRecipe::Add(a.clone(), b.clone()), a.expansion.add(&b.expansion))
    }

    pub(crate) fn sub(a: &Arc<NestedGerm>, b: &Arc<NestedGerm>) -> NestedGerm {
        NestedGerm::of(&a.scale, GermRecipe::Sub(a.clone(), b.clone()), a.expansion.sub(&b.expansion))
    }

    pub(crate) fn neg(a: &Arc<NestedGerm>) -> NestedGerm {
        NestedGerm::of(&a.scale, GermRecipe::Neg(a.clone()), a.expansion.neg())
    }

    pub(crate) fn mul(a: &Arc<NestedGerm>, b: &Arc<NestedGerm>) -> NestedGerm {
        NestedGerm::of(&a.scale, GermRecipe::Mul(a.clone(), b.clone()), a.expansion.mul(&b.expansion))
    }

    /// The orders a sign or a leading term looks through.
    fn reach(&self) -> i64 {
        self.expansion.floor() + SIGN_ORDERS * i64::from(self.expansion.ramification())
    }

    /// The leading term: its order on the expansion's grid and its coefficient; refused when the first
    /// orders all vanish or a coefficient cannot be told from zero.
    pub(crate) fn leading(&self) -> Result<(i64, Number), String> {
        match self.expansion.leading_through(self.reach()) {
            Ok(Some(term)) => Ok(term),
            Ok(None) => Err(crate::germ::GermError::Flat.to_string()),
            Err(e) => Err(e.to_string()),
        }
    }

    /// The leading order in the infinitesimal's own units.
    pub(crate) fn order(&self) -> Result<Rational, String> {
        let (k, _) = self.leading()?;
        Ok(Rational::from_ratio_i64(k, i64::from(self.expansion.ramification())).expect("q is positive"))
    }

    pub(crate) fn sign(&self) -> Result<i32, String> {
        let (_, c) = self.leading()?;
        c.sign().map_err(|e| e.to_string())
    }

    pub(crate) fn recip(a: &Arc<NestedGerm>) -> Result<NestedGerm, String> {
        let (k, c) = a.leading()?;
        Ok(NestedGerm::of(&a.scale, GermRecipe::Recip(a.clone()), a.expansion.recip(k, c)))
    }

    /// The real `n`-th root, the nonnegative one for an even `n`.
    pub(crate) fn root(a: &Arc<NestedGerm>, n: u32) -> Result<NestedGerm, String> {
        if n == 1 {
            return Ok((**a).clone());
        }
        let (k, c) = a.leading()?;
        let expansion = a.expansion.root(n, k, c).map_err(|e| e.to_string())?;
        Ok(NestedGerm::of(&a.scale, GermRecipe::Root(a.clone(), n), expansion))
    }

    /// `a^k` for an integer `k`, by repeated squaring.
    pub(crate) fn pow(a: &Arc<NestedGerm>, k: i32) -> Result<NestedGerm, String> {
        let mut base = if k < 0 { NestedGerm::recip(a)?.arc() } else { a.clone() };
        let mut acc: Option<Arc<NestedGerm>> = None;
        let mut e = k.unsigned_abs();
        if e == 0 {
            return Ok(NestedGerm::constant(Number::one(), &a.scale));
        }
        while e > 0 {
            if e & 1 == 1 {
                acc = Some(match acc {
                    None => base.clone(),
                    Some(x) => NestedGerm::mul(&x, &base).arc(),
                });
            }
            e >>= 1;
            if e > 0 {
                base = NestedGerm::mul(&base, &base).arc();
            }
        }
        Ok((*acc.expect("k ≠ 0")).clone())
    }

    /// `exp(a)`, refused for a value infinite in the infinitesimal, whose exponential lies beyond every
    /// power of it; `shown` is how the value prints.
    pub(crate) fn exp(a: &Arc<NestedGerm>, shown: &str) -> Result<NestedGerm, String> {
        let (k, _) = a.leading()?;
        if k < 0 {
            return Err(format!(
                "the exponential of {shown} is beyond every power of {}",
                a.scale.name
            ));
        }
        Ok(NestedGerm::of(&a.scale, GermRecipe::Exp(a.clone()), a.expansion.exp()))
    }

    /// The natural logarithm of a positive value of order `0` whose leading coefficient has a
    /// logarithm; a value of another order, or whose leading coefficient is one, carries the logarithm
    /// of that infinitesimal. `shown` is how the value prints.
    pub(crate) fn ln(a: &Arc<NestedGerm>, shown: &str) -> Result<NestedGerm, String> {
        let (k, c) = a.leading()?;
        if c.sign().map_err(|e| e.to_string())? < 0 {
            return Err("logarithm of a nonpositive number".to_string());
        }
        let carries = |infinite: bool, name: &str| {
            format!(
                "the logarithm of {shown} carries ln({}{name})",
                if infinite { "1/" } else { "" }
            )
        };
        if k != 0 {
            return Err(carries(k < 0, &a.scale.name));
        }
        let mut lead = c.clone();
        while let Some((scale, order)) = leading_order(&lead)? {
            if !order.is_zero() {
                return Err(carries(order.is_negative(), &scale.name));
            }
            lead = leading_coefficient(&lead)?;
        }
        Ok(NestedGerm::of(&a.scale, GermRecipe::Ln(a.clone()), a.expansion.ln(c)))
    }

    /// What the value is infinitely close to from the standpoint of `at` (the standard part with no
    /// `at`): the leading coefficient's own, when the value is limited there; `None` when infinite.
    pub(crate) fn standard_part_at(&self, at: Option<&Arc<Scale>>) -> Result<Option<Number>, String> {
        if let Some(at) = at {
            if self.scale.order(at).expect("the standpoint is ordered with the value's infinitesimals")
                != std::cmp::Ordering::Less
            {
                return Ok(Some(Number::hyper(tower::HyperValue {
                    value: HyperForm::Nested(Arc::new(self.clone())),
                    scale: self.scale.clone(),
                })));
            }
        }
        let first = self.expansion.floor();
        for k in first..0 {
            let c = self.expansion.coefficient(k).map_err(|e| e.to_string())?;
            if !c.is_zero().map_err(|e| e.to_string())? {
                return Ok(None);
            }
        }
        let c0 = self.expansion.coefficient(0).map_err(|e| e.to_string())?;
        match at {
            None => tower::standard_part(&c0).map(Some),
            Some(scale) => tower::standard_part_relative(&c0, &generator(scale)).map(Some),
        }
    }

    /// The two leading terms and, from the order after the last one shown, what is left; each
    /// coefficient prints as its own value, parenthesized when it carries a remainder of its own.
    pub(crate) fn render(&self) -> String {
        let name = &self.scale.name;
        let q = i64::from(self.expansion.ramification());
        let reach = self.reach();
        let mut terms: Vec<(i64, Number)> = Vec::new();
        let mut k = self.expansion.floor();
        let mut next: Option<i64> = None;
        while k <= reach {
            match self.expansion.coefficient(k) {
                Ok(c) => match c.is_zero() {
                    Ok(true) => {}
                    Ok(false) if terms.len() < 2 => terms.push((k, c)),
                    Ok(false) => {
                        next = Some(k);
                        break;
                    }
                    Err(_) => break,
                },
                Err(_) => break,
            }
            k += 1;
        }
        let exponent = |k: i64| Rational::from_ratio_i64(k, q).expect("q is positive");
        let mut out = String::new();
        for (i, (k, c)) in terms.iter().enumerate() {
            let text = c.display_string();
            let text = if *k == 0 && text.contains("O(") { format!("({text})") } else { text };
            let t = expansion_term(name, &text, &exponent(*k));
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
        let rest = match (shown.is_empty(), next) {
            (true, _) => remainder(name, &exponent(k)),
            (false, Some(n)) => remainder(name, &remainder_after(&shown, &exponent(n))),
            (false, None) => remainder(name, &remainder_after(&shown, &exponent(k))),
        };
        if out.is_empty() {
            rest
        } else {
            format!("{out} + {rest}")
        }
    }
}

// ── bytes ─────────────────────────────────────────────────────────────────────────────────────

impl NestedGerm {
    /// Whether the germ can travel as data: every leaf of its recipe can.
    pub(crate) fn is_transmissible(&self) -> bool {
        match &self.recipe {
            GermRecipe::Exact(_) => true,
            GermRecipe::Germ(g) => g.is_transmissible(),
            GermRecipe::Constant(n) => tower::transmissible(n),
            GermRecipe::Add(a, b) | GermRecipe::Sub(a, b) | GermRecipe::Mul(a, b) => {
                a.is_transmissible() && b.is_transmissible()
            }
            GermRecipe::Neg(a)
            | GermRecipe::Recip(a)
            | GermRecipe::Root(a, _)
            | GermRecipe::Exp(a)
            | GermRecipe::Ln(a) => a.is_transmissible(),
            GermRecipe::MatrixExp(spec, _, _) => spec.a.iter().flatten().all(tower::transmissible),
            GermRecipe::Flow(spec, _) => spec.a.iter().flatten().chain(spec.start.iter()).all(tower::transmissible),
            GermRecipe::Power(spec, _) => spec
                .a
                .iter()
                .flatten()
                .chain(spec.start.iter())
                .chain(std::iter::once(&spec.count))
                .all(tower::transmissible),
        }
    }

    /// The germ's bytes: its infinitesimal by name and declaration, then its recipe — a tag and its
    /// parts: `0` an exact value, `1` a germ of the infinitesimal alone, `2` a constant (a computable
    /// real or a value of the larger infinitesimals), `3` a sum, `4` a difference, `5` a negation, `6` a
    /// product, `7` a reciprocal, `8` a root and its index, `9` an exponential, `10` a logarithm. Each
    /// leaf writes itself whole, its own infinitesimals with it.
    pub(crate) fn to_bytes(&self) -> Vec<u8> {
        use crate::exact_codec::write_len;
        let mut out = Vec::new();
        write_len(self.scale.name.len(), &mut out);
        out.extend_from_slice(self.scale.name.as_bytes());
        let scale = self.scale.to_bytes();
        write_len(scale.len(), &mut out);
        out.extend_from_slice(&scale);
        self.write_recipe(&mut out);
        out
    }

    fn write_recipe(&self, out: &mut Vec<u8>) {
        use crate::exact_codec::{write_len, ExactCodec};
        let blob = |bytes: Vec<u8>, out: &mut Vec<u8>| {
            write_len(bytes.len(), out);
            out.extend_from_slice(&bytes);
        };
        match &self.recipe {
            GermRecipe::Exact(m) => {
                out.push(0);
                blob(m.to_bytes(), out);
            }
            GermRecipe::Germ(g) => {
                out.push(1);
                blob(g.to_bytes(), out);
            }
            GermRecipe::Constant(n) => {
                out.push(2);
                blob(number_bytes(n), out);
            }
            GermRecipe::Add(a, b) | GermRecipe::Sub(a, b) | GermRecipe::Mul(a, b) => {
                out.push(match &self.recipe {
                    GermRecipe::Add(..) => 3,
                    GermRecipe::Sub(..) => 4,
                    _ => 6,
                });
                a.write_recipe(out);
                b.write_recipe(out);
            }
            GermRecipe::Neg(a) => {
                out.push(5);
                a.write_recipe(out);
            }
            GermRecipe::Recip(a) => {
                out.push(7);
                a.write_recipe(out);
            }
            GermRecipe::Root(a, n) => {
                out.push(8);
                a.write_recipe(out);
                u64::from(*n).write(out);
            }
            GermRecipe::Exp(a) => {
                out.push(9);
                a.write_recipe(out);
            }
            GermRecipe::Ln(a) => {
                out.push(10);
                a.write_recipe(out);
            }
            GermRecipe::MatrixExp(spec, i, j) => {
                out.push(11);
                write_matrix(&spec.a, out);
                spec.t.write(out);
                (*i as u64).write(out);
                (*j as u64).write(out);
            }
            GermRecipe::Flow(spec, i) => {
                out.push(12);
                write_matrix(&spec.a, out);
                spec.t.write(out);
                spec.delta.write(out);
                write_numbers(&spec.start, out);
                (*i as u64).write(out);
            }
            GermRecipe::Power(spec, i) => {
                out.push(13);
                write_matrix(&spec.a, out);
                blob(number_bytes(&spec.count), out);
                write_numbers(&spec.start, out);
                (*i as u64).write(out);
            }
        }
    }

    /// The germ [`NestedGerm::to_bytes`] wrote; `None` for a forged or non-canonical encoding.
    pub(crate) fn from_bytes(buf: &[u8]) -> Option<NestedGerm> {
        use crate::exact_codec::{read_len, MAX_WIRE_LEN};
        let mut pos = 0;
        let n = read_len(buf, &mut pos, MAX_WIRE_LEN)?;
        let name = std::str::from_utf8(buf.get(pos..pos.checked_add(n)?)?).ok()?.to_string();
        pos += n;
        let n = read_len(buf, &mut pos, MAX_WIRE_LEN)?;
        let scale = Scale::from_bytes(&name, buf.get(pos..pos.checked_add(n)?)?)?;
        pos += n;
        let germ = NestedGerm::read_recipe(buf, &mut pos, &scale, 0)?;
        (pos == buf.len() && germ.to_bytes() == buf).then_some(germ)
    }

    fn read_recipe(buf: &[u8], pos: &mut usize, scale: &Arc<Scale>, depth: usize) -> Option<NestedGerm> {
        use crate::exact_codec::{read_byte, read_len, ExactCodec, MAX_WIRE_DEGREE, MAX_WIRE_DEPTH, MAX_WIRE_LEN};
        if depth > MAX_WIRE_DEPTH {
            return None;
        }
        let blob = |pos: &mut usize| -> Option<Vec<u8>> {
            let n = read_len(buf, pos, MAX_WIRE_LEN)?;
            let bytes = buf.get(*pos..pos.checked_add(n)?)?.to_vec();
            *pos += n;
            Some(bytes)
        };
        let sub = |pos: &mut usize| NestedGerm::read_recipe(buf, pos, scale, depth + 1).map(Arc::new);
        Some(match read_byte(buf, pos)? {
            0 => NestedGerm::exact(&Multi::from_bytes(&blob(pos)?)?, scale),
            1 => NestedGerm::germ(&Germ::from_bytes(&blob(pos)?)?, scale),
            2 => NestedGerm::constant(number_from_bytes(&blob(pos)?)?, scale),
            3 => {
                let a = sub(pos)?;
                NestedGerm::add(&a, &sub(pos)?)
            }
            4 => {
                let a = sub(pos)?;
                NestedGerm::sub(&a, &sub(pos)?)
            }
            5 => NestedGerm::neg(&sub(pos)?),
            6 => {
                let a = sub(pos)?;
                NestedGerm::mul(&a, &sub(pos)?)
            }
            7 => NestedGerm::recip(&sub(pos)?).ok()?,
            8 => {
                let a = sub(pos)?;
                let n = u32::try_from(u64::read(buf, pos)?).ok().filter(|n| (2..=MAX_WIRE_DEGREE as u32).contains(n))?;
                NestedGerm::root(&a, n).ok()?
            }
            9 => NestedGerm::exp(&sub(pos)?, "").ok()?,
            10 => NestedGerm::ln(&sub(pos)?, "").ok()?,
            11 => {
                let a = read_matrix(buf, pos)?;
                let t = Rational::read(buf, pos)?;
                let (i, j) = (read_index(buf, pos, a.len())?, read_index(buf, pos, a.len())?);
                let e = crate::nested_flow::matrix_exp(&a, &t).ok()?;
                nested_of(&e[i][j], scale)?
            }
            12 => {
                let a = read_matrix(buf, pos)?;
                let t = Rational::read(buf, pos)?;
                let delta = Rational::read(buf, pos)?;
                let start = read_numbers(buf, pos)?;
                let i = read_index(buf, pos, a.len())?;
                let state = crate::nested_flow::flow(scale, &a, t, delta, &start).ok()?;
                nested_of(state.get(i)?, scale)?
            }
            13 => {
                let a = read_matrix(buf, pos)?;
                let count = number_from_bytes(&blob(pos)?)?;
                let start = read_numbers(buf, pos)?;
                let i = read_index(buf, pos, a.len())?;
                let state = crate::nested_flow::power(scale, &a, count, &start).ok()?;
                nested_of(state.get(i)?, scale)?
            }
            _ => return None,
        })
    }
}

/// The germ a number holds, at `scale`.
fn nested_of(n: &Number, scale: &Arc<Scale>) -> Option<NestedGerm> {
    match n.exact()? {
        ExactRef::Hyper(h) => match &h.value {
            HyperForm::Nested(g) if g.scale.same_infinitesimal(scale) => Some((**g).clone()),
            _ => None,
        },
        _ => None,
    }
}

fn write_numbers(numbers: &[Number], out: &mut Vec<u8>) {
    use crate::exact_codec::write_len;
    write_len(numbers.len(), out);
    for n in numbers {
        let bytes = number_bytes(n);
        write_len(bytes.len(), out);
        out.extend_from_slice(&bytes);
    }
}

fn write_matrix(a: &[Vec<Number>], out: &mut Vec<u8>) {
    use crate::exact_codec::write_len;
    write_len(a.len(), out);
    for row in a {
        write_numbers(row, out);
    }
}

fn read_numbers(buf: &[u8], pos: &mut usize) -> Option<Vec<Number>> {
    use crate::exact_codec::{read_len, MAX_WIRE_DEGREE, MAX_WIRE_LEN};
    let n = read_len(buf, pos, MAX_WIRE_DEGREE)?;
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        let len = read_len(buf, pos, MAX_WIRE_LEN)?;
        let bytes = buf.get(*pos..pos.checked_add(len)?)?;
        *pos += len;
        out.push(number_from_bytes(bytes)?);
    }
    Some(out)
}

fn read_matrix(buf: &[u8], pos: &mut usize) -> Option<Vec<Vec<Number>>> {
    use crate::exact_codec::{read_len, MAX_WIRE_DEGREE};
    let n = read_len(buf, pos, MAX_WIRE_DEGREE).filter(|n| *n > 0)?;
    let mut rows = Vec::with_capacity(n);
    for _ in 0..n {
        let row = read_numbers(buf, pos)?;
        if row.len() != n {
            return None;
        }
        rows.push(row);
    }
    Some(rows)
}

fn read_index(buf: &[u8], pos: &mut usize, below: usize) -> Option<usize> {
    use crate::exact_codec::ExactCodec;
    usize::try_from(u64::read(buf, pos)?).ok().filter(|i| *i < below)
}

/// A number's canonical bytes: a tag, `0` a rational, `1` an exact value of one kind
/// ([`tower::exact_bytes`]) and `2` a value of the infinitesimals ([`tower::scaled_bytes`]), then its
/// parts.
fn number_bytes(n: &Number) -> Vec<u8> {
    use crate::exact_codec::{write_len, ExactCodec};
    let mut out = Vec::new();
    let blob = |bytes: &[u8], out: &mut Vec<u8>| {
        write_len(bytes.len(), out);
        out.extend_from_slice(bytes);
    };
    if let Some(r) = n.standard_rational() {
        out.push(0);
        r.write(&mut out);
    } else if let Some((kind, bytes)) = tower::exact_bytes(n) {
        out.push(1);
        out.push(kind);
        blob(&bytes, &mut out);
    } else if let Some((kind, name, residues, bytes)) = tower::scaled_bytes(n) {
        out.push(2);
        out.push(kind);
        blob(name.as_bytes(), &mut out);
        blob(&residues, &mut out);
        blob(&bytes, &mut out);
    } else {
        out.push(3);
    }
    out
}

fn number_from_bytes(buf: &[u8]) -> Option<Number> {
    use crate::exact_codec::{read_byte, read_len, ExactCodec, MAX_WIRE_LEN};
    let mut pos = 0;
    let blob = |pos: &mut usize| -> Option<Vec<u8>> {
        let n = read_len(buf, pos, MAX_WIRE_LEN)?;
        let bytes = buf.get(*pos..pos.checked_add(n)?)?.to_vec();
        *pos += n;
        Some(bytes)
    };
    let n = match read_byte(buf, &mut pos)? {
        0 => Number::from_rational(Rational::read(buf, &mut pos)?),
        1 => {
            let kind = read_byte(buf, &mut pos)?;
            tower::from_exact_bytes(kind, &blob(&mut pos)?)?
        }
        2 => {
            let kind = read_byte(buf, &mut pos)?;
            let name = String::from_utf8(blob(&mut pos)?).ok()?;
            let residues = blob(&mut pos)?;
            tower::from_scaled_bytes(kind, &name, &residues, &blob(&mut pos)?)?
        }
        _ => return None,
    };
    (pos == buf.len()).then_some(n)
}
