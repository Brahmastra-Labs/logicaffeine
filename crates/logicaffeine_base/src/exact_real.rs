//! Exact real numbers: every computable real, to any precision, with equality decided wherever a
//! theorem decides it.
//!
//! An [`ExactReal`] is one of four forms. Three are **exact forms** whose equality is decidable:
//!
//! - an [`Algebraic`] number (rationals included);
//! - an **exponential-algebraic sum** `Σ aₖ·e^{βₖ}` with complex algebraic `aₖ`, `βₖ` — what
//!   `exp`, `sin` and `cos` of algebraic arguments produce, and what linear differential equations
//!   integrate to. By the Lindemann–Weierstrass theorem the `e^{βₖ}` for distinct algebraic `βₖ`
//!   are linearly independent over the algebraic numbers, so a sum is zero exactly when every
//!   coefficient is zero after like exponents are merged: equality is a finite symbolic check;
//! - a **Laurent polynomial in π** `Σ aₖ·πᵏ` with algebraic `aₖ` — zero exactly when every
//!   coefficient is, because π is transcendental.
//!
//! Everything else — a mix of π with exponentials, a logarithm, an exponential of a
//! transcendental, a number a program defines by a function ([`ExactReal::defined`]) — is a
//! **general computable real**: an expression DAG that encloses its value in a dyadic interval at
//! any requested working precision. Equality of general reals is
//! undecidable (it contains the halting problem), so `compare` refines until the values separate
//! and reports a typed [`RealError::Inseparable`] when a budget runs out first; `compare_within`
//! is total. There is deliberately no `PartialEq`: numeric equality is `compare`, which answers
//! `Equal` only when an exact form proves it.
//!
//! Evaluation is fixed-point dyadic arithmetic (`m·2^e`, no gcds): π by Machin's formula (cached at
//! the highest precision computed), `ln 2 = 2·atanh(1/3)`, `exp` by argument halving, Taylor and
//! repeated squaring, `log` by `atanh`, `sin`/`cos` by reduction modulo 2π, Taylor and the
//! double-angle formulas, square roots by integer Newton. Every primitive returns a rigorous
//! enclosure with its truncation errors counted, so every printed digit is certain.

use crate::algebraic::{negative_root, Algebraic};
use crate::interval::{ceil_div, floor_div, one, Dyadic, Iv};
use crate::numeric::{rational_from_f64_exact, BigInt, Rational};
use crate::ode::{Field, FlowError};
use std::cmp::Ordering;
use std::fmt;
use std::sync::{Arc, Mutex, OnceLock};

// ── errors ────────────────────────────────────────────────────────────────────────────────────

/// Why an exact real could not answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RealError {
    /// Two quantities agreed to `2^-bits` and no theorem decided whether they are equal.
    Inseparable { what: String, bits: u32 },
    /// The operation is undefined at an exactly known argument.
    Domain(String),
}

impl fmt::Display for RealError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RealError::Inseparable { what, bits } => write!(
                f,
                "cannot separate {what} within 2^-{bits}; they may be equal"
            ),
            RealError::Domain(why) => write!(f, "{why}"),
        }
    }
}

/// The answer of a comparison within a tolerance: which side, or within the tolerance.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Closeness {
    Less,
    Greater,
    Close,
}

// ── primitive enclosures ──────────────────────────────────────────────────────────────────────

/// `atan(1/n)·2^w`, as integer bounds, from the alternating series with every truncation counted.
fn atan_recip_scaled(n: i64, w: i64) -> (BigInt, BigInt) {
    let nb = BigInt::from_i64(n);
    let n2 = nb.mul(&nb);
    let mut power = one().shl_bits(w as u64).div_rem(&nb).expect("n > 0").0;
    let mut sum = BigInt::zero();
    let mut k = 0i64;
    while !power.is_zero() {
        let term = power.div_rem(&BigInt::from_i64(2 * k + 1)).expect("odd").0;
        sum = if k % 2 == 0 { sum.add(&term) } else { sum.sub(&term) };
        power = power.div_rem(&n2).expect("n² > 0").0;
        k += 1;
    }
    let err = BigInt::from_i64(3 * k + 3);
    (sum.sub(&err), sum.add(&err))
}

fn pi_cache() -> &'static Mutex<Option<(i64, BigInt, BigInt)>> {
    static CACHE: OnceLock<Mutex<Option<(i64, BigInt, BigInt)>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(None))
}

/// π enclosed on the `2^-p` grid, by Machin's `π = 16·atan(1/5) − 4·atan(1/239)`.
fn pi_iv(p: i64) -> Iv {
    let p = p.max(8);
    {
        let cache = pi_cache().lock().expect("the π cache is never poisoned");
        if let Some((w, lo, hi)) = cache.as_ref() {
            if *w >= p {
                let iv = Iv {
                    lo: Dyadic::new(lo.clone(), -w),
                    hi: Dyadic::new(hi.clone(), -w),
                };
                return iv.round_out(p);
            }
        }
    }
    let w = p + 24;
    let (a_lo, a_hi) = atan_recip_scaled(5, w);
    let (b_lo, b_hi) = atan_recip_scaled(239, w);
    let sixteen = BigInt::from_i64(16);
    let four = BigInt::from_i64(4);
    let lo = sixteen.mul(&a_lo).sub(&four.mul(&b_hi));
    let hi = sixteen.mul(&a_hi).sub(&four.mul(&b_lo));
    *pi_cache().lock().expect("the π cache is never poisoned") = Some((w, lo.clone(), hi.clone()));
    Iv {
        lo: Dyadic::new(lo, -w),
        hi: Dyadic::new(hi, -w),
    }
    .round_out(p)
}

/// `atanh(z)·2^w` for a rational `0 ≤ z ≤ 1/3`, as integer bounds: the positive series
/// `Σ z^{2j+1}/(2j+1)` truncated where its terms vanish, every division counted.
fn atanh_scaled(z_num: &BigInt, z_den: &BigInt, w: i64) -> (BigInt, BigInt) {
    let z2_num = z_num.mul(z_num);
    let z2_den = z_den.mul(z_den);
    let mut power = floor_div(&z_num.shl_bits(w as u64), z_den);
    let mut sum = BigInt::zero();
    let mut k = 0i64;
    while !power.is_zero() {
        sum = sum.add(&power.div_rem(&BigInt::from_i64(2 * k + 1)).expect("odd").0);
        power = floor_div(&power.mul(&z2_num), &z2_den);
        k += 1;
    }
    (sum.clone(), sum.add(&BigInt::from_i64(2 * k + 4)))
}

fn ln2_cache() -> &'static Mutex<Option<(i64, BigInt, BigInt)>> {
    static CACHE: OnceLock<Mutex<Option<(i64, BigInt, BigInt)>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(None))
}

/// `ln 2 = 2·atanh(1/3)` enclosed on the `2^-p` grid.
fn ln2_iv(p: i64) -> Iv {
    let p = p.max(8);
    {
        let cache = ln2_cache().lock().expect("the ln 2 cache is never poisoned");
        if let Some((w, lo, hi)) = cache.as_ref() {
            if *w >= p {
                return Iv {
                    lo: Dyadic::new(lo.clone(), -w),
                    hi: Dyadic::new(hi.clone(), -w),
                }
                .round_out(p);
            }
        }
    }
    let w = p + 16;
    let (lo, hi) = atanh_scaled(&one(), &BigInt::from_i64(3), w);
    let (lo, hi) = (lo.shl_bits(1), hi.shl_bits(1));
    *ln2_cache().lock().expect("the ln 2 cache is never poisoned") =
        Some((w, lo.clone(), hi.clone()));
    Iv {
        lo: Dyadic::new(lo, -w),
        hi: Dyadic::new(hi, -w),
    }
    .round_out(p)
}

/// `e^x` for a dyadic point, on the `2^-p` grid: the raw enclosure at a working precision that
/// covers the argument halvings and the size of the result, rounded outward.
fn exp_point(x: &Dyadic, p: i64) -> Iv {
    let s = (x.mag_bits() + 2).max(0);
    let growth = if x.is_positive() {
        (x.ceil_scaled(0).to_i64().unwrap_or(i64::MAX / 4) * 3 / 2).max(0)
    } else {
        0
    };
    exp_raw(x, p + s + growth + 16).round_out(p)
}

/// `e^x` enclosed at working precision `w`: halve until `|y| ≤ 1/4`, sum the Taylor series in
/// fixed point at `2^-w` with every truncation counted (each step loses under one unit and the
/// tail after the last nonzero term is under one unit), then square back with outward rounding.
fn exp_raw(x: &Dyadic, w: i64) -> Iv {
    let s = (x.mag_bits() + 2).max(0);
    let y = x.shift(-s);
    let unit = one().shl_bits(w as u64);
    let yw = y.floor_scaled(w);
    let mut term = unit.clone();
    let mut sum = unit;
    let mut k = 1i64;
    loop {
        term = floor_div(&term.mul(&yw), &BigInt::from_i64(k).shl_bits(w as u64));
        if term.is_zero() {
            break;
        }
        sum = sum.add(&term);
        k += 1;
    }
    let err = BigInt::from_i64(2 * k + 6);
    let mut iv = Iv {
        lo: Dyadic::new(sum.sub(&err), -w),
        hi: Dyadic::new(sum.add(&err), -w),
    };
    for _ in 0..s {
        iv = iv.sq().round_out(w);
    }
    iv
}

/// `ln x` for a positive dyadic point: `x = r·2^k` with `r ∈ [1, 2)`, `ln r = 2·atanh((r−1)/(r+1))`.
fn log_point(x: &Dyadic, p: i64) -> Iv {
    let k = x.m.bit_len() as i64 + x.e - 1;
    let r = x.shift(-k);
    let r_num = r.floor_scaled(p + 64);
    let den = one().shl_bits((p + 64) as u64);
    let w = p + 24;
    let (z_lo_num, z_hi_num) = (r_num.sub(&den), r_num.add(&one()).sub(&den));
    let (lo_scaled, _) = atanh_scaled(&z_lo_num, &r_num.add(&den), w);
    let (_, hi_scaled) = atanh_scaled(&z_hi_num, &r_num.add(&one()).add(&den), w);
    let log_r = Iv {
        lo: Dyadic::new(lo_scaled.shl_bits(1), -w),
        hi: Dyadic::new(hi_scaled.shl_bits(1), -w),
    };
    let kb = Iv::point(Dyadic::int(k));
    let extra = (k.unsigned_abs() as f64).log2().ceil() as i64 + 4;
    kb.mul(&ln2_iv(w + extra)).add(&log_r).round_out(p)
}

/// `(sin x, cos x)` for a dyadic point with `|x| < 8`, on the `2^-p` grid.
fn sin_cos_small(x: &Dyadic, p: i64) -> (Iv, Iv) {
    let (s, c) = sin_cos_raw(x, p + 32);
    (s.round_out(p), c.round_out(p))
}

/// `(sin x, cos x)` for `|x| < 8` at working precision `w`: divide by 16, sum both Taylor series
/// in fixed point with every truncation counted, then four double-angle steps in interval
/// arithmetic.
fn sin_cos_raw(x: &Dyadic, w: i64) -> (Iv, Iv) {
    let s = 4i64;
    let y = x.shift(-s);
    let yw = y.floor_scaled(w);
    let scale = one().shl_bits(w as u64);
    let mut term = yw.clone();
    let mut sin_sum = yw.clone();
    let mut cos_sum = scale.clone();
    let mut cos_term = scale;
    let mut k = 1i64;
    loop {
        cos_term = floor_div(
            &cos_term.mul(&yw).mul(&yw).negated(),
            &BigInt::from_i64((2 * k - 1) * (2 * k)).shl_bits(2 * w as u64),
        );
        term = floor_div(
            &term.mul(&yw).mul(&yw).negated(),
            &BigInt::from_i64((2 * k) * (2 * k + 1)).shl_bits(2 * w as u64),
        );
        if term.is_zero() && cos_term.is_zero() {
            break;
        }
        sin_sum = sin_sum.add(&term);
        cos_sum = cos_sum.add(&cos_term);
        k += 1;
    }
    let err = Dyadic::new(BigInt::from_i64(2 * k + 6), -w);
    let mut sn = Iv::point(Dyadic::new(sin_sum, -w)).widen(&err);
    let mut cs = Iv::point(Dyadic::new(cos_sum, -w)).widen(&err);
    let two = Iv::point(Dyadic::int(2));
    let unit = Iv::point(Dyadic::int(1));
    for _ in 0..s {
        let s2 = two.mul(&sn.mul(&cs)).round_out(w);
        let c2 = unit.add(&two.mul(&sn.sq()).neg()).round_out(w);
        sn = s2.clamp_unit();
        cs = c2.clamp_unit();
    }
    (sn, cs)
}

/// `(sin x, cos x)` for any dyadic point: subtract a whole number of turns `⌊x/2π⌋·2π` (exact
/// periodicity, computed with an enclosure of π), then enclose at the reduced interval's midpoint
/// and widen by its radius — both functions are 1-Lipschitz.
fn sin_cos_point(x: &Dyadic, p: i64) -> (Iv, Iv) {
    if x.mag_bits() <= 2 {
        return sin_cos_small(x, p);
    }
    let guard = x.mag_bits().max(0) + 16;
    let two_pi = pi_iv(p + guard).mul(&Iv::point(Dyadic::int(2)));
    let k_num = x.floor_scaled(64);
    let approx_turns = floor_div(&k_num, &two_pi.lo.floor_scaled(64).max(BigInt::from_i64(1)));
    let k = Iv::point(Dyadic::new(approx_turns, 0));
    let reduced = Iv::point(x.clone()).add(&k.mul(&two_pi).neg());
    let mid = reduced.lo.add(&reduced.hi).shift(-1);
    let radius = reduced.width().shift(-1);
    let (s, c) = sin_cos_small(&mid, p + 4);
    (
        s.widen(&radius).clamp_unit().round_out(p),
        c.widen(&radius).clamp_unit().round_out(p),
    )
}

/// `⌊√n⌋` for `n ≥ 0` by integer Newton.
fn isqrt(n: &BigInt) -> BigInt {
    if n.is_zero() {
        return BigInt::zero();
    }
    let mut x = one().shl_bits(n.bit_len() / 2 + 1);
    loop {
        let y = x.add(&n.div_rem(&x).expect("x > 0").0).shr_bits(1).expect("positive");
        if y >= x {
            return x;
        }
        x = y;
    }
}

fn sqrt_floor(d: &Dyadic, p: i64) -> Dyadic {
    Dyadic::new(isqrt(&d.floor_scaled(2 * p).max(BigInt::zero())), -p)
}

fn sqrt_ceil(d: &Dyadic, p: i64) -> Dyadic {
    let n = d.ceil_scaled(2 * p).max(BigInt::zero());
    let s = isqrt(&n);
    let s = if s.mul(&s) < n { s.add(&one()) } else { s };
    Dyadic::new(s, -p)
}

/// `1/[lo, hi]` for an interval excluding zero, rounded outward to `2^-p`.
fn recip_iv(x: &Iv, p: i64) -> Iv {
    let num = one();
    let bound = |d: &Dyadic, up: bool| -> Dyadic {
        let shift = p - d.e;
        let (n, dd) = if shift >= 0 {
            (num.shl_bits(shift as u64), d.m.clone())
        } else {
            (num.clone(), d.m.shl_bits((-shift) as u64))
        };
        let (n, dd) = if dd.is_negative() {
            (n.negated(), dd.negated())
        } else {
            (n, dd)
        };
        let q = if up { ceil_div(&n, &dd) } else { floor_div(&n, &dd) };
        Dyadic::new(q, -p)
    };
    Iv {
        lo: bound(&x.hi, false),
        hi: bound(&x.lo, true),
    }
}

// ── complex algebraic numbers and the exact forms ─────────────────────────────────────────────

use crate::complex_algebraic::ComplexAlgebraic as Calg;

/// `Σ coefficient·e^{exponent}`, conjugate-symmetric (so real-valued), each exponent once, no zero
/// coefficient, sorted by exponent descending.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct ExpSum {
    terms: Vec<(Calg, Calg)>,
}

impl ExpSum {
    fn from_terms(raw: Vec<(Calg, Calg)>) -> ExpSum {
        let mut merged: Vec<(Calg, Calg)> = Vec::new();
        let mut raw = raw;
        raw.sort_by(|a, b| b.0.cmp(&a.0));
        for (e, c) in raw {
            match merged.last_mut() {
                Some((le, lc)) if *le == e => *lc = lc.add(&c),
                _ => merged.push((e, c)),
            }
        }
        merged.retain(|(_, c)| !c.is_zero());
        ExpSum { terms: merged }
    }

    fn constant(a: &Algebraic) -> ExpSum {
        ExpSum::from_terms(vec![(Calg::zero(), Calg::real(a.clone()))])
    }

    fn add(&self, o: &ExpSum) -> ExpSum {
        ExpSum::from_terms(self.terms.iter().chain(o.terms.iter()).cloned().collect())
    }

    fn neg(&self) -> ExpSum {
        ExpSum {
            terms: self.terms.iter().map(|(e, c)| (e.clone(), c.neg())).collect(),
        }
    }

    fn mul(&self, o: &ExpSum) -> ExpSum {
        let mut raw = Vec::with_capacity(self.terms.len() * o.terms.len());
        for (e1, c1) in &self.terms {
            for (e2, c2) in &o.terms {
                raw.push((e1.add(e2), c1.mul(c2)));
            }
        }
        ExpSum::from_terms(raw)
    }

    /// The reciprocal of a single term `c·e^β` is `c⁻¹·e^{−β}`; a longer sum has no exact
    /// reciprocal in this ring.
    fn recip(&self) -> Option<ExpSum> {
        match self.terms.as_slice() {
            [(e, c)] => Some(ExpSum {
                terms: vec![(e.neg(), c.recip()?)],
            }),
            _ => None,
        }
    }

    /// The constant, when the sum has no exponential term.
    fn as_constant(&self) -> Option<Algebraic> {
        match self.terms.as_slice() {
            [] => Some(Algebraic::from_i64(0)),
            [(e, c)] if e.is_zero() => Some(c.re.clone()),
            _ => None,
        }
    }
}

/// `Σ coefficient·π^power` (powers may be negative), no zero coefficient, powers descending.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct PiPoly {
    terms: Vec<(i32, Algebraic)>,
}

impl PiPoly {
    fn from_terms(raw: Vec<(i32, Algebraic)>) -> PiPoly {
        let mut raw = raw;
        raw.sort_by_key(|t| std::cmp::Reverse(t.0));
        let mut merged: Vec<(i32, Algebraic)> = Vec::new();
        for (k, c) in raw {
            match merged.last_mut() {
                Some((lk, lc)) if *lk == k => *lc = lc.add(&c),
                _ => merged.push((k, c)),
            }
        }
        merged.retain(|(_, c)| !c.is_zero());
        PiPoly { terms: merged }
    }

    fn constant(a: &Algebraic) -> PiPoly {
        PiPoly::from_terms(vec![(0, a.clone())])
    }

    fn add(&self, o: &PiPoly) -> PiPoly {
        PiPoly::from_terms(self.terms.iter().chain(o.terms.iter()).cloned().collect())
    }

    fn neg(&self) -> PiPoly {
        PiPoly {
            terms: self.terms.iter().map(|(k, c)| (*k, c.neg())).collect(),
        }
    }

    fn mul(&self, o: &PiPoly) -> PiPoly {
        let mut raw = Vec::new();
        for (k1, c1) in &self.terms {
            for (k2, c2) in &o.terms {
                raw.push((k1 + k2, c1.mul(c2)));
            }
        }
        PiPoly::from_terms(raw)
    }

    fn recip(&self) -> Option<PiPoly> {
        match self.terms.as_slice() {
            [(k, c)] => Some(PiPoly {
                terms: vec![(-k, c.recip()?)],
            }),
            _ => None,
        }
    }

    fn as_constant(&self) -> Option<Algebraic> {
        match self.terms.as_slice() {
            [] => Some(Algebraic::from_i64(0)),
            [(0, c)] => Some(c.clone()),
            _ => None,
        }
    }
}

// ── the expression DAG ────────────────────────────────────────────────────────────────────────

/// An algebraic leaf keeps the narrowest dyadic isolating interval found so far.
struct AlgLeaf {
    value: Algebraic,
    cache: Mutex<Option<Iv>>,
}

enum Node {
    Alg(AlgLeaf),
    Pi,
    Add(ExactReal, ExactReal),
    Mul(ExactReal, ExactReal),
    Neg(ExactReal),
    Recip(ExactReal),
    Exp(ExactReal),
    Log(ExactReal),
    Sin(ExactReal),
    Cos(ExactReal),
    Sqrt(ExactReal),
    /// One component of the solution of a polynomial differential equation.
    Flow(Arc<FlowSolution>, usize),
    /// A number a program's function defines.
    Defined(Arc<Defined>),
}

// ── real numbers a program defines ────────────────────────────────────────────────────────────

/// A program's function a real number is defined by, as the tier running the program calls it from
/// inside a refinement. The tier evaluates the function in a context of its own, so a call made in
/// the middle of another computation changes nothing the program can see.
pub trait Approximant: Send + Sync {
    /// The function's answer at `n`, as the rational it is; `Err` is the function's own failure, or
    /// an answer that is no rational.
    fn call(&self, n: u64) -> Result<Rational, String>;
}

/// How a program's function gives its real number.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Definition {
    /// `f(i)` is the digit in place `i` of the number written in base `base`: the number is
    /// `f(0).f(1)f(2)…`, digit `i` worth `base^-i`.
    Digits { base: u32 },
    /// `f(n)` is within `2^-n` of the number.
    Approximations,
}

impl Definition {
    /// The definition `kind` names — `digits` in `base`, or `approximations` — refused in the words
    /// every tier uses when it is neither, or a base below 2.
    pub fn named(kind: &str, base: i64) -> Result<Definition, String> {
        match kind {
            "digits" => u32::try_from(base)
                .ok()
                .filter(|b| *b >= 2)
                .map(|base| Definition::Digits { base })
                .ok_or_else(|| format!("a real number's digits are in a base from 2 up, got {base}")),
            "approximations" => Ok(Definition::Approximations),
            other => Err(format!(
                "a real number is defined by its digits or its approximations, got {other}"
            )),
        }
    }
}

/// What a real number built from a function a program defines is, where a task, a channel or the
/// wire refuses it: it stays with the program that defines it.
pub const STAYS_WITH_ITS_PROGRAM: &str = "a real number a function of the program defines";

/// The sentence a program stops with when it hands a task, or sends through a channel, a value that
/// holds a real number a function of the program defines.
pub fn not_sendable() -> String {
    format!("cannot send value through a channel: NotSendable({STAYS_WITH_ITS_PROGRAM:?})")
}

/// The refusal of a function's answer at `n` that is a value of `type_name`, not a rational.
pub fn not_a_rational_answer(name: &str, n: u64, type_name: &str) -> String {
    format!("{name}({n}) is {type_name}, and a real number is defined by whole or rational answers")
}

/// What a defined real has read of its function so far.
enum Known {
    /// The digits from place `0` on.
    Digits(Vec<u32>),
    /// The approximations made, by the precision each was asked at.
    Approximations(Vec<(u32, Rational)>),
}

/// A real number a program's function defines, with what it has read of the function.
pub(crate) struct Defined {
    /// The function, as the program spells it.
    name: String,
    definition: Definition,
    source: Arc<dyn Approximant>,
    known: Mutex<Known>,
}

/// `2^-n` as a rational.
fn two_to_minus(n: u32) -> Rational {
    Rational::new(one(), one().shl_bits(u64::from(n))).expect("a power of two is nonzero")
}

impl Defined {
    fn describe(&self) -> String {
        match self.definition {
            Definition::Digits { base } => {
                format!("the real number whose digits in base {base} are {}", self.name)
            }
            Definition::Approximations => format!("the real number approximated by {}", self.name),
        }
    }

    /// An enclosure at most `2^-(p+1)` wide, from as many digits, or as close an approximation, as
    /// that takes. The function is called without the record of what it answered held, so an answer
    /// that reads this number again finds it as it was.
    fn enclose(&self, p: i64) -> Result<Iv, RealError> {
        match self.definition {
            Definition::Digits { base } => self.enclose_digits(base, p),
            Definition::Approximations => self.enclose_approximations(p),
        }
    }

    /// The first `count` digits, read from the function where they are not yet known.
    fn digits(&self, base: u32, count: usize) -> Result<Vec<u32>, RealError> {
        loop {
            let next = match &*self.known.lock().expect("unpoisoned") {
                Known::Digits(d) if d.len() >= count => return Ok(d[..count].to_vec()),
                Known::Digits(d) => d.len(),
                Known::Approximations(_) => unreachable!("a real defined by digits reads digits"),
            };
            let answer = self.source.call(next as u64).map_err(RealError::Domain)?;
            let digit = answer
                .to_bigint()
                .filter(|_| answer.is_integer())
                .and_then(|d| d.to_u64())
                .filter(|d| *d < u64::from(base))
                .ok_or_else(|| {
                    RealError::Domain(format!(
                        "digit {next} of {} is {answer}: a digit in base {base} is a whole number from 0 to {}",
                        self.describe(),
                        base - 1
                    ))
                })?;
            if let Known::Digits(d) = &mut *self.known.lock().expect("unpoisoned") {
                if d.len() == next {
                    d.push(digit as u32);
                }
            }
        }
    }

    /// `[S, S + base^-k]` for the sum `S` of the digits in places `0..=k`, with `base^-k ≤ 2^-(p+2)`.
    fn enclose_digits(&self, base: u32, p: i64) -> Result<Iv, RealError> {
        let b = Rational::from_i64(i64::from(base));
        let mut places = 0usize;
        let mut tail = Rational::from_i64(1);
        let bound = two_to_minus(u32::try_from((p + 2).max(0)).unwrap_or(u32::MAX));
        while tail > bound {
            tail = tail.div(&b).expect("a base is positive");
            places += 1;
        }
        let digits = self.digits(base, places + 1)?;
        let mut sum = Rational::zero();
        let mut weight = Rational::from_i64(1);
        for d in &digits {
            sum = sum.add(&weight.mul(&Rational::from_i64(i64::from(*d))));
            weight = weight.div(&b).expect("a base is positive");
        }
        Ok(Iv {
            lo: Dyadic::from_rational_floor(&sum, p + 2),
            hi: Dyadic::from_rational_ceil(&sum.add(&tail), p + 2),
        })
    }

    /// `[q − 2^-n, q + 2^-n]` for the approximation `q` at precision `n = p + 2`, every
    /// approximation checked against those made before: two that lie further apart than their
    /// precisions allow approximate no one number.
    fn enclose_approximations(&self, p: i64) -> Result<Iv, RealError> {
        let n = u32::try_from((p + 2).max(0)).unwrap_or(u32::MAX);
        let known = match &*self.known.lock().expect("unpoisoned") {
            Known::Approximations(made) => made.iter().find(|(m, _)| *m == n).map(|(_, q)| q.clone()),
            Known::Digits(_) => unreachable!("a real defined by approximations reads approximations"),
        };
        let q = match known {
            Some(q) => q,
            None => {
                let q = self.source.call(u64::from(n)).map_err(RealError::Domain)?;
                if let Known::Approximations(made) = &mut *self.known.lock().expect("unpoisoned") {
                    for (m, r) in made.iter() {
                        if q.sub(r).abs() > two_to_minus(n).add(&two_to_minus(*m)) {
                            return Err(RealError::Domain(format!(
                                "{name}({n}) is {q} and {name}({m}) is {r}, further apart than 2^-{n} + 2^-{m}: \
                                 {name} approximates no one real number",
                                name = self.name
                            )));
                        }
                    }
                    if !made.iter().any(|(m, _)| *m == n) {
                        made.push((n, q.clone()));
                    }
                }
                q
            }
        };
        let eps = two_to_minus(n);
        Ok(Iv {
            lo: Dyadic::from_rational_floor(&q.sub(&eps), p + 2),
            hi: Dyadic::from_rational_ceil(&q.add(&eps), p + 2),
        })
    }
}

/// The solution of `x' = f(x)` from `start` at time `t`, every component enclosed together.
pub(crate) struct FlowSolution {
    field: Field,
    start: Vec<ExactReal>,
    t: Rational,
    /// The working precision of the tightest enclosure computed so far, and that enclosure.
    cache: Mutex<Option<(i64, Vec<Iv>)>>,
}

impl fmt::Debug for FlowSolution {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "FlowSolution(t = {}, {} components)", self.t, self.start.len())
    }
}

impl FlowSolution {
    /// Component `i` within `2^-(p+2)`, the walk's working precision doubling until it is.
    fn enclose(&self, i: usize, p: i64, cap: i64) -> Result<Iv, RealError> {
        let within = |iv: &Iv| iv.width().mag_bits() <= -(p + 2);
        if let Some((_, ivs)) = self.cache.lock().expect("unpoisoned").as_ref() {
            if within(&ivs[i]) {
                return Ok(ivs[i].round_out(p + 2));
            }
        }
        let mut q = (p + 32).max(64);
        loop {
            let start = self
                .start
                .iter()
                .map(|x| x.enclosed(q + 8, cap))
                .collect::<Result<Vec<Iv>, RealError>>()?;
            match crate::ode::flow(&self.field, &start, &self.t, q, (q / 2).max(40)) {
                Ok(ivs) => {
                    let done = within(&ivs[i]);
                    let answer = ivs[i].round_out(p + 2);
                    *self.cache.lock().expect("unpoisoned") = Some((q, ivs));
                    if done {
                        return Ok(answer);
                    }
                }
                Err(FlowError::Escapes) => {
                    return Err(RealError::Domain(
                        "the solution of the flow leaves every bound before its time".to_string(),
                    ))
                }
                Err(FlowError::Shape) => {
                    return Err(RealError::Domain(
                        "a flow's field and its start disagree in shape".to_string(),
                    ))
                }
            }
            if q >= cap {
                return Err(RealError::Inseparable {
                    what: "the solution of the flow".to_string(),
                    bits: cap as u32,
                });
            }
            q = (q * 2).min(cap);
        }
    }
}

/// The sign of the integer polynomial `min` at the dyadic `m·2^-k`, scaled by `2^{k·deg}`.
fn poly_sign_at(min: &[BigInt], d: &Dyadic) -> Ordering {
    let k = (-d.e).max(0);
    let m = if d.e > 0 { d.m.shl_bits(d.e as u64) } else { d.m.clone() };
    let deg = min.len() - 1;
    let mut acc = BigInt::zero();
    let mut mp = one();
    for (i, c) in min.iter().enumerate() {
        acc = acc.add(&c.mul(&mp).shl_bits((k as u64) * (deg - i) as u64));
        mp = mp.mul(&m);
    }
    acc.cmp(&BigInt::zero())
}

impl AlgLeaf {
    /// The root's dyadic isolating interval refined to width `2^-p`. The first call seeds it by
    /// rounding the algebraic number's own isolating interval INWARD onto a dyadic grid fine enough
    /// that the root stays between the rounded ends (checked by the sign of the minimal polynomial,
    /// which changes only at the root inside); later calls continue from the cached interval.
    fn enclose(&self, p: i64) -> Iv {
        if let Some(r) = self.value.to_rational() {
            return Iv {
                lo: Dyadic::from_rational_floor(&r, p),
                hi: Dyadic::from_rational_ceil(&r, p),
            };
        }
        let min_poly = self.value.minimal_polynomial();
        let min = min_poly.primitive_int();
        let mut cache = self.cache.lock().expect("an algebraic leaf cache is never poisoned");
        let mut iv = match cache.take() {
            Some(iv) => iv,
            None => {
                let (lo, hi) = self.value.enclosure(8);
                let lo_sign = min_poly.eval(&lo).cmp(&Rational::zero());
                let hi_sign = min_poly.eval(&hi).cmp(&Rational::zero());
                let mut k = 16i64;
                loop {
                    let dlo = Dyadic::from_rational_ceil(&lo, k);
                    let dhi = Dyadic::from_rational_floor(&hi, k);
                    if dlo.cmp(&dhi) == Ordering::Less
                        && poly_sign_at(&min, &dlo) == lo_sign
                        && poly_sign_at(&min, &dhi) == hi_sign
                    {
                        break Iv { lo: dlo, hi: dhi };
                    }
                    k *= 2;
                }
            }
        };
        let eps = Dyadic::new(one(), -p);
        let lo_sign = poly_sign_at(&min, &iv.lo);
        while iv.width().cmp(&eps) == Ordering::Greater {
            let mid = iv.lo.add(&iv.hi).shift(-1);
            let s = poly_sign_at(&min, &mid);
            if s == Ordering::Equal {
                iv = Iv::point(mid);
                break;
            }
            if s == lo_sign {
                iv.lo = mid;
            } else {
                iv.hi = mid;
            }
        }
        *cache = Some(iv.clone());
        iv
    }
}

impl Node {
    /// An enclosure computed with working precision `p`; `cap` bounds every precision search.
    fn enclose(&self, p: i64, cap: i64) -> Result<Iv, RealError> {
        match self {
            Node::Alg(leaf) => Ok(leaf.enclose(p + 2)),
            Node::Pi => Ok(pi_iv(p + 2)),
            Node::Add(a, b) => {
                Ok(a.enclosed(p + 2, cap)?.add(&b.enclosed(p + 2, cap)?).round_out(p + 1))
            }
            Node::Neg(a) => Ok(a.enclosed(p, cap)?.neg()),
            Node::Mul(a, b) => {
                let ma = a.enclosed(4, cap)?.mag_bits().max(0);
                let mb = b.enclosed(4, cap)?.mag_bits().max(0);
                let x = a.enclosed(p + mb + 4, cap)?;
                let y = b.enclosed(p + ma + 4, cap)?;
                Ok(x.mul(&y).round_out(p + 2))
            }
            Node::Recip(a) => {
                let mut q = p.max(8);
                loop {
                    let x = a.enclosed(q, cap)?;
                    if !x.contains_zero() {
                        let nearest = if x.lo.is_positive() { x.lo.clone() } else { x.hi.neg() };
                        let lower = 1 - nearest.mag_bits();
                        let need = p + 2 * lower.max(0) + 6;
                        if q >= need {
                            return Ok(recip_iv(&x, p + 2));
                        }
                        q = need;
                        continue;
                    }
                    if q >= cap {
                        return Err(RealError::Inseparable {
                            what: "the divisor from zero".to_string(),
                            bits: cap as u32,
                        });
                    }
                    q = (q * 2).min(cap);
                }
            }
            Node::Exp(a) => {
                let rough = a.enclosed(4, cap)?;
                let growth = if rough.hi.is_positive() {
                    rough.hi.ceil_scaled(0).to_i64().unwrap_or(i64::MAX / 8) * 3 / 2 + 2
                } else {
                    2
                };
                let x = a.enclosed(p + growth, cap)?;
                Ok(Iv {
                    lo: exp_point(&x.lo, p + 4).lo,
                    hi: exp_point(&x.hi, p + 4).hi,
                })
            }
            Node::Log(a) => {
                let mut q = p.max(8);
                loop {
                    let x = a.enclosed(q, cap)?;
                    if x.hi.is_negative() || (x.hi.m.is_zero() && x.lo.m.is_zero()) {
                        return Err(RealError::Domain(
                            "logarithm of a nonpositive number".to_string(),
                        ));
                    }
                    if x.lo.is_positive() {
                        let lower = 1 - x.lo.mag_bits();
                        let need = p + lower.max(0) + 6;
                        if q >= need {
                            return Ok(Iv {
                                lo: log_point(&x.lo, p + 4).lo,
                                hi: log_point(&x.hi, p + 4).hi,
                            });
                        }
                        q = need;
                        continue;
                    }
                    if q >= cap {
                        return Err(RealError::Inseparable {
                            what: "the logarithm's argument from zero".to_string(),
                            bits: cap as u32,
                        });
                    }
                    q = (q * 2).min(cap);
                }
            }
            Node::Sqrt(a) => {
                let mut q = p.max(8);
                loop {
                    let x = a.enclosed(q, cap)?;
                    if x.hi.is_negative() {
                        return Err(RealError::Domain(
                            "square root of a negative number".to_string(),
                        ));
                    }
                    if x.lo.is_positive() {
                        let lower = 1 - x.lo.mag_bits();
                        let need = p + lower.max(0) + 6;
                        if q >= need {
                            return Ok(Iv {
                                lo: sqrt_floor(&x.lo, p + 2),
                                hi: sqrt_ceil(&x.hi, p + 2),
                            });
                        }
                        q = need;
                        continue;
                    }
                    if q >= cap {
                        return Err(RealError::Inseparable {
                            what: "the square root's argument from zero".to_string(),
                            bits: cap as u32,
                        });
                    }
                    q = (q * 2).min(cap);
                }
            }
            Node::Flow(solution, i) => solution.enclose(*i, p, cap),
            Node::Defined(d) => d.enclose(p),
            Node::Sin(a) | Node::Cos(a) => {
                let x = a.enclosed(p + 2, cap)?;
                let mid = x.lo.add(&x.hi).shift(-1);
                let radius = x.width().shift(-1);
                let (s, c) = sin_cos_point(&mid, p + 4);
                let v = if matches!(self, Node::Sin(_)) { s } else { c };
                Ok(v.widen(&radius).clamp_unit().round_out(p + 2))
            }
        }
    }
}

// ── the public type ───────────────────────────────────────────────────────────────────────────

#[derive(Clone)]
enum Form {
    Alg(Algebraic),
    Exp(ExpSum),
    Pi(PiPoly),
    General(Arc<Node>),
}

/// A computable real number. See the module documentation.
#[derive(Clone)]
pub struct ExactReal {
    form: Form,
    tree: Arc<OnceLock<Arc<Node>>>,
    /// The tightest enclosure computed so far, with the working precision it was computed at; every
    /// clone shares it, so a node a deep expression reaches many times is refined once per
    /// precision rather than once per path to it.
    best: Arc<Mutex<Option<(i64, Iv)>>>,
}

/// Significant digits a general computable real prints with.
pub const DISPLAY_DIGITS: u32 = 30;

/// The precision budget, in bits, of a comparison whose budget the caller does not state.
pub const DEFAULT_BUDGET: u32 = 4096;

fn half_real() -> ExactReal {
    ExactReal::from_rational(Rational::from_ratio_i64(1, 2).expect("2 ≠ 0"))
}

impl ExactReal {
    fn of(form: Form) -> ExactReal {
        let form = match form {
            Form::Exp(s) => match s.as_constant() {
                Some(a) => Form::Alg(a),
                None => Form::Exp(s),
            },
            Form::Pi(p) => match p.as_constant() {
                Some(a) => Form::Alg(a),
                None => Form::Pi(p),
            },
            other => other,
        };
        ExactReal {
            form,
            tree: Arc::new(OnceLock::new()),
            best: Arc::new(Mutex::new(None)),
        }
    }

    /// An enclosure computed with working precision at least `p`: the tightest one already known
    /// when it was computed at `p` or above, else a fresh one, kept when it is the tightest.
    fn enclosed(&self, p: i64, cap: i64) -> Result<Iv, RealError> {
        if let Some((q, iv)) = self.best.lock().expect("unpoisoned").as_ref() {
            if *q >= p {
                return Ok(iv.clone());
            }
        }
        let iv = self.node().enclose(p, cap)?;
        let mut best = self.best.lock().expect("unpoisoned");
        if best.as_ref().is_none_or(|(q, _)| *q < p) {
            *best = Some((p, iv.clone()));
        }
        Ok(iv)
    }

    fn general(node: Node) -> ExactReal {
        ExactReal::of(Form::General(Arc::new(node)))
    }

    pub fn from_i64(n: i64) -> ExactReal {
        Self::from_algebraic(Algebraic::from_i64(n))
    }

    pub fn from_rational(r: Rational) -> ExactReal {
        Self::from_algebraic(Algebraic::from_rational(r))
    }

    pub fn from_algebraic(a: Algebraic) -> ExactReal {
        Self::of(Form::Alg(a))
    }

    /// The exact value of a finite float; `None` for NaN and the infinities.
    pub fn from_f64(f: f64) -> Option<ExactReal> {
        rational_from_f64_exact(f).map(Self::from_rational)
    }

    pub fn pi() -> ExactReal {
        Self::of(Form::Pi(PiPoly {
            terms: vec![(1, Algebraic::from_i64(1))],
        }))
    }

    pub fn e() -> ExactReal {
        Self::from_i64(1).exp()
    }

    /// The value as an algebraic number, when it is one exactly.
    pub fn exact_algebraic(&self) -> Option<Algebraic> {
        match &self.form {
            Form::Alg(a) => Some(a.clone()),
            _ => None,
        }
    }

    /// True only when an exact form proves the value is zero.
    pub fn is_exactly_zero(&self) -> bool {
        matches!(&self.form, Form::Alg(a) if a.is_zero())
    }

    /// Held in a form whose equality is decidable.
    pub fn is_exact_form(&self) -> bool {
        !matches!(&self.form, Form::General(_))
    }

    fn exp_sum(&self) -> Option<ExpSum> {
        match &self.form {
            Form::Alg(a) => Some(ExpSum::constant(a)),
            Form::Exp(s) => Some(s.clone()),
            _ => None,
        }
    }

    fn pi_poly(&self) -> Option<PiPoly> {
        match &self.form {
            Form::Alg(a) => Some(PiPoly::constant(a)),
            Form::Pi(p) => Some(p.clone()),
            _ => None,
        }
    }

    fn sum_of(terms: Vec<ExactReal>) -> ExactReal {
        terms
            .into_iter()
            .reduce(|acc, t| ExactReal::general(Node::Add(acc, t)))
            .unwrap_or_else(|| ExactReal::from_i64(0))
    }

    fn product_of(factors: Vec<ExactReal>) -> ExactReal {
        factors
            .into_iter()
            .reduce(|acc, t| ExactReal::general(Node::Mul(acc, t)))
            .unwrap_or_else(|| ExactReal::from_i64(1))
    }

    /// The expression DAG that evaluates this value; built once and shared by every clone.
    fn node(&self) -> Arc<Node> {
        self.tree
            .get_or_init(|| match &self.form {
                Form::General(n) => n.clone(),
                Form::Alg(a) => Arc::new(Node::Alg(AlgLeaf {
                    value: a.clone(),
                    cache: Mutex::new(None),
                })),
                Form::Pi(p) => {
                    let terms = p
                        .terms
                        .iter()
                        .map(|(k, c)| {
                            let base = if *k >= 0 {
                                ExactReal::general(Node::Pi)
                            } else {
                                ExactReal::general(Node::Recip(ExactReal::general(Node::Pi)))
                            };
                            let mut factors = vec![ExactReal::from_algebraic(c.clone())];
                            factors.extend((0..k.unsigned_abs()).map(|_| base.clone()));
                            ExactReal::product_of(factors)
                        })
                        .collect();
                    ExactReal::sum_of(terms).node()
                }
                Form::Exp(s) => {
                    let terms = s
                        .terms
                        .iter()
                        .map(|(e, c)| {
                            let mut factors = Vec::new();
                            if !e.re.is_zero() {
                                factors.push(ExactReal::general(Node::Exp(
                                    ExactReal::from_algebraic(e.re.clone()),
                                )));
                            }
                            if e.im.is_zero() {
                                factors.push(ExactReal::from_algebraic(c.re.clone()));
                            } else {
                                let v = ExactReal::from_algebraic(e.im.clone());
                                let mut parts = Vec::new();
                                if !c.re.is_zero() {
                                    parts.push(ExactReal::product_of(vec![
                                        ExactReal::from_algebraic(c.re.clone()),
                                        ExactReal::general(Node::Cos(v.clone())),
                                    ]));
                                }
                                if !c.im.is_zero() {
                                    parts.push(ExactReal::product_of(vec![
                                        ExactReal::from_algebraic(c.im.neg()),
                                        ExactReal::general(Node::Sin(v)),
                                    ]));
                                }
                                factors.push(ExactReal::sum_of(parts));
                            }
                            ExactReal::product_of(factors)
                        })
                        .collect();
                    ExactReal::sum_of(terms).node()
                }
            })
            .clone()
    }

    pub fn add(&self, o: &ExactReal) -> ExactReal {
        if let (Form::Alg(a), Form::Alg(b)) = (&self.form, &o.form) {
            return Self::from_algebraic(a.add(b));
        }
        if let (Some(a), Some(b)) = (self.exp_sum(), o.exp_sum()) {
            return Self::of(Form::Exp(a.add(&b)));
        }
        if let (Some(a), Some(b)) = (self.pi_poly(), o.pi_poly()) {
            return Self::of(Form::Pi(a.add(&b)));
        }
        Self::general(Node::Add(self.clone(), o.clone()))
    }

    pub fn neg(&self) -> ExactReal {
        match &self.form {
            Form::Alg(a) => Self::from_algebraic(a.neg()),
            Form::Exp(s) => Self::of(Form::Exp(s.neg())),
            Form::Pi(p) => Self::of(Form::Pi(p.neg())),
            Form::General(_) => Self::general(Node::Neg(self.clone())),
        }
    }

    pub fn sub(&self, o: &ExactReal) -> ExactReal {
        self.add(&o.neg())
    }

    pub fn mul(&self, o: &ExactReal) -> ExactReal {
        if let (Form::Alg(a), Form::Alg(b)) = (&self.form, &o.form) {
            return Self::from_algebraic(a.mul(b));
        }
        if let (Some(a), Some(b)) = (self.exp_sum(), o.exp_sum()) {
            return Self::of(Form::Exp(a.mul(&b)));
        }
        if let (Some(a), Some(b)) = (self.pi_poly(), o.pi_poly()) {
            return Self::of(Form::Pi(a.mul(&b)));
        }
        Self::general(Node::Mul(self.clone(), o.clone()))
    }

    /// `1/self`: refused when the value is exactly zero; a general value that turns out to be zero
    /// fails when it is evaluated.
    pub fn recip(&self) -> Result<ExactReal, RealError> {
        if self.is_exactly_zero() {
            return Err(RealError::Domain("division by zero".to_string()));
        }
        Ok(match &self.form {
            Form::Alg(a) => Self::from_algebraic(a.recip().expect("nonzero")),
            Form::Exp(s) => match s.recip() {
                Some(r) => Self::of(Form::Exp(r)),
                None => Self::general(Node::Recip(self.clone())),
            },
            Form::Pi(p) => match p.recip() {
                Some(r) => Self::of(Form::Pi(r)),
                None => Self::general(Node::Recip(self.clone())),
            },
            Form::General(_) => Self::general(Node::Recip(self.clone())),
        })
    }

    pub fn div(&self, o: &ExactReal) -> Result<ExactReal, RealError> {
        Ok(self.mul(&o.recip()?))
    }

    /// `self^k`; `None` only for zero to a negative power.
    pub fn pow(&self, k: i32) -> Option<ExactReal> {
        if k < 0 {
            return self.recip().ok()?.pow(-k);
        }
        if let Form::Alg(a) = &self.form {
            return a.pow(k).map(Self::from_algebraic);
        }
        let mut acc = Self::from_i64(1);
        let mut base = self.clone();
        let mut e = k as u32;
        while e > 0 {
            if e & 1 == 1 {
                acc = acc.mul(&base);
            }
            e >>= 1;
            if e > 0 {
                base = base.mul(&base);
            }
        }
        Some(acc)
    }

    /// The real number `source` defines (see [`Definition`]), `name` being how the program spells the
    /// function. Its digits or approximations are read from the function only as a refinement needs
    /// them, and each is read once.
    pub fn defined(definition: Definition, name: &str, source: Arc<dyn Approximant>) -> ExactReal {
        let known = match definition {
            Definition::Digits { .. } => Known::Digits(Vec::new()),
            Definition::Approximations => Known::Approximations(Vec::new()),
        };
        ExactReal::general(Node::Defined(Arc::new(Defined {
            name: name.to_string(),
            definition,
            source,
            known: Mutex::new(known),
        })))
    }

    /// Whether the number can travel as data: one built from a function a program defines cannot, its
    /// function staying with the program.
    pub fn is_transmissible(&self) -> bool {
        match &self.form {
            Form::General(n) => n.is_transmissible(),
            _ => true,
        }
    }

    /// The solution of `x' = f(x)` from `start` at time `t ≥ 0`, one general real per component,
    /// enclosed by validated Taylor integration ([`crate::ode`]); `None` when the field's shape
    /// does not match the start or the time is negative. The components share one integration.
    pub fn flow(field: &Field, start: &[ExactReal], t: &Rational) -> Option<Vec<ExactReal>> {
        if start.is_empty()
            || start.len() != field.dimension()
            || field.components.iter().any(|c| c.iter().any(|m| m.powers.len() != start.len()))
            || t.is_negative()
        {
            return None;
        }
        let solution = Arc::new(FlowSolution {
            field: field.clone(),
            start: start.to_vec(),
            t: t.clone(),
            cache: Mutex::new(None),
        });
        Some(
            (0..start.len())
                .map(|i| ExactReal::general(Node::Flow(solution.clone(), i)))
                .collect(),
        )
    }

    /// The nonnegative square root; exact where [`ExactReal::nth_root`] is.
    pub fn sqrt(&self) -> Result<ExactReal, RealError> {
        self.nth_root(2)
    }

    /// The real `n`-th root (`n ≥ 1`), the nonnegative one for even `n`. Exact on algebraic numbers
    /// and on a single real exponential term, `ⁿ√(c·e^β) = ⁿ√c·e^(β/n)`; an even root of a value
    /// held exactly as negative is refused. Any other value's square root is evaluated as one, and
    /// its higher roots are `e^(ln|x|/n)` with the sign of `x`, which must then be told from zero.
    pub fn nth_root(&self, n: u32) -> Result<ExactReal, RealError> {
        assert!(n >= 1, "a root has a positive index");
        if n == 1 {
            return Ok(self.clone());
        }
        let refused = || RealError::Domain(negative_root(n));
        match &self.form {
            Form::Alg(a) => return a.nth_root(n).map(Self::from_algebraic).ok_or_else(refused),
            Form::Exp(s) => {
                if let [(e, c)] = s.terms.as_slice() {
                    if e.is_real() && c.is_real() {
                        let root = c.re.nth_root(n).ok_or_else(refused)?;
                        let beta = e.re.div(&Algebraic::from_i64(i64::from(n))).expect("n ≥ 1");
                        return Ok(Self::of(Form::Exp(ExpSum::from_terms(vec![(
                            Calg::real(beta),
                            Calg::real(root),
                        )]))));
                    }
                }
            }
            _ => {}
        }
        if n == 2 {
            return Ok(Self::general(Node::Sqrt(self.clone())));
        }
        let index = Self::from_rational(Rational::from_ratio_i64(1, i64::from(n)).expect("n ≥ 1"));
        match self.compare(&Self::from_i64(0), DEFAULT_BUDGET)? {
            Ordering::Equal => Ok(Self::from_i64(0)),
            Ordering::Greater => Ok(self.log()?.mul(&index).exp()),
            Ordering::Less if n % 2 == 0 => Err(refused()),
            Ordering::Less => Ok(self.neg().log()?.mul(&index).exp().neg()),
        }
    }

    pub fn exp(&self) -> ExactReal {
        match &self.form {
            Form::Alg(a) if a.is_zero() => Self::from_i64(1),
            Form::Alg(a) => Self::of(Form::Exp(ExpSum::from_terms(vec![(
                Calg::real(a.clone()),
                Calg::real(Algebraic::from_i64(1)),
            )]))),
            Form::General(n) => match n.as_ref() {
                Node::Log(inner) => inner.clone(),
                _ => Self::general(Node::Exp(self.clone())),
            },
            _ => Self::general(Node::Exp(self.clone())),
        }
    }

    /// The natural logarithm: refused at an exactly known nonpositive value; exact on a single real
    /// exponential term with a positive algebraic coefficient (`log(c·e^β) = β + log c`).
    pub fn log(&self) -> Result<ExactReal, RealError> {
        match &self.form {
            Form::Alg(a) if a.signum() <= 0 => Err(RealError::Domain(
                "logarithm of a nonpositive number".to_string(),
            )),
            Form::Alg(a) if *a == Algebraic::from_i64(1) => Ok(Self::from_i64(0)),
            Form::Exp(s) => match s.terms.as_slice() {
                [(e, c)] if e.is_real() && c.is_real() && c.re.signum() > 0 => {
                    let beta = Self::from_algebraic(e.re.clone());
                    if c.re == Algebraic::from_i64(1) {
                        Ok(beta)
                    } else {
                        Ok(beta.add(&Self::from_algebraic(c.re.clone()).log()?))
                    }
                }
                _ => Ok(Self::general(Node::Log(self.clone()))),
            },
            Form::General(n) => match n.as_ref() {
                Node::Exp(inner) => Ok(inner.clone()),
                _ => Ok(Self::general(Node::Log(self.clone()))),
            },
            _ => Ok(Self::general(Node::Log(self.clone()))),
        }
    }

    fn trig(&self, sine: bool) -> ExactReal {
        match &self.form {
            Form::Alg(a) if a.is_zero() => Self::from_i64(if sine { 0 } else { 1 }),
            Form::Alg(a) => {
                let half = Algebraic::from_rational(Rational::from_ratio_i64(1, 2).expect("2 ≠ 0"));
                let plus = Calg::imag(a.clone());
                let minus = Calg::imag(a.neg());
                let terms = if sine {
                    vec![(plus, Calg::imag(half.neg())), (minus, Calg::imag(half))]
                } else {
                    vec![(plus, Calg::real(half.clone())), (minus, Calg::real(half))]
                };
                Self::of(Form::Exp(ExpSum::from_terms(terms)))
            }
            _ => Self::general(if sine {
                Node::Sin(self.clone())
            } else {
                Node::Cos(self.clone())
            }),
        }
    }

    pub fn sin(&self) -> ExactReal {
        self.trig(true)
    }

    pub fn cos(&self) -> ExactReal {
        self.trig(false)
    }

    pub fn cosh(&self) -> ExactReal {
        self.exp().add(&self.neg().exp()).mul(&half_real())
    }

    pub fn sinh(&self) -> ExactReal {
        self.exp().sub(&self.neg().exp()).mul(&half_real())
    }

    /// A closed interval holding the value, at most `2^-bits` wide.
    pub fn enclosure(&self, bits: u32) -> Result<(Rational, Rational), RealError> {
        let iv = self.enclose_width(bits as i64, Self::cap_for(bits))?;
        Ok((iv.lo.to_rational(), iv.hi.to_rational()))
    }

    fn cap_for(bits: u32) -> i64 {
        4 * bits as i64 + DEFAULT_BUDGET as i64
    }

    /// An enclosure at most `2^-bits` wide, raising the working precision as needed up to `cap`.
    fn enclose_width(&self, bits: i64, cap: i64) -> Result<Iv, RealError> {
        if let Form::Alg(a) = &self.form {
            if let Some(r) = a.to_rational() {
                return Ok(Iv {
                    lo: Dyadic::from_rational_floor(&r, bits + 2),
                    hi: Dyadic::from_rational_ceil(&r, bits + 2),
                });
            }
        }
        let eps = Dyadic::new(one(), -bits);
        let mut p = bits + 8;
        loop {
            let iv = self.enclosed(p, cap)?;
            if iv.width().cmp(&eps) != Ordering::Greater {
                return Ok(iv);
            }
            if p >= cap {
                return Err(RealError::Inseparable {
                    what: format!("an enclosure of {} narrower than 2^-{bits}", self.describe()),
                    bits: cap as u32,
                });
            }
            p = (p + p / 2 + 16).min(cap);
        }
    }

    /// The order of two values. When their difference has an exact form the answer is exact (a
    /// nonzero exact difference is refined until its sign shows, which always terminates);
    /// otherwise the difference is refined to `2^-budget` and the comparison fails loudly if the
    /// values have not separated.
    pub fn compare(&self, o: &ExactReal, budget: u32) -> Result<Ordering, RealError> {
        let d = self.sub(o);
        if d.is_exactly_zero() {
            return Ok(Ordering::Equal);
        }
        let cap = if d.is_exact_form() { i64::MAX / 4 } else { budget as i64 };
        let mut p = 8i64;
        loop {
            let iv = d.enclosed(p, cap.max(p))?;
            if iv.lo.is_positive() {
                return Ok(Ordering::Greater);
            }
            if iv.hi.is_negative() {
                return Ok(Ordering::Less);
            }
            if p >= cap {
                return Err(RealError::Inseparable {
                    what: format!("{} and {}", self.describe(), o.describe()),
                    bits: budget,
                });
            }
            p = (p * 2).min(cap);
        }
    }

    /// Total comparison against a tolerance: `Close` when the difference is certainly within
    /// `tolerance`, otherwise the certain side — decided from one enclosure of the difference
    /// narrower than a quarter of the tolerance. Fails only when the difference itself cannot be
    /// evaluated (a division by a quantity inseparable from zero, a logarithm of a nonpositive
    /// number).
    pub fn compare_within(&self, o: &ExactReal, tolerance: &Rational) -> Result<Closeness, RealError> {
        let d = self.sub(o);
        if d.is_exactly_zero() {
            return Ok(Closeness::Close);
        }
        let quarter = tolerance.mul(&Rational::from_ratio_i64(1, 4).expect("4 ≠ 0"));
        let mut bits = 1i64;
        while Rational::new(one(), one().shl_bits(bits as u64)).expect("nonzero") > quarter {
            bits += 1;
        }
        let tol = Dyadic::from_rational_floor(tolerance, bits + 4);
        let iv = d.enclose_width(bits, Self::cap_for(bits as u32))?;
        Ok(
            if iv.lo.cmp(&tol.neg()) != Ordering::Less && iv.hi.cmp(&tol) != Ordering::Greater {
                Closeness::Close
            } else if iv.lo.is_positive() {
                Closeness::Greater
            } else {
                Closeness::Less
            },
        )
    }

    /// `⌊self⌋`: exact on algebraic numbers; otherwise refined until the enclosure lies within one
    /// unit interval, failing at `2^-budget`.
    pub fn floor(&self, budget: u32) -> Result<BigInt, RealError> {
        if let Form::Alg(a) = &self.form {
            return Ok(a.floor());
        }
        let mut p = 8i64;
        loop {
            let iv = self.enclosed(p, (budget as i64).max(p))?;
            let (fl, fh) = (iv.lo.floor_scaled(0), iv.hi.floor_scaled(0));
            if fl == fh {
                return Ok(fl);
            }
            if p >= budget as i64 {
                return Err(RealError::Inseparable {
                    what: format!("{} from the integer {fh}", self.describe()),
                    bits: budget,
                });
            }
            p = (p * 2).min(budget as i64);
        }
    }

    /// `round(|self|·10^k)`, half away from zero: exact on rationals, otherwise refined until the
    /// enclosure rounds to one integer.
    fn scaled_round(&self, k: i64) -> Result<BigInt, RealError> {
        let ten_k = Rational::from_bigint(BigInt::from_i64(10).pow(k.unsigned_abs() as u32));
        let scale = |r: Rational| -> Rational {
            if k >= 0 {
                r.mul(&ten_k)
            } else {
                r.div(&ten_k).expect("10^k ≠ 0")
            }
        };
        if let Some(r) = self.exact_algebraic().and_then(|a| a.to_rational()) {
            return Ok(scale(r.abs()).round());
        }
        let half = Rational::from_ratio_i64(1, 2).expect("2 ≠ 0");
        let digits_bits = (k.max(0) as f64 * std::f64::consts::LOG2_10).ceil() as i64;
        let cap = Self::cap_for(digits_bits as u32 + 64);
        let mut p = 16 + digits_bits;
        loop {
            let iv = self.enclosed(p, cap.max(p))?;
            let (lo, hi) = if !iv.hi.is_positive() {
                (iv.hi.neg(), iv.lo.neg())
            } else {
                (iv.lo.clone(), iv.hi.clone())
            };
            if !lo.is_negative() {
                let a = scale(lo.to_rational()).add(&half).floor();
                let b = scale(hi.to_rational()).add(&half).floor();
                if a == b {
                    return Ok(a);
                }
            }
            if p >= cap {
                return Err(RealError::Inseparable {
                    what: format!("{} from a rounding boundary", self.describe()),
                    bits: cap as u32,
                });
            }
            p = (p + p / 2 + 16).min(cap);
        }
    }

    /// `sig` significant digits, correctly rounded (half away from zero on an exact tie), in plain
    /// positional notation; a value that cannot be printed shows its error in angle brackets.
    pub fn to_decimal(&self, sig: u32) -> String {
        self.try_to_decimal(sig).unwrap_or_else(|e| format!("⟨{e}⟩"))
    }

    /// [`ExactReal::to_decimal`], with the failure (a value inseparable from zero or from a
    /// rounding boundary within the budget) returned rather than printed.
    pub fn try_to_decimal(&self, sig: u32) -> Result<String, RealError> {
        if self.is_exactly_zero() {
            return Ok("0".to_string());
        }
        let sig = sig.max(1) as i64;
        let zero = Self::from_i64(0);
        let negative = self.compare(&zero, DEFAULT_BUDGET)? == Ordering::Less;
        let abs = if negative { self.neg() } else { self.clone() };
        let mut exp10 = abs.decimal_exponent()?;
        let digits = loop {
            let text = abs.scaled_round(sig - 1 - exp10)?.to_string();
            if text.len() as i64 > sig {
                exp10 += 1;
                continue;
            }
            break text;
        };
        let body = if exp10 >= sig - 1 {
            format!("{digits}{}", "0".repeat((exp10 - sig + 1) as usize))
        } else if exp10 >= 0 {
            let (a, b) = digits.split_at((exp10 + 1) as usize);
            format!("{a}.{b}")
        } else {
            format!("0.{}{digits}", "0".repeat((-exp10 - 1) as usize))
        };
        Ok(if negative { format!("-{body}") } else { body })
    }

    /// `⌊log₁₀ self⌋` for a positive value.
    fn decimal_exponent(&self) -> Result<i64, RealError> {
        let ten = Self::from_i64(10);
        let mut power = Self::from_i64(1);
        let mut e = 0i64;
        if self.compare(&power, DEFAULT_BUDGET)? != Ordering::Less {
            loop {
                let next = power.mul(&ten);
                if self.compare(&next, DEFAULT_BUDGET)? == Ordering::Less {
                    return Ok(e);
                }
                power = next;
                e += 1;
            }
        }
        let tenth = Self::from_rational(Rational::from_ratio_i64(1, 10).expect("10 ≠ 0"));
        loop {
            power = power.mul(&tenth);
            e -= 1;
            if self.compare(&power, DEFAULT_BUDGET)? != Ordering::Less {
                return Ok(e);
            }
        }
    }

    /// The integer part and exactly `frac` digits after the point, truncated toward zero.
    pub fn truncated_decimal(&self, frac: u32) -> Result<String, RealError> {
        let negative = self.compare(&Self::from_i64(0), DEFAULT_BUDGET)? == Ordering::Less;
        let abs = if negative { self.neg() } else { self.clone() };
        let ten_k = Self::from_rational(Rational::from_bigint(BigInt::from_i64(10).pow(frac)));
        let digits_bits = (frac as f64 * std::f64::consts::LOG2_10).ceil() as u32;
        let n = abs.mul(&ten_k).floor(digits_bits + DEFAULT_BUDGET)?;
        let text = format!("{:0>width$}", n.to_string(), width = frac as usize + 1);
        let (int, fr) = text.split_at(text.len() - frac as usize);
        let sign = if negative { "-" } else { "" };
        Ok(if frac == 0 {
            format!("{sign}{int}")
        } else {
            format!("{sign}{int}.{fr}")
        })
    }

    /// A structural description: exact forms symbolically, general reals as the formula that
    /// built them (`π + e`, `log(2)`, `1/(π - e)`), never by evaluating them.
    pub fn describe(&self) -> String {
        match &self.form {
            Form::General(n) => n.describe(),
            _ => self.to_string(),
        }
    }
}

impl Node {
    fn is_transmissible(&self) -> bool {
        match self {
            Node::Alg(_) | Node::Pi => true,
            Node::Add(a, b) | Node::Mul(a, b) => a.is_transmissible() && b.is_transmissible(),
            Node::Neg(a)
            | Node::Recip(a)
            | Node::Exp(a)
            | Node::Log(a)
            | Node::Sin(a)
            | Node::Cos(a)
            | Node::Sqrt(a) => a.is_transmissible(),
            Node::Flow(solution, _) => solution.start.iter().all(ExactReal::is_transmissible),
            Node::Defined(_) => false,
        }
    }

    fn describe(&self) -> String {
        let wrap = |x: &ExactReal| {
            let d = x.describe();
            if d.contains(' ') {
                format!("({d})")
            } else {
                d
            }
        };
        match self {
            Node::Alg(leaf) => leaf.value.to_string(),
            Node::Pi => "π".to_string(),
            Node::Add(a, b) => {
                if let Form::General(n) = &b.form {
                    if let Node::Neg(inner) = n.as_ref() {
                        return format!("{} - {}", a.describe(), wrap(inner));
                    }
                }
                let bd = b.describe();
                match bd.strip_prefix('-') {
                    Some(rest) if !rest.contains(' ') => format!("{} - {rest}", a.describe()),
                    _ if bd.contains(' ') => format!("{} + ({bd})", a.describe()),
                    _ => format!("{} + {bd}", a.describe()),
                }
            }
            Node::Mul(a, b) => format!("{}·{}", wrap(a), wrap(b)),
            Node::Neg(a) => format!("-{}", wrap(a)),
            Node::Recip(a) => format!("1/{}", wrap(a)),
            Node::Exp(a) => format!("exp({})", a.describe()),
            Node::Log(a) => format!("log({})", a.describe()),
            Node::Sin(a) => format!("sin({})", a.describe()),
            Node::Cos(a) => format!("cos({})", a.describe()),
            Node::Sqrt(a) => format!("√({})", a.describe()),
            Node::Flow(solution, i) => format!("x{} of a flow at time {}", i + 1, solution.t),
            Node::Defined(d) => d.describe(),
        }
    }
}

// ── display ───────────────────────────────────────────────────────────────────────────────────

/// A coefficient in front of a product: bare when it is a plain integer or radical, in
/// parentheses when it is a sum or a fraction.
fn coefficient_text(a: &Algebraic) -> String {
    let s = a.to_string();
    if s.contains(' ') || s.contains('/') {
        format!("({s})")
    } else {
        s
    }
}

fn exponent_text(a: &Algebraic, base: &str) -> String {
    if *a == Algebraic::from_i64(1) {
        return base.to_string();
    }
    match a.to_rational() {
        Some(r) if r.is_integer() && !r.is_negative() => format!("{base}^{r}"),
        _ => format!("{base}^({a})"),
    }
}

/// Write `Σ sign·factors` with the first sign attached and the rest spaced.
fn write_sum(f: &mut fmt::Formatter<'_>, terms: Vec<(bool, String)>) -> fmt::Result {
    for (i, (negative, body)) in terms.iter().enumerate() {
        match (i, negative) {
            (0, true) => write!(f, "-{body}")?,
            (0, false) => write!(f, "{body}")?,
            (_, true) => write!(f, " - {body}")?,
            (_, false) => write!(f, " + {body}")?,
        }
    }
    Ok(())
}

/// `coefficient·factor·…`, with a unit coefficient dropped (the sign is carried separately).
fn product_text(coefficient: &Algebraic, factors: &[String]) -> (bool, String) {
    let negative = coefficient.signum() < 0;
    let magnitude = if negative { coefficient.neg() } else { coefficient.clone() };
    let mut parts: Vec<String> = Vec::new();
    if factors.is_empty() {
        return (negative, magnitude.to_string());
    }
    if magnitude != Algebraic::from_i64(1) {
        parts.push(coefficient_text(&magnitude));
    }
    parts.extend(factors.iter().cloned());
    (negative, parts.join("·"))
}

impl fmt::Display for ExpSum {
    /// Exponential terms by decreasing growth: a real exponent prints as `c·e^u`; a conjugate pair
    /// `a·e^{u+iv} + ā·e^{u−iv}` prints as its real form `2Re(a)·e^u·cos(v) − 2Im(a)·e^u·sin(v)`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let two = Algebraic::from_i64(2);
        let mut terms = Vec::new();
        for (e, c) in &self.terms {
            if e.im.signum() < 0 {
                continue;
            }
            let mut factors = Vec::new();
            if !e.re.is_zero() {
                factors.push(exponent_text(&e.re, "e"));
            }
            if e.im.is_zero() {
                terms.push(product_text(&c.re, &factors));
                continue;
            }
            let cos_coef = c.re.mul(&two);
            let sin_coef = c.im.mul(&two).neg();
            if !cos_coef.is_zero() {
                let mut fs = factors.clone();
                fs.push(format!("cos({})", e.im));
                terms.push(product_text(&cos_coef, &fs));
            }
            if !sin_coef.is_zero() {
                let mut fs = factors;
                fs.push(format!("sin({})", e.im));
                terms.push(product_text(&sin_coef, &fs));
            }
        }
        write_sum(f, terms)
    }
}

impl fmt::Display for PiPoly {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let terms = self
            .terms
            .iter()
            .map(|(k, c)| {
                let factors = match k {
                    0 => Vec::new(),
                    1 => vec!["π".to_string()],
                    k if *k > 1 => vec![format!("π^{k}")],
                    k => vec![format!("π^({k})")],
                };
                product_text(c, &factors)
            })
            .collect();
        write_sum(f, terms)
    }
}

impl fmt::Display for ExactReal {
    /// An exact form symbolically (`√2`, `e^2 + 1`, `cos(1)`, `π^2 + 1`); a general real as
    /// [`DISPLAY_DIGITS`] correctly rounded significant digits.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.form {
            Form::Alg(a) => write!(f, "{a}"),
            Form::Exp(s) => write!(f, "{s}"),
            Form::Pi(p) => write!(f, "{p}"),
            Form::General(_) => write!(f, "{}", self.to_decimal(DISPLAY_DIGITS)),
        }
    }
}

impl fmt::Debug for ExactReal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ExactReal({self})")
    }
}

// ── the canonical encoding ────────────────────────────────────────────────────────────────────

impl crate::exact_codec::ExactCodec for ExactReal {
    /// A form tag, then the form: `0` an algebraic number; `1` an exponential sum (each term's
    /// exponent and coefficient as real and imaginary algebraic parts); `2` a polynomial in π
    /// (each power and coefficient); `3` a general real as the operation that built it (`0` add,
    /// `1` multiply, `2` negate, `3` reciprocal, `4` exp, `5` log, `6` sin, `7` cos, `8` square
    /// root) and its operands.
    fn write(&self, out: &mut Vec<u8>) {
        use crate::exact_codec::write_len;
        match &self.form {
            Form::Alg(a) => {
                out.push(0);
                a.write(out);
            }
            Form::Exp(sum) => {
                out.push(1);
                write_len(sum.terms.len(), out);
                for (e, c) in &sum.terms {
                    e.re.write(out);
                    e.im.write(out);
                    c.re.write(out);
                    c.im.write(out);
                }
            }
            Form::Pi(poly) => {
                out.push(2);
                write_len(poly.terms.len(), out);
                for (k, c) in &poly.terms {
                    (*k as i64).write(out);
                    c.write(out);
                }
            }
            Form::General(node) => {
                out.push(3);
                let (tag, operands): (u8, Vec<&ExactReal>) = match node.as_ref() {
                    Node::Add(a, b) => (0, vec![a, b]),
                    Node::Mul(a, b) => (1, vec![a, b]),
                    Node::Neg(a) => (2, vec![a]),
                    Node::Recip(a) => (3, vec![a]),
                    Node::Exp(a) => (4, vec![a]),
                    Node::Log(a) => (5, vec![a]),
                    Node::Sin(a) => (6, vec![a]),
                    Node::Cos(a) => (7, vec![a]),
                    Node::Sqrt(a) => (8, vec![a]),
                    Node::Flow(solution, i) => {
                        out.push(9);
                        write_len(solution.field.components.len(), out);
                        for monomials in &solution.field.components {
                            write_len(monomials.len(), out);
                            for m in monomials {
                                m.coefficient.write(out);
                                for e in &m.powers {
                                    u64::from(*e).write(out);
                                }
                            }
                        }
                        for x in &solution.start {
                            x.write(out);
                        }
                        solution.t.write(out);
                        write_len(*i, out);
                        return;
                    }
                    Node::Defined(d) => {
                        out.push(10);
                        let description = d.describe();
                        write_len(description.len(), out);
                        out.extend_from_slice(description.as_bytes());
                        return;
                    }
                    Node::Alg(_) | Node::Pi => {
                        unreachable!("an algebraic or π leaf is an exact form, never a general real")
                    }
                };
                out.push(tag);
                for x in operands {
                    x.write(out);
                }
            }
        }
    }

    fn read(buf: &[u8], pos: &mut usize) -> Option<Self> {
        read_real(buf, pos, 0)
    }
}

/// A real at nesting `depth`, rebuilt through the operations that make it so that only canonical
/// encodings survive the round trip; an exponential sum must be conjugate-symmetric (real).
fn read_real(buf: &[u8], pos: &mut usize, depth: usize) -> Option<ExactReal> {
    use crate::exact_codec::{read_byte, read_len, ExactCodec, MAX_WIRE_DEPTH, MAX_WIRE_LEN};
    if depth > MAX_WIRE_DEPTH {
        return None;
    }
    match read_byte(buf, pos)? {
        0 => Some(ExactReal::from_algebraic(Algebraic::read(buf, pos)?)),
        1 => {
            let n = read_len(buf, pos, MAX_WIRE_LEN)?;
            let mut raw = Vec::with_capacity(n.min(64));
            for _ in 0..n {
                let e = Calg {
                    re: Algebraic::read(buf, pos)?,
                    im: Algebraic::read(buf, pos)?,
                };
                let c = Calg {
                    re: Algebraic::read(buf, pos)?,
                    im: Algebraic::read(buf, pos)?,
                };
                raw.push((e, c));
            }
            let sum = ExpSum::from_terms(raw);
            let conjugate = |z: &Calg| Calg {
                re: z.re.clone(),
                im: z.im.neg(),
            };
            let real = sum
                .terms
                .iter()
                .all(|(e, c)| sum.terms.iter().any(|(f, d)| *f == conjugate(e) && *d == conjugate(c)));
            real.then(|| ExactReal::of(Form::Exp(sum)))
        }
        2 => {
            let n = read_len(buf, pos, MAX_WIRE_LEN)?;
            let mut raw = Vec::with_capacity(n.min(64));
            for _ in 0..n {
                let k = i32::try_from(i64::read(buf, pos)?).ok()?;
                raw.push((k, Algebraic::read(buf, pos)?));
            }
            Some(ExactReal::of(Form::Pi(PiPoly::from_terms(raw))))
        }
        3 => {
            let tag = read_byte(buf, pos)?;
            if tag == 9 {
                return read_flow(buf, pos, depth);
            }
            let a = read_real(buf, pos, depth + 1)?;
            match tag {
                0 => Some(a.add(&read_real(buf, pos, depth + 1)?)),
                1 => Some(a.mul(&read_real(buf, pos, depth + 1)?)),
                2 => Some(a.neg()),
                3 => a.recip().ok(),
                4 => Some(a.exp()),
                5 => a.log().ok(),
                6 => Some(a.sin()),
                7 => Some(a.cos()),
                8 => a.sqrt().ok(),
                _ => None,
            }
        }
        _ => None,
    }
}

/// A flow's component: its field, its start and its time, rebuilt through [`ExactReal::flow`].
fn read_flow(buf: &[u8], pos: &mut usize, depth: usize) -> Option<ExactReal> {
    use crate::exact_codec::{read_len, ExactCodec, MAX_WIRE_LEN};
    use crate::ode::Monomial;
    let n = read_len(buf, pos, MAX_WIRE_LEN)?;
    let mut components = Vec::with_capacity(n.min(64));
    for _ in 0..n {
        let count = read_len(buf, pos, MAX_WIRE_LEN)?;
        let mut monomials = Vec::with_capacity(count.min(64));
        for _ in 0..count {
            let coefficient = Rational::read(buf, pos)?;
            let mut powers = Vec::with_capacity(n);
            for _ in 0..n {
                powers.push(u32::try_from(u64::read(buf, pos)?).ok()?);
            }
            monomials.push(Monomial { coefficient, powers });
        }
        components.push(monomials);
    }
    let mut start = Vec::with_capacity(n);
    for _ in 0..n {
        start.push(read_real(buf, pos, depth + 1)?);
    }
    let t = Rational::read(buf, pos)?;
    let i = read_len(buf, pos, n.saturating_sub(1))?;
    ExactReal::flow(&Field { components }, &start, &t)?.into_iter().nth(i)
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_base/exact_real/tests.rs"]
mod tests;
