//! Core runtime type definitions.
//!
//! This module defines the primitive types used by LOGOS programs at runtime.
//! These are type aliases that map LOGOS types to their Rust equivalents.
//!
//! ## Type Mappings
//!
//! | LOGOS Type | Rust Type | Description |
//! |------------|-----------|-------------|
//! | `Nat` | `u64` | Natural numbers (non-negative) |
//! | `Int` | `i64` | Signed integers |
//! | `Real` | `f64` | Floating-point numbers |
//! | `Text` | `String` | UTF-8 strings |
//! | `Bool` | `bool` | Boolean values |
//! | `Unit` | `()` | The unit type |
//! | `Char` | `char` | Unicode scalar values |
//! | `Byte` | `u8` | Raw bytes |
//! | `Seq<T>` | `LogosSeq<T>` | Ordered sequences (reference semantics) |
//! | `Set<T>` | `HashSet<T>` | Unordered unique elements |
//! | `Map<K,V>` | `LogosMap<K,V>` | Key-value mappings (reference semantics) |

use std::cell::RefCell;
use std::hash::Hash;
use std::rc::Rc;

/// Non-negative integers. Maps to Peano `Nat` in the kernel.
pub type Nat = u64;
/// Signed integers.
pub type Int = i64;
/// IEEE 754 floating-point numbers.
pub type Real = f64;
/// UTF-8 encoded text strings.
pub type Text = String;
/// Boolean truth values.
pub type Bool = bool;
/// The unit type (single value).
pub type Unit = ();
/// Unicode scalar values.
pub type Char = char;
/// Raw bytes (0-255).
pub type Byte = u8;

/// Ordered sequence with reference semantics.
///
/// `LogosSeq<T>` wraps `Rc<RefCell<Vec<T>>>` to provide shared mutable access.
/// Cloning a `LogosSeq` produces a shallow copy (shared reference), not a deep copy.
/// Use `.deep_clone()` for an independent copy (LOGOS `copy of`).
#[derive(Debug)]
pub struct LogosSeq<T>(pub Rc<RefCell<Vec<T>>>);

impl<T> LogosSeq<T> {
    pub fn new() -> Self {
        Self(Rc::new(RefCell::new(Vec::new())))
    }

    pub fn from_vec(v: Vec<T>) -> Self {
        Self(Rc::new(RefCell::new(v)))
    }

    pub fn with_capacity(cap: usize) -> Self {
        Self(Rc::new(RefCell::new(Vec::with_capacity(cap))))
    }

    pub fn push(&self, value: T) {
        self.0.borrow_mut().push(value);
    }

    pub fn pop(&self) -> Option<T> {
        self.0.borrow_mut().pop()
    }

    pub fn len(&self) -> usize {
        self.0.borrow().len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.borrow().is_empty()
    }

    pub fn remove(&self, index: usize) -> T {
        self.0.borrow_mut().remove(index)
    }

    pub fn borrow(&self) -> std::cell::Ref<'_, Vec<T>> {
        self.0.borrow()
    }

    pub fn borrow_mut(&self) -> std::cell::RefMut<'_, Vec<T>> {
        self.0.borrow_mut()
    }
}

impl<T: Clone> LogosSeq<T> {
    pub fn deep_clone(&self) -> Self {
        Self(Rc::new(RefCell::new(self.0.borrow().clone())))
    }

    pub fn to_vec(&self) -> Vec<T> {
        self.0.borrow().clone()
    }

    pub fn extend_from_slice(&self, other: &[T]) {
        self.0.borrow_mut().extend_from_slice(other);
    }

    pub fn iter(&self) -> LogosSeqIter<T> {
        LogosSeqIter {
            data: self.to_vec(),
            pos: 0,
        }
    }
}

pub struct LogosSeqIter<T> {
    data: Vec<T>,
    pos: usize,
}

impl<T: Clone> Iterator for LogosSeqIter<T> {
    type Item = T;
    fn next(&mut self) -> Option<T> {
        if self.pos < self.data.len() {
            let val = self.data[self.pos].clone();
            self.pos += 1;
            Some(val)
        } else {
            None
        }
    }
}

impl<T: Ord> LogosSeq<T> {
    pub fn sort(&self) {
        self.0.borrow_mut().sort();
    }
}

impl<T> LogosSeq<T> {
    pub fn reverse(&self) {
        self.0.borrow_mut().reverse();
    }
}

impl<T> Clone for LogosSeq<T> {
    fn clone(&self) -> Self {
        Self(Rc::clone(&self.0))
    }
}

impl<T> Default for LogosSeq<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: PartialEq> PartialEq for LogosSeq<T> {
    fn eq(&self, other: &Self) -> bool {
        *self.0.borrow() == *other.0.borrow()
    }
}

impl<T: std::fmt::Display> std::fmt::Display for LogosSeq<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let inner = self.0.borrow();
        write!(f, "[")?;
        for (i, item) in inner.iter().enumerate() {
            if i > 0 { write!(f, ", ")?; }
            write!(f, "{}", item)?;
        }
        write!(f, "]")
    }
}

impl<T> From<Vec<T>> for LogosSeq<T> {
    fn from(v: Vec<T>) -> Self {
        Self::from_vec(v)
    }
}

impl<T: PartialEq> LogosContains<T> for LogosSeq<T> {
    #[inline(always)]
    fn logos_contains(&self, value: &T) -> bool {
        self.0.borrow().contains(value)
    }
}

impl<T: Clone> IntoIterator for LogosSeq<T> {
    type Item = T;
    type IntoIter = std::vec::IntoIter<T>;

    fn into_iter(self) -> Self::IntoIter {
        self.to_vec().into_iter()
    }
}

/// Key-value mapping with reference semantics.
///
/// `LogosMap<K, V>` wraps `Rc<RefCell<FxHashMap<K, V>>>` to provide shared mutable access.
/// Cloning a `LogosMap` produces a shallow copy (shared reference), not a deep copy.
/// Use `.deep_clone()` for an independent copy (LOGOS `copy of`).
#[derive(Debug)]
pub struct LogosMap<K, V>(pub Rc<RefCell<rustc_hash::FxHashMap<K, V>>>);

impl<K: Eq + Hash, V> LogosMap<K, V> {
    pub fn new() -> Self {
        Self(Rc::new(RefCell::new(rustc_hash::FxHashMap::default())))
    }

    pub fn with_capacity(cap: usize) -> Self {
        Self(Rc::new(RefCell::new(
            rustc_hash::FxHashMap::with_capacity_and_hasher(cap, Default::default()),
        )))
    }

    pub fn from_map(m: rustc_hash::FxHashMap<K, V>) -> Self {
        Self(Rc::new(RefCell::new(m)))
    }

    pub fn insert(&self, key: K, value: V) -> Option<V> {
        self.0.borrow_mut().insert(key, value)
    }

    pub fn remove(&self, key: &K) -> Option<V> {
        self.0.borrow_mut().remove(key)
    }

    pub fn len(&self) -> usize {
        self.0.borrow().len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.borrow().is_empty()
    }

    pub fn contains_key(&self, key: &K) -> bool {
        self.0.borrow().contains_key(key)
    }

    pub fn borrow(&self) -> std::cell::Ref<'_, rustc_hash::FxHashMap<K, V>> {
        self.0.borrow()
    }

    pub fn borrow_mut(&self) -> std::cell::RefMut<'_, rustc_hash::FxHashMap<K, V>> {
        self.0.borrow_mut()
    }
}

impl<K: Eq + Hash + Clone, V: Clone> LogosMap<K, V> {
    pub fn deep_clone(&self) -> Self {
        Self(Rc::new(RefCell::new(self.0.borrow().clone())))
    }

    pub fn get(&self, key: &K) -> Option<V> {
        self.0.borrow().get(key).cloned()
    }

    pub fn values(&self) -> Vec<V> {
        self.0.borrow().values().cloned().collect()
    }

    pub fn keys(&self) -> Vec<K> {
        self.0.borrow().keys().cloned().collect()
    }
}

impl<K, V> Clone for LogosMap<K, V> {
    fn clone(&self) -> Self {
        Self(Rc::clone(&self.0))
    }
}

impl<K: Eq + Hash, V> Default for LogosMap<K, V> {
    fn default() -> Self {
        Self::new()
    }
}

impl<K: PartialEq + Eq + Hash, V: PartialEq> PartialEq for LogosMap<K, V> {
    fn eq(&self, other: &Self) -> bool {
        *self.0.borrow() == *other.0.borrow()
    }
}

impl<K: std::fmt::Display + Eq + Hash, V: std::fmt::Display> std::fmt::Display for LogosMap<K, V> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let inner = self.0.borrow();
        write!(f, "{{")?;
        for (i, (k, v)) in inner.iter().enumerate() {
            if i > 0 { write!(f, ", ")?; }
            write!(f, "{}: {}", k, v)?;
        }
        write!(f, "}}")
    }
}

impl<K: Eq + Hash, V> LogosContains<K> for LogosMap<K, V> {
    #[inline(always)]
    fn logos_contains(&self, key: &K) -> bool {
        self.0.borrow().contains_key(key)
    }
}

/// Ordered sequences with reference semantics.
pub type Seq<T> = LogosSeq<T>;

/// Key-value mappings with reference semantics.
pub type Map<K, V> = LogosMap<K, V>;

/// Unordered collections of unique elements with FxHash.
pub type Set<T> = rustc_hash::FxHashSet<T>;

/// Unified containment testing for all collection types.
///
/// This trait provides a consistent `logos_contains` method across Logos's
/// collection types, abstracting over the different containment semantics
/// of vectors (by value), sets (by membership), maps (by key), and
/// strings (by substring or character).
///
/// # Implementations
///
/// - [`Vec<T>`]: Tests if the vector contains an element equal to the value
/// - [`HashSet<T>`]: Tests if the element is a member of the set
/// - [`HashMap<K, V>`]: Tests if a key exists in the map
/// - [`String`]: Tests for substring (`&str`) or character (`char`) presence
/// - [`ORSet<T, B>`]: Tests if the element is in the CRDT set
///
/// # Examples
///
/// ```
/// use logicaffeine_data::LogosContains;
///
/// // Vector: contains by value equality
/// let v = vec![1, 2, 3];
/// assert!(v.logos_contains(&2));
/// assert!(!v.logos_contains(&5));
///
/// // String: contains by substring
/// let s = String::from("hello world");
/// assert!(s.logos_contains(&"world"));
///
/// // String: contains by character
/// assert!(s.logos_contains(&'o'));
/// ```
pub trait LogosContains<T> {
    /// Check if this collection contains the given value.
    fn logos_contains(&self, value: &T) -> bool;
}

impl<T: PartialEq> LogosContains<T> for Vec<T> {
    #[inline(always)]
    fn logos_contains(&self, value: &T) -> bool {
        self.contains(value)
    }
}

impl<T: PartialEq> LogosContains<T> for [T] {
    #[inline(always)]
    fn logos_contains(&self, value: &T) -> bool {
        self.contains(value)
    }
}

impl<T: Eq + Hash> LogosContains<T> for rustc_hash::FxHashSet<T> {
    #[inline(always)]
    fn logos_contains(&self, value: &T) -> bool {
        self.contains(value)
    }
}

impl<K: Eq + Hash, V> LogosContains<K> for rustc_hash::FxHashMap<K, V> {
    #[inline(always)]
    fn logos_contains(&self, key: &K) -> bool {
        self.contains_key(key)
    }
}

impl LogosContains<&str> for String {
    #[inline(always)]
    fn logos_contains(&self, value: &&str) -> bool {
        self.contains(*value)
    }
}

impl LogosContains<String> for String {
    #[inline(always)]
    fn logos_contains(&self, value: &String) -> bool {
        self.contains(value.as_str())
    }
}

impl LogosContains<char> for String {
    #[inline(always)]
    fn logos_contains(&self, value: &char) -> bool {
        self.contains(*value)
    }
}

impl<T: Eq + Hash + Clone, B: crate::crdt::SetBias> LogosContains<T>
    for crate::crdt::ORSet<T, B>
{
    #[inline(always)]
    fn logos_contains(&self, value: &T) -> bool {
        self.contains(value)
    }
}

/// Dynamic value type for heterogeneous collections.
///
/// `Value` enables tuples and other heterogeneous data structures in Logos.
/// It supports basic arithmetic between compatible types and provides
/// runtime type coercion where sensible.
///
/// # Variants
///
/// - `Int(i64)` - Integer values
/// - `Float(f64)` - Floating-point values
/// - `Bool(bool)` - Boolean values
/// - `Text(String)` - String values
/// - `Char(char)` - Single character values
/// - `Nothing` - Unit/null value
///
/// # Arithmetic
///
/// Arithmetic operations are supported between numeric types:
/// - `Int op Int` → `Int`
/// - `Float op Float` → `Float`
/// - `Int op Float` or `Float op Int` → `Float` (promotion)
/// - `Text + Text` → `Text` (concatenation)
///
/// # Panics
///
/// Arithmetic on incompatible variants panics at runtime.
///
/// # Examples
///
/// ```
/// use logicaffeine_data::Value;
///
/// let a = Value::Int(10);
/// let b = Value::Int(3);
/// assert_eq!(a + b, Value::Int(13));
///
/// let x = Value::Float(2.5);
/// let y = Value::Int(2);
/// assert_eq!(x * y, Value::Float(5.0));
/// ```
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    /// Integer values.
    Int(i64),
    /// Floating-point values.
    Float(f64),
    /// Boolean values.
    Bool(bool),
    /// String values.
    Text(String),
    /// Single character values.
    Char(char),
    /// Unit/null value.
    Nothing,
}

impl std::fmt::Display for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Value::Int(n) => write!(f, "{}", n),
            Value::Float(n) => write!(f, "{}", n),
            Value::Bool(b) => write!(f, "{}", b),
            Value::Text(s) => write!(f, "{}", s),
            Value::Char(c) => write!(f, "{}", c),
            Value::Nothing => write!(f, "nothing"),
        }
    }
}

// Conversion traits for Value
impl From<i64> for Value {
    fn from(n: i64) -> Self { Value::Int(n) }
}

impl From<f64> for Value {
    fn from(n: f64) -> Self { Value::Float(n) }
}

impl From<bool> for Value {
    fn from(b: bool) -> Self { Value::Bool(b) }
}

impl From<String> for Value {
    fn from(s: String) -> Self { Value::Text(s) }
}

impl From<&str> for Value {
    fn from(s: &str) -> Self { Value::Text(s.to_string()) }
}

impl From<char> for Value {
    fn from(c: char) -> Self { Value::Char(c) }
}

/// Tuple type: Vec of heterogeneous Values (uses LogosIndex from indexing module)
pub type Tuple = Vec<Value>;

// NOTE: Showable impl for Value is in logicaffeine_system (io module)
// This crate (logicaffeine_data) has NO IO dependencies.

// Arithmetic operations for Value
impl std::ops::Add for Value {
    type Output = Value;

    #[inline]
    fn add(self, other: Value) -> Value {
        match (self, other) {
            (Value::Int(a), Value::Int(b)) => Value::Int(a + b),
            (Value::Float(a), Value::Float(b)) => Value::Float(a + b),
            (Value::Int(a), Value::Float(b)) => Value::Float(a as f64 + b),
            (Value::Float(a), Value::Int(b)) => Value::Float(a + b as f64),
            (Value::Text(a), Value::Text(b)) => Value::Text(format!("{}{}", a, b)),
            _ => panic!("Cannot add these value types"),
        }
    }
}

impl std::ops::Sub for Value {
    type Output = Value;

    #[inline]
    fn sub(self, other: Value) -> Value {
        match (self, other) {
            (Value::Int(a), Value::Int(b)) => Value::Int(a - b),
            (Value::Float(a), Value::Float(b)) => Value::Float(a - b),
            (Value::Int(a), Value::Float(b)) => Value::Float(a as f64 - b),
            (Value::Float(a), Value::Int(b)) => Value::Float(a - b as f64),
            _ => panic!("Cannot subtract these value types"),
        }
    }
}

impl std::ops::Mul for Value {
    type Output = Value;

    #[inline]
    fn mul(self, other: Value) -> Value {
        match (self, other) {
            (Value::Int(a), Value::Int(b)) => Value::Int(a * b),
            (Value::Float(a), Value::Float(b)) => Value::Float(a * b),
            (Value::Int(a), Value::Float(b)) => Value::Float(a as f64 * b),
            (Value::Float(a), Value::Int(b)) => Value::Float(a * b as f64),
            _ => panic!("Cannot multiply these value types"),
        }
    }
}

impl std::ops::Div for Value {
    type Output = Value;

    #[inline]
    fn div(self, other: Value) -> Value {
        match (self, other) {
            (Value::Int(a), Value::Int(b)) => Value::Int(a / b),
            (Value::Float(a), Value::Float(b)) => Value::Float(a / b),
            (Value::Int(a), Value::Float(b)) => Value::Float(a as f64 / b),
            (Value::Float(a), Value::Int(b)) => Value::Float(a / b as f64),
            _ => panic!("Cannot divide these value types"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn value_int_arithmetic() {
        assert_eq!(Value::Int(10) + Value::Int(3), Value::Int(13));
        assert_eq!(Value::Int(10) - Value::Int(3), Value::Int(7));
        assert_eq!(Value::Int(10) * Value::Int(3), Value::Int(30));
        assert_eq!(Value::Int(10) / Value::Int(3), Value::Int(3));
    }

    #[test]
    fn value_float_arithmetic() {
        assert_eq!(Value::Float(2.5) + Value::Float(1.5), Value::Float(4.0));
        assert_eq!(Value::Float(5.0) - Value::Float(1.5), Value::Float(3.5));
        assert_eq!(Value::Float(2.0) * Value::Float(3.0), Value::Float(6.0));
        assert_eq!(Value::Float(7.0) / Value::Float(2.0), Value::Float(3.5));
    }

    #[test]
    fn value_cross_type_promotion() {
        assert_eq!(Value::Int(2) + Value::Float(1.5), Value::Float(3.5));
        assert_eq!(Value::Float(2.5) + Value::Int(2), Value::Float(4.5));
        assert_eq!(Value::Int(3) * Value::Float(2.0), Value::Float(6.0));
        assert_eq!(Value::Float(6.0) / Value::Int(2), Value::Float(3.0));
    }

    #[test]
    fn value_text_concat() {
        assert_eq!(
            Value::Text("hello".to_string()) + Value::Text(" world".to_string()),
            Value::Text("hello world".to_string())
        );
    }

    #[test]
    #[should_panic(expected = "divide by zero")]
    fn value_div_by_zero_panics() {
        let _ = Value::Int(1) / Value::Int(0);
    }

    #[test]
    #[should_panic(expected = "Cannot add")]
    fn value_incompatible_types_panic() {
        let _ = Value::Bool(true) + Value::Int(1);
    }

    #[test]
    fn value_display() {
        assert_eq!(format!("{}", Value::Int(42)), "42");
        assert_eq!(format!("{}", Value::Float(3.14)), "3.14");
        assert_eq!(format!("{}", Value::Bool(true)), "true");
        assert_eq!(format!("{}", Value::Text("hi".to_string())), "hi");
        assert_eq!(format!("{}", Value::Char('a')), "a");
        assert_eq!(format!("{}", Value::Nothing), "nothing");
    }

    #[test]
    fn value_from_conversions() {
        assert_eq!(Value::from(42i64), Value::Int(42));
        assert_eq!(Value::from(3.14f64), Value::Float(3.14));
        assert_eq!(Value::from(true), Value::Bool(true));
        assert_eq!(Value::from("hello"), Value::Text("hello".to_string()));
        assert_eq!(Value::from("hello".to_string()), Value::Text("hello".to_string()));
        assert_eq!(Value::from('x'), Value::Char('x'));
    }
}
