//! Polymorphic indexing traits for Logos collections.
//!
//! Logos uses **1-based indexing** to match natural language conventions.
//! These traits provide get/set operations that automatically convert
//! 1-based indices to 0-based for underlying Rust collections.
//!
//! # Supported Collections
//!
//! - [`Vec<T>`]: Indexed by `i64` (1-based, converted to 0-based internally)
//! - [`HashMap<K, V>`]: Indexed by key `K` (pass-through semantics)
//! - [`HashMap<String, V>`]: Also supports `&str` keys for convenience
//!
//! # Panics
//!
//! Vector indexing operations panic if the index is out of bounds
//! (less than 1 or greater than collection length). Map operations
//! panic if the key is not found.

use rustc_hash::FxHashMap;
use std::hash::Hash;

/// Immutable element access by index.
///
/// Provides 1-based indexing for Logos collections. Index `1` refers
/// to the first element, index `2` to the second, and so on.
///
/// # Examples
///
/// ```
/// use logicaffeine_data::LogosIndex;
///
/// let v = vec!["a", "b", "c"];
/// assert_eq!(v.logos_get(1i64), "a");  // 1-based!
/// assert_eq!(v.logos_get(3i64), "c");
/// ```
///
/// # Panics
///
/// Panics if the index is less than 1 or greater than the collection length.
pub trait LogosIndex<I> {
    /// The type of element returned by indexing.
    type Output;
    /// Get the element at the given index.
    fn logos_get(&self, index: I) -> Self::Output;
}

/// Mutable element access by index.
///
/// Provides 1-based mutable indexing for Logos collections.
///
/// # Examples
///
/// ```
/// use logicaffeine_data::LogosIndexMut;
///
/// let mut v = vec![1, 2, 3];
/// v.logos_set(2i64, 20);
/// assert_eq!(v, vec![1, 20, 3]);
/// ```
///
/// # Panics
///
/// Panics if the index is less than 1 or greater than the collection length.
pub trait LogosIndexMut<I>: LogosIndex<I> {
    /// Set the element at the given index.
    fn logos_set(&mut self, index: I, value: Self::Output);
}

// === Vec<T> with i64 (1-based indexing) ===

impl<T: Clone> LogosIndex<i64> for Vec<T> {
    type Output = T;

    #[inline(always)]
    fn logos_get(&self, index: i64) -> T {
        if index < 1 {
            panic!("Index {} is invalid: LOGOS uses 1-based indexing (minimum is 1)", index);
        }
        let idx = (index - 1) as usize;
        if idx >= self.len() {
            panic!("Index {} is out of bounds for seq of length {}", index, self.len());
        }
        unsafe { self.get_unchecked(idx).clone() }
    }
}

impl<T: Clone> LogosIndexMut<i64> for Vec<T> {
    #[inline(always)]
    fn logos_set(&mut self, index: i64, value: T) {
        if index < 1 {
            panic!("Index {} is invalid: LOGOS uses 1-based indexing (minimum is 1)", index);
        }
        let idx = (index - 1) as usize;
        if idx >= self.len() {
            panic!("Index {} is out of bounds for seq of length {}", index, self.len());
        }
        unsafe { *self.get_unchecked_mut(idx) = value; }
    }
}

// === [T] slice with i64 (1-based indexing, used by &mut [T] borrow params) ===

impl<T: Clone> LogosIndex<i64> for [T] {
    type Output = T;

    #[inline(always)]
    fn logos_get(&self, index: i64) -> T {
        if index < 1 {
            panic!("Index {} is invalid: LOGOS uses 1-based indexing (minimum is 1)", index);
        }
        let idx = (index - 1) as usize;
        if idx >= self.len() {
            panic!("Index {} is out of bounds for seq of length {}", index, self.len());
        }
        unsafe { self.get_unchecked(idx).clone() }
    }
}

impl<T: Clone> LogosIndexMut<i64> for [T] {
    #[inline(always)]
    fn logos_set(&mut self, index: i64, value: T) {
        if index < 1 {
            panic!("Index {} is invalid: LOGOS uses 1-based indexing (minimum is 1)", index);
        }
        let idx = (index - 1) as usize;
        if idx >= self.len() {
            panic!("Index {} is out of bounds for seq of length {}", index, self.len());
        }
        unsafe { *self.get_unchecked_mut(idx) = value; }
    }
}

// === &mut [T] with i64 (thin wrapper for UFCS compatibility) ===
//
// When the codegen emits `LogosIndex::logos_get(&arr, i)` where `arr: &mut [T]`,
// the first argument is `&&mut [T]`. Rust doesn't auto-coerce this to `&[T]`
// in UFCS, so we need an explicit impl that delegates to the `[T]` impl.

impl<T: Clone> LogosIndex<i64> for &mut [T] {
    type Output = T;

    #[inline(always)]
    fn logos_get(&self, index: i64) -> T {
        <[T] as LogosIndex<i64>>::logos_get(self, index)
    }
}

impl<T: Clone> LogosIndexMut<i64> for &mut [T] {
    #[inline(always)]
    fn logos_set(&mut self, index: i64, value: T) {
        <[T] as LogosIndexMut<i64>>::logos_set(self, index, value)
    }
}

// === String with i64 (1-based character indexing) ===

impl LogosIndex<i64> for String {
    type Output = String;

    #[inline(always)]
    fn logos_get(&self, index: i64) -> String {
        if index < 1 {
            panic!("Index {} is invalid: LOGOS uses 1-based indexing (minimum is 1)", index);
        }
        let idx = (index - 1) as usize;
        match self.as_bytes().get(idx) {
            Some(&b) if b.is_ascii() => {
                // Fast path: ASCII byte
                String::from(b as char)
            }
            _ => {
                // Slow path: Unicode or out of bounds
                self.chars().nth(idx)
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| panic!("Index {} is out of bounds for text of length {}", index, self.chars().count()))
            }
        }
    }
}

// === String with i64 (1-based character indexing, char return) ===

/// Zero-allocation character access for string comparisons.
///
/// Unlike [`LogosIndex`] for `String` which returns a `String`,
/// this trait returns a `char` — avoiding heap allocation entirely.
/// Used by the codegen optimizer for string-index-vs-string-index comparisons.
pub trait LogosGetChar {
    fn logos_get_char(&self, index: i64) -> char;
}

impl LogosGetChar for String {
    #[inline(always)]
    fn logos_get_char(&self, index: i64) -> char {
        if index < 1 {
            panic!("Index {} is invalid: LOGOS uses 1-based indexing (minimum is 1)", index);
        }
        let idx = (index - 1) as usize;
        match self.as_bytes().get(idx) {
            Some(&b) if b.is_ascii() => b as char,
            _ => {
                self.chars().nth(idx)
                    .unwrap_or_else(|| panic!(
                        "Index {} is out of bounds for text of length {}",
                        index, self.chars().count()
                    ))
            }
        }
    }
}

// === HashMap<K, V> with K (key-based indexing) ===

impl<K: Eq + Hash, V: Clone> LogosIndex<K> for FxHashMap<K, V> {
    type Output = V;

    #[inline(always)]
    fn logos_get(&self, key: K) -> V {
        self.get(&key).cloned().expect("Key not found in map")
    }
}

impl<K: Eq + Hash, V: Clone> LogosIndexMut<K> for FxHashMap<K, V> {
    #[inline(always)]
    fn logos_set(&mut self, key: K, value: V) {
        self.insert(key, value);
    }
}

// === &str convenience for HashMap<String, V> ===

impl<V: Clone> LogosIndex<&str> for FxHashMap<String, V> {
    type Output = V;

    #[inline(always)]
    fn logos_get(&self, key: &str) -> V {
        self.get(key).cloned().expect("Key not found in map")
    }
}

impl<V: Clone> LogosIndexMut<&str> for FxHashMap<String, V> {
    #[inline(always)]
    fn logos_set(&mut self, key: &str, value: V) {
        self.insert(key.to_string(), value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vec_1_based_indexing() {
        let v = vec![10, 20, 30];
        assert_eq!(LogosIndex::logos_get(&v, 1i64), 10);
        assert_eq!(LogosIndex::logos_get(&v, 2i64), 20);
        assert_eq!(LogosIndex::logos_get(&v, 3i64), 30);
    }

    #[test]
    #[should_panic(expected = "1-based indexing")]
    fn vec_zero_index_panics() {
        let v = vec![10, 20, 30];
        let _ = LogosIndex::logos_get(&v, 0i64);
    }

    #[test]
    fn vec_set_1_based() {
        let mut v = vec![10, 20, 30];
        LogosIndexMut::logos_set(&mut v, 2i64, 99);
        assert_eq!(v, vec![10, 99, 30]);
    }

    #[test]
    fn hashmap_string_key() {
        let mut m: FxHashMap<String, i64> = FxHashMap::default();
        m.insert("iron".to_string(), 42);
        assert_eq!(LogosIndex::logos_get(&m, "iron".to_string()), 42);
    }

    #[test]
    fn hashmap_str_key() {
        let mut m: FxHashMap<String, i64> = FxHashMap::default();
        m.insert("iron".to_string(), 42);
        assert_eq!(LogosIndex::logos_get(&m, "iron"), 42);
    }

    #[test]
    fn hashmap_set_key() {
        let mut m: FxHashMap<String, i64> = FxHashMap::default();
        LogosIndexMut::logos_set(&mut m, "iron", 42i64);
        assert_eq!(m.get("iron"), Some(&42));
    }
}
