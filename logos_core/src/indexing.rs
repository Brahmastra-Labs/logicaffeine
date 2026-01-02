//! Phase 57: Polymorphic Indexing
//!
//! Provides trait-based indexing that handles:
//! - Vec<T> with i64 (1-based, converted to 0-based)
//! - HashMap<K, V> with K (pass-through)

use std::collections::HashMap;
use std::hash::Hash;

/// Get element by index (immutable).
pub trait LogosIndex<I> {
    type Output;
    fn logos_get(&self, index: I) -> Self::Output;
}

/// Set element by index (mutable).
pub trait LogosIndexMut<I>: LogosIndex<I> {
    fn logos_set(&mut self, index: I, value: Self::Output);
}

// === Vec<T> with i64 (1-based indexing) ===

impl<T: Clone> LogosIndex<i64> for Vec<T> {
    type Output = T;

    fn logos_get(&self, index: i64) -> T {
        if index < 1 {
            panic!("Index {} is invalid: LOGOS uses 1-based indexing (minimum is 1)", index);
        }
        let idx = (index - 1) as usize;
        if idx >= self.len() {
            panic!("Index {} is out of bounds for seq of length {}", index, self.len());
        }
        self[idx].clone()
    }
}

impl<T: Clone> LogosIndexMut<i64> for Vec<T> {
    fn logos_set(&mut self, index: i64, value: T) {
        if index < 1 {
            panic!("Index {} is invalid: LOGOS uses 1-based indexing (minimum is 1)", index);
        }
        let idx = (index - 1) as usize;
        if idx >= self.len() {
            panic!("Index {} is out of bounds for seq of length {}", index, self.len());
        }
        self[idx] = value;
    }
}

// === HashMap<K, V> with K (key-based indexing) ===

impl<K: Eq + Hash, V: Clone> LogosIndex<K> for HashMap<K, V> {
    type Output = V;

    fn logos_get(&self, key: K) -> V {
        self.get(&key).cloned().expect("Key not found in map")
    }
}

impl<K: Eq + Hash, V: Clone> LogosIndexMut<K> for HashMap<K, V> {
    fn logos_set(&mut self, key: K, value: V) {
        self.insert(key, value);
    }
}

// === &str convenience for HashMap<String, V> ===

impl<V: Clone> LogosIndex<&str> for HashMap<String, V> {
    type Output = V;

    fn logos_get(&self, key: &str) -> V {
        self.get(key).cloned().expect("Key not found in map")
    }
}

impl<V: Clone> LogosIndexMut<&str> for HashMap<String, V> {
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
        let mut m: HashMap<String, i64> = HashMap::new();
        m.insert("iron".to_string(), 42);
        assert_eq!(LogosIndex::logos_get(&m, "iron".to_string()), 42);
    }

    #[test]
    fn hashmap_str_key() {
        let mut m: HashMap<String, i64> = HashMap::new();
        m.insert("iron".to_string(), 42);
        assert_eq!(LogosIndex::logos_get(&m, "iron"), 42);
    }

    #[test]
    fn hashmap_set_key() {
        let mut m: HashMap<String, i64> = HashMap::new();
        LogosIndexMut::logos_set(&mut m, "iron", 42i64);
        assert_eq!(m.get("iron"), Some(&42));
    }
}
