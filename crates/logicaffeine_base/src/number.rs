//! A number of the exact tower as base holds one itself: the coefficient of an expansion of a value of
//! several infinitesimals whose coefficients are not all exact — `exp(dt + dx)`, whose coefficients in
//! `dx` are the germ of `e^dt` — and the value the flows of such expansions start from.
//!
//! Every operation is the tower's own ([`crate::tower`]), with integers and rationals below it, so a
//! coefficient computes, compares, refuses and prints exactly as a program's value of the same kind.

use crate::algebraic::Algebraic;
use crate::exact_real::ExactReal;
use crate::expansion::{Coefficient, ExpansionError};
use crate::limit_value::Converging;
use crate::numeric::{BigInt, Rational};
use crate::ordinal::Ordinal;
use crate::tower::{
    self, display, structurally_equal, DependsValue, ExactRef, HyperValue, NaturalValue, Op, RealFn, Rel, TowerValue,
};

/// A number of the exact tower. See the module documentation.
#[derive(Clone, Debug)]
pub enum Number {
    Int(BigInt),
    Rat(Rational),
    Bool(bool),
    Nothing,
    Alg(Algebraic),
    Real(ExactReal),
    Hyper(HyperValue),
    Nat(NaturalValue),
    Ord(Ordinal),
    Dep(Box<DependsValue<Number>>),
    Conv(Box<Converging<Number>>),
}

impl TowerValue for Number {
    fn exact(&self) -> Option<ExactRef<'_, Self>> {
        Some(match self {
            Number::Alg(a) => ExactRef::Algebraic(a),
            Number::Real(x) => ExactRef::Real(x),
            Number::Hyper(h) => ExactRef::Hyper(h),
            Number::Nat(n) => ExactRef::Natural(n),
            Number::Ord(o) => ExactRef::Ordinal(o),
            Number::Dep(d) => ExactRef::Depends(d),
            Number::Conv(c) => ExactRef::Converging(c),
            Number::Int(_) | Number::Rat(_) | Number::Bool(_) | Number::Nothing => return None,
        })
    }

    fn standard_rational(&self) -> Option<Rational> {
        match self {
            Number::Int(n) => Some(Rational::from_bigint(n.clone())),
            Number::Rat(r) => Some(r.clone()),
            _ => None,
        }
    }

    fn integer(&self) -> Option<BigInt> {
        match self {
            Number::Int(n) => Some(n.clone()),
            _ => None,
        }
    }

    fn exact_rational(&self) -> Option<Rational> {
        self.standard_rational()
    }

    fn is_standard_number(&self) -> bool {
        matches!(self, Number::Int(_) | Number::Rat(_))
    }

    fn text(&self) -> Option<&str> {
        None
    }

    fn type_name(&self) -> String {
        match self {
            Number::Int(_) => "Int",
            Number::Rat(_) => "Rational",
            Number::Bool(_) => "Bool",
            Number::Nothing => "Nothing",
            Number::Alg(_) => "Algebraic",
            Number::Real(_) => "ExactReal",
            Number::Hyper(_) => "Hyperreal",
            Number::Nat(_) => "Hypernatural",
            Number::Ord(_) => "Ordinal",
            Number::Dep(_) => "Depends",
            Number::Conv(_) => "Converging",
        }
        .to_string()
    }

    fn display_string(&self) -> String {
        match self {
            Number::Int(n) => n.to_string(),
            Number::Rat(r) => r.to_string(),
            Number::Bool(b) => b.to_string(),
            Number::Nothing => "nothing".to_string(),
            other => display(other).expect("an exact value displays"),
        }
    }

    fn values_equal(&self, other: &Self) -> bool {
        if let Some(same) = structurally_equal(self, other) {
            return same;
        }
        match (self, other) {
            (Number::Bool(a), Number::Bool(b)) => a == b,
            (Number::Nothing, Number::Nothing) => true,
            _ => match (self.standard_rational(), other.standard_rational()) {
                (Some(a), Some(b)) => a == b,
                _ => false,
            },
        }
    }

    fn is_nothing(&self) -> bool {
        matches!(self, Number::Nothing)
    }

    fn from_rational(r: Rational) -> Self {
        match r.to_bigint() {
            Some(n) if r.is_integer() => Number::Int(n),
            _ => Number::Rat(r),
        }
    }

    fn from_bigint(b: BigInt) -> Self {
        Number::Int(b)
    }

    fn boolean(b: bool) -> Self {
        Number::Bool(b)
    }

    fn nothing() -> Self {
        Number::Nothing
    }

    fn algebraic(a: Algebraic) -> Self {
        Number::Alg(a)
    }

    fn exact_real(x: ExactReal) -> Self {
        Number::Real(x)
    }

    fn hyper(h: HyperValue) -> Self {
        Number::Hyper(h)
    }

    fn natural(n: NaturalValue) -> Self {
        Number::Nat(n)
    }

    fn ordinal(o: Ordinal) -> Self {
        Number::Ord(o)
    }

    fn depends(d: DependsValue<Self>) -> Self {
        Number::Dep(Box::new(d))
    }

    fn converging(c: Converging<Self>) -> Self {
        Number::Conv(Box::new(c))
    }
}

impl Number {
    /// `a op b`: the exact tower first, then integers and rationals, exact division exact.
    pub fn arith(op: Op, a: Number, b: Number) -> Result<Number, String> {
        if let Some(r) = tower::arith(op, &a, &b, &Number::arith) {
            return r;
        }
        let (Some(x), Some(y)) = (a.standard_rational(), b.standard_rational()) else {
            return Err(format!("cannot combine {} and {}", a.type_name(), b.type_name()));
        };
        let zero = || "Division by zero".to_string();
        Ok(match op {
            Op::Add => Number::from_rational(x.add(&y)),
            Op::Sub => Number::from_rational(x.sub(&y)),
            Op::Mul => Number::from_rational(x.mul(&y)),
            Op::Div | Op::ExactDiv => Number::from_rational(x.div(&y).ok_or_else(zero)?),
            Op::FloorDiv => Number::from_bigint(x.div(&y).ok_or_else(zero)?.floor()),
            Op::Mod => {
                let q = x.div(&y).ok_or_else(zero)?.floor();
                Number::from_rational(x.sub(&y.mul(&Rational::from_bigint(q))))
            }
            Op::Pow => match b.integer().and_then(|k| k.to_i64()).and_then(|k| i32::try_from(k).ok()) {
                Some(k) => Number::from_rational(
                    x.pow(k).ok_or_else(|| "zero cannot be raised to a negative power".to_string())?,
                ),
                None => match b.exact_rational() {
                    Some(_) => match tower::arith(Op::Pow, &Number::algebraic(Algebraic::from_rational(x)), &b, &Number::arith) {
                        Some(r) => r?,
                        None => return Err(format!("cannot raise {} to the {} power", a.type_name(), b.type_name())),
                    },
                    None => return Err(format!("cannot raise {} to the {} power", a.type_name(), b.type_name())),
                },
            },
        })
    }

    /// `a rel b`, a `Bool`, or the refusal of an answer that depends on the residue or is provisional.
    pub fn holds(rel: Rel, a: &Number, b: &Number) -> Result<bool, String> {
        let answer = Number::relation(rel, a.clone(), b.clone())?;
        if let Some(why) = tower::undecided_branch(&answer) {
            return Err(why);
        }
        match answer {
            Number::Bool(b) => Ok(b),
            other => Err(format!("a comparison answered {}", other.display_string())),
        }
    }

    fn relation(rel: Rel, a: Number, b: Number) -> Result<Number, String> {
        if let Some(r) = tower::relation(rel, &a, &b, &Number::relation) {
            return r;
        }
        let (Some(x), Some(y)) = (a.standard_rational(), b.standard_rational()) else {
            return Ok(Number::Bool(match rel {
                Rel::Eq => a.values_equal(&b),
                Rel::NotEq => !a.values_equal(&b),
                _ => return Err(format!("cannot compare {} and {}", a.type_name(), b.type_name())),
            }));
        };
        let o = x.cmp(&y);
        Ok(Number::Bool(match rel {
            Rel::Lt => o.is_lt(),
            Rel::Gt => o.is_gt(),
            Rel::LtEq => o.is_le(),
            Rel::GtEq => o.is_ge(),
            Rel::Eq => o.is_eq(),
            Rel::NotEq => o.is_ne(),
        }))
    }
}

fn refused(e: String) -> ExpansionError {
    ExpansionError::Coefficient(e)
}

impl Coefficient for Number {
    fn zero() -> Self {
        Number::Int(BigInt::from_i64(0))
    }
    fn one() -> Self {
        Number::Int(BigInt::from_i64(1))
    }
    fn of_rational(r: Rational) -> Self {
        <Number as TowerValue>::from_rational(r)
    }
    fn add(&self, o: &Self) -> Result<Self, ExpansionError> {
        Number::arith(Op::Add, self.clone(), o.clone()).map_err(refused)
    }
    fn sub(&self, o: &Self) -> Result<Self, ExpansionError> {
        Number::arith(Op::Sub, self.clone(), o.clone()).map_err(refused)
    }
    fn neg(&self) -> Result<Self, ExpansionError> {
        Number::arith(Op::Sub, Number::zero(), self.clone()).map_err(refused)
    }
    fn mul(&self, o: &Self) -> Result<Self, ExpansionError> {
        Number::arith(Op::Mul, self.clone(), o.clone()).map_err(refused)
    }
    fn is_zero(&self) -> Result<bool, ExpansionError> {
        if let Some(r) = self.standard_rational() {
            return Ok(r.is_zero());
        }
        Number::holds(Rel::Eq, self, &Number::zero()).map_err(refused)
    }
    fn recip(&self) -> Result<Self, ExpansionError> {
        if self.is_zero()? {
            return Err(ExpansionError::DivisionByZero);
        }
        Number::arith(Op::ExactDiv, Number::one(), self.clone()).map_err(refused)
    }
    fn sign(&self) -> Result<i32, ExpansionError> {
        if Number::holds(Rel::Lt, self, &Number::zero()).map_err(refused)? {
            return Ok(-1);
        }
        Ok(i32::from(Number::holds(Rel::Gt, self, &Number::zero()).map_err(refused)?))
    }
    fn nth_root(&self, n: u32) -> Result<Self, ExpansionError> {
        let exponent = <Number as TowerValue>::from_rational(Rational::from_ratio_i64(1, i64::from(n)).expect("n ≥ 1"));
        Number::arith(Op::Pow, self.clone(), exponent).map_err(refused)
    }
    fn exp(&self) -> Result<Self, ExpansionError> {
        tower::real_fn(RealFn::Exp, self).map_err(refused)
    }
    fn ln(&self) -> Result<Self, ExpansionError> {
        tower::real_fn(RealFn::Ln, self).map_err(refused)
    }
}
