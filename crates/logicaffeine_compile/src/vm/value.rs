//! The VM value type.
//!
//! `Value` is a newtype over the tree-walker's [`RuntimeValue`], and every
//! operation delegates to the shared semantics kernel (`crate::semantics`) —
//! the same functions the tree-walker calls — so the two engines cannot
//! diverge on value semantics. Keeping the wrapper here is the seam
//! VM_PLAN.md calls for: a later swap to a NaN-boxed `u64` touches only this
//! file.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::ast::stmt::BinaryOpKind;
use crate::interpreter::RuntimeValue;
use crate::semantics::{arith, collections, compare};

/// A VM value.
#[derive(Clone, Debug)]
pub struct Value(RuntimeValue);

impl Value {
    pub fn int(n: i64) -> Self { Value(RuntimeValue::Int(n)) }
    pub fn float(f: f64) -> Self { Value(RuntimeValue::Float(f)) }
    pub fn bool(b: bool) -> Self { Value(RuntimeValue::Bool(b)) }
    pub fn text(s: String) -> Self { Value(RuntimeValue::Text(Rc::new(s))) }
    pub fn nothing() -> Self { Value(RuntimeValue::Nothing) }

    pub fn from_runtime(rv: RuntimeValue) -> Self { Value(rv) }
    pub fn into_runtime(self) -> RuntimeValue { self.0 }
    pub fn as_runtime(&self) -> &RuntimeValue { &self.0 }
    pub fn as_runtime_mut(&mut self) -> &mut RuntimeValue { &mut self.0 }

    pub fn is_truthy(&self) -> bool { self.0.is_truthy() }
    pub fn to_display_string(&self) -> String { self.0.to_display_string() }
    pub fn type_name(&self) -> &str { self.0.type_name() }

    /// Value equality for the `equals`/`==` operator and for set/list
    /// membership (epsilon floats, structural inductives — the kernel's rules).
    pub fn values_equal(&self, other: &Value) -> bool {
        compare::values_equal(&self.0, &other.0)
    }

    pub fn as_int(&self) -> Option<i64> {
        match &self.0 {
            RuntimeValue::Int(n) => Some(*n),
            _ => None,
        }
    }

    // ---- Collections (shared kernel) ------------------------------------------

    pub fn list(items: Vec<Value>) -> Self {
        Value(RuntimeValue::List(Rc::new(RefCell::new(items.into_iter().map(|v| v.0).collect()))))
    }
    pub fn empty_list() -> Self {
        Value(RuntimeValue::List(Rc::new(RefCell::new(Vec::new()))))
    }
    pub fn empty_set() -> Self {
        Value(RuntimeValue::Set(Rc::new(RefCell::new(Vec::new()))))
    }
    pub fn empty_map() -> Self {
        Value(RuntimeValue::Map(Rc::new(RefCell::new(HashMap::new()))))
    }
    pub fn int_range(lo: i64, hi: i64) -> Self {
        Value(RuntimeValue::List(Rc::new(RefCell::new((lo..=hi).map(RuntimeValue::Int).collect()))))
    }

    pub fn list_push(&self, value: Value) -> Result<(), String> {
        collections::list_push(&self.0, value.0)
    }

    pub fn len(&self) -> Result<i64, String> {
        match collections::length_of(&self.0)? {
            RuntimeValue::Int(n) => Ok(n),
            _ => unreachable!("length_of always returns Int"),
        }
    }

    pub fn index_get(&self, idx: &Value) -> Result<Value, String> {
        collections::index_get(&self.0, &idx.0).map(Value)
    }

    pub fn set_add(&self, value: Value) -> Result<(), String> {
        collections::set_add(&self.0, value.0)
    }

    pub fn remove_from(&self, value: &Value) -> Result<(), String> {
        collections::remove_from(&self.0, &value.0)
    }

    pub fn index_set(&self, idx: &Value, value: Value) -> Result<(), String> {
        collections::index_set(&self.0, &idx.0, value.0)
    }

    pub fn contains(&self, value: &Value) -> Result<bool, String> {
        match collections::contains(&self.0, &value.0)? {
            RuntimeValue::Bool(b) => Ok(b),
            _ => unreachable!("contains always returns Bool"),
        }
    }

    // ---- Arithmetic (shared kernel; Int×Int inlined per the locked
    // wrapping-i64 spec — proven bit-identical by fast_path_kernel_differential) --

    #[inline]
    pub fn add(&self, rhs: &Value) -> Result<Value, String> {
        if let (RuntimeValue::Int(a), RuntimeValue::Int(b)) = (&self.0, &rhs.0) {
            return Ok(Value::int(a.wrapping_add(*b)));
        }
        arith::add(self.0.clone(), rhs.0.clone()).map(Value)
    }

    #[inline]
    pub fn sub(&self, rhs: &Value) -> Result<Value, String> {
        if let (RuntimeValue::Int(a), RuntimeValue::Int(b)) = (&self.0, &rhs.0) {
            return Ok(Value::int(a.wrapping_sub(*b)));
        }
        arith::subtract(self.0.clone(), rhs.0.clone()).map(Value)
    }

    #[inline]
    pub fn mul(&self, rhs: &Value) -> Result<Value, String> {
        if let (RuntimeValue::Int(a), RuntimeValue::Int(b)) = (&self.0, &rhs.0) {
            return Ok(Value::int(a.wrapping_mul(*b)));
        }
        arith::multiply(self.0.clone(), rhs.0.clone()).map(Value)
    }

    pub fn div(&self, rhs: &Value) -> Result<Value, String> {
        arith::divide(self.0.clone(), rhs.0.clone()).map(Value)
    }

    pub fn modulo(&self, rhs: &Value) -> Result<Value, String> {
        arith::modulo(self.0.clone(), rhs.0.clone()).map(Value)
    }

    /// Eager `and` (both operands evaluated): kernel semantics — bitwise for
    /// Int×Int, truthiness otherwise.
    pub fn and_eager(&self, rhs: &Value) -> Result<Value, String> {
        arith::binary_op(BinaryOpKind::And, self.0.clone(), rhs.0.clone()).map(Value)
    }

    /// Eager `or` (see [`Value::and_eager`]).
    pub fn or_eager(&self, rhs: &Value) -> Result<Value, String> {
        arith::binary_op(BinaryOpKind::Or, self.0.clone(), rhs.0.clone()).map(Value)
    }

    pub fn concat(&self, rhs: &Value) -> Result<Value, String> {
        arith::concat(self.0.clone(), rhs.0.clone()).map(Value)
    }

    pub fn bitxor(&self, rhs: &Value) -> Result<Value, String> {
        arith::binary_op(BinaryOpKind::BitXor, self.0.clone(), rhs.0.clone()).map(Value)
    }

    pub fn shl(&self, rhs: &Value) -> Result<Value, String> {
        arith::binary_op(BinaryOpKind::Shl, self.0.clone(), rhs.0.clone()).map(Value)
    }

    pub fn shr(&self, rhs: &Value) -> Result<Value, String> {
        arith::binary_op(BinaryOpKind::Shr, self.0.clone(), rhs.0.clone()).map(Value)
    }

    /// `not x` — logical for Bool, bitwise for Int, error otherwise.
    pub fn not_op(&self) -> Result<Value, String> {
        arith::not_value(self.0.clone()).map(Value)
    }

    pub fn is_int(&self) -> bool {
        matches!(self.0, RuntimeValue::Int(_))
    }

    // ---- Comparison (shared kernel; Int×Int inlined — same proof) ---------------

    #[inline]
    pub fn lt(&self, rhs: &Value) -> Result<Value, String> {
        if let (RuntimeValue::Int(a), RuntimeValue::Int(b)) = (&self.0, &rhs.0) {
            return Ok(Value::bool(a < b));
        }
        compare::compare(BinaryOpKind::Lt, &self.0, &rhs.0).map(Value)
    }
    #[inline]
    pub fn gt(&self, rhs: &Value) -> Result<Value, String> {
        if let (RuntimeValue::Int(a), RuntimeValue::Int(b)) = (&self.0, &rhs.0) {
            return Ok(Value::bool(a > b));
        }
        compare::compare(BinaryOpKind::Gt, &self.0, &rhs.0).map(Value)
    }
    #[inline]
    pub fn lte(&self, rhs: &Value) -> Result<Value, String> {
        if let (RuntimeValue::Int(a), RuntimeValue::Int(b)) = (&self.0, &rhs.0) {
            return Ok(Value::bool(a <= b));
        }
        compare::compare(BinaryOpKind::LtEq, &self.0, &rhs.0).map(Value)
    }
    #[inline]
    pub fn gte(&self, rhs: &Value) -> Result<Value, String> {
        if let (RuntimeValue::Int(a), RuntimeValue::Int(b)) = (&self.0, &rhs.0) {
            return Ok(Value::bool(a >= b));
        }
        compare::compare(BinaryOpKind::GtEq, &self.0, &rhs.0).map(Value)
    }
    #[inline]
    pub fn eq_op(&self, rhs: &Value) -> Value {
        if let (RuntimeValue::Int(a), RuntimeValue::Int(b)) = (&self.0, &rhs.0) {
            return Value::bool(a == b);
        }
        Value::bool(self.values_equal(rhs))
    }
    #[inline]
    pub fn neq_op(&self, rhs: &Value) -> Value {
        if let (RuntimeValue::Int(a), RuntimeValue::Int(b)) = (&self.0, &rhs.0) {
            return Value::bool(a != b);
        }
        Value::bool(!self.values_equal(rhs))
    }
}

#[cfg(test)]
mod fast_path_kernel_differential {
    //! Every Value op with an Int×Int fast path must be BIT-IDENTICAL to the
    //! kernel over the full edge grid — the fast path is a transcription of
    //! the locked wrapping-i64 spec, and this test is the proof.
    use super::*;

    const EDGES: [i64; 12] = [
        i64::MIN,
        i64::MIN + 1,
        -4611686018427387904, // i64::MIN / 2
        -1000003,
        -2,
        -1,
        0,
        1,
        2,
        1000003,
        4611686018427387903, // i64::MAX / 2
        i64::MAX,
    ];

    fn assert_same(op_name: &str, a: i64, b: i64, fast: Result<Value, String>, kernel: Result<RuntimeValue, String>) {
        match (fast, kernel) {
            (Ok(f), Ok(k)) => assert_eq!(
                f.into_runtime(),
                k,
                "{op_name}({a}, {b}) fast path diverged from kernel"
            ),
            (Err(f), Err(k)) => assert_eq!(f, k, "{op_name}({a}, {b}) error strings diverged"),
            (f, k) => panic!("{op_name}({a}, {b}): fast {f:?} vs kernel {k:?}"),
        }
    }

    #[test]
    fn int_arith_matches_kernel_over_edge_grid() {
        for &a in &EDGES {
            for &b in &EDGES {
                let (va, vb) = (Value::int(a), Value::int(b));
                assert_same("add", a, b, va.add(&vb), arith::add(RuntimeValue::Int(a), RuntimeValue::Int(b)));
                assert_same("sub", a, b, va.sub(&vb), arith::subtract(RuntimeValue::Int(a), RuntimeValue::Int(b)));
                assert_same("mul", a, b, va.mul(&vb), arith::multiply(RuntimeValue::Int(a), RuntimeValue::Int(b)));
            }
        }
    }

    #[test]
    fn int_comparisons_match_kernel_over_edge_grid() {
        for &a in &EDGES {
            for &b in &EDGES {
                let (va, vb) = (Value::int(a), Value::int(b));
                assert_same("lt", a, b, va.lt(&vb), compare::compare(BinaryOpKind::Lt, &RuntimeValue::Int(a), &RuntimeValue::Int(b)));
                assert_same("gt", a, b, va.gt(&vb), compare::compare(BinaryOpKind::Gt, &RuntimeValue::Int(a), &RuntimeValue::Int(b)));
                assert_same("lte", a, b, va.lte(&vb), compare::compare(BinaryOpKind::LtEq, &RuntimeValue::Int(a), &RuntimeValue::Int(b)));
                assert_same("gte", a, b, va.gte(&vb), compare::compare(BinaryOpKind::GtEq, &RuntimeValue::Int(a), &RuntimeValue::Int(b)));
                assert_eq!(
                    va.eq_op(&vb).is_truthy(),
                    compare::values_equal(&RuntimeValue::Int(a), &RuntimeValue::Int(b)),
                    "eq({a}, {b}) diverged"
                );
            }
        }
    }

    #[test]
    fn mixed_operands_still_route_through_the_kernel() {
        // Int×Float promotion is kernel territory — the fast path must not
        // capture it.
        let sum = Value::int(2).add(&Value::float(0.5)).unwrap();
        assert_eq!(sum.into_runtime(), RuntimeValue::Float(2.5));
        assert!(Value::int(2).lt(&Value::float(2.5)).unwrap().is_truthy());
        // Int×Text: whatever the kernel says (it concatenates), bit-for-bit.
        let fast = Value::int(2).add(&Value::text("x".into()));
        let kernel = arith::add(RuntimeValue::Int(2), RuntimeValue::Text(Rc::new("x".into())));
        assert_eq!(fast.map(Value::into_runtime), kernel);
        // Int×Bool arithmetic is a kernel ERROR — the fast path must not mask it.
        let fast = Value::int(2).mul(&Value::bool(true));
        let kernel = arith::multiply(RuntimeValue::Int(2), RuntimeValue::Bool(true));
        assert_eq!(fast.map(Value::into_runtime), kernel);
        assert!(matches!(Value::int(2).mul(&Value::bool(true)), Err(_)));
    }
}

#[cfg(test)]
mod value_comparison_tests {
    use super::*;

    #[test]
    fn float_relational_is_ieee() {
        // -0.0 == 0.0 under IEEE 754.
        assert!(!Value::float(-0.0).lt(&Value::float(0.0)).unwrap().is_truthy());
        assert!(Value::float(-0.0).lte(&Value::float(0.0)).unwrap().is_truthy());
        // NaN is unordered: every relational comparison is false (never an error).
        let nan = Value::float(f64::NAN);
        assert!(!nan.lt(&Value::float(1.0)).unwrap().is_truthy());
        assert!(!nan.gt(&Value::float(1.0)).unwrap().is_truthy());
        assert!(!nan.lte(&nan).unwrap().is_truthy());
        assert!(!Value::float(1.0).gte(&nan).unwrap().is_truthy());
        // Ordinary comparisons still hold.
        assert!(Value::float(1.5).lt(&Value::float(2.5)).unwrap().is_truthy());
        assert!(Value::int(2).lt(&Value::float(2.5)).unwrap().is_truthy());
        assert!(Value::int(5).gte(&Value::int(5)).unwrap().is_truthy());
    }

    #[test]
    fn float_equality_matches_treewalker() {
        // NaN is never equal to itself — the bit-equality bug must be gone.
        assert!(!Value::float(f64::NAN).eq_op(&Value::float(f64::NAN)).is_truthy());
        assert!(Value::float(f64::NAN).neq_op(&Value::float(f64::NAN)).is_truthy());
        // Exact equal floats are equal.
        assert!(Value::float(1.5).eq_op(&Value::float(1.5)).is_truthy());
        // Near-equal floats compare equal (epsilon — matching the tree-walker).
        let sum = Value::float(0.1).add(&Value::float(0.2)).unwrap(); // 0.30000000000000004
        assert!(sum.eq_op(&Value::float(0.3)).is_truthy());
        // Distinct floats are not equal.
        assert!(!Value::float(1.0).eq_op(&Value::float(2.0)).is_truthy());
        // Other types unaffected.
        assert!(Value::int(5).eq_op(&Value::int(5)).is_truthy());
        assert!(Value::text("hi".to_string()).eq_op(&Value::text("hi".to_string())).is_truthy());
        assert!(!Value::int(5).eq_op(&Value::int(6)).is_truthy());
    }
}
