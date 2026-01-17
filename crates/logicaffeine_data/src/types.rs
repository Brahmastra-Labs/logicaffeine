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
//! | `Seq<T>` | `Vec<T>` | Ordered sequences |
//! | `Set<T>` | `HashSet<T>` | Unordered unique elements |
//! | `Map<K,V>` | `HashMap<K,V>` | Key-value mappings |

use std::hash::Hash;

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

/// Ordered sequences (lists).
pub type Seq<T> = Vec<T>;

/// Key-value mappings with hash-based lookup.
pub type Map<K, V> = std::collections::HashMap<K, V>;

/// Unordered collections of unique elements.
pub type Set<T> = std::collections::HashSet<T>;

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
    fn logos_contains(&self, value: &T) -> bool {
        self.contains(value)
    }
}

impl<T: Eq + Hash> LogosContains<T> for std::collections::HashSet<T> {
    fn logos_contains(&self, value: &T) -> bool {
        self.contains(value)
    }
}

impl<K: Eq + Hash, V> LogosContains<K> for std::collections::HashMap<K, V> {
    fn logos_contains(&self, key: &K) -> bool {
        self.contains_key(key)
    }
}

impl LogosContains<&str> for String {
    fn logos_contains(&self, value: &&str) -> bool {
        self.contains(*value)
    }
}

impl LogosContains<char> for String {
    fn logos_contains(&self, value: &char) -> bool {
        self.contains(*value)
    }
}

impl<T: Eq + Hash + Clone, B: crate::crdt::SetBias> LogosContains<T>
    for crate::crdt::ORSet<T, B>
{
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
