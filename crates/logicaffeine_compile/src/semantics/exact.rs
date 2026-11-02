//! The exact real types in the interpreter's value model: real algebraic numbers, computable reals,
//! the program's infinitesimal and the hyperreals it generates, infinite integers, ordinals, and
//! the values a supertask leaves.
//!
//! How they mix with the numeric tower and with each other is decided once, in
//! [`logicaffeine_base::tower`], generic over a value model; this module is where `RuntimeValue`
//! joins it, so the tree-walker and the VM compute exactly what a compiled program computes.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use logicaffeine_base::algebraic::Algebraic;
use logicaffeine_base::exact_real::ExactReal;
use logicaffeine_base::germ::Germ;
use logicaffeine_base::hypernatural::Hypernatural;
use logicaffeine_base::hyperreal::Hyperreal;
use logicaffeine_base::limit_value::{Converging, Depends, LimitRule};
use logicaffeine_base::numeric::{rational_from_f64_exact, BigInt, Rational};
use logicaffeine_base::ordinal::Ordinal;
use logicaffeine_base::tower::{self, ExactRef, Shape, TowerValue, ValueAlgebra};

pub use logicaffeine_base::tower::{Branch, HyperForm, HyperValue, NaturalValue, Op, Rel, Rounding, Scale};

use crate::ast::stmt::BinaryOpKind;
use crate::interpreter::{ListRepr, RuntimeValue};
use crate::semantics::builtins::{call_builtin, BuiltinId};

/// A runtime value that depends on the residue of `⌈1/dt⌉`.
pub type DependsValue = tower::DependsValue<RuntimeValue>;

/// The canonical runtime value of an algebraic number: a rational is an `Int`/`BigInt`/`Rational`.
pub fn from_algebraic(a: Algebraic) -> RuntimeValue {
    tower::from_algebraic(a)
}

/// The canonical runtime value of a computable real: an algebraic one is an algebraic number.
pub fn from_exact_real(x: ExactReal) -> RuntimeValue {
    tower::from_exact_real(x)
}

/// The canonical runtime value of a hyperreal.
pub fn from_hyper(value: Hyperreal, scale: &Arc<Scale>) -> RuntimeValue {
    tower::from_hyper(value, scale)
}

/// The canonical runtime value of a germ.
pub fn from_germ(g: Germ, scale: &Arc<Scale>) -> RuntimeValue {
    tower::from_germ(g, scale)
}

/// The canonical runtime value of an infinite integer.
pub fn from_natural(value: Hypernatural, scale: &Arc<Scale>) -> RuntimeValue {
    tower::from_natural(value, scale)
}

/// The canonical runtime value of a residue-dependent value.
pub fn from_depends(value: Depends<Branch<RuntimeValue>>, scale: &Arc<Scale>) -> RuntimeValue {
    tower::from_depends(value, scale)
}

/// A finite ordinal is a natural number.
pub fn from_ordinal(o: Ordinal) -> RuntimeValue {
    tower::from_ordinal(o)
}

/// True for a value of one of the exact types.
pub fn is_exact_value(v: &RuntimeValue) -> bool {
    tower::is_exact_value(v)
}

/// Apply `op` when either operand is an exact type; `None` when neither is.
pub fn arith(
    op: Op,
    left: &RuntimeValue,
    right: &RuntimeValue,
    recurse: &dyn Fn(Op, RuntimeValue, RuntimeValue) -> Result<RuntimeValue, String>,
) -> Option<Result<RuntimeValue, String>> {
    tower::arith(op, left, right, recurse)
}

/// A number as a germ, or `None`.
pub fn germ_of(v: &RuntimeValue) -> Option<Germ> {
    tower::germ_of(v)
}

/// Apply `rel` when either operand is an exact type; `None` when neither is.
pub fn relation(
    rel: Rel,
    left: &RuntimeValue,
    right: &RuntimeValue,
    recurse: &dyn Fn(Rel, RuntimeValue, RuntimeValue) -> Result<RuntimeValue, String>,
) -> Option<Result<RuntimeValue, String>> {
    tower::relation(rel, left, right, recurse)
}

/// Structural equality for collections and hashing; `None` when neither value is exact.
pub fn structurally_equal(left: &RuntimeValue, right: &RuntimeValue) -> Option<bool> {
    tower::structurally_equal(left, right)
}

/// How an exact value prints.
pub fn display(v: &RuntimeValue) -> Option<String> {
    tower::display(v)
}

/// The standard part of a limited value.
pub fn standard_part(v: &RuntimeValue) -> Result<RuntimeValue, String> {
    tower::standard_part(v)
}

/// The integer part of an exact value under `mode`; `None` when `v` is not one.
pub fn integer_part(
    v: &RuntimeValue,
    mode: Rounding,
    recurse: &dyn Fn(RuntimeValue) -> Result<RuntimeValue, String>,
) -> Option<Result<RuntimeValue, String>> {
    tower::integer_part(v, mode, recurse)
}

/// `x ≈ 0`: zero or an infinitesimal.
pub fn is_infinitesimal(v: &RuntimeValue) -> Result<RuntimeValue, String> {
    tower::is_infinitesimal(v)
}

/// Bounded by a standard number.
pub fn is_limited(v: &RuntimeValue) -> Result<RuntimeValue, String> {
    tower::is_limited(v)
}

/// The program's infinitesimal, its reciprocal's residues modulo `modulus` limited to `allowed`.
pub fn new_infinitesimal(name: &str, modulus: u64, allowed: &[u64]) -> Result<RuntimeValue, String> {
    tower::new_infinitesimal(name, modulus, allowed)
}

/// An infinitesimal declared much smaller than `larger` and much larger than `smaller`, each an
/// infinitesimal or `nothing`.
pub fn new_ordered_infinitesimal(
    name: &str,
    larger: &RuntimeValue,
    smaller: &RuntimeValue,
    modulus: u64,
    allowed: &[u64],
) -> Result<RuntimeValue, String> {
    tower::new_ordered_infinitesimal(name, larger, smaller, modulus, allowed)
}

/// The infinite natural `⌈1/dt⌉` generated by an infinitesimal value.
pub fn generator_of(v: &RuntimeValue) -> Result<RuntimeValue, String> {
    tower::generator_of(v)
}

/// A limit rule by its English name.
pub fn limit_rule(name: &str) -> Result<LimitRule, String> {
    tower::limit_rule(name)
}

/// Resolve a residue-dependent value under a limit rule.
pub fn apply_limit(v: &RuntimeValue, rule: LimitRule) -> Result<RuntimeValue, String> {
    tower::apply_limit(v, rule)
}

/// A provisional answer: the current guess after `steps` cases with `revisions` changes.
pub fn provisional(value: RuntimeValue, steps: u64, revisions: u64) -> RuntimeValue {
    tower::provisional(value, steps, revisions)
}

/// Why a value cannot decide a branch, or `None`.
pub fn undecided_branch(v: &RuntimeValue) -> Option<String> {
    tower::undecided_branch(v)
}

/// `not` lifted over residue branches and through provisional values; `None` for any other value.
pub fn not(
    v: &RuntimeValue,
    recurse: &dyn Fn(RuntimeValue) -> Result<RuntimeValue, String>,
) -> Option<Result<RuntimeValue, String>> {
    tower::not(v, recurse)
}

/// An algebraic number, computable real or ordinal as its kind and canonical bytes.
pub fn exact_bytes(v: &RuntimeValue) -> Option<(u8, Vec<u8>)> {
    tower::exact_bytes(v)
}

/// The value [`exact_bytes`] wrote; `None` for a forged or non-canonical encoding.
pub fn from_exact_bytes(kind: u8, bytes: &[u8]) -> Option<RuntimeValue> {
    tower::from_exact_bytes(kind, bytes)
}

/// The infinitesimal's name and its reciprocal's residue facts.
pub fn scale_parts(scale: &Scale) -> (String, Vec<u8>) {
    tower::scale_parts(scale)
}

/// The scale [`scale_parts`] wrote.
pub fn scale_from_parts(name: &str, residues: &[u8]) -> Option<Arc<Scale>> {
    tower::scale_from_parts(name, residues)
}

/// A hyperreal, infinite integer or germ as its kind, scale and canonical bytes.
pub fn scaled_bytes(v: &RuntimeValue) -> Option<(u8, String, Vec<u8>, Vec<u8>)> {
    tower::scaled_bytes(v)
}

/// The value [`scaled_bytes`] wrote; `None` for a forged or non-canonical encoding.
pub fn from_scaled_bytes(kind: u8, name: &str, residues: &[u8], bytes: &[u8]) -> Option<RuntimeValue> {
    tower::from_scaled_bytes(kind, name, residues, bytes)
}

/// A residue-dependent value rebuilt from its scale, modulus and per-class values.
pub fn depends_from_parts(
    name: &str,
    residues: &[u8],
    modulus: u64,
    branches: Vec<Option<RuntimeValue>>,
) -> Option<RuntimeValue> {
    tower::depends_from_parts(name, residues, modulus, branches)
}

impl TowerValue for RuntimeValue {
    fn exact(&self) -> Option<ExactRef<'_, Self>> {
        Some(match self {
            RuntimeValue::Algebraic(a) => ExactRef::Algebraic(a),
            RuntimeValue::ExactReal(x) => ExactRef::Real(x),
            RuntimeValue::Hyperreal(h) => ExactRef::Hyper(h),
            RuntimeValue::Hypernatural(n) => ExactRef::Natural(n),
            RuntimeValue::Ordinal(o) => ExactRef::Ordinal(o),
            RuntimeValue::Depends(d) => ExactRef::Depends(d),
            RuntimeValue::Converging(c) => ExactRef::Converging(c),
            _ => return None,
        })
    }

    fn standard_rational(&self) -> Option<Rational> {
        match self {
            RuntimeValue::Int(n) => Some(Rational::from_i64(*n)),
            RuntimeValue::BigInt(b) => Some(Rational::from_bigint((**b).clone())),
            RuntimeValue::Rational(r) => Some((**r).clone()),
            RuntimeValue::Decimal(d) => Some(d.to_rational()),
            RuntimeValue::Float(f) => rational_from_f64_exact(*f),
            _ => None,
        }
    }

    fn integer(&self) -> Option<BigInt> {
        match self {
            RuntimeValue::Int(n) => Some(BigInt::from_i64(*n)),
            RuntimeValue::BigInt(b) => Some((**b).clone()),
            _ => None,
        }
    }

    fn exact_rational(&self) -> Option<Rational> {
        match self {
            RuntimeValue::Int(n) => Some(Rational::from_i64(*n)),
            RuntimeValue::BigInt(b) => Some(Rational::from_bigint((**b).clone())),
            RuntimeValue::Rational(r) => Some((**r).clone()),
            _ => None,
        }
    }

    fn is_standard_number(&self) -> bool {
        matches!(
            self,
            RuntimeValue::Int(_)
                | RuntimeValue::BigInt(_)
                | RuntimeValue::Rational(_)
                | RuntimeValue::Decimal(_)
                | RuntimeValue::Float(_)
        )
    }

    fn text(&self) -> Option<&str> {
        match self {
            RuntimeValue::Text(t) => Some(t.as_str()),
            _ => None,
        }
    }

    fn type_name(&self) -> String {
        RuntimeValue::type_name(self).to_string()
    }

    fn display_string(&self) -> String {
        self.to_display_string()
    }

    fn values_equal(&self, other: &Self) -> bool {
        super::compare::values_equal(self, other)
    }

    fn is_nothing(&self) -> bool {
        matches!(self, RuntimeValue::Nothing)
    }

    fn from_rational(r: Rational) -> Self {
        RuntimeValue::from_rational(r)
    }

    fn from_bigint(b: BigInt) -> Self {
        RuntimeValue::from_bigint(b)
    }

    fn boolean(b: bool) -> Self {
        RuntimeValue::Bool(b)
    }

    fn nothing() -> Self {
        RuntimeValue::Nothing
    }

    fn algebraic(a: Algebraic) -> Self {
        RuntimeValue::Algebraic(Rc::new(a))
    }

    fn exact_real(x: ExactReal) -> Self {
        RuntimeValue::ExactReal(Rc::new(x))
    }

    fn hyper(h: HyperValue) -> Self {
        RuntimeValue::Hyperreal(Rc::new(h))
    }

    fn natural(n: NaturalValue) -> Self {
        RuntimeValue::Hypernatural(Rc::new(n))
    }

    fn ordinal(o: Ordinal) -> Self {
        RuntimeValue::Ordinal(Rc::new(o))
    }

    fn depends(d: DependsValue) -> Self {
        RuntimeValue::Depends(Rc::new(d))
    }

    fn converging(c: Converging<Self>) -> Self {
        RuntimeValue::Converging(Rc::new(c))
    }
}

impl ValueAlgebra for RuntimeValue {
    fn add(a: Self, b: Self) -> Result<Self, String> {
        super::arith::add(a, b)
    }

    fn subtract(a: Self, b: Self) -> Result<Self, String> {
        super::arith::subtract(a, b)
    }

    fn multiply(a: Self, b: Self) -> Result<Self, String> {
        super::arith::multiply(a, b)
    }

    fn exact_divide(a: Self, b: Self) -> Result<Self, String> {
        super::arith::exact_divide(a, b)
    }

    fn modulo(a: Self, b: Self) -> Result<Self, String> {
        super::arith::modulo(a, b)
    }

    fn power(a: Self, b: Self) -> Result<Self, String> {
        super::arith::power(a, b)
    }

    fn holds(rel: Rel, a: &Self, b: &Self) -> Result<bool, String> {
        let op = match rel {
            Rel::Lt => BinaryOpKind::Lt,
            Rel::Gt => BinaryOpKind::Gt,
            Rel::LtEq => BinaryOpKind::LtEq,
            Rel::GtEq => BinaryOpKind::GtEq,
            Rel::Eq => BinaryOpKind::Eq,
            Rel::NotEq => BinaryOpKind::NotEq,
        };
        super::arith::binary_op(op, a.clone(), b.clone())?.decided_truth()
    }

    fn floor(v: Self) -> Result<Self, String> {
        call_builtin(BuiltinId::Floor, vec![v])
    }

    fn ceil(v: Self) -> Result<Self, String> {
        call_builtin(BuiltinId::Ceil, vec![v])
    }

    fn list(values: Vec<Self>) -> Self {
        RuntimeValue::List(Rc::new(RefCell::new(ListRepr::from_values(values))))
    }

    fn shape(&self) -> Shape<Self> {
        match self {
            RuntimeValue::Bool(_) => Shape::Bool,
            RuntimeValue::Int(n) => Shape::Int(*n),
            RuntimeValue::Float(_) => Shape::Float,
            RuntimeValue::BigInt(b) => Shape::BigInt((**b).clone()),
            RuntimeValue::Rational(r) => Shape::Rational((**r).clone()),
            RuntimeValue::Decimal(_) => Shape::Decimal,
            RuntimeValue::Text(t) => Shape::Text(t.len()),
            RuntimeValue::List(l) => Shape::List(l.borrow().to_values()),
            RuntimeValue::Tuple(items) => Shape::Tuple(items.to_vec()),
            _ => Shape::Other,
        }
    }
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/semantics/exact/tests.rs"]
mod tests;
