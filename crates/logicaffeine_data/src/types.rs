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

impl<T: serde::Serialize> serde::Serialize for LogosSeq<T> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.borrow().serialize(serializer)
    }
}

impl<'de, T: serde::Deserialize<'de>> serde::Deserialize<'de> for LogosSeq<T> {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let vec = Vec::<T>::deserialize(deserializer)?;
        Ok(Self::from_vec(vec))
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

impl<K: serde::Serialize + Eq + Hash, V: serde::Serialize> serde::Serialize for LogosMap<K, V> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.borrow().serialize(serializer)
    }
}

impl<'de, K: serde::Deserialize<'de> + Eq + Hash, V: serde::Deserialize<'de>> serde::Deserialize<'de> for LogosMap<K, V> {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let map = rustc_hash::FxHashMap::<K, V>::deserialize(deserializer)?;
        Ok(Self::from_map(map))
    }
}

impl<K: Eq + Hash, V> LogosContains<K> for LogosMap<K, V> {
    #[inline(always)]
    fn logos_contains(&self, key: &K) -> bool {
        self.0.borrow().contains_key(key)
    }
}

/// A specialized open-addressing `i64 → i64` map with VALUE semantics and no
/// `Rc<RefCell>` indirection.
///
/// The code generator emits this in place of `LogosMap<i64, i64>` for
/// non-aliased local `Map of Int to Int` variables (see `codegen::i64_map`):
/// linear probing over two flat `Vec`s with `Copy` keys and values — no
/// per-operation `RefCell` borrow, no key/value clone, `&mut self` mutation
/// that LLVM can keep in registers. This is the C open-addressing shape, and it
/// is only selected where the alias analysis proves the map never escapes or is
/// shared, so the loss of reference semantics is invisible to the program.
///
/// `0` is the empty-slot sentinel; the real key `0` is tracked separately so the
/// map is correct for the entire `i64` key space. The sentinel is `0`, not
/// `i64::MIN`, so the probe table allocates via `vec![[0, 0]; slots]` — the bit
/// pattern Rust's `IsZero` specialization lowers to `alloc_zeroed` (calloc):
/// lazily-zeroed pages, no eager memset/page-fault storm up front.
#[derive(Debug, Clone)]
pub struct LogosI64Map {
    /// AoS probe table of `[key, value]` slots; `key == EMPTY` marks empty.
    /// Interleaving key and value keeps both on the SAME cache line, so a lookup
    /// hit reads ONE line, not two (the SoA tax) — matching C's `struct Entry`.
    /// `[i64; 2]` (not a tuple) so the zero fill hits `alloc_zeroed`.
    entries: Vec<[i64; 2]>,
    mask: usize,
    len: usize,
    has_zero_key: bool,
    zero_key_val: i64,
}

impl LogosI64Map {
    const EMPTY: i64 = 0;

    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            mask: 0,
            len: 0,
            has_zero_key: false,
            zero_key_val: 0,
        }
    }

    pub fn with_capacity(cap: usize) -> Self {
        let mut m = Self::new();
        if cap > 0 {
            // Headroom for a ≤0.75 load factor, rounded up to a power of two
            // (so the probe mask is `slots - 1`), with a floor of 8 slots.
            let slots = ((cap * 4) / 3 + 1).next_power_of_two().max(8);
            // `[Self::EMPTY, 0] == [0, 0]` → `alloc_zeroed` (calloc): lazily
            // zeroed, no eager memset of the whole table.
            m.entries = vec![[Self::EMPTY, 0]; slots];
            m.mask = slots - 1;
        }
        m
    }

    /// Live entries stored in the probe table (excludes the zero key, which
    /// lives outside the table).
    #[inline]
    fn table_len(&self) -> usize {
        self.len - self.has_zero_key as usize
    }

    /// See [`LogosI64Set::assume_table_invariant`]: `entries.len() == mask + 1`
    /// once `mask != 0`, so masked probe indices need no bounds check.
    #[inline(always)]
    fn assume_table_invariant(&self) {
        unsafe { std::hint::assert_unchecked(self.mask + 1 == self.entries.len()); }
    }

    #[inline]
    fn slot(&self, key: i64) -> usize {
        // Fibonacci hashing with an avalanche xor-shift: mixes the high bits of
        // the multiply down so sequential keys (0, 1, 2, …) spread across slots.
        let mut h = key as u64;
        h = h.wrapping_mul(0x9E37_79B9_7F4A_7C15);
        h ^= h >> 32;
        (h as usize) & self.mask
    }

    pub fn insert(&mut self, key: i64, value: i64) {
        if key == Self::EMPTY {
            if !self.has_zero_key {
                self.has_zero_key = true;
                self.len += 1;
            }
            self.zero_key_val = value;
            return;
        }
        if self.mask == 0 || (self.table_len() + 1) * 4 > (self.mask + 1) * 3 {
            self.grow();
        }
        self.assume_table_invariant();
        let mut i = self.slot(key);
        loop {
            let k = self.entries[i][0];
            if k == Self::EMPTY {
                self.entries[i] = [key, value];
                self.len += 1;
                return;
            }
            if k == key {
                self.entries[i][1] = value;
                return;
            }
            i = (i + 1) & self.mask;
        }
    }

    fn grow(&mut self) {
        let new_slots = if self.mask == 0 { 8 } else { (self.mask + 1) * 2 };
        let old = std::mem::replace(&mut self.entries, vec![[Self::EMPTY, 0]; new_slots]);
        self.mask = new_slots - 1;
        self.assume_table_invariant();
        for &[k, v] in old.iter() {
            if k != Self::EMPTY {
                let mut i = self.slot(k);
                while self.entries[i][0] != Self::EMPTY {
                    i = (i + 1) & self.mask;
                }
                self.entries[i] = [k, v];
            }
        }
    }

    pub fn get(&self, key: &i64) -> Option<i64> {
        let key = *key;
        if key == Self::EMPTY {
            return if self.has_zero_key {
                Some(self.zero_key_val)
            } else {
                None
            };
        }
        if self.mask == 0 {
            return None;
        }
        self.assume_table_invariant();
        let mut i = self.slot(key);
        loop {
            let [k, v] = self.entries[i];
            if k == key {
                return Some(v);
            }
            if k == Self::EMPTY {
                return None;
            }
            i = (i + 1) & self.mask;
        }
    }

    pub fn contains_key(&self, key: &i64) -> bool {
        self.get(key).is_some()
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

impl Default for LogosI64Map {
    fn default() -> Self {
        Self::new()
    }
}

impl LogosContains<i64> for LogosI64Map {
    #[inline(always)]
    fn logos_contains(&self, key: &i64) -> bool {
        self.get(key).is_some()
    }
}

/// Open-addressing `i64` SET — the keys-only sibling of [`LogosI64Map`], emitted
/// for a `Map of Int to Int` the alias analysis proves is used ONLY as a set
/// (insert + contains; the value is never read — two_sum's `seen`). ONE flat
/// `Vec<i64>` with linear probing and a `0` empty-slot sentinel (the real key
/// `0` tracked separately for full key-space correctness). With no value array
/// it has HALF the store traffic and footprint of the map — 8 bytes/slot,
/// smaller than C's `struct Entry { long key; int occupied; }`.
///
/// The sentinel is `0`, not `i64::MIN`, on purpose: `vec![0; slots]` is the bit
/// pattern Rust's `IsZero` specialization lowers to `alloc_zeroed` (calloc), so
/// the table comes from the OS lazily-zeroed — no eager 1 GiB memset/page-fault
/// storm up front (the kernel-time gap that made two_sum lose to C's `calloc`).
#[derive(Debug, Clone)]
pub struct LogosI64Set {
    keys: Vec<i64>,
    mask: usize,
    len: usize,
    has_zero_key: bool,
}

impl LogosI64Set {
    const EMPTY: i64 = 0;

    pub fn new() -> Self {
        Self { keys: Vec::new(), mask: 0, len: 0, has_zero_key: false }
    }

    pub fn with_capacity(cap: usize) -> Self {
        let mut s = Self::new();
        if cap > 0 {
            let slots = ((cap * 4) / 3 + 1).next_power_of_two().max(8);
            // `Self::EMPTY == 0` → `alloc_zeroed` (calloc): lazily-zeroed pages,
            // no eager memset/fault of the whole table.
            s.keys = vec![Self::EMPTY; slots];
            s.mask = slots - 1;
        }
        s
    }

    #[inline]
    fn table_len(&self) -> usize {
        self.len - self.has_zero_key as usize
    }

    /// The probe table is a power-of-two `Vec` whose length is exactly
    /// `mask + 1` (maintained by `with_capacity`/`grow`); every probe index is
    /// `… & mask`, so once `mask != 0` every `self.keys[i]` is in bounds. Telling
    /// LLVM the invariant lets it drop the per-probe bounds check (matching C's
    /// raw indexed loads). Caller must have established `mask != 0`.
    #[inline(always)]
    fn assume_table_invariant(&self) {
        unsafe { std::hint::assert_unchecked(self.mask + 1 == self.keys.len()); }
    }

    #[inline]
    fn slot(&self, key: i64) -> usize {
        let mut h = key as u64;
        h = h.wrapping_mul(0x9E37_79B9_7F4A_7C15);
        h ^= h >> 32;
        (h as usize) & self.mask
    }

    /// Insert a key. The `_value` parameter mirrors [`LogosI64Map::insert`]'s
    /// call shape so codegen needs no special-casing at the insert site — by the
    /// set-usage proof the value is never read.
    #[inline]
    pub fn insert(&mut self, key: i64, _value: i64) {
        if key == Self::EMPTY {
            if !self.has_zero_key {
                self.has_zero_key = true;
                self.len += 1;
            }
            return;
        }
        if self.mask == 0 || (self.table_len() + 1) * 4 > (self.mask + 1) * 3 {
            self.grow();
        }
        self.assume_table_invariant();
        let mut i = self.slot(key);
        loop {
            let k = self.keys[i];
            if k == Self::EMPTY {
                self.keys[i] = key;
                self.len += 1;
                return;
            }
            if k == key {
                return;
            }
            i = (i + 1) & self.mask;
        }
    }

    fn grow(&mut self) {
        let new_slots = if self.mask == 0 { 8 } else { (self.mask + 1) * 2 };
        let old_keys = std::mem::replace(&mut self.keys, vec![Self::EMPTY; new_slots]);
        self.mask = new_slots - 1;
        self.assume_table_invariant();
        for &k in old_keys.iter() {
            if k != Self::EMPTY {
                let mut i = self.slot(k);
                while self.keys[i] != Self::EMPTY {
                    i = (i + 1) & self.mask;
                }
                self.keys[i] = k;
            }
        }
    }

    #[inline]
    pub fn contains_key(&self, key: &i64) -> bool {
        let key = *key;
        if key == Self::EMPTY {
            return self.has_zero_key;
        }
        if self.mask == 0 {
            return false;
        }
        self.assume_table_invariant();
        let mut i = self.slot(key);
        loop {
            let k = self.keys[i];
            if k == key {
                return true;
            }
            if k == Self::EMPTY {
                return false;
            }
            i = (i + 1) & self.mask;
        }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

impl Default for LogosI64Set {
    fn default() -> Self {
        Self::new()
    }
}

impl LogosContains<i64> for LogosI64Set {
    #[inline(always)]
    fn logos_contains(&self, key: &i64) -> bool {
        self.contains_key(key)
    }
}

/// i32-narrowed open-addressing `i64 → i64` map — the same probe-table shape as
/// [`LogosI64Map`] but with 8-byte `(i32, i32)` slots, emitted when the compiler
/// PROVES every key AND value stays in `i32` range. Halving the slot width halves
/// the table's memory traffic — the dominant cost of this random-access workload —
/// for maps the dense gate cannot capture (no proven contiguous key window). The
/// call surface is `i64` (keys/values widen at the boundary; the proof makes the
/// narrowing cast lossless), so codegen emits it identically to `LogosI64Map`.
#[derive(Debug, Clone)]
pub struct LogosI32Map {
    entries: Vec<[i32; 2]>,
    mask: usize,
    len: usize,
    has_zero_key: bool,
    zero_key_val: i32,
}

impl LogosI32Map {
    const EMPTY: i32 = 0;

    pub fn new() -> Self {
        Self { entries: Vec::new(), mask: 0, len: 0, has_zero_key: false, zero_key_val: 0 }
    }

    pub fn with_capacity(cap: usize) -> Self {
        let mut m = Self::new();
        if cap > 0 {
            let slots = ((cap * 4) / 3 + 1).next_power_of_two().max(8);
            // `[0, 0]` → `alloc_zeroed` (calloc): lazily zeroed, no eager memset.
            m.entries = vec![[Self::EMPTY, 0]; slots];
            m.mask = slots - 1;
        }
        m
    }

    #[inline]
    fn table_len(&self) -> usize {
        self.len - self.has_zero_key as usize
    }

    /// See [`LogosI64Set::assume_table_invariant`]: `entries.len() == mask + 1`
    /// once `mask != 0`, so masked probe indices need no bounds check.
    #[inline(always)]
    fn assume_table_invariant(&self) {
        unsafe { std::hint::assert_unchecked(self.mask + 1 == self.entries.len()); }
    }

    #[inline]
    fn slot(&self, key: i32) -> usize {
        let mut h = key as u32 as u64;
        h = h.wrapping_mul(0x9E37_79B9_7F4A_7C15);
        h ^= h >> 32;
        (h as usize) & self.mask
    }

    pub fn insert(&mut self, key: i64, value: i64) {
        let key = key as i32;
        let value = value as i32;
        if key == Self::EMPTY {
            if !self.has_zero_key {
                self.has_zero_key = true;
                self.len += 1;
            }
            self.zero_key_val = value;
            return;
        }
        if self.mask == 0 || (self.table_len() + 1) * 4 > (self.mask + 1) * 3 {
            self.grow();
        }
        self.assume_table_invariant();
        let mut i = self.slot(key);
        loop {
            let k = self.entries[i][0];
            if k == Self::EMPTY {
                self.entries[i] = [key, value];
                self.len += 1;
                return;
            }
            if k == key {
                self.entries[i][1] = value;
                return;
            }
            i = (i + 1) & self.mask;
        }
    }

    fn grow(&mut self) {
        let new_slots = if self.mask == 0 { 8 } else { (self.mask + 1) * 2 };
        let old = std::mem::replace(&mut self.entries, vec![[Self::EMPTY, 0]; new_slots]);
        self.mask = new_slots - 1;
        self.assume_table_invariant();
        for &[k, v] in old.iter() {
            if k != Self::EMPTY {
                let mut i = self.slot(k);
                while self.entries[i][0] != Self::EMPTY {
                    i = (i + 1) & self.mask;
                }
                self.entries[i] = [k, v];
            }
        }
    }

    pub fn get(&self, key: &i64) -> Option<i64> {
        let key = *key as i32;
        if key == Self::EMPTY {
            return if self.has_zero_key { Some(self.zero_key_val as i64) } else { None };
        }
        if self.mask == 0 {
            return None;
        }
        self.assume_table_invariant();
        let mut i = self.slot(key);
        loop {
            let [k, v] = self.entries[i];
            if k == key {
                return Some(v as i64);
            }
            if k == Self::EMPTY {
                return None;
            }
            i = (i + 1) & self.mask;
        }
    }

    pub fn contains_key(&self, key: &i64) -> bool {
        self.get(key).is_some()
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

impl Default for LogosI32Map {
    fn default() -> Self {
        Self::new()
    }
}

impl LogosContains<i64> for LogosI32Map {
    #[inline(always)]
    fn logos_contains(&self, key: &i64) -> bool {
        self.get(key).is_some()
    }
}

/// i32-narrowed open-addressing set — the keys-only sibling of [`LogosI32Map`]
/// (the i32 analogue of [`LogosI64Set`]). One flat `Vec<i32>`: 4 bytes per slot,
/// a quarter of `LogosI64Map`'s footprint. Emitted for a set-usage `Int → Int`
/// map whose keys are all proven to fit `i32`. The empty-slot sentinel is `0`
/// (the real key `0` tracked separately) so `vec![0; slots]` allocates via
/// `alloc_zeroed` — lazily-zeroed, no eager memset of the table.
#[derive(Debug, Clone)]
pub struct LogosI32Set {
    keys: Vec<i32>,
    mask: usize,
    len: usize,
    has_zero_key: bool,
}

impl LogosI32Set {
    const EMPTY: i32 = 0;

    pub fn new() -> Self {
        Self { keys: Vec::new(), mask: 0, len: 0, has_zero_key: false }
    }

    pub fn with_capacity(cap: usize) -> Self {
        let mut s = Self::new();
        if cap > 0 {
            let slots = ((cap * 4) / 3 + 1).next_power_of_two().max(8);
            s.keys = vec![Self::EMPTY; slots];
            s.mask = slots - 1;
        }
        s
    }

    #[inline]
    fn table_len(&self) -> usize {
        self.len - self.has_zero_key as usize
    }

    /// See [`LogosI64Set::assume_table_invariant`]: `keys.len() == mask + 1` once
    /// `mask != 0`, so masked probe indices need no bounds check.
    #[inline(always)]
    fn assume_table_invariant(&self) {
        unsafe { std::hint::assert_unchecked(self.mask + 1 == self.keys.len()); }
    }

    #[inline]
    fn slot(&self, key: i32) -> usize {
        let mut h = key as u32 as u64;
        h = h.wrapping_mul(0x9E37_79B9_7F4A_7C15);
        h ^= h >> 32;
        (h as usize) & self.mask
    }

    pub fn insert(&mut self, key: i64, _value: i64) {
        let key = key as i32;
        if key == Self::EMPTY {
            if !self.has_zero_key {
                self.has_zero_key = true;
                self.len += 1;
            }
            return;
        }
        if self.mask == 0 || (self.table_len() + 1) * 4 > (self.mask + 1) * 3 {
            self.grow();
        }
        self.assume_table_invariant();
        let mut i = self.slot(key);
        loop {
            let k = self.keys[i];
            if k == Self::EMPTY {
                self.keys[i] = key;
                self.len += 1;
                return;
            }
            if k == key {
                return;
            }
            i = (i + 1) & self.mask;
        }
    }

    fn grow(&mut self) {
        let new_slots = if self.mask == 0 { 8 } else { (self.mask + 1) * 2 };
        let old_keys = std::mem::replace(&mut self.keys, vec![Self::EMPTY; new_slots]);
        self.mask = new_slots - 1;
        self.assume_table_invariant();
        for &k in old_keys.iter() {
            if k != Self::EMPTY {
                let mut i = self.slot(k);
                while self.keys[i] != Self::EMPTY {
                    i = (i + 1) & self.mask;
                }
                self.keys[i] = k;
            }
        }
    }

    pub fn contains_key(&self, key: &i64) -> bool {
        let key = *key as i32;
        if key == Self::EMPTY {
            return self.has_zero_key;
        }
        if self.mask == 0 {
            return false;
        }
        self.assume_table_invariant();
        let mut i = self.slot(key);
        loop {
            let k = self.keys[i];
            if k == key {
                return true;
            }
            if k == Self::EMPTY {
                return false;
            }
            i = (i + 1) & self.mask;
        }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

impl Default for LogosI32Set {
    fn default() -> Self {
        Self::new()
    }
}

impl LogosContains<i64> for LogosI32Set {
    #[inline(always)]
    fn logos_contains(&self, key: &i64) -> bool {
        self.contains_key(key)
    }
}

/// Direct-addressed `i64 → i64` map — the densest representation of a
/// `Map of Int to Int`, emitted when the compiler PROVES every key ever inserted
/// or queried lands in a statically bounded window `[lo, lo + slots)` whose width
/// is ≤ the map's own `with capacity` hint. Keys index a flat value array
/// (`data[key - lo]`); a parallel presence bitset distinguishes a stored value
/// from an absent slot, so `get` returns `None` for an in-range key that was
/// never inserted. No hashing, no probing, no sparse table — both build and scan
/// phases are sequential array accesses, which the backend vectorizes and the
/// prefetcher loves. This is perfect hashing for dense integer keys: it removes
/// the random-access hash table that costs `LogosI64Map` its cache locality.
#[derive(Debug, Clone)]
pub struct LogosDenseI64Map {
    /// Value slots, indexed by `key - lo`. Zero-initialized; a zero is only a
    /// real value when its presence bit is set.
    data: Vec<i64>,
    /// One bit per slot: set ⇔ the slot holds an inserted value.
    present: Vec<u64>,
    /// The window's lower bound; `data[key - lo]` rebases the key space (so a
    /// 1-based or negative key domain maps onto `[0, slots)`).
    lo: i64,
    len: usize,
}

impl LogosDenseI64Map {
    pub fn new() -> Self {
        Self { data: Vec::new(), present: Vec::new(), lo: 0, len: 0 }
    }

    /// A window `[0, cap)` — the offset-free form. Mirrors the `LogosI64Map`
    /// constructor name so a non-offset dense map reuses the same emission.
    pub fn with_capacity(cap: usize) -> Self {
        Self::with_bounds(0, cap)
    }

    /// A window `[lo, lo + slots)`. `slots` is sized to the proven capacity hint;
    /// the soundness gate guarantees every key falls inside, so `key - lo` is a
    /// valid index for every insert and get the program performs.
    pub fn with_bounds(lo: i64, slots: usize) -> Self {
        Self {
            data: vec![0; slots],
            present: vec![0; slots.div_ceil(64)],
            lo,
            len: 0,
        }
    }

    #[inline]
    pub fn insert(&mut self, key: i64, value: i64) {
        let idx = (key - self.lo) as usize;
        debug_assert!(
            key >= self.lo && idx < self.data.len(),
            "dense map key {key} outside proven window [{}, {})",
            self.lo,
            self.lo + self.data.len() as i64
        );
        let bit = 1u64 << (idx & 63);
        let word = &mut self.present[idx >> 6];
        if *word & bit == 0 {
            *word |= bit;
            self.len += 1;
        }
        self.data[idx] = value;
    }

    #[inline]
    pub fn get(&self, key: &i64) -> Option<i64> {
        let idx = (*key - self.lo) as usize;
        debug_assert!(
            *key >= self.lo && idx < self.data.len(),
            "dense map key {key} outside proven window [{}, {})",
            self.lo,
            self.lo + self.data.len() as i64
        );
        if self.present[idx >> 6] & (1u64 << (idx & 63)) != 0 {
            Some(self.data[idx])
        } else {
            None
        }
    }

    #[inline]
    pub fn contains_key(&self, key: &i64) -> bool {
        let idx = (*key - self.lo) as usize;
        debug_assert!(*key >= self.lo && idx < self.data.len());
        self.present[idx >> 6] & (1u64 << (idx & 63)) != 0
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

impl Default for LogosDenseI64Map {
    fn default() -> Self {
        Self::new()
    }
}

impl LogosContains<i64> for LogosDenseI64Map {
    #[inline(always)]
    fn logos_contains(&self, key: &i64) -> bool {
        self.contains_key(key)
    }
}

/// The presence-elided sibling of [`LogosDenseI64Map`], emitted ONLY when the
/// compiler additionally proves the get/contains key domain is a subset of a
/// CONTIGUOUSLY FULLY-COVERED insert range — i.e. every key the program reads was
/// definitely written first. With that proof the presence bit is invariably set,
/// so it is dropped: `get` is a bare `Some(data[key - lo])` load, byte-identical
/// to a C array read, and no presence bitset is allocated. The gate forbids this
/// type for any map with a `contains` use (there is no way to answer membership
/// without presence), so `contains_key`/`LogosContains` are intentionally absent
/// — a generated `contains` on this type would fail to compile, surfacing a gate
/// bug loudly rather than silently miscompiling.
#[derive(Debug, Clone)]
pub struct LogosDenseI64MapNoPresence {
    data: Vec<i64>,
    lo: i64,
    /// Insert count. Exact under the proven regime (each covered key written
    /// once); never emitted (a `length of m` use disqualifies the map upstream).
    len: usize,
}

impl LogosDenseI64MapNoPresence {
    pub fn new() -> Self {
        Self { data: Vec::new(), lo: 0, len: 0 }
    }

    pub fn with_capacity(cap: usize) -> Self {
        Self::with_bounds(0, cap)
    }

    pub fn with_bounds(lo: i64, slots: usize) -> Self {
        Self { data: vec![0; slots], lo, len: 0 }
    }

    #[inline]
    pub fn insert(&mut self, key: i64, value: i64) {
        let idx = (key - self.lo) as usize;
        debug_assert!(
            key >= self.lo && idx < self.data.len(),
            "dense map key {key} outside proven window [{}, {})",
            self.lo,
            self.lo + self.data.len() as i64
        );
        self.data[idx] = value;
        self.len += 1;
    }

    #[inline]
    pub fn get(&self, key: &i64) -> Option<i64> {
        let idx = (*key - self.lo) as usize;
        debug_assert!(
            *key >= self.lo && idx < self.data.len(),
            "dense map key {key} outside proven window [{}, {})",
            self.lo,
            self.lo + self.data.len() as i64
        );
        Some(self.data[idx])
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

impl Default for LogosDenseI64MapNoPresence {
    fn default() -> Self {
        Self::new()
    }
}

/// Direct-addressed `i64` SET — the keys-only, value-free sibling of
/// [`LogosDenseI64Map`] (the dense analogue of [`LogosI64Set`]). Membership lives
/// in a presence bitset over the proven window `[lo, lo + slots)`; `insert` sets
/// a bit, `contains` tests one. No value array → 1 bit per key, the smallest
/// footprint of any map/set representation.
#[derive(Debug, Clone)]
pub struct LogosDenseI64Set {
    present: Vec<u64>,
    lo: i64,
    len: usize,
}

impl LogosDenseI64Set {
    pub fn new() -> Self {
        Self { present: Vec::new(), lo: 0, len: 0 }
    }

    pub fn with_capacity(cap: usize) -> Self {
        Self::with_bounds(0, cap)
    }

    pub fn with_bounds(lo: i64, slots: usize) -> Self {
        Self { present: vec![0; slots.div_ceil(64)], lo, len: 0 }
    }

    /// Insert a key. The `_value` mirrors [`LogosDenseI64Map::insert`]'s call
    /// shape so codegen needs no special-casing at the insert site.
    #[inline]
    pub fn insert(&mut self, key: i64, _value: i64) {
        let idx = (key - self.lo) as usize;
        debug_assert!(
            key >= self.lo && idx < self.present.len() * 64,
            "dense set key {key} outside proven window starting at {}",
            self.lo
        );
        let bit = 1u64 << (idx & 63);
        let word = &mut self.present[idx >> 6];
        if *word & bit == 0 {
            *word |= bit;
            self.len += 1;
        }
    }

    #[inline]
    pub fn contains_key(&self, key: &i64) -> bool {
        let idx = (*key - self.lo) as usize;
        debug_assert!(*key >= self.lo && idx < self.present.len() * 64);
        self.present[idx >> 6] & (1u64 << (idx & 63)) != 0
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

impl Default for LogosDenseI64Set {
    fn default() -> Self {
        Self::new()
    }
}

impl LogosContains<i64> for LogosDenseI64Set {
    #[inline(always)]
    fn logos_contains(&self, key: &i64) -> bool {
        self.contains_key(key)
    }
}

/// Precomputed unsigned division by a **loop-invariant runtime divisor** — the
/// magic-multiply that replaces a hardware `div`/`idiv` once the divisor is
/// pinned. Codegen emits `LogosDivU64::new(n)` in a loop's preheader (computed
/// ONCE) and rewrites each in-loop `x % n` / `x / n` to `.rem(x)` / `.div(x)`,
/// turning a ~20–40-cycle division into a multiply-high plus a shift. gcc and
/// rustc both leave a runtime-invariant divisor as a real `div` (neither
/// synthesizes this), so it is a strict win over the C baseline on division-hot
/// loops (graph_bfs's `% n` adjacency build).
///
/// The construction is the standard Granlund–Montgomery / libdivide unsigned
/// algorithm, exact for every divisor in `1..=u64::MAX` (power-of-two fast path,
/// the 64-bit-magic path, and the 65-bit "add marker" path). Codegen only emits
/// it where the dividend is proven non-negative and `n > 0` (the existing
/// positivity guard), so the `i64`→`u64` reinterpretation is value-preserving.
#[derive(Debug, Clone, Copy)]
pub struct LogosDivU64 {
    magic: u64,
    /// Low 6 bits: shift amount. `ADD_MARKER` (0x40): the 65-bit-magic path.
    /// `SHIFT_PATH` (0x80): the divisor is a power of two (pure shift).
    more: u8,
    d: u64,
}

impl LogosDivU64 {
    const SHIFT_MASK: u8 = 0x3F;
    const ADD_MARKER: u8 = 0x40;
    const SHIFT_PATH: u8 = 0x80;

    /// Build the magic numbers for divisor `d` (must be non-zero). Runs once per
    /// loop in the preheader, so its cost (a single 128/64 division) amortizes
    /// over the whole loop.
    #[inline]
    pub fn new(d: u64) -> Self {
        debug_assert!(d != 0, "LogosDivU64: divisor must be non-zero");
        if d & (d - 1) == 0 {
            // Power of two (including d == 1, shift 0): the division is a shift.
            return Self { magic: 0, more: (d.trailing_zeros() as u8) | Self::SHIFT_PATH, d };
        }
        let floor_log_2_d = 63 - d.leading_zeros();
        // proposed_m = floor(2^(64 + floor_log_2_d) / d); rem the remainder.
        let numer = 1u128 << (64 + floor_log_2_d);
        let proposed_m = (numer / d as u128) as u64;
        let rem = (numer % d as u128) as u64;
        let e = d - rem;
        let (magic, more) = if e < (1u64 << floor_log_2_d) {
            // The 64-bit magic suffices.
            (proposed_m + 1, floor_log_2_d as u8)
        } else {
            // Need the 65th bit — fold it into an extra add at division time.
            let twice_rem = rem.wrapping_mul(2);
            let bump = (twice_rem >= d || twice_rem < rem) as u64;
            (proposed_m.wrapping_mul(2).wrapping_add(bump) + 1,
             (floor_log_2_d as u8) | Self::ADD_MARKER)
        };
        Self { magic, more, d }
    }

    #[inline(always)]
    pub fn div(&self, numer: u64) -> u64 {
        if self.more & Self::SHIFT_PATH != 0 {
            return numer >> (self.more & Self::SHIFT_MASK);
        }
        let q = (((self.magic as u128) * (numer as u128)) >> 64) as u64;
        if self.more & Self::ADD_MARKER != 0 {
            let t = ((numer - q) >> 1).wrapping_add(q);
            t >> (self.more & Self::SHIFT_MASK)
        } else {
            q >> self.more
        }
    }

    #[inline(always)]
    pub fn rem(&self, numer: u64) -> u64 {
        numer - self.div(numer).wrapping_mul(self.d)
    }

    /// The raw `(magic, more)` pair, for backends (the VM / JIT interpreter
    /// tiers) that bake the precomputed constants into a single fused op instead
    /// of holding a `LogosDivU64` struct. The encoding of `more` is exactly the
    /// one `div`/`rem` consume (low 6 bits = shift; `0x40` = the 65-bit
    /// add-marker path; `0x80` = the pure-shift power-of-two path).
    #[inline]
    pub fn parts(&self) -> (u64, u8) {
        (self.magic, self.more)
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

    /// A deterministic LCG so the `LogosI64Map` fuzz is reproducible without a
    /// `rand` dependency (same seed → same op stream every run).
    struct Lcg(u64);
    impl Lcg {
        fn next_u64(&mut self) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            self.0
        }
    }

    /// Differential fuzz: a long random stream of inserts/gets/contains on a
    /// `LogosI64Map` must agree with `std::collections::HashMap` (the oracle) on
    /// every read, across many resizes and over a key space that forces both
    /// overwrites (small range → collisions) and the `0` sentinel path (the real
    /// key `0`, tracked outside the table) as well as `i64::MIN` as an ordinary
    /// in-table key.
    #[test]
    fn i64map_matches_hashmap_oracle() {
        use std::collections::HashMap;
        // The key pool deliberately includes `0` (the sentinel value) and both
        // `i64` extremes so the empty-slot encoding and the side-tracked zero key
        // are exercised under fuzz, not just in isolation.
        let pool: Vec<i64> = {
            let mut v: Vec<i64> = (-40..40).collect();
            v.extend_from_slice(&[i64::MIN, i64::MAX, i64::MIN + 1, i64::MAX - 1, 0]);
            v
        };
        for seed in [1u64, 7, 1234567, 0x9E3779B9, u64::MAX / 3] {
            let mut rng = Lcg(seed);
            let mut map = LogosI64Map::new();
            let mut oracle: HashMap<i64, i64> = HashMap::new();
            for _ in 0..50_000 {
                let key = pool[(rng.next_u64() as usize) % pool.len()];
                match rng.next_u64() % 3 {
                    0 => {
                        let val = rng.next_u64() as i64;
                        map.insert(key, val);
                        oracle.insert(key, val);
                    }
                    1 => {
                        assert_eq!(
                            map.get(&key),
                            oracle.get(&key).copied(),
                            "get disagreed for key {key} (seed {seed})"
                        );
                    }
                    _ => {
                        assert_eq!(
                            map.contains_key(&key),
                            oracle.contains_key(&key),
                            "contains disagreed for key {key} (seed {seed})"
                        );
                    }
                }
                assert_eq!(map.len(), oracle.len(), "len diverged (seed {seed})");
            }
            // Final sweep: every pooled key must read identically.
            for &k in &pool {
                assert_eq!(map.get(&k), oracle.get(&k).copied(), "final get key {k}");
            }
        }
    }

    /// `LogosI64Set` agrees with `std::HashSet` under a 50k-op fuzz across seeds,
    /// with `0` (the sentinel value) and both `i64` extremes in the key pool. The
    /// set's `insert(key, _value)` mirrors the map call shape; the value is ignored.
    #[test]
    fn i64set_matches_hashset_oracle() {
        use std::collections::HashSet;
        let pool: Vec<i64> = {
            let mut v: Vec<i64> = (-40..40).collect();
            v.extend_from_slice(&[i64::MIN, i64::MAX, i64::MIN + 1, i64::MAX - 1, 0]);
            v
        };
        for seed in [1u64, 7, 1234567, 0x9E3779B9, u64::MAX / 3] {
            let mut rng = Lcg(seed);
            let mut set = LogosI64Set::new();
            let mut oracle: HashSet<i64> = HashSet::new();
            for _ in 0..50_000 {
                let key = pool[(rng.next_u64() as usize) % pool.len()];
                if rng.next_u64() % 2 == 0 {
                    set.insert(key, 1);
                    oracle.insert(key);
                } else {
                    assert_eq!(
                        set.contains_key(&key),
                        oracle.contains(&key),
                        "contains disagreed for key {key} (seed {seed})"
                    );
                }
                assert_eq!(set.len(), oracle.len(), "len diverged (seed {seed})");
            }
            for &k in &pool {
                assert_eq!(set.contains_key(&k), oracle.contains(&k), "final contains key {k}");
            }
        }
    }

    /// `LogosDivU64` (loop-invariant libdivide) must agree with the hardware
    /// `/` and `%` for EVERY (numerator, divisor) pair — exhaustively over small
    /// values (every divisor 1..=512 against numerators 0..=4096, which forces
    /// the power-of-two fast path, the 64-bit-magic path, AND the ADD_MARKER
    /// 65-bit-magic path), and under a wide fuzz that hammers the full `u64`
    /// range including `u64::MAX`, `2^63`, and the exact benchmark divisors. Any
    /// disagreement here is a miscompiled division — the test is the spec.
    #[test]
    fn divu64_matches_hardware_div_and_rem() {
        // Exhaustive small grid — divisor 1..=512, numerator 0..=4096.
        for d in 1u64..=512 {
            let m = LogosDivU64::new(d);
            for x in 0u64..=4096 {
                assert_eq!(m.div(x), x / d, "div({x}, {d})");
                assert_eq!(m.rem(x), x % d, "rem({x}, {d})");
            }
        }

        // Boundary divisors (powers of two, primes, near-2^k, benchmark sizes)
        // crossed with boundary + fuzzed numerators across the full u64 range.
        let divisors: Vec<u64> = vec![
            1, 2, 3, 4, 5, 7, 8, 9, 10, 16, 31, 37, 41, 43, 47, 64, 100, 127, 128,
            1000, 1024, 65535, 65536, 65537, 1_000_000, 2_147_483_648, 3_000_000,
            5_000_000, 1_000_000_007, (1u64 << 62) - 1, 1u64 << 62, (1u64 << 63) - 1,
            1u64 << 63, u64::MAX - 1, u64::MAX,
        ];
        let mut rng = Lcg(0xDEAD_BEEF_CAFE_F00D);
        for &d in &divisors {
            let m = LogosDivU64::new(d);
            let mut numerators: Vec<u64> = vec![
                0, 1, 2, d.wrapping_sub(1), d, d.wrapping_add(1), d.wrapping_mul(2),
                i64::MAX as u64, 1u64 << 63, u64::MAX - 1, u64::MAX,
            ];
            for _ in 0..2000 {
                numerators.push(rng.next_u64());
            }
            for &x in &numerators {
                assert_eq!(m.div(x), x / d, "div({x}, {d}) [fuzz]");
                assert_eq!(m.rem(x), x % d, "rem({x}, {d}) [fuzz]");
            }
        }
    }

    /// `i64::MIN` is a genuine, distinguishable key — present after insert,
    /// absent before, and independent of the rest of the table. (It is now an
    /// ordinary in-table key; the empty-slot sentinel is `0`.)
    #[test]
    fn i64map_sentinel_key_is_a_real_key() {
        let mut m = LogosI64Map::new();
        assert_eq!(m.get(&i64::MIN), None);
        assert!(!m.contains_key(&i64::MIN));

        // Fill enough ordinary keys to force several resizes around it.
        for k in 0..1000i64 {
            m.insert(k, k * 3);
        }
        assert_eq!(m.get(&i64::MIN), None, "sentinel must stay absent");

        m.insert(i64::MIN, 999);
        assert_eq!(m.get(&i64::MIN), Some(999));
        assert!(m.contains_key(&i64::MIN));
        // It does not perturb ordinary keys, nor they it.
        for k in 0..1000i64 {
            assert_eq!(m.get(&k), Some(k * 3));
        }
        m.insert(i64::MIN, -1);
        assert_eq!(m.get(&i64::MIN), Some(-1), "sentinel value overwrites");
        assert_eq!(m.len(), 1001, "sentinel counts once toward len");
    }

    /// Overwriting an existing key replaces its value and never grows `len`.
    #[test]
    fn i64map_overwrite_preserves_len() {
        let mut m = LogosI64Map::new();
        for k in 0..100i64 {
            m.insert(k, 1);
        }
        assert_eq!(m.len(), 100);
        for k in 0..100i64 {
            m.insert(k, k * 10);
        }
        assert_eq!(m.len(), 100, "overwrites must not change len");
        for k in 0..100i64 {
            assert_eq!(m.get(&k), Some(k * 10));
        }
    }

    /// `with_capacity` pre-sized then filled exactly to capacity reads back
    /// every entry (the headroom math must not under-allocate).
    #[test]
    fn i64map_with_capacity_fills_correctly() {
        for cap in [0usize, 1, 7, 8, 100, 1000] {
            let mut m = LogosI64Map::with_capacity(cap);
            let n = cap.max(1) as i64;
            for k in 0..n {
                m.insert(k, k + 7);
            }
            assert_eq!(m.len(), n as usize, "cap {cap}");
            for k in 0..n {
                assert_eq!(m.get(&k), Some(k + 7), "cap {cap} key {k}");
            }
            assert_eq!(m.get(&(n + 1)), None);
        }
    }

    /// `Clone` has value semantics — mutating the original leaves the clone
    /// untouched (the whole point of selecting this map only when non-aliased).
    #[test]
    fn i64map_clone_is_independent() {
        let mut a = LogosI64Map::new();
        for k in 0..50i64 {
            a.insert(k, k);
        }
        let b = a.clone();
        for k in 0..50i64 {
            a.insert(k, k + 1000);
        }
        a.insert(100, 100);
        for k in 0..50i64 {
            assert_eq!(b.get(&k), Some(k), "clone must not see later mutations");
        }
        assert_eq!(b.get(&100), None, "clone must not gain new keys");
        assert_eq!(b.len(), 50);
    }

    /// The key `0` is a genuine, distinguishable key — never confused with an
    /// empty slot — present after insert, absent before, and independent of the
    /// rest of the table across many resizes. With the `0` empty-slot sentinel
    /// the real key `0` lives outside the probe table; this pins that side path.
    #[test]
    fn i64map_zero_key_is_a_real_key() {
        let mut m = LogosI64Map::new();
        assert_eq!(m.get(&0), None);
        assert!(!m.contains_key(&0));

        // Ordinary non-zero keys force several resizes around the zero key.
        for k in 1..=1000i64 {
            m.insert(k, k * 3);
        }
        assert_eq!(m.get(&0), None, "zero key must stay absent");

        m.insert(0, 999);
        assert_eq!(m.get(&0), Some(999));
        assert!(m.contains_key(&0));
        for k in 1..=1000i64 {
            assert_eq!(m.get(&k), Some(k * 3), "ordinary keys undisturbed by zero key");
        }
        m.insert(0, -1);
        assert_eq!(m.get(&0), Some(-1), "zero key value overwrites");
        assert_eq!(m.len(), 1001, "zero key counts once toward len");
    }

    /// The set analogue: `0` is a real member, distinct from empty slots, stable
    /// across resizes driven by non-zero members.
    #[test]
    fn i64set_zero_key_is_a_real_key() {
        let mut s = LogosI64Set::new();
        assert!(!s.contains_key(&0));
        for k in 1..=1000i64 {
            s.insert(k, 1);
        }
        assert!(!s.contains_key(&0), "zero key must stay absent");
        s.insert(0, 1);
        assert!(s.contains_key(&0));
        for k in 1..=1000i64 {
            assert!(s.contains_key(&k), "members undisturbed by zero key");
        }
        s.insert(0, 1);
        assert_eq!(s.len(), 1001, "zero key counts once toward len");
    }

    /// A present key whose VALUE is `0` must read back as `Some(0)`, never `None`
    /// — emptiness is decided by the key lane, never the value lane (the classic
    /// open-addressing trap once the value array is zero-initialized via calloc).
    #[test]
    fn i64map_value_zero_distinct_from_absent() {
        let mut m = LogosI64Map::new();
        m.insert(5, 0);
        m.insert(7, 0);
        assert_eq!(m.get(&5), Some(0), "present key with value 0 is not absent");
        assert_eq!(m.get(&7), Some(0));
        assert_eq!(m.get(&6), None, "truly absent key is None");

        // Same, with a pre-sized (calloc-zeroed) table and the zero key itself.
        let mut m2 = LogosI64Map::with_capacity(64);
        m2.insert(0, 0);
        m2.insert(3, 0);
        assert_eq!(m2.get(&0), Some(0), "zero key with zero value is present");
        assert_eq!(m2.get(&3), Some(0));
        assert_eq!(m2.get(&1), None);
    }

    /// `i64::MIN` is an ORDINARY in-table key (no longer a sentinel): it coexists
    /// with `i64::MAX` and ordinary keys, each with a distinct value, across
    /// resizes, and overwrites like any other key.
    #[test]
    fn i64map_min_key_is_ordinary() {
        let mut m = LogosI64Map::new();
        m.insert(i64::MIN, 11);
        m.insert(i64::MAX, 22);
        for k in 1..=1000i64 {
            m.insert(k, k);
        }
        assert_eq!(m.get(&i64::MIN), Some(11));
        assert_eq!(m.get(&i64::MAX), Some(22));
        m.insert(i64::MIN, 33);
        assert_eq!(m.get(&i64::MIN), Some(33), "i64::MIN overwrites like any key");
        assert_eq!(m.get(&i64::MAX), Some(22), "and does not perturb i64::MAX");
        assert_eq!(m.len(), 1002, "i64::MIN + i64::MAX + 1000 ordinary keys");
    }

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

    /// Differential fuzz: a `LogosDenseI64Map` built over a proven window
    /// `[lo, lo+cap)` must agree with `std::collections::HashMap` on every
    /// `get`/`contains`/`len` — for keys IN the window (the only keys the
    /// soundness gate ever lets reach this representation), including in-window
    /// keys that are never inserted (which must read `None`), overwrites, and a
    /// negative offset `lo`. The window is the contract; we never index outside it.
    #[test]
    fn dense_i64map_matches_hashmap_oracle() {
        use std::collections::HashMap;
        // (lo, cap) windows: 0-based, 1-based (the collect shape), and negative.
        for &(lo, cap) in &[(0i64, 64usize), (1, 100), (-40, 80), (-1, 9), (1000, 50)] {
            for seed in [1u64, 7, 1234567, 0x9E3779B9, u64::MAX / 3] {
                let mut rng = Lcg(seed);
                let mut map = LogosDenseI64Map::with_bounds(lo, cap);
                let mut oracle: HashMap<i64, i64> = HashMap::new();
                for _ in 0..40_000 {
                    // Draw a key strictly inside the window [lo, lo+cap).
                    let key = lo + (rng.next_u64() % cap as u64) as i64;
                    match rng.next_u64() % 3 {
                        0 => {
                            let val = rng.next_u64() as i64;
                            map.insert(key, val);
                            oracle.insert(key, val);
                        }
                        1 => assert_eq!(
                            map.get(&key),
                            oracle.get(&key).copied(),
                            "get disagreed for key {key} (lo {lo} cap {cap} seed {seed})"
                        ),
                        _ => assert_eq!(
                            map.contains_key(&key),
                            oracle.contains_key(&key),
                            "contains disagreed for key {key} (lo {lo} cap {cap} seed {seed})"
                        ),
                    }
                    assert_eq!(map.len(), oracle.len(), "len diverged (lo {lo} cap {cap} seed {seed})");
                }
                // Final sweep: every in-window key reads identically, present or absent.
                for k in lo..lo + cap as i64 {
                    assert_eq!(
                        map.get(&k),
                        oracle.get(&k).copied(),
                        "final get key {k} (lo {lo} cap {cap})"
                    );
                }
            }
        }
    }

    /// A never-inserted but in-range key reads `None` (the presence bitset is
    /// what distinguishes "stored 0" from "absent"), and an offset `lo` rebases
    /// correctly with no aliasing between neighbouring keys.
    #[test]
    fn dense_i64map_absent_in_range_key_is_none() {
        let mut m = LogosDenseI64Map::with_bounds(-5, 20); // window [-5, 15)
        assert_eq!(m.get(&3), None);
        m.insert(-5, 100);
        m.insert(14, 200);
        m.insert(0, 0); // value 0 is a real stored value, NOT "absent"
        assert_eq!(m.get(&-5), Some(100));
        assert_eq!(m.get(&14), Some(200));
        assert_eq!(m.get(&0), Some(0), "stored 0 must read as present");
        assert_eq!(m.get(&3), None, "never-inserted in-range key stays absent");
        assert_eq!(m.get(&-1), None);
        assert_eq!(m.len(), 3);
        // Overwrite preserves len; neighbours untouched.
        m.insert(0, 42);
        assert_eq!(m.get(&0), Some(42));
        assert_eq!(m.get(&-5), Some(100));
        assert_eq!(m.len(), 3, "overwrite must not change len");
    }

    /// The presence-elided `LogosDenseI64MapNoPresence` is selected ONLY when the
    /// compiler proves every queried key was inserted (contiguous full coverage),
    /// so its `get` is a pure `Some(data[k-lo])` load. Under that regime it must
    /// agree with a `HashMap` for every covered key — including a negative `lo`.
    #[test]
    fn dense_i64map_nopresence_full_coverage() {
        for &(lo, cap) in &[(0i64, 64usize), (1, 1000), (-40, 80)] {
            let mut m = LogosDenseI64MapNoPresence::with_bounds(lo, cap);
            // Fully cover the window (the proven precondition for this type).
            for k in lo..lo + cap as i64 {
                m.insert(k, k.wrapping_mul(3).wrapping_add(7));
            }
            for k in lo..lo + cap as i64 {
                assert_eq!(
                    m.get(&k),
                    Some(k.wrapping_mul(3).wrapping_add(7)),
                    "nopresence get key {k} (lo {lo} cap {cap})"
                );
            }
            assert_eq!(m.len(), cap, "nopresence len (lo {lo} cap {cap})");
        }
    }

    /// Differential fuzz for the dense SET sibling against `std::HashSet`.
    #[test]
    fn dense_i64set_matches_hashset_oracle() {
        use std::collections::HashSet;
        for &(lo, cap) in &[(0i64, 64usize), (1, 100), (-40, 80), (5, 5)] {
            for seed in [1u64, 7, 1234567, 0x9E3779B9, u64::MAX / 3] {
                let mut rng = Lcg(seed);
                let mut set = LogosDenseI64Set::with_bounds(lo, cap);
                let mut oracle: HashSet<i64> = HashSet::new();
                for _ in 0..40_000 {
                    let key = lo + (rng.next_u64() % cap as u64) as i64;
                    if rng.next_u64() % 2 == 0 {
                        set.insert(key, 1);
                        oracle.insert(key);
                    } else {
                        assert_eq!(
                            set.contains_key(&key),
                            oracle.contains(&key),
                            "contains disagreed for key {key} (lo {lo} cap {cap} seed {seed})"
                        );
                    }
                    assert_eq!(set.len(), oracle.len(), "len diverged (lo {lo} cap {cap} seed {seed})");
                }
                for k in lo..lo + cap as i64 {
                    assert_eq!(set.contains_key(&k), oracle.contains(&k), "final contains key {k}");
                }
            }
        }
    }

    /// The exact `collect` benchmark shape: a 1-based window `[1, n]` allocated to
    /// capacity `n`, insert `i -> i*2`, then look every key up. The bound is
    /// `lo = 1`, so key `n` maps to index `n-1 < n` — the off-by-one that a naive
    /// `lo = 0` would blow. Both dense flavours must report all `n` found.
    #[test]
    fn dense_i64map_collect_benchmark_shape() {
        let n: i64 = 5000;
        // Presence-tracking flavour.
        let mut m = LogosDenseI64Map::with_bounds(1, n as usize);
        for i in 1..=n {
            m.insert(i, i << 1);
        }
        let found = (1..=n).filter(|&i| m.get(&i) == Some(i << 1)).count();
        assert_eq!(found as i64, n, "presence flavour must find every key");
        // Presence-elided flavour (proven full coverage).
        let mut mp = LogosDenseI64MapNoPresence::with_bounds(1, n as usize);
        for i in 1..=n {
            mp.insert(i, i << 1);
        }
        let found_np = (1..=n).filter(|&i| mp.get(&i) == Some(i << 1)).count();
        assert_eq!(found_np as i64, n, "nopresence flavour must find every key");
    }

    /// Differential fuzz: a `LogosI32Map` driven by keys AND values inside `i32`
    /// range (the only inputs the narrowing gate ever sends it) must agree with a
    /// `HashMap` on every `get`/`contains`/`len` — across resizes, overwrites, the
    /// `0` sentinel path (the real key `0`), and `i32::MIN`/`i32::MAX` as ordinary
    /// in-table keys.
    #[test]
    fn i32map_matches_hashmap_oracle() {
        use std::collections::HashMap;
        let pool: Vec<i64> = {
            let mut v: Vec<i64> = (-40..40).collect();
            v.extend_from_slice(&[
                i32::MIN as i64, i32::MAX as i64, (i32::MIN + 1) as i64, (i32::MAX - 1) as i64, 0,
            ]);
            v
        };
        for seed in [1u64, 7, 1234567, 0x9E3779B9, u64::MAX / 3] {
            let mut rng = Lcg(seed);
            let mut map = LogosI32Map::new();
            let mut oracle: HashMap<i64, i64> = HashMap::new();
            for _ in 0..50_000 {
                let key = pool[(rng.next_u64() as usize) % pool.len()];
                match rng.next_u64() % 3 {
                    0 => {
                        // A value inside i32 range (what the narrowing proof guarantees).
                        let val = (rng.next_u64() as i32) as i64;
                        map.insert(key, val);
                        oracle.insert(key, val);
                    }
                    1 => assert_eq!(
                        map.get(&key),
                        oracle.get(&key).copied(),
                        "get disagreed for key {key} (seed {seed})"
                    ),
                    _ => assert_eq!(
                        map.contains_key(&key),
                        oracle.contains_key(&key),
                        "contains disagreed for key {key} (seed {seed})"
                    ),
                }
                assert_eq!(map.len(), oracle.len(), "len diverged (seed {seed})");
            }
            for &k in &pool {
                assert_eq!(map.get(&k), oracle.get(&k).copied(), "final get key {k}");
            }
        }
    }

    /// `LogosI32Set` agrees with `std::HashSet` under fuzz over `i32`-range keys.
    #[test]
    fn i32set_matches_hashset_oracle() {
        use std::collections::HashSet;
        let pool: Vec<i64> = {
            let mut v: Vec<i64> = (-40..40).collect();
            v.extend_from_slice(&[
                i32::MIN as i64, i32::MAX as i64, (i32::MIN + 1) as i64, (i32::MAX - 1) as i64, 0,
            ]);
            v
        };
        for seed in [1u64, 7, 1234567, 0x9E3779B9, u64::MAX / 3] {
            let mut rng = Lcg(seed);
            let mut set = LogosI32Set::new();
            let mut oracle: HashSet<i64> = HashSet::new();
            for _ in 0..50_000 {
                let key = pool[(rng.next_u64() as usize) % pool.len()];
                if rng.next_u64() % 2 == 0 {
                    set.insert(key, 1);
                    oracle.insert(key);
                } else {
                    assert_eq!(
                        set.contains_key(&key),
                        oracle.contains(&key),
                        "contains disagreed for key {key} (seed {seed})"
                    );
                }
                assert_eq!(set.len(), oracle.len(), "len diverged (seed {seed})");
            }
            for &k in &pool {
                assert_eq!(set.contains_key(&k), oracle.contains(&k), "final contains key {k}");
            }
        }
    }
}
