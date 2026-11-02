//! The value a compiled LOGOS program holds wherever the exact extension reaches: real algebraic
//! numbers, computable reals, the infinitesimal and the hyperreals it generates, infinite integers,
//! ordinals, residue-dependent and provisional values — and every standard value that meets them.
//!
//! [`LogosValue`] is the compiled counterpart of the interpreter's runtime value on exactly that
//! ground. It keeps each value in its canonical form, so a variable whose value is an `Int` still
//! divides, promotes to a big integer and prints as an `Int`, and a binding widened to hold a
//! hyperreal later means what it meant before. How the exact types mix is
//! [`logicaffeine_base::tower`], and the supertask runtime is [`logicaffeine_base::supertask`],
//! both generic over the value model: this module gives them this one. The standard rules here —
//! integers that promote instead of wrapping, `/` that truncates two integers, exact rationals, IEEE
//! floats, text that concatenates — are the interpreter's, sentence for sentence, held together by
//! the value-model differential in the test suite.

use std::cmp::Ordering;
use std::fmt;
use std::sync::Arc;

use logicaffeine_base::algebraic::Algebraic;
use logicaffeine_base::exact_real::ExactReal;
use logicaffeine_base::limit_value::Converging;
use logicaffeine_base::numeric::{
    cmp_bigint_f64_exact, cmp_i64_f64_exact, cmp_rational_f64_exact, float_max, float_min,
    rational_from_f64_exact, BigInt, Decimal, Rational,
};
use logicaffeine_base::ordinal::Ordinal;
use logicaffeine_base::tower::{
    self, DependsValue, ExactRef, HyperValue, NaturalValue, Op, Rounding, Shape, TowerValue,
    ValueAlgebra,
};

/// A comparison, as a compiled program names it.
pub use logicaffeine_base::tower::Rel;

use crate::failure::{answered, index_out_of_bounds, DIVISION_BY_ZERO, MODULO_BY_ZERO};

/// A builtin of the exact extension, as a compiled program names it.
pub use logicaffeine_base::extension::Builtin as ExtensionBuiltin;

/// `builtin` applied to `args` in a compiled program: [`logicaffeine_base::extension::call`] on this
/// value model, the interpreter's own dispatcher, stopping with its refusal.
pub fn logos_extension(builtin: ExtensionBuiltin, args: Vec<LogosValue>) -> LogosValue {
    answered(logicaffeine_base::extension::call(builtin, args))
}

/// The integer and rational types, by names the value's own variants do not shadow.
type Integer = BigInt;
type Ratio = Rational;

/// A value of a compiled program where the exact extension reaches. See the module documentation.
#[derive(Clone, Debug)]
pub enum LogosValue {
    Int(i64),
    /// An integer outside `i64`; never one that fits.
    BigInt(Arc<BigInt>),
    /// A rational that is not whole.
    Rational(Arc<Rational>),
    Float(f64),
    Decimal(Arc<Decimal>),
    Bool(bool),
    Text(Arc<str>),
    Nothing,
    List(Arc<Vec<LogosValue>>),
    Tuple(Arc<Vec<LogosValue>>),
    Algebraic(Arc<Algebraic>),
    ExactReal(Arc<ExactReal>),
    Hyperreal(Arc<HyperValue>),
    Hypernatural(Arc<NaturalValue>),
    Ordinal(Arc<Ordinal>),
    Depends(Arc<DependsValue<LogosValue>>),
    Converging(Arc<Converging<LogosValue>>),
    /// A value of a kind the exact types do not mix with — a date, a span of time, a moment — held
    /// as its own compiled value, which prints, compares and combines as the interpreter's does.
    Other(Arc<dyn OtherValue>),
}

/// A compiled value of a kind outside the exact tower, as a [`LogosValue`] holds it: each answers for
/// itself how it prints, what it equals, how it orders and what it combines with, exactly as the
/// interpreter's value of that kind does. A pair it has no arm for is refused in the interpreter's
/// words by the caller.
pub trait OtherValue: fmt::Debug + Send + Sync + 'static {
    /// The kind's name as the language writes it.
    fn type_name(&self) -> &str;
    /// How the value prints.
    fn display_string(&self) -> String;
    /// The value, for a kind to recognise one of its own.
    fn as_any(&self) -> &dyn std::any::Any;
    /// The language's equality with another value of a kind outside the tower.
    fn equals(&self, o: &dyn OtherValue) -> bool;
    /// The order of `self` against `o`, `None` when the language orders no such pair.
    fn order(&self, o: &LogosValue) -> Option<Option<Ordering>>;
    /// `self op o`, or `o op self` when `reversed`; `None` when the language has no arm for the pair.
    fn combine(&self, op: Op, o: &LogosValue, reversed: bool) -> Option<Result<LogosValue, String>>;
}

impl LogosValue {
    /// An integer in its smallest form.
    pub fn from_bigint(b: BigInt) -> LogosValue {
        match b.to_i64() {
            Some(n) => LogosValue::Int(n),
            None => LogosValue::BigInt(Arc::new(b)),
        }
    }

    /// A rational in its smallest form: an integer when it is whole.
    pub fn from_rational(r: Rational) -> LogosValue {
        if r.is_integer() {
            return LogosValue::from_bigint(r.to_bigint().expect("a whole rational is an integer"));
        }
        LogosValue::Rational(Arc::new(r))
    }

    /// Text.
    pub fn text(t: &str) -> LogosValue {
        LogosValue::Text(Arc::from(t))
    }

    /// A list.
    pub fn list(items: Vec<LogosValue>) -> LogosValue {
        LogosValue::List(Arc::new(items))
    }

    /// A value of a kind outside the exact tower.
    pub fn other(v: impl OtherValue) -> LogosValue {
        LogosValue::Other(Arc::new(v))
    }

    /// The compiled value of a kind outside the tower this holds, when it holds one of that kind.
    pub fn other_as<T: OtherValue>(&self) -> Option<&T> {
        match self {
            LogosValue::Other(v) => v.as_any().downcast_ref::<T>(),
            _ => None,
        }
    }

    /// `a op b` where either side is of a kind outside the tower: the kind's own arm.
    fn other_arith(&self, op: Op, o: &LogosValue) -> Option<Result<LogosValue, String>> {
        if let LogosValue::Other(a) = self {
            if let Some(r) = a.combine(op, o, false) {
                return Some(r);
            }
        }
        match o {
            LogosValue::Other(b) => b.combine(op, self, true),
            _ => None,
        }
    }

    /// The program's infinitesimal named `name`, its reciprocal's residues modulo `modulus` limited
    /// to `allowed`.
    pub fn infinitesimal(name: &str, modulus: i64, allowed: &[i64]) -> Result<LogosValue, String> {
        let allowed: Vec<u64> = allowed.iter().map(|r| *r as u64).collect();
        tower::new_infinitesimal(name, modulus as u64, &allowed)
    }

    /// The value's type as the language names it.
    pub fn type_name(&self) -> &str {
        match self {
            LogosValue::Int(_) | LogosValue::BigInt(_) => "Int",
            LogosValue::Rational(_) => "Rational",
            LogosValue::Float(_) => "Float",
            LogosValue::Decimal(_) => "Decimal",
            LogosValue::Bool(_) => "Bool",
            LogosValue::Text(_) => "Text",
            LogosValue::Nothing => "Nothing",
            LogosValue::List(_) => "List",
            LogosValue::Tuple(_) => "Tuple",
            LogosValue::Algebraic(_) => "Algebraic",
            LogosValue::ExactReal(_) => "ExactReal",
            LogosValue::Hyperreal(_) => "Hyperreal",
            LogosValue::Hypernatural(_) => "Hypernatural",
            LogosValue::Ordinal(_) => "Ordinal",
            LogosValue::Depends(_) => "Depends",
            LogosValue::Converging(_) => "Converging",
            LogosValue::Other(v) => v.type_name(),
        }
    }

    /// How the value prints.
    pub fn display_string(&self) -> String {
        match self {
            LogosValue::Int(n) => n.to_string(),
            LogosValue::BigInt(b) => b.to_string(),
            LogosValue::Rational(r) => r.to_string(),
            LogosValue::Float(f) => crate::fmt::fmt_f64(*f),
            LogosValue::Decimal(d) => d.to_string(),
            LogosValue::Bool(b) => if *b { "true" } else { "false" }.to_string(),
            LogosValue::Text(t) => t.to_string(),
            LogosValue::Nothing => "nothing".to_string(),
            LogosValue::List(items) => format!(
                "[{}]",
                items.iter().map(LogosValue::display_string).collect::<Vec<_>>().join(", ")
            ),
            LogosValue::Tuple(items) => format!(
                "({})",
                items.iter().map(LogosValue::display_string).collect::<Vec<_>>().join(", ")
            ),
            LogosValue::Other(v) => v.display_string(),
            exact => tower::display(exact).expect("an exact value displays"),
        }
    }

    /// The truth a condition reads: a residue-dependent answer, or a provisional one no proof
    /// settled, is refused with the sentence every tier gives.
    pub fn decided_truth(&self) -> Result<bool, String> {
        if let Some(why) = tower::undecided_branch(self) {
            return Err(why);
        }
        Ok(self.is_truthy())
    }

    fn is_truthy(&self) -> bool {
        match self {
            LogosValue::Bool(b) => *b,
            LogosValue::Int(n) => *n != 0,
            LogosValue::Float(f) => *f != 0.0,
            LogosValue::BigInt(b) => !b.is_zero(),
            LogosValue::Rational(r) => !r.is_zero(),
            LogosValue::Decimal(d) => !d.is_zero(),
            LogosValue::Text(t) => !t.is_empty(),
            LogosValue::List(items) => !items.is_empty(),
            LogosValue::Nothing => false,
            _ => true,
        }
    }

    fn big(&self) -> Option<BigInt> {
        match self {
            LogosValue::Int(n) => Some(BigInt::from_i64(*n)),
            LogosValue::BigInt(b) => Some((**b).clone()),
            _ => None,
        }
    }

    fn rat(&self) -> Option<Rational> {
        match self {
            LogosValue::Int(n) => Some(Rational::from_i64(*n)),
            LogosValue::BigInt(b) => Some(Rational::from_bigint((**b).clone())),
            LogosValue::Rational(r) => Some((**r).clone()),
            LogosValue::Decimal(d) => Some(d.to_rational()),
            _ => None,
        }
    }

    fn dec(&self) -> Option<Decimal> {
        match self {
            LogosValue::Int(n) => Some(Decimal::from_i64(*n)),
            LogosValue::BigInt(b) => Some(Decimal::from_bigint((**b).clone())),
            LogosValue::Decimal(d) => Some((**d).clone()),
            _ => None,
        }
    }

    fn f64(&self) -> Option<f64> {
        match self {
            LogosValue::Int(n) => Some(*n as f64),
            LogosValue::BigInt(b) => Some(b.to_f64()),
            LogosValue::Rational(r) => Some(r.to_f64()),
            LogosValue::Decimal(d) => Some(d.to_rational().to_f64()),
            LogosValue::Float(f) => Some(*f),
            _ => None,
        }
    }

    fn is_decimal(&self) -> bool {
        matches!(self, LogosValue::Decimal(_))
    }

    fn is_float(&self) -> bool {
        matches!(self, LogosValue::Float(_))
    }

    /// `+`, `−` or `×` of two decimals, or of a decimal and an integer, a rational, a float.
    fn decimal_op(
        &self,
        o: &LogosValue,
        dec: impl Fn(&Decimal, &Decimal) -> Decimal,
        rat: impl Fn(&Rational, &Rational) -> Rational,
        flt: impl Fn(f64, f64) -> f64,
    ) -> Option<LogosValue> {
        if !self.is_decimal() && !o.is_decimal() {
            return None;
        }
        if let (Some(a), Some(b)) = (self.dec(), o.dec()) {
            return Some(LogosValue::Decimal(Arc::new(dec(&a, &b))));
        }
        if let (Some(a), Some(b)) = (self.rat(), o.rat()) {
            return Some(LogosValue::from_rational(rat(&a, &b)));
        }
        Some(LogosValue::Float(flt(self.f64()?, o.f64()?)))
    }

    /// `+`, `−` or `×` of the numeric tower: exact integers that promote, exact rationals, a float
    /// operand making the result a float. `None` when either side is not a number.
    fn tower_op(
        &self,
        o: &LogosValue,
        int: impl Fn(i64, i64) -> Option<i64>,
        big: impl Fn(&BigInt, &BigInt) -> BigInt,
        rat: impl Fn(&Rational, &Rational) -> Rational,
        flt: impl Fn(f64, f64) -> f64,
    ) -> Option<LogosValue> {
        use LogosValue::*;
        Some(match (self, o) {
            (Int(a), Int(b)) => match int(*a, *b) {
                Some(n) => Int(n),
                None => LogosValue::from_bigint(big(&Integer::from_i64(*a), &Integer::from_i64(*b))),
            },
            (Int(_) | BigInt(_), Int(_) | BigInt(_)) => {
                LogosValue::from_bigint(big(&self.big()?, &o.big()?))
            }
            (Float(_), Int(_) | BigInt(_) | Float(_) | Rational(_))
            | (Int(_) | BigInt(_) | Rational(_), Float(_)) => Float(flt(self.f64()?, o.f64()?)),
            (Rational(_), Int(_) | BigInt(_) | Rational(_)) | (Int(_) | BigInt(_), Rational(_)) => {
                LogosValue::from_rational(rat(&self.rat()?, &o.rat()?))
            }
            _ => return None,
        })
    }

    fn arith(op: Op, a: LogosValue, b: LogosValue) -> Result<LogosValue, String> {
        match op {
            Op::Add => a.add(&b),
            Op::Sub => a.sub(&b),
            Op::Mul => a.mul(&b),
            Op::Div => a.div(&b),
            Op::ExactDiv => a.exact_div(&b),
            Op::FloorDiv => a.floor_div(&b),
            Op::Mod => a.rem(&b),
            Op::Pow => a.pow(&b),
        }
    }

    /// `a + b`.
    pub fn add(&self, o: &LogosValue) -> Result<LogosValue, String> {
        if let Some(r) = tower::arith(Op::Add, self, o, &LogosValue::arith) {
            return r;
        }
        if let Some(r) = self.decimal_op(o, |a, b| a.add(b), |a, b| a.add(b), |a, b| a + b) {
            return Ok(r);
        }
        if let Some(r) = self.tower_op(o, i64::checked_add, BigInt::add, Rational::add, |a, b| a + b) {
            return Ok(r);
        }
        if let Some(r) = self.other_arith(Op::Add, o) {
            return r;
        }
        match (self, o) {
            (LogosValue::Text(a), LogosValue::Text(b)) => Ok(LogosValue::text(&format!("{a}{b}"))),
            (LogosValue::Text(a), other) => Ok(LogosValue::text(&format!("{a}{}", other.display_string()))),
            (other, LogosValue::Text(b)) => Ok(LogosValue::text(&format!("{}{b}", other.display_string()))),
            (LogosValue::List(a), LogosValue::List(b)) => {
                Ok(LogosValue::list(a.iter().chain(b.iter()).cloned().collect()))
            }
            _ => Err(format!("Cannot add {} and {}", self.type_name(), o.type_name())),
        }
    }

    /// `a − b`.
    pub fn sub(&self, o: &LogosValue) -> Result<LogosValue, String> {
        if let Some(r) = tower::arith(Op::Sub, self, o, &LogosValue::arith) {
            return r;
        }
        if let Some(r) = self.decimal_op(o, |a, b| a.sub(b), |a, b| a.sub(b), |a, b| a - b) {
            return Ok(r);
        }
        if let Some(r) = self.tower_op(o, i64::checked_sub, BigInt::sub, Rational::sub, |a, b| a - b) {
            return Ok(r);
        }
        if let Some(r) = self.other_arith(Op::Sub, o) {
            return r;
        }
        Err(format!("Cannot subtract {} from {}", o.type_name(), self.type_name()))
    }

    /// `a × b`.
    pub fn mul(&self, o: &LogosValue) -> Result<LogosValue, String> {
        if let Some(r) = tower::arith(Op::Mul, self, o, &LogosValue::arith) {
            return r;
        }
        if let Some(r) = self.decimal_op(o, |a, b| a.mul(b), |a, b| a.mul(b), |a, b| a * b) {
            return Ok(r);
        }
        if let Some(r) = self.tower_op(o, i64::checked_mul, BigInt::mul, Rational::mul, |a, b| a * b) {
            return Ok(r);
        }
        match (self, o) {
            (LogosValue::List(items), LogosValue::Int(n)) | (LogosValue::Int(n), LogosValue::List(items)) => {
                let count = (*n).max(0) as usize;
                let mut out = Vec::with_capacity(items.len() * count);
                for _ in 0..count {
                    out.extend(items.iter().cloned());
                }
                Ok(LogosValue::list(out))
            }
            _ => Err(format!("Cannot multiply {} and {}", self.type_name(), o.type_name())),
        }
    }

    /// The polymorphic `/`: two integers truncate, a rational divides exactly, a float divides as a
    /// float, the exact types divide in their field.
    pub fn div(&self, o: &LogosValue) -> Result<LogosValue, String> {
        use LogosValue::*;
        if let Some(r) = tower::arith(Op::Div, self, o, &LogosValue::arith) {
            return r;
        }
        if self.is_decimal() || o.is_decimal() {
            if let (Some(a), Some(b)) = (self.rat(), o.rat()) {
                return a
                    .div(&b)
                    .map(LogosValue::from_rational)
                    .ok_or_else(|| DIVISION_BY_ZERO.to_string());
            }
            if let (Some(a), Some(b)) = (self.f64(), o.f64()) {
                if b == 0.0 {
                    return Err(DIVISION_BY_ZERO.to_string());
                }
                return Ok(Float(a / b));
            }
        }
        match (self, o) {
            (Int(a), Int(b)) => {
                if *b == 0 {
                    return Err(DIVISION_BY_ZERO.to_string());
                }
                Ok(match a.checked_div(*b) {
                    Some(q) => Int(q),
                    None => LogosValue::from_bigint(
                        Integer::from_i64(*a).div_rem(&Integer::from_i64(*b)).expect("nonzero divisor").0,
                    ),
                })
            }
            (Int(_) | BigInt(_), Int(_) | BigInt(_)) => {
                let (a, b) = (self.big().expect("integer"), o.big().expect("integer"));
                if b.is_zero() {
                    return Err(DIVISION_BY_ZERO.to_string());
                }
                Ok(LogosValue::from_bigint(a.div_rem(&b).expect("nonzero divisor").0))
            }
            (Float(_), Float(_) | Int(_) | BigInt(_) | Rational(_))
            | (Int(_) | BigInt(_) | Rational(_), Float(_)) => {
                let (a, b) = (self.f64().expect("number"), o.f64().expect("number"));
                let divisor_is_zero = match o {
                    Int(n) => *n == 0,
                    BigInt(_) | Rational(_) => false,
                    _ => b == 0.0,
                };
                if divisor_is_zero {
                    return Err(DIVISION_BY_ZERO.to_string());
                }
                Ok(Float(a / b))
            }
            (Rational(_), Int(_) | BigInt(_) | Rational(_)) | (Int(_) | BigInt(_), Rational(_)) => self
                .rat()
                .expect("rational")
                .div(&o.rat().expect("rational"))
                .map(LogosValue::from_rational)
                .ok_or_else(|| DIVISION_BY_ZERO.to_string()),
            _ => Err(format!("Cannot divide {} by {}", self.type_name(), o.type_name())),
        }
    }

    /// `/` in an exact context: every integer and rational divides exactly; a float defers to `/`.
    pub fn exact_div(&self, o: &LogosValue) -> Result<LogosValue, String> {
        if let Some(r) = tower::arith(Op::ExactDiv, self, o, &LogosValue::arith) {
            return r;
        }
        if let (Some(a), Some(b)) = (self.rat(), o.rat()) {
            return a
                .div(&b)
                .map(LogosValue::from_rational)
                .ok_or_else(|| DIVISION_BY_ZERO.to_string());
        }
        if self.is_float() || o.is_float() {
            return self.div(o);
        }
        Err(format!("Cannot divide {} by {}", self.type_name(), o.type_name()))
    }

    /// Floor division.
    pub fn floor_div(&self, o: &LogosValue) -> Result<LogosValue, String> {
        if let Some(r) = tower::arith(Op::FloorDiv, self, o, &LogosValue::arith) {
            return r;
        }
        if self.is_float() || o.is_float() {
            if let (Some(a), Some(b)) = (self.f64(), o.f64()) {
                if b == 0.0 {
                    return Err(DIVISION_BY_ZERO.to_string());
                }
                return Ok(LogosValue::Float((a / b).floor()));
            }
        }
        if let (Some(a), Some(b)) = (self.rat(), o.rat()) {
            return a
                .div(&b)
                .map(|q| LogosValue::from_bigint(q.floor()))
                .ok_or_else(|| DIVISION_BY_ZERO.to_string());
        }
        Err(format!("Cannot floor-divide {} by {}", self.type_name(), o.type_name()))
    }

    /// The remainder of two integers, its sign the dividend's.
    pub fn rem(&self, o: &LogosValue) -> Result<LogosValue, String> {
        use LogosValue::*;
        if let Some(r) = tower::arith(Op::Mod, self, o, &LogosValue::arith) {
            return r;
        }
        match (self, o) {
            (Int(a), Int(b)) => {
                if *b == 0 {
                    return Err(MODULO_BY_ZERO.to_string());
                }
                Ok(Int(a.checked_rem(*b).unwrap_or(0)))
            }
            (Int(_) | BigInt(_), Int(_) | BigInt(_)) => {
                let (a, b) = (self.big().expect("integer"), o.big().expect("integer"));
                if b.is_zero() {
                    return Err(MODULO_BY_ZERO.to_string());
                }
                Ok(LogosValue::from_bigint(a.div_rem(&b).expect("nonzero divisor").1))
            }
            _ => Err(format!(
                "Cannot compute modulo of {} and {}",
                self.type_name(),
                o.type_name()
            )),
        }
    }

    /// `base ^ exponent`.
    pub fn pow(&self, o: &LogosValue) -> Result<LogosValue, String> {
        use LogosValue::*;
        if let Some(r) = tower::arith(Op::Pow, self, o, &LogosValue::arith) {
            return r;
        }
        if self.is_float() || o.is_float() {
            let b = self
                .f64()
                .ok_or_else(|| format!("cannot raise {} to a power", self.type_name()))?;
            let e = o
                .f64()
                .ok_or_else(|| format!("cannot raise to a {} power", o.type_name()))?;
            return Ok(Float(b.powf(e)));
        }
        match (self, o) {
            (Int(b), Int(e)) => {
                if *e < 0 {
                    return Err(
                        "negative exponent on an integer (an Int can't hold a fraction — use a Float base)"
                            .to_string(),
                    );
                }
                let e = u32::try_from(*e).map_err(|_| "exponent too large".to_string())?;
                Ok(match b.checked_pow(e) {
                    Some(r) => Int(r),
                    None => LogosValue::from_bigint(Integer::from_i64(*b).pow(e)),
                })
            }
            (BigInt(b), Int(e)) => {
                let e = u32::try_from(*e).map_err(|_| {
                    "negative or too-large exponent on an integer (use a Float base)".to_string()
                })?;
                Ok(LogosValue::from_bigint(b.pow(e)))
            }
            (Rational(b), Int(e)) => {
                let e = i32::try_from(*e).map_err(|_| "exponent too large".to_string())?;
                b.pow(e)
                    .map(LogosValue::from_rational)
                    .ok_or_else(|| "zero raised to a negative power".to_string())
            }
            _ => Err(format!(
                "cannot raise {} to the {} power",
                self.type_name(),
                o.type_name()
            )),
        }
    }

    fn relate(rel: Rel, a: LogosValue, b: LogosValue) -> Result<LogosValue, String> {
        a.relation(rel, &b)
    }

    /// `a rel b`: a `Bool`, or a residue-dependent one where the answer depends on `⌈1/dt⌉`.
    pub fn relation(&self, rel: Rel, o: &LogosValue) -> Result<LogosValue, String> {
        if let Some(r) = tower::relation(rel, self, o, &LogosValue::relate) {
            return r;
        }
        match rel {
            Rel::Eq => Ok(LogosValue::Bool(self.values_equal(o))),
            Rel::NotEq => Ok(LogosValue::Bool(!self.values_equal(o))),
            _ => self.order(o).map(|ord| {
                LogosValue::Bool(ord.is_some_and(|o| match rel {
                    Rel::Lt => o == Ordering::Less,
                    Rel::Gt => o == Ordering::Greater,
                    Rel::LtEq => o != Ordering::Greater,
                    Rel::GtEq => o != Ordering::Less,
                    Rel::Eq | Rel::NotEq => unreachable!("handled above"),
                }))
            }),
        }
    }

    /// The order of two standard numbers, `None` for an unordered float; a refusal for anything
    /// else.
    fn order(&self, o: &LogosValue) -> Result<Option<Ordering>, String> {
        use LogosValue::*;
        Ok(match (self, o) {
            (Int(a), Int(b)) => Some(a.cmp(b)),
            (Int(_) | BigInt(_), Int(_) | BigInt(_)) => {
                Some(self.big().expect("integer").cmp(&o.big().expect("integer")))
            }
            (BigInt(a), Float(b)) => cmp_bigint_f64_exact(a, *b),
            (Float(a), BigInt(b)) => cmp_bigint_f64_exact(b, *a).map(Ordering::reverse),
            (Rational(_), Int(_) | BigInt(_) | Rational(_)) | (Int(_) | BigInt(_), Rational(_)) => {
                Some(self.rat().expect("rational").cmp(&o.rat().expect("rational")))
            }
            (Rational(a), Float(b)) => cmp_rational_f64_exact(a, *b),
            (Float(a), Rational(b)) => cmp_rational_f64_exact(b, *a).map(Ordering::reverse),
            (Float(a), Float(b)) => a.partial_cmp(b),
            (Int(a), Float(b)) => cmp_i64_f64_exact(*a, *b),
            (Float(a), Int(b)) => cmp_i64_f64_exact(*b, *a).map(Ordering::reverse),
            (Other(a), _) if a.order(o).is_some() => a.order(o).expect("checked"),
            (_, Other(b)) if b.order(self).is_some() => b.order(self).expect("checked").map(Ordering::reverse),
            (l, r) if l.is_decimal() || r.is_decimal() => match (l.rat(), r.rat()) {
                (Some(a), Some(b)) => Some(a.cmp(&b)),
                _ => match (l.f64(), r.f64()) {
                    (Some(a), Some(b)) => a.partial_cmp(&b),
                    _ => {
                        return Err(format!(
                            "Cannot compare {} and {}",
                            l.type_name(),
                            r.type_name()
                        ))
                    }
                },
            },
            _ => {
                return Err(format!(
                    "Cannot compare {} and {}",
                    self.type_name(),
                    o.type_name()
                ))
            }
        })
    }

    /// The language's equality: exact across numeric types, structural on lists and tuples.
    pub fn values_equal(&self, o: &LogosValue) -> bool {
        use LogosValue::*;
        if let Some(same) = tower::structurally_equal(self, o) {
            return same;
        }
        match (self, o) {
            (Int(a), Int(b)) => a == b,
            (BigInt(a), BigInt(b)) => a == b,
            (Rational(a), Rational(b)) => a == b,
            (Decimal(a), Decimal(b)) => a == b,
            (Float(a), Float(b)) => a == b,
            (Int(a), Float(b)) | (Float(b), Int(a)) => {
                cmp_i64_f64_exact(*a, *b) == Some(Ordering::Equal)
            }
            (BigInt(a), Float(b)) | (Float(b), BigInt(a)) => {
                cmp_bigint_f64_exact(a, *b) == Some(Ordering::Equal)
            }
            (Rational(a), Float(b)) | (Float(b), Rational(a)) => {
                cmp_rational_f64_exact(a, *b) == Some(Ordering::Equal)
            }
            (Bool(a), Bool(b)) => a == b,
            (Text(a), Text(b)) => a == b,
            (Nothing, Nothing) => true,
            (List(a), List(b)) | (Tuple(a), Tuple(b)) => {
                a.len() == b.len() && a.iter().zip(b.iter()).all(|(x, y)| x.values_equal(y))
            }
            (Other(a), Other(b)) => a.equals(&**b),
            _ => false,
        }
    }

    /// `not v`, lifted over residue branches and through provisional values.
    pub fn not(&self) -> Result<LogosValue, String> {
        if let Some(r) = tower::not(self, &|v| v.not()) {
            return r;
        }
        Ok(LogosValue::Bool(!self.is_truthy()))
    }

    fn rounded(&self, mode: Rounding, name: &str) -> Result<LogosValue, String> {
        use LogosValue::*;
        let recurse = |x: LogosValue| x.rounded(mode, name);
        if let Some(r) = tower::integer_part(self, mode, &recurse) {
            return r;
        }
        let whole = |f: f64| match mode {
            Rounding::Floor => f.floor(),
            Rounding::Ceil => f.ceil(),
            Rounding::Nearest => f.round(),
        };
        let exact = |r: Ratio| match mode {
            Rounding::Floor => r.floor(),
            Rounding::Ceil => r.ceil(),
            Rounding::Nearest => r.round(),
        };
        match self {
            Float(f) => Ok(Int(whole(*f) as i64)),
            Int(_) | BigInt(_) => Ok(self.clone()),
            Rational(r) => Ok(LogosValue::from_bigint(exact((**r).clone()))),
            Decimal(d) => Ok(LogosValue::from_bigint(exact(d.to_rational()))),
            _ => Err(format!("{name}() requires a number, got {}", self.type_name())),
        }
    }

    /// `a & b`: bitwise on integers, logical on booleans, and a non-negative big integer under a low
    /// mask `2^k - 1` leaves its low `k` bits.
    pub fn bit_and(&self, o: &LogosValue) -> Result<LogosValue, String> {
        use LogosValue::*;
        match (self, o) {
            (Int(a), Int(b)) => Ok(Int(a & b)),
            (BigInt(big), Int(m)) | (Int(m), BigInt(big))
                if *m >= 0 && (*m as u64).wrapping_add(1).is_power_of_two() && !big.is_negative() =>
            {
                let modulus = Integer::from_i64(*m).add(&Integer::from_i64(1));
                Ok(match big.div_rem(&modulus) {
                    Some((_, r)) => LogosValue::from_bigint(r),
                    None => Int(0),
                })
            }
            (Bool(a), Bool(b)) => Ok(Bool(a & b)),
            _ => Err(crate::failure::BITAND_OPERANDS.to_string()),
        }
    }

    /// `a | b`.
    pub fn bit_or(&self, o: &LogosValue) -> Result<LogosValue, String> {
        use LogosValue::*;
        match (self, o) {
            (Int(a), Int(b)) => Ok(Int(a | b)),
            (Bool(a), Bool(b)) => Ok(Bool(a | b)),
            _ => Err(crate::failure::BITOR_OPERANDS.to_string()),
        }
    }

    /// `a ^ b`.
    pub fn bit_xor(&self, o: &LogosValue) -> Result<LogosValue, String> {
        use LogosValue::*;
        match (self, o) {
            (Int(a), Int(b)) => Ok(Int(a ^ b)),
            (Bool(a), Bool(b)) => Ok(Bool(a ^ b)),
            _ => Err(crate::failure::BITXOR_OPERANDS.to_string()),
        }
    }

    /// `a << b`, the count taken modulo the width.
    pub fn shl(&self, o: &LogosValue) -> Result<LogosValue, String> {
        match (self, o) {
            (LogosValue::Int(a), LogosValue::Int(b)) => Ok(LogosValue::Int(a.wrapping_shl(*b as u32))),
            _ => Err(crate::failure::SHL_OPERANDS.to_string()),
        }
    }

    /// `a >> b`, the count taken modulo the width.
    pub fn shr(&self, o: &LogosValue) -> Result<LogosValue, String> {
        match (self, o) {
            (LogosValue::Int(a), LogosValue::Int(b)) => Ok(LogosValue::Int(a.wrapping_shr(*b as u32))),
            _ => Err(crate::failure::SHR_OPERANDS.to_string()),
        }
    }

    /// `a is approximately b`, on the two values as floats.
    pub fn approx_eq(&self, o: &LogosValue) -> Result<LogosValue, String> {
        use LogosValue::*;
        let as_f64 = |v: &LogosValue| match v {
            Float(f) => Some(*f),
            Int(n) => Some(*n as f64),
            BigInt(b) => Some(b.to_f64()),
            Rational(r) => Some(r.to_f64()),
            _ => None,
        };
        match (as_f64(self), as_f64(o)) {
            (Some(a), Some(b)) => Ok(Bool(crate::ops::logos_approx_eq(a, b))),
            _ => Err(format!(
                "`is approximately` compares numbers, got {} and {}",
                self.type_name(),
                o.type_name()
            )),
        }
    }

    /// `a followed by b`: the items of two sequences, in order.
    pub fn seq_concat(&self, o: &LogosValue) -> Result<LogosValue, String> {
        match (self, o) {
            (LogosValue::List(a), LogosValue::List(b)) => {
                Ok(LogosValue::list(a.iter().chain(b.iter()).cloned().collect()))
            }
            _ => Err("`followed by` requires two sequences (merge two sequences into one)".to_string()),
        }
    }

    /// The builtin `abs`: exact on integers (promoting `|i64::MIN|`), rationals and decimals.
    pub fn abs(&self) -> Result<LogosValue, String> {
        use LogosValue::*;
        match self {
            Int(n) => Ok(match n.checked_abs() {
                Some(a) => Int(a),
                None => LogosValue::from_bigint(Integer::from_i64(*n).abs()),
            }),
            BigInt(b) => Ok(LogosValue::from_bigint(b.abs())),
            Rational(r) => Ok(LogosValue::from_rational(r.abs())),
            Decimal(d) => Ok(Decimal(Arc::new(d.abs()))),
            Float(f) => Ok(Float(f.abs())),
            _ => Err(format!("abs() requires a number, got {}", self.type_name())),
        }
    }

    /// The builtin `sqrt`, a float.
    pub fn sqrt(&self) -> Result<LogosValue, String> {
        use LogosValue::*;
        match self {
            Float(f) => Ok(Float(f.sqrt())),
            Int(n) => Ok(Float((*n as f64).sqrt())),
            BigInt(b) => Ok(Float(b.to_f64().sqrt())),
            _ => Err(format!("sqrt() requires a number, got {}", self.type_name())),
        }
    }

    /// The builtin `min`.
    pub fn min(&self, o: &LogosValue) -> Result<LogosValue, String> {
        use LogosValue::*;
        match (self, o) {
            (Int(x), Int(y)) => Ok(Int(*x.min(y))),
            (Float(x), Float(y)) => Ok(Float(float_min(*x, *y))),
            (Int(x), Float(y)) => Ok(Float(float_min(*x as f64, *y))),
            (Float(x), Int(y)) => Ok(Float(float_min(*x, *y as f64))),
            (Decimal(x), Decimal(y)) => Ok(Decimal(if x <= y { x.clone() } else { y.clone() })),
            _ => Err("min() requires numbers".to_string()),
        }
    }

    /// The builtin `max`.
    pub fn max(&self, o: &LogosValue) -> Result<LogosValue, String> {
        use LogosValue::*;
        match (self, o) {
            (Int(x), Int(y)) => Ok(Int(*x.max(y))),
            (Float(x), Float(y)) => Ok(Float(float_max(*x, *y))),
            (Int(x), Float(y)) => Ok(Float(float_max(*x as f64, *y))),
            (Float(x), Int(y)) => Ok(Float(float_max(*x, *y as f64))),
            (Decimal(x), Decimal(y)) => Ok(Decimal(if x >= y { x.clone() } else { y.clone() })),
            _ => Err("max() requires numbers".to_string()),
        }
    }

    /// The builtin `pow`: an integer power is exact, a negative one a float.
    pub fn power(&self, o: &LogosValue) -> Result<LogosValue, String> {
        use LogosValue::*;
        match (self, o) {
            (Int(b), Int(e)) => Ok(if *e >= 0 {
                match b.checked_pow(*e as u32) {
                    Some(p) => Int(p),
                    None => LogosValue::from_bigint(Integer::from_i64(*b).pow(*e as u32)),
                }
            } else {
                Float((*b as f64).powi(*e as i32))
            }),
            (BigInt(b), Int(e)) => Ok(if *e >= 0 {
                LogosValue::from_bigint(b.pow(*e as u32))
            } else {
                Float(b.to_f64().powi(*e as i32))
            }),
            (Float(b), Int(e)) => Ok(Float(b.powi(*e as i32))),
            (Float(b), Float(e)) => Ok(Float(b.powf(*e))),
            (Int(b), Float(e)) => Ok(Float((*b as f64).powf(*e))),
            _ => Err("pow() requires numbers".to_string()),
        }
    }

    /// `length of self`: a list's or a tuple's item count, a text's byte count.
    pub fn length_of(&self) -> Result<i64, String> {
        match self {
            LogosValue::List(items) | LogosValue::Tuple(items) => Ok(items.len() as i64),
            LogosValue::Text(t) => Ok(t.len() as i64),
            _ => Err(format!("Cannot get length of {}", self.type_name())),
        }
    }

    /// `self contains value`: an equal item of a list, a substring of a text.
    pub fn contains_value(&self, value: &LogosValue) -> Result<bool, String> {
        match self {
            LogosValue::List(items) => Ok(items.iter().any(|i| i.values_equal(value))),
            LogosValue::Text(t) => match value {
                LogosValue::Text(needle) => Ok(t.contains(&**needle)),
                other => Err(format!("Cannot check if Text contains {}", other.type_name())),
            },
            _ => Err(format!("Cannot check contains on {}", self.type_name())),
        }
    }

    /// The values a `Repeat` over `self` binds, taken once before the loop runs: a list's items, a
    /// text's characters.
    pub fn items(&self) -> Result<Vec<LogosValue>, String> {
        match self {
            LogosValue::List(items) => Ok(items.to_vec()),
            LogosValue::Text(t) => Ok(t.chars().map(|c| LogosValue::text(&c.to_string())).collect()),
            _ => Err(format!("Cannot iterate over {}", self.type_name())),
        }
    }

    /// `Push value to self`.
    pub fn push_value(&mut self, value: LogosValue) -> Result<(), String> {
        match self {
            LogosValue::List(items) => {
                Arc::make_mut(items).push(value);
                Ok(())
            }
            _ => Err("Can only push to a List".to_string()),
        }
    }

    /// `Set item index of self to value`.
    pub fn set_item(&mut self, index: &LogosValue, value: LogosValue) -> Result<(), String> {
        match (&mut *self, index) {
            (LogosValue::List(items), LogosValue::Int(i)) => {
                let at = crate::indexing::logos_offset(*i, items.len()).ok_or_else(|| index_out_of_bounds(i))?;
                Arc::make_mut(items)[at] = value;
                Ok(())
            }
            (LogosValue::List(_), _) => Err("List index must be an integer".to_string()),
            _ => Err(format!("Cannot index into {}", self.type_name())),
        }
    }

    /// `Pop from self`: the last item, or nothing from an empty list.
    pub fn pop_value(&mut self) -> Result<LogosValue, String> {
        match self {
            LogosValue::List(items) => Ok(Arc::make_mut(items).pop().unwrap_or(LogosValue::Nothing)),
            _ => Err("Can only pop from a List".to_string()),
        }
    }

    /// `item index of self`: the 1-based item of a list or a tuple, the character of a text.
    pub fn item(&self, index: &LogosValue) -> Result<LogosValue, String> {
        use LogosValue::*;
        match (self, index) {
            (List(items) | Tuple(items), Int(i)) => crate::indexing::logos_offset(*i, items.len())
                .map(|at| items[at].clone())
                .ok_or_else(|| index_out_of_bounds(i)),
            (Text(t), Int(i)) => {
                let chars = t.chars().count();
                crate::indexing::logos_offset(*i, chars)
                    .map(|at| LogosValue::text(&t.chars().nth(at).expect("offset in range").to_string()))
                    .ok_or_else(|| index_out_of_bounds(i))
            }
            _ => Err(format!("Cannot index {} with {}", self.type_name(), index.type_name())),
        }
    }

    /// `items start through end of self`: the 1-based, inclusive run of a list's items ([`crate::logos_slice_range`]);
    /// one that reaches outside the list is empty.
    pub fn slice(&self, start: &LogosValue, end: &LogosValue) -> Result<LogosValue, String> {
        match (self, start, end) {
            (LogosValue::List(items), LogosValue::Int(s), LogosValue::Int(e)) => {
                Ok(LogosValue::list(items[crate::logos_slice_range(*s, *e, items.len())].to_vec()))
            }
            _ => Err("Slice requires List and Int indices".to_string()),
        }
    }

    /// The machine integer this is where the language takes one: refused with `past_word` for an integer past
    /// `i64`, with `fraction` for a rational that is not whole, and by its type for any other value.
    pub fn integer_at(&self, past_word: &str, fraction: &str) -> Result<i64, String> {
        match self {
            LogosValue::Int(n) => Ok(*n),
            LogosValue::BigInt(_) => Err(past_word.to_string()),
            LogosValue::Rational(_) => Err(fraction.to_string()),
            other => Err(format!("Expected Int, found {}", other.type_name())),
        }
    }

    /// The real an operator over a real answers.
    pub fn into_real(self) -> f64 {
        match self {
            LogosValue::Float(f) => f,
            other => unreachable!("an operator over a real answered a {}", other.type_name()),
        }
    }

    /// The language's `floor`.
    pub fn floor(&self) -> Result<LogosValue, String> {
        self.rounded(Rounding::Floor, "floor")
    }

    /// The language's `ceil`.
    pub fn ceil(&self) -> Result<LogosValue, String> {
        self.rounded(Rounding::Ceil, "ceil")
    }

    /// The language's `round`.
    pub fn round(&self) -> Result<LogosValue, String> {
        self.rounded(Rounding::Nearest, "round")
    }
}

impl fmt::Display for LogosValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.display_string())
    }
}

/// An unset slot of an exact type, as the interpreter starts one: nothing.
impl Default for LogosValue {
    fn default() -> Self {
        LogosValue::Nothing
    }
}

impl PartialEq for LogosValue {
    fn eq(&self, o: &Self) -> bool {
        self.values_equal(o)
    }
}

impl From<i64> for LogosValue {
    fn from(n: i64) -> Self {
        LogosValue::Int(n)
    }
}

impl From<f64> for LogosValue {
    fn from(f: f64) -> Self {
        LogosValue::Float(f)
    }
}

impl From<bool> for LogosValue {
    fn from(b: bool) -> Self {
        LogosValue::Bool(b)
    }
}

impl From<&str> for LogosValue {
    fn from(t: &str) -> Self {
        LogosValue::text(t)
    }
}

impl From<String> for LogosValue {
    fn from(t: String) -> Self {
        LogosValue::text(&t)
    }
}

impl From<crate::types::LogosRational> for LogosValue {
    fn from(r: crate::types::LogosRational) -> Self {
        LogosValue::from_rational(r.0)
    }
}

impl From<crate::types::LogosInt> for LogosValue {
    fn from(n: crate::types::LogosInt) -> Self {
        match n {
            crate::types::LogosInt::Small(n) => LogosValue::Int(n),
            crate::types::LogosInt::Big(b) => LogosValue::from_bigint(*b),
        }
    }
}

impl<T: Into<LogosValue>> From<Vec<T>> for LogosValue {
    fn from(items: Vec<T>) -> Self {
        LogosValue::list(items.into_iter().map(Into::into).collect())
    }
}

macro_rules! from_machine_integer {
    ($($t:ty),*) => {$(
        impl From<$t> for LogosValue {
            fn from(n: $t) -> Self {
                match i64::try_from(n) {
                    Ok(n) => LogosValue::Int(n),
                    Err(_) => LogosValue::from_bigint(
                        BigInt::parse_decimal(&n.to_string()).expect("a machine integer's digits"),
                    ),
                }
            }
        }
    )*};
}

from_machine_integer!(i8, i16, i32, i128, u8, u16, u32, u64, u128, usize);

impl From<()> for LogosValue {
    fn from(_: ()) -> Self {
        LogosValue::Nothing
    }
}

impl From<crate::types::LogosDecimal> for LogosValue {
    fn from(d: crate::types::LogosDecimal) -> Self {
        LogosValue::Decimal(Arc::new(d.0))
    }
}

impl<T: Clone + Into<LogosValue>> From<crate::types::LogosSeq<T>> for LogosValue {
    fn from(items: crate::types::LogosSeq<T>) -> Self {
        LogosValue::list(items.0.borrow().iter().cloned().map(Into::into).collect())
    }
}

impl crate::ops::Truthy for LogosValue {
    fn truthy(&self) -> bool {
        answered(self.decided_truth())
    }
}

impl crate::indexing::LogosIndex<i64> for LogosValue {
    type Output = LogosValue;
    fn logos_get(&self, index: i64) -> LogosValue {
        answered(self.item(&LogosValue::Int(index)))
    }
}

impl crate::indexing::LogosIndex<LogosValue> for LogosValue {
    type Output = LogosValue;
    fn logos_get(&self, index: LogosValue) -> LogosValue {
        answered(self.item(&index))
    }
}

impl TowerValue for LogosValue {
    fn exact(&self) -> Option<ExactRef<'_, Self>> {
        Some(match self {
            LogosValue::Algebraic(a) => ExactRef::Algebraic(a),
            LogosValue::ExactReal(x) => ExactRef::Real(x),
            LogosValue::Hyperreal(h) => ExactRef::Hyper(h),
            LogosValue::Hypernatural(n) => ExactRef::Natural(n),
            LogosValue::Ordinal(o) => ExactRef::Ordinal(o),
            LogosValue::Depends(d) => ExactRef::Depends(d),
            LogosValue::Converging(c) => ExactRef::Converging(c),
            _ => return None,
        })
    }

    fn standard_rational(&self) -> Option<Rational> {
        match self {
            LogosValue::Float(f) => rational_from_f64_exact(*f),
            other => other.rat(),
        }
    }

    fn integer(&self) -> Option<BigInt> {
        self.big()
    }

    fn exact_rational(&self) -> Option<Rational> {
        match self {
            LogosValue::Int(n) => Some(Rational::from_i64(*n)),
            LogosValue::BigInt(b) => Some(Rational::from_bigint((**b).clone())),
            LogosValue::Rational(r) => Some((**r).clone()),
            _ => None,
        }
    }

    fn is_standard_number(&self) -> bool {
        matches!(
            self,
            LogosValue::Int(_)
                | LogosValue::BigInt(_)
                | LogosValue::Rational(_)
                | LogosValue::Decimal(_)
                | LogosValue::Float(_)
        )
    }

    fn text(&self) -> Option<&str> {
        match self {
            LogosValue::Text(t) => Some(t),
            _ => None,
        }
    }

    fn type_name(&self) -> String {
        LogosValue::type_name(self).to_string()
    }

    fn display_string(&self) -> String {
        LogosValue::display_string(self)
    }

    fn values_equal(&self, other: &Self) -> bool {
        LogosValue::values_equal(self, other)
    }

    fn is_nothing(&self) -> bool {
        matches!(self, LogosValue::Nothing)
    }

    fn from_rational(r: Rational) -> Self {
        LogosValue::from_rational(r)
    }

    fn from_bigint(b: BigInt) -> Self {
        LogosValue::from_bigint(b)
    }

    fn boolean(b: bool) -> Self {
        LogosValue::Bool(b)
    }

    fn nothing() -> Self {
        LogosValue::Nothing
    }

    fn algebraic(a: Algebraic) -> Self {
        LogosValue::Algebraic(Arc::new(a))
    }

    fn exact_real(x: ExactReal) -> Self {
        LogosValue::ExactReal(Arc::new(x))
    }

    fn hyper(h: HyperValue) -> Self {
        LogosValue::Hyperreal(Arc::new(h))
    }

    fn natural(n: NaturalValue) -> Self {
        LogosValue::Hypernatural(Arc::new(n))
    }

    fn ordinal(o: Ordinal) -> Self {
        LogosValue::Ordinal(Arc::new(o))
    }

    fn depends(d: DependsValue<Self>) -> Self {
        LogosValue::Depends(Arc::new(d))
    }

    fn converging(c: Converging<Self>) -> Self {
        LogosValue::Converging(Arc::new(c))
    }
}

impl ValueAlgebra for LogosValue {
    fn add(a: Self, b: Self) -> Result<Self, String> {
        a.add(&b)
    }

    fn subtract(a: Self, b: Self) -> Result<Self, String> {
        a.sub(&b)
    }

    fn multiply(a: Self, b: Self) -> Result<Self, String> {
        a.mul(&b)
    }

    fn exact_divide(a: Self, b: Self) -> Result<Self, String> {
        a.exact_div(&b)
    }

    fn modulo(a: Self, b: Self) -> Result<Self, String> {
        a.rem(&b)
    }

    fn power(a: Self, b: Self) -> Result<Self, String> {
        a.pow(&b)
    }

    fn holds(rel: Rel, a: &Self, b: &Self) -> Result<bool, String> {
        a.relation(rel, b)?.decided_truth()
    }

    fn floor(v: Self) -> Result<Self, String> {
        LogosValue::floor(&v)
    }

    fn ceil(v: Self) -> Result<Self, String> {
        LogosValue::ceil(&v)
    }

    fn list(values: Vec<Self>) -> Self {
        LogosValue::list(values)
    }

    fn shape(&self) -> Shape<Self> {
        match self {
            LogosValue::Bool(_) => Shape::Bool,
            LogosValue::Int(n) => Shape::Int(*n),
            LogosValue::Float(_) => Shape::Float,
            LogosValue::BigInt(b) => Shape::BigInt((**b).clone()),
            LogosValue::Rational(r) => Shape::Rational((**r).clone()),
            LogosValue::Decimal(_) => Shape::Decimal,
            LogosValue::Text(t) => Shape::Text(t.len()),
            LogosValue::List(items) => Shape::List(items.to_vec()),
            LogosValue::Tuple(items) => Shape::Tuple(items.to_vec()),
            _ => Shape::Other,
        }
    }
}

// ── real numbers a compiled program defines ───────────────────────────────────────────────────

/// A function a compiled program defines a real number by: its answer at a place, or its refusal.
/// The function reads nothing but its argument, so any thread may call it.
type Definer = Arc<dyn Fn(i64) -> Result<LogosValue, String> + Send + Sync>;

/// The rational a compiled function's answer is; anything else is refused in a defined real's words.
pub(crate) fn answered_rational(name: &str, n: u64, v: &LogosValue) -> Result<Rational, String> {
    match v {
        LogosValue::Int(i) => Ok(Rational::from_i64(*i)),
        LogosValue::BigInt(b) => Ok(Rational::from_bigint((**b).clone())),
        LogosValue::Rational(r) => Ok((**r).clone()),
        other => Err(logicaffeine_base::exact_real::not_a_rational_answer(name, n, other.type_name())),
    }
}

/// A compiled function a real number is defined by, and how the program spells it.
struct Compiled {
    name: String,
    f: Definer,
}

impl logicaffeine_base::exact_real::Approximant for Compiled {
    fn call(&self, n: u64) -> Result<Rational, String> {
        let arg = i64::try_from(n).map_err(|_| format!("{}({n}) is past every Int", self.name))?;
        answered_rational(&self.name, n, &(self.f)(arg)?)
    }
}

impl LogosValue {
    /// The real number `f` defines by its digits in `base` or its approximations (`kind`, as
    /// [`logicaffeine_base::exact_real::Definition::named`] reads it), `name` being how the program
    /// spells `f`.
    pub fn defined_real(
        kind: &str,
        base: i64,
        name: &str,
        f: impl Fn(i64) -> LogosValue + Send + Sync + 'static,
    ) -> LogosValue {
        answered(Self::define_real(kind, base, name, move |n| Ok(f(n))))
    }

    /// [`LogosValue::defined_real`] for a function that may refuse an answer, refused itself when
    /// `kind` and `base` name no definition.
    pub fn define_real(
        kind: &str,
        base: i64,
        name: &str,
        f: impl Fn(i64) -> Result<LogosValue, String> + Send + Sync + 'static,
    ) -> Result<LogosValue, String> {
        let definition = logicaffeine_base::exact_real::Definition::named(kind, base)?;
        let source = Arc::new(Compiled {
            name: name.to_string(),
            f: Arc::new(f),
        });
        Ok(LogosValue::ExactReal(Arc::new(ExactReal::defined(definition, name, source))))
    }

    /// `Ok` when the value may cross to a task, `Err` with the interpreter's sentence when it holds a real
    /// number a function of the program defines, which stays with the program.
    pub fn sendable(&self) -> Result<(), String> {
        match self {
            LogosValue::List(items) | LogosValue::Tuple(items) => items.iter().try_for_each(LogosValue::sendable),
            v if logicaffeine_base::tower::transmissible(v) => Ok(()),
            _ => Err(logicaffeine_base::exact_real::not_sendable()),
        }
    }
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_data/exact/tests.rs"]
mod tests;
