//! Core Type Definitions (Spec 3.2)

use std::hash::Hash;

pub type Nat = u64;
pub type Int = i64;
pub type Real = f64;
pub type Text = String;
pub type Bool = bool;
pub type Unit = ();
pub type Char = char;
pub type Byte = u8;

// Phase 30: Collections
pub type Seq<T> = Vec<T>;

// Phase 57: Map type alias
pub type Map<K, V> = std::collections::HashMap<K, V>;

// Set collection type
pub type Set<T> = std::collections::HashSet<T>;

/// Unified contains trait for all collection types
pub trait LogosContains<T> {
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
