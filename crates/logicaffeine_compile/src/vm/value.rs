//! The VM value type.
//!
//! `Value` is the register-file cell. It has two representations, selected at
//! compile time:
//!
//! * **default** — `Value(RuntimeValue)`, a newtype over the tree-walker's fat
//!   16-byte enum. Every operation delegates to the shared semantics kernel
//!   (`crate::semantics`) — the same functions the tree-walker calls — so the
//!   two engines cannot diverge on value semantics.
//! * **`feature = "narrow-value"`** — `Value(Narrow)`, the 8-byte NaN-boxed
//!   value from [`super::nanbox`]. Inline scalars (`Int` in ±2^47, `Float`,
//!   `Bool`, `Nothing`) live in one machine word; everything else (large ints,
//!   collections, strings, structs, temporals) boxes behind a heap tag. The
//!   register file (`Vec<Value>`) is then half the width, and every Move/clone
//!   in the dispatch loop copies 8 bytes instead of 16.
//!
//! Both representations expose the SAME public API and route non-inline operands
//! through `crate::semantics` exactly the same way, so the VM↔tree-walker
//! differential gate holds under either build. This is the seam the VM's design calls
//! for: the representation swap touches only this file (plus the `as_runtime`
//! borrow sites in `machine.rs`, which use [`RuntimeRef`] uniformly).

use std::cell::RefCell;
use std::rc::Rc;

use crate::ast::stmt::BinaryOpKind;
use crate::interpreter::RuntimeValue;
use crate::semantics::{arith, collections, compare};

/// A borrow of a [`Value`] as a `RuntimeValue`, usable identically under both
/// representations.
///
/// * default: `RuntimeRef<'a>` borrows the `Value`'s own `RuntimeValue` — zero
///   cost, exactly the old `&RuntimeValue` return.
/// * narrow: a heap `Value` borrows the boxed pointee directly; an inline scalar
///   materialises a fresh `RuntimeValue` that the `RuntimeRef` owns.
///
/// `Deref<Target = RuntimeValue>` makes `r.method()`, `&*r`, and `(*r).clone()`
/// behave the same in either build. Match scrutinees do not auto-deref, so the
/// `machine.rs` sites match on `&*value.as_runtime()`.
pub struct RuntimeRef<'a>(RuntimeRefInner<'a>);

enum RuntimeRefInner<'a> {
    Borrowed(&'a RuntimeValue),
    #[cfg(feature = "narrow-value")]
    Owned(RuntimeValue),
}

impl std::ops::Deref for RuntimeRef<'_> {
    type Target = RuntimeValue;
    #[inline]
    fn deref(&self) -> &RuntimeValue {
        match &self.0 {
            RuntimeRefInner::Borrowed(r) => r,
            #[cfg(feature = "narrow-value")]
            RuntimeRefInner::Owned(v) => v,
        }
    }
}

impl AsRef<RuntimeValue> for RuntimeRef<'_> {
    #[inline]
    fn as_ref(&self) -> &RuntimeValue {
        self
    }
}

impl std::fmt::Debug for RuntimeRef<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        (**self).fmt(f)
    }
}

// ===========================================================================
// Default representation: Value(RuntimeValue)
// ===========================================================================

#[cfg(not(feature = "narrow-value"))]
mod repr {
    use super::*;

    /// A VM value (fat 16-byte representation).
    #[derive(Clone, Debug)]
    pub struct Value(pub(super) RuntimeValue);

    impl Value {
        #[inline]
        pub fn int(n: i64) -> Self {
            Value(RuntimeValue::Int(n))
        }
        #[inline]
        pub fn float(f: f64) -> Self {
            Value(RuntimeValue::Float(f))
        }
        #[inline]
        pub fn bool(b: bool) -> Self {
            Value(RuntimeValue::Bool(b))
        }
        #[inline]
        pub fn nothing() -> Self {
            Value(RuntimeValue::Nothing)
        }
        /// Whether this is the default/absent value — a register's pristine initial state. A write
        /// that overwrites `Nothing` destroys no information (the default is known), so it costs zero
        /// Landauer bits (see `HistoryTape::landauer_bytes`).
        #[inline]
        pub fn is_nothing(&self) -> bool {
            matches!(&*self.as_runtime(), RuntimeValue::Nothing)
        }

        #[inline]
        pub fn from_runtime(rv: RuntimeValue) -> Self {
            Value(rv)
        }
        #[inline]
        pub fn into_runtime(self) -> RuntimeValue {
            self.0
        }

        /// Borrow as a `RuntimeValue`. Zero cost: the `Value` IS a
        /// `RuntimeValue`.
        #[inline]
        pub fn as_runtime(&self) -> RuntimeRef<'_> {
            RuntimeRef(RuntimeRefInner::Borrowed(&self.0))
        }

        /// Borrow the `RuntimeValue` DIRECTLY (lifetime tied to `&self`, no
        /// `RuntimeRef` temporary) when one already exists in place. Default:
        /// always `Some`. The seam for the `machine.rs` sites that destructure a
        /// heap arm and hold a borrow (an `Rc`/`RefMut`) across statements — see
        /// the narrow impl, where an inline scalar returns `None`.
        #[inline]
        pub fn as_runtime_ref(&self) -> Option<&RuntimeValue> {
            Some(&self.0)
        }

        /// Mutably borrow as a `RuntimeValue`.
        #[inline]
        pub fn as_runtime_mut(&mut self) -> &mut RuntimeValue {
            &mut self.0
        }

        #[inline]
        pub fn as_int(&self) -> Option<i64> {
            match &self.0 {
                RuntimeValue::Int(n) => Some(*n),
                _ => None,
            }
        }
        #[inline]
        pub fn as_bool(&self) -> Option<bool> {
            match &self.0 {
                RuntimeValue::Bool(b) => Some(*b),
                _ => None,
            }
        }
        #[inline]
        pub fn as_float(&self) -> Option<f64> {
            match &self.0 {
                RuntimeValue::Float(f) => Some(*f),
                _ => None,
            }
        }
        #[inline]
        pub fn is_int(&self) -> bool {
            matches!(self.0, RuntimeValue::Int(_))
        }

        // ---- Arithmetic (Int×Int inlined; EXACT — overflow promotes to BigInt) ---
        //
        // The fast path uses `checked_*` and, only on i64 overflow, falls through to
        // the shared `arith` operators (which promote to BigInt). This keeps the VM
        // byte-identical to the tree-walker at the overflow boundary — integer math
        // is exact on every tier, never silently wrapping.

        #[inline]
        pub fn add(&self, rhs: &Value) -> Result<Value, String> {
            if let (RuntimeValue::Int(a), RuntimeValue::Int(b)) = (&self.0, &rhs.0) {
                if let Some(s) = a.checked_add(*b) {
                    return Ok(Value::int(s));
                }
            }
            arith::add(self.0.clone(), rhs.0.clone()).map(Value)
        }
        #[inline]
        pub fn sub(&self, rhs: &Value) -> Result<Value, String> {
            if let (RuntimeValue::Int(a), RuntimeValue::Int(b)) = (&self.0, &rhs.0) {
                if let Some(s) = a.checked_sub(*b) {
                    return Ok(Value::int(s));
                }
            }
            arith::subtract(self.0.clone(), rhs.0.clone()).map(Value)
        }
        #[inline]
        pub fn mul(&self, rhs: &Value) -> Result<Value, String> {
            if let (RuntimeValue::Int(a), RuntimeValue::Int(b)) = (&self.0, &rhs.0) {
                if let Some(p) = a.checked_mul(*b) {
                    return Ok(Value::int(p));
                }
            }
            arith::multiply(self.0.clone(), rhs.0.clone()).map(Value)
        }

        // ---- Comparison (Int×Int inlined — same proof) -----------------------

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
        pub fn eq_op(&self, rhs: &Value) -> Result<Value, String> {
            if let (RuntimeValue::Int(a), RuntimeValue::Int(b)) = (&self.0, &rhs.0) {
                return Ok(Value::bool(a == b));
            }
            compare::equality(&self.0, &rhs.0, false).map(Value)
        }
        #[inline]
        pub fn neq_op(&self, rhs: &Value) -> Result<Value, String> {
            if let (RuntimeValue::Int(a), RuntimeValue::Int(b)) = (&self.0, &rhs.0) {
                return Ok(Value::bool(a != b));
            }
            compare::equality(&self.0, &rhs.0, true).map(Value)
        }
    }
}

// ===========================================================================
// Narrow representation: Value(Narrow)
// ===========================================================================

#[cfg(feature = "narrow-value")]
mod repr {
    use super::*;
    use crate::vm::nanbox::Narrow;

    /// A VM value (8-byte NaN-boxed representation).
    #[derive(Clone, Debug)]
    pub struct Value(pub(super) Narrow);

    impl Value {
        #[inline]
        pub fn int(n: i64) -> Self {
            Value(Narrow::int(n))
        }
        #[inline]
        pub fn float(f: f64) -> Self {
            Value(Narrow::float(f))
        }
        #[inline]
        pub fn bool(b: bool) -> Self {
            Value(Narrow::bool(b))
        }
        #[inline]
        pub fn nothing() -> Self {
            Value(Narrow::nothing())
        }
        /// Whether this is the default/absent value — a register's pristine initial state. A write
        /// that overwrites `Nothing` destroys no information (the default is known), so it costs zero
        /// Landauer bits (see `HistoryTape::landauer_bytes`).
        #[inline]
        pub fn is_nothing(&self) -> bool {
            matches!(&*self.as_runtime(), RuntimeValue::Nothing)
        }

        #[inline]
        pub fn from_runtime(rv: RuntimeValue) -> Self {
            Value(Narrow::from_runtime(rv))
        }
        #[inline]
        pub fn into_runtime(self) -> RuntimeValue {
            self.0.into_runtime()
        }

        /// Borrow as a `RuntimeValue`. A heap value borrows the boxed pointee
        /// directly (zero copy); an inline scalar materialises a fresh
        /// `RuntimeValue` the [`RuntimeRef`] owns. Both deref to `&RuntimeValue`.
        #[inline]
        pub fn as_runtime(&self) -> RuntimeRef<'_> {
            match self.0.as_heap() {
                Some(rv) => RuntimeRef(RuntimeRefInner::Borrowed(rv)),
                None => RuntimeRef(RuntimeRefInner::Owned(self.0.to_runtime())),
            }
        }

        /// Borrow the boxed `RuntimeValue` DIRECTLY (lifetime tied to `&self`)
        /// for the heap arms; `None` for an inline scalar (which is never a List,
        /// Map, Struct, or Text — the only shapes the borrow-holding `machine.rs`
        /// sites destructure, so a `None` falls into their `else`/error arm
        /// exactly as a non-matching variant would). This is the seam that keeps
        /// an `Rc`/`RefMut` borrow alive across statements without a `RuntimeRef`
        /// temporary in the way.
        #[inline]
        pub fn as_runtime_ref(&self) -> Option<&RuntimeValue> {
            self.0.as_heap()
        }

        /// Mutably borrow as a `RuntimeValue`. Promotes an inline scalar to the
        /// heap form first so a stable `&mut RuntimeValue` always exists (the VM
        /// only mutates heap arms — Struct/Text — through this; a scalar
        /// promotion is lossless and the mutation, if any, lands in the box).
        #[inline]
        pub fn as_runtime_mut(&mut self) -> &mut RuntimeValue {
            self.0.make_heap_mut()
        }

        #[inline]
        pub fn as_int(&self) -> Option<i64> {
            self.0.as_int()
        }
        #[inline]
        pub fn as_bool(&self) -> Option<bool> {
            self.0.as_bool()
        }
        #[inline]
        pub fn as_float(&self) -> Option<f64> {
            self.0.as_float()
        }
        #[inline]
        pub fn is_int(&self) -> bool {
            self.0.as_int().is_some()
        }

        // ---- Arithmetic --------------------------------------------------------
        // Inline Int×Int rides the nanbox fast path (bit-identical to the kernel,
        // proven by nanbox's arith-parity test); every other shape (floats, large
        // boxed ints, mixed operands) routes through `crate::semantics` exactly as
        // the default representation does, via a single materialised round-trip.

        #[inline]
        pub fn add(&self, rhs: &Value) -> Result<Value, String> {
            if let Some(n) = self.0.int_add(&rhs.0) {
                return Ok(Value(n));
            }
            arith::add(self.0.to_runtime(), rhs.0.to_runtime()).map(Value::from_runtime)
        }
        #[inline]
        pub fn sub(&self, rhs: &Value) -> Result<Value, String> {
            if let Some(n) = self.0.int_sub(&rhs.0) {
                return Ok(Value(n));
            }
            arith::subtract(self.0.to_runtime(), rhs.0.to_runtime()).map(Value::from_runtime)
        }
        #[inline]
        pub fn mul(&self, rhs: &Value) -> Result<Value, String> {
            if let Some(n) = self.0.int_mul(&rhs.0) {
                return Ok(Value(n));
            }
            arith::multiply(self.0.to_runtime(), rhs.0.to_runtime()).map(Value::from_runtime)
        }

        // ---- Comparison --------------------------------------------------------

        #[inline]
        pub fn lt(&self, rhs: &Value) -> Result<Value, String> {
            if let Some(b) = self.0.int_lt(&rhs.0) {
                return Ok(Value::bool(b));
            }
            compare::compare(BinaryOpKind::Lt, &self.0.to_runtime(), &rhs.0.to_runtime())
                .map(Value::from_runtime)
        }
        #[inline]
        pub fn gt(&self, rhs: &Value) -> Result<Value, String> {
            if let (Some(a), Some(b)) = (self.0.as_inline_int(), rhs.0.as_inline_int()) {
                return Ok(Value::bool(a > b));
            }
            compare::compare(BinaryOpKind::Gt, &self.0.to_runtime(), &rhs.0.to_runtime())
                .map(Value::from_runtime)
        }
        #[inline]
        pub fn lte(&self, rhs: &Value) -> Result<Value, String> {
            if let (Some(a), Some(b)) = (self.0.as_inline_int(), rhs.0.as_inline_int()) {
                return Ok(Value::bool(a <= b));
            }
            compare::compare(
                BinaryOpKind::LtEq,
                &self.0.to_runtime(),
                &rhs.0.to_runtime(),
            )
            .map(Value::from_runtime)
        }
        #[inline]
        pub fn gte(&self, rhs: &Value) -> Result<Value, String> {
            if let (Some(a), Some(b)) = (self.0.as_inline_int(), rhs.0.as_inline_int()) {
                return Ok(Value::bool(a >= b));
            }
            compare::compare(
                BinaryOpKind::GtEq,
                &self.0.to_runtime(),
                &rhs.0.to_runtime(),
            )
            .map(Value::from_runtime)
        }
        #[inline]
        pub fn eq_op(&self, rhs: &Value) -> Result<Value, String> {
            if let Some(b) = self.0.int_eq(&rhs.0) {
                return Ok(Value::bool(b));
            }
            compare::equality(&self.0.to_runtime(), &rhs.0.to_runtime(), false)
                .map(Value::from_runtime)
        }
        #[inline]
        pub fn neq_op(&self, rhs: &Value) -> Result<Value, String> {
            if let Some(b) = self.0.int_eq(&rhs.0) {
                return Ok(Value::bool(!b));
            }
            compare::equality(&self.0.to_runtime(), &rhs.0.to_runtime(), true)
                .map(Value::from_runtime)
        }
    }
}

pub use repr::Value;

// ===========================================================================
// Representation-independent API (delegates through the kernel)
// ===========================================================================

impl Value {
    #[inline]
    pub fn text(s: String) -> Self {
        Value::from_runtime(RuntimeValue::Text(Rc::new(s)))
    }

    /// The truth of a value that decides a branch — [`RuntimeValue::decided_truth`].
    #[inline]
    pub fn decided_truth(&self) -> Result<bool, String> {
        if let Some(b) = self.as_bool() {
            return Ok(b);
        }
        if let Some(n) = self.as_int() {
            return Ok(n != 0);
        }
        self.as_runtime().decided_truth()
    }

    #[inline]
    pub fn is_truthy(&self) -> bool {
        // Inline scalar fast paths avoid materialising; heap arms defer to the
        // kernel's truthiness (which only the boxed types reach).
        if let Some(b) = self.as_bool() {
            return b;
        }
        if let Some(n) = self.as_int() {
            return n != 0;
        }
        self.as_runtime().is_truthy()
    }
    #[inline]
    pub fn to_display_string(&self) -> String {
        self.as_runtime().to_display_string()
    }
    /// The value's type name. Borrows `&self` (Struct/Inductive names live in the
    /// value; the scalar names are `'static` literals) — never a temporary, so it
    /// is valid under the narrow representation where `as_runtime()` would
    /// materialise an inline scalar into a short-lived `RuntimeValue`.
    #[inline]
    pub fn type_name(&self) -> &str {
        // Inline scalars have `'static` names; the rest borrow the in-place
        // (heap, under narrow) `RuntimeValue` directly.
        if self.as_bool().is_some() {
            return "Bool";
        }
        if self.as_int().is_some() {
            return "Int";
        }
        if self.as_float().is_some() {
            return "Float";
        }
        match self.as_runtime_ref() {
            Some(rv) => rv.type_name(),
            None => "Nothing",
        }
    }

    /// Value equality for the `equals`/`==` operator and for set/list
    /// membership (epsilon floats, structural inductives — the kernel's rules).
    #[inline]
    pub fn values_equal(&self, other: &Value) -> bool {
        compare::values_equal(&self.as_runtime(), &other.as_runtime())
    }

    // ---- Collections (shared kernel) ------------------------------------------

    pub fn list(items: Vec<Value>) -> Self {
        let values: Vec<RuntimeValue> = items.into_iter().map(Value::into_runtime).collect();
        Value::from_runtime(RuntimeValue::List(Rc::new(RefCell::new(
            crate::interpreter::ListRepr::from_values(values),
        ))))
    }
    /// Wrap a raw Int vector as a list value (the native tier's fresh
    /// allocations re-box through here).
    pub fn int_list(items: Vec<i64>) -> Self {
        Value::from_runtime(RuntimeValue::List(Rc::new(RefCell::new(
            crate::interpreter::ListRepr::Ints(items),
        ))))
    }

    /// A `Seq of Word32` holding these masked words, in the representation the VM gives one: packed words once it
    /// holds any (the VM retags its empty list on the first word pushed), an empty list of Ints before that.
    pub fn word32_list(items: Vec<i64>) -> Self {
        if items.is_empty() {
            return Value::empty_list();
        }
        Value::from_runtime(RuntimeValue::List(Rc::new(RefCell::new(
            crate::interpreter::ListRepr::Words32(items),
        ))))
    }

    pub fn empty_list() -> Self {
        Value::from_runtime(RuntimeValue::List(Rc::new(RefCell::new(
            crate::interpreter::ListRepr::Ints(Vec::new()),
        ))))
    }
    /// A fresh empty Int sequence whose statically-proven maximum length is
    /// reserved once. The logical value remains the same as `empty_list`.
    pub fn empty_list_with_capacity(capacity: usize) -> Self {
        Value::from_runtime(RuntimeValue::List(Rc::new(RefCell::new(
            crate::interpreter::ListRepr::Ints(Vec::with_capacity(capacity)),
        ))))
    }
    /// A fresh empty half-width Int sequence (`ListRepr::IntsI32`), backing a
    /// narrowing-proven `new Seq of Int` under `LOGOS_NARROW_VM`.
    pub fn empty_list_i32() -> Self {
        Value::from_runtime(RuntimeValue::List(Rc::new(RefCell::new(
            crate::interpreter::ListRepr::IntsI32(Vec::new()),
        ))))
    }
    pub fn empty_set() -> Self {
        Value::from_runtime(RuntimeValue::Set(Rc::new(RefCell::new(Vec::new()))))
    }
    pub fn empty_map() -> Self {
        Value::from_runtime(RuntimeValue::Map(Rc::new(RefCell::new(
            crate::interpreter::MapStorage::default(),
        ))))
    }
    /// A fresh empty map with storage reserved for a runtime capacity hint.
    /// A hint must never make an otherwise-valid program fail, so allocation
    /// refusal degrades to the ordinary growable empty representation.
    pub fn empty_map_with_capacity(capacity: usize) -> Self {
        let mut map = crate::interpreter::MapStorage::default();
        let _ = map.try_reserve(capacity);
        Value::from_runtime(RuntimeValue::Map(Rc::new(RefCell::new(map))))
    }
    pub fn int_range(lo: i64, hi: i64) -> Self {
        Value::from_runtime(RuntimeValue::List(Rc::new(RefCell::new(
            crate::interpreter::ListRepr::Ints((lo..=hi).collect()),
        ))))
    }

    pub fn list_push(&self, value: Value) -> Result<(), String> {
        collections::list_push(&self.as_runtime(), value.into_runtime())
    }

    pub fn len(&self) -> Result<i64, String> {
        match collections::length_of(&self.as_runtime())? {
            RuntimeValue::Int(n) => Ok(n),
            _ => unreachable!("length_of always returns Int"),
        }
    }

    pub fn index_get(&self, idx: &Value) -> Result<Value, String> {
        collections::index_get(&self.as_runtime(), &idx.as_runtime()).map(Value::from_runtime)
    }

    pub fn set_add(&self, value: Value) -> Result<(), String> {
        collections::set_add(&self.as_runtime(), value.into_runtime())
    }

    pub fn remove_from(&self, value: &Value) -> Result<(), String> {
        collections::remove_from(&self.as_runtime(), &value.as_runtime())
    }

    pub fn index_set(&self, idx: &Value, value: Value) -> Result<(), String> {
        collections::index_set(&self.as_runtime(), &idx.as_runtime(), value.into_runtime())
    }

    pub fn contains(&self, value: &Value) -> Result<bool, String> {
        match collections::contains(&self.as_runtime(), &value.as_runtime())? {
            RuntimeValue::Bool(b) => Ok(b),
            _ => unreachable!("contains always returns Bool"),
        }
    }

    // ---- General arithmetic (always through the kernel; no inline fast path) ---

    pub fn div(&self, rhs: &Value) -> Result<Value, String> {
        arith::divide(self.runtime_owned(), rhs.runtime_owned()).map(Value::from_runtime)
    }

    /// EXACT division (`Op::ExactDiv`) — the type-directed sibling of [`Value::div`]:
    /// `7 / 2 → 7/2` (a Rational), never the floored `3`. Routes through the shared
    /// kernel exactly like `div`, so the VM stays bit-identical to the tree-walker.
    pub fn exact_div(&self, rhs: &Value) -> Result<Value, String> {
        arith::exact_divide(self.runtime_owned(), rhs.runtime_owned()).map(Value::from_runtime)
    }

    /// FLOOR division (`Op::FloorDiv`, `a // b`) — the quotient rounded toward negative
    /// infinity (`-7 // 2 → -4`), distinct from the truncating [`Value::div`]. Routes
    /// through the shared kernel, so the VM stays bit-identical to the tree-walker.
    pub fn floor_div(&self, rhs: &Value) -> Result<Value, String> {
        arith::floor_divide(self.runtime_owned(), rhs.runtime_owned()).map(Value::from_runtime)
    }

    pub fn modulo(&self, rhs: &Value) -> Result<Value, String> {
        arith::modulo(self.runtime_owned(), rhs.runtime_owned()).map(Value::from_runtime)
    }

    pub fn concat(&self, rhs: &Value) -> Result<Value, String> {
        arith::concat(self.runtime_owned(), rhs.runtime_owned()).map(Value::from_runtime)
    }

    /// `a followed by b` — merge two sequences into one (reuses the tree-walker semantics).
    pub fn seq_concat(&self, rhs: &Value) -> Result<Value, String> {
        arith::seq_concat(self.runtime_owned(), rhs.runtime_owned()).map(Value::from_runtime)
    }

    pub fn pow(&self, rhs: &Value) -> Result<Value, String> {
        arith::binary_op(BinaryOpKind::Pow, self.runtime_owned(), rhs.runtime_owned())
            .map(Value::from_runtime)
    }

    pub fn bitxor(&self, rhs: &Value) -> Result<Value, String> {
        arith::binary_op(
            BinaryOpKind::BitXor,
            self.runtime_owned(),
            rhs.runtime_owned(),
        )
        .map(Value::from_runtime)
    }

    pub fn bitand(&self, rhs: &Value) -> Result<Value, String> {
        arith::binary_op(
            BinaryOpKind::BitAnd,
            self.runtime_owned(),
            rhs.runtime_owned(),
        )
        .map(Value::from_runtime)
    }

    pub fn bitor(&self, rhs: &Value) -> Result<Value, String> {
        arith::binary_op(
            BinaryOpKind::BitOr,
            self.runtime_owned(),
            rhs.runtime_owned(),
        )
        .map(Value::from_runtime)
    }

    pub fn shl(&self, rhs: &Value) -> Result<Value, String> {
        arith::binary_op(BinaryOpKind::Shl, self.runtime_owned(), rhs.runtime_owned())
            .map(Value::from_runtime)
    }

    pub fn shr(&self, rhs: &Value) -> Result<Value, String> {
        arith::binary_op(BinaryOpKind::Shr, self.runtime_owned(), rhs.runtime_owned())
            .map(Value::from_runtime)
    }

    /// `not x` — logical for Bool, bitwise for Int, error otherwise.
    pub fn not_op(&self) -> Result<Value, String> {
        arith::not_value(self.runtime_owned()).map(Value::from_runtime)
    }

    /// Materialise an owned `RuntimeValue` (a clone in the default repr; a
    /// decode in the narrow repr). The kernel's `arith`/`binary_op` entry points
    /// take owned operands, so the general (non-inlined) ops funnel through here.
    #[inline]
    fn runtime_owned(&self) -> RuntimeValue {
        self.as_runtime().clone()
    }
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/vm/value/fast_path_kernel_differential.rs"]
mod fast_path_kernel_differential;

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/vm/value/value_comparison_tests.rs"]
mod value_comparison_tests;
