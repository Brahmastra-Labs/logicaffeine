//! Polymorphic indexing traits for Logos collections.
//!
//! Logos uses **1-based indexing** to match natural language conventions.
//! These traits provide get/set operations that automatically convert
//! 1-based indices to 0-based for underlying Rust collections.
//!
//! # Supported Collections
//!
//! - [`Vec<T>`]: Indexed by `i64` (1-based, converted to 0-based internally)
//! - `HashMap<K, V>`: Indexed by key `K` (pass-through semantics)
//! - `HashMap<String, V>`: Also supports `&str` keys for convenience
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

/// The 0-based offset of the item a 1-based LOGOS index names in a sequence of `len` items, or `None` when it
/// names none. Index 1 is the first item and a negative index counts from the back (-1 is the last); 0 and
/// anything past either end name nothing. ONE definition: every engine, every generated program and every
/// fast path indexes through this rule. The arithmetic is 64-bit on every target, so a wasm32 build reads
/// the same item a native one does.
#[inline(always)]
pub fn logos_offset(index: i64, len: usize) -> Option<usize> {
    let offset = index.wrapping_sub(1) as u64;
    if offset < len as u64 {
        Some(offset as usize)
    } else {
        logos_offset_from_the_back(index, len)
    }
}

/// The rest of [`logos_offset`], off the hot path: an index counted from the back, or none.
#[cold]
#[inline(never)]
fn logos_offset_from_the_back(index: i64, len: usize) -> Option<usize> {
    let back = index.unsigned_abs();
    (index <= -1 && back <= len as u64).then(|| len - back as usize)
}

/// [`logos_offset`], failing with the one sentence every tier reports when the index names no item.
#[inline(always)]
pub fn resolve_logos_index(index: i64, len: usize) -> usize {
    match logos_offset(index, len) {
        Some(offset) => offset,
        None => no_item_at(index),
    }
}

#[cold]
#[inline(never)]
fn no_item_at(index: i64) -> ! {
    panic!("{}", crate::failure::index_out_of_bounds(index))
}

/// The 0-based range `items start through end` names in a sequence of `len` items, or an empty range when
/// the slice reaches outside it (or runs backwards). ONE definition, like [`logos_offset`], and 64-bit on
/// every target.
#[inline]
pub fn logos_slice_range(start: i64, end: i64, len: usize) -> core::ops::Range<usize> {
    let from = (start as u64).saturating_sub(1);
    let to = end as u64;
    if from < to && to <= len as u64 {
        from as usize..to as usize
    } else {
        0..0
    }
}

/// The items a counted loop over the indices `first` through `last` reads from a sequence of `len` items, in
/// order, as the 0-based range they occupy: empty when `first` is past `last`, and otherwise the stop
/// [`resolve_logos_index`] makes at the first index in the loop's order that names no item. A backend that
/// copies such a loop in one step copies this range, so it reads what the loop reads and stops where the loop
/// stops. (A run that crosses 0 always stops at 0; one wholly below 0 counts from the back.)
#[inline]
pub fn logos_run_through(first: i64, last: i64, len: usize) -> core::ops::Range<usize> {
    if first > last {
        return 0..0;
    }
    let len64 = len as u64;
    if first >= 1 {
        if last as u64 <= len64 {
            return (first - 1) as usize..last as usize;
        }
        return no_item_at(first.max(len as i64 + 1));
    }
    if first.unsigned_abs() > len64 {
        return no_item_at(first);
    }
    if last <= -1 {
        return len - first.unsigned_abs() as usize..len - last.unsigned_abs() as usize + 1;
    }
    no_item_at(0)
}

/// [`logos_run_through`] for a loop that stops before `stop` (`while i < stop`).
#[inline]
pub fn logos_run_before(first: i64, stop: i64, len: usize) -> core::ops::Range<usize> {
    match stop.checked_sub(1) {
        Some(last) => logos_run_through(first, last, len),
        None => 0..0,
    }
}

/// The text a counted loop produces when it copies bytes `first` through `last` of `s` into a sequence and
/// reads them back with `text_from_bytes`: the bytes under [`logos_run_through`], and the stop
/// `text_from_bytes` makes when they do not form UTF-8. A slice of text is text exactly when both of its
/// ends fall between characters, so the common case costs two boundary checks.
pub fn logos_text_run_through(s: &str, first: i64, last: i64) -> &str {
    text_bytes_as_text(s, logos_run_through(first, last, s.len()))
}

/// [`logos_text_run_through`] for a loop that stops before `stop`.
pub fn logos_text_run_before(s: &str, first: i64, stop: i64) -> &str {
    text_bytes_as_text(s, logos_run_before(first, stop, s.len()))
}

fn text_bytes_as_text(s: &str, range: core::ops::Range<usize>) -> &str {
    if s.is_char_boundary(range.start) && s.is_char_boundary(range.end) {
        return &s[range];
    }
    match core::str::from_utf8(&s.as_bytes()[range]) {
        Ok(text) => text,
        Err(e) => panic!("{}", crate::failure::invalid_utf8(e)),
    }
}

/// Borrowing access to the item a 1-based LOGOS index names, under [`resolve_logos_index`]. A generated
/// program reads and writes a sequence's items through this, so its fast path costs one compare and every
/// other index means what it means on the interpreter.
pub trait LogosItems<T> {
    /// The item `index` names.
    fn logos_item(&self, index: i64) -> &T;
    /// The item `index` names, to write.
    fn logos_item_mut(&mut self, index: i64) -> &mut T;
    /// [`logos_item`](Self::logos_item) for an `index` the caller has proven is at least 1: the item, counted
    /// from the front, or the failure past the end. With no index from the back to resolve, a miss only fails,
    /// and a loop whose indices are proven in range compiles to plain loads.
    fn logos_item_front(&self, index: i64) -> &T;
    /// [`logos_item_front`](Self::logos_item_front), to write.
    fn logos_item_front_mut(&mut self, index: i64) -> &mut T;
    /// Items `start` through `end`, under [`logos_slice_range`].
    fn logos_slice(&self, start: i64, end: i64) -> &[T];
    /// The items a counted loop from `first` through `last` reads, under [`logos_run_through`].
    fn logos_run_through(&self, first: i64, last: i64) -> &[T];
    /// The items a counted loop from `first` up to, not including, `stop` reads, under [`logos_run_before`].
    fn logos_run_before(&self, first: i64, stop: i64) -> &[T];
    /// `While lo < hi: swap items lo and hi, then step lo up and hi down`, under the one index rule: the
    /// slice's own (vectorized) reversal when both ends count from the front and name items, and the loop
    /// itself, index by index, otherwise.
    fn logos_reverse_between(&mut self, lo: i64, hi: i64);
}

impl<T> LogosItems<T> for [T] {
    #[inline(always)]
    fn logos_item(&self, index: i64) -> &T {
        let offset = resolve_logos_index(index, self.len());
        // SAFETY: `resolve_logos_index` returns an offset below `len` or does not return.
        unsafe { self.get_unchecked(offset) }
    }

    #[inline(always)]
    fn logos_item_mut(&mut self, index: i64) -> &mut T {
        let offset = resolve_logos_index(index, self.len());
        // SAFETY: as in `logos_item`.
        unsafe { self.get_unchecked_mut(offset) }
    }

    #[inline(always)]
    fn logos_item_front(&self, index: i64) -> &T {
        debug_assert!(index >= 1, "a front index is at least 1, not {index}");
        let offset = index.wrapping_sub(1) as u64;
        if offset < self.len() as u64 {
            // SAFETY: `offset` is below `len`.
            unsafe { self.get_unchecked(offset as usize) }
        } else {
            no_item_at(index)
        }
    }

    #[inline(always)]
    fn logos_item_front_mut(&mut self, index: i64) -> &mut T {
        debug_assert!(index >= 1, "a front index is at least 1, not {index}");
        let offset = index.wrapping_sub(1) as u64;
        if offset < self.len() as u64 {
            // SAFETY: `offset` is below `len`.
            unsafe { self.get_unchecked_mut(offset as usize) }
        } else {
            no_item_at(index)
        }
    }

    #[inline]
    fn logos_slice(&self, start: i64, end: i64) -> &[T] {
        &self[logos_slice_range(start, end, self.len())]
    }

    #[inline]
    fn logos_run_through(&self, first: i64, last: i64) -> &[T] {
        &self[logos_run_through(first, last, self.len())]
    }

    #[inline]
    fn logos_run_before(&self, first: i64, stop: i64) -> &[T] {
        &self[logos_run_before(first, stop, self.len())]
    }

    fn logos_reverse_between(&mut self, lo: i64, hi: i64) {
        if lo >= 1 && lo < hi && hi as u64 <= self.len() as u64 {
            self[(lo - 1) as usize..hi as usize].reverse();
            return;
        }
        let (mut lo, mut hi) = (lo, hi);
        while lo < hi {
            let a = resolve_logos_index(lo, self.len());
            let b = resolve_logos_index(hi, self.len());
            self.swap(a, b);
            lo += 1;
            hi -= 1;
        }
    }
}

// === Vec<T> with i64 (1-based indexing) ===

impl<T: Clone> LogosIndex<i64> for Vec<T> {
    type Output = T;

    #[inline(always)]
    fn logos_get(&self, index: i64) -> T {
        let idx = resolve_logos_index(index, self.len());
        unsafe { self.get_unchecked(idx).clone() }
    }
}

impl<T: Clone> LogosIndexMut<i64> for Vec<T> {
    #[inline(always)]
    fn logos_set(&mut self, index: i64, value: T) {
        let idx = resolve_logos_index(index, self.len());
        unsafe {
            *self.get_unchecked_mut(idx) = value;
        }
    }
}

// === LogosTuple (a newtype over Vec<Value>) with i64 (1-based indexing) ===
// UFCS `LogosIndex::logos_get(&t, n)` does NOT deref-coerce, so the tuple newtype needs its own impl
// (delegating to the inner Vec) for `item N of t` / `t[n]`.
impl LogosIndex<i64> for crate::types::LogosTuple {
    type Output = crate::types::Value;

    #[inline(always)]
    fn logos_get(&self, index: i64) -> crate::types::Value {
        let idx = resolve_logos_index(index, self.0.len());
        unsafe { self.0.get_unchecked(idx).clone() }
    }
}

impl LogosIndexMut<i64> for crate::types::LogosTuple {
    #[inline(always)]
    fn logos_set(&mut self, index: i64, value: crate::types::Value) {
        let idx = resolve_logos_index(index, self.0.len());
        unsafe {
            *self.0.get_unchecked_mut(idx) = value;
        }
    }
}

// === A map entry `(key, value)` with i64 (1-based indexing): what `Repeat for entry in m` binds ===
// The generated Rust reads a constant `item 1`/`item 2` as the key or the value itself; any other index reads
// either one as a `Value`, and one past both fails as any index past the end does.
impl<K: Clone + Into<crate::types::Value>, V: Clone + Into<crate::types::Value>> LogosIndex<i64> for (K, V) {
    type Output = crate::types::Value;

    #[inline(always)]
    fn logos_get(&self, index: i64) -> crate::types::Value {
        match resolve_logos_index(index, 2) {
            0 => self.0.clone().into(),
            _ => self.1.clone().into(),
        }
    }
}

// === [T] slice with i64 (1-based indexing, used by &mut [T] borrow params) ===

impl<T: Clone> LogosIndex<i64> for [T] {
    type Output = T;

    #[inline(always)]
    fn logos_get(&self, index: i64) -> T {
        let idx = resolve_logos_index(index, self.len());
        unsafe { self.get_unchecked(idx).clone() }
    }
}

impl<T: Clone> LogosIndexMut<i64> for [T] {
    #[inline(always)]
    fn logos_set(&mut self, index: i64, value: T) {
        let idx = resolve_logos_index(index, self.len());
        unsafe {
            *self.get_unchecked_mut(idx) = value;
        }
    }
}

// === &mut [T] and &[T] with i64 (thin wrappers for UFCS compatibility) ===
//
// When the codegen emits `LogosIndex::logos_get(&arr, i)` where `arr: &mut [T]`,
// the first argument is `&&mut [T]` (or `&&[T]` for a borrowed slice). Rust doesn't auto-coerce
// these to `&[T]` in UFCS, so each has an explicit impl that delegates to the `[T]` impl.

impl<T: Clone> LogosIndex<i64> for &mut [T] {
    type Output = T;

    #[inline(always)]
    fn logos_get(&self, index: i64) -> T {
        <[T] as LogosIndex<i64>>::logos_get(self, index)
    }
}

impl<T: Clone> LogosIndex<i64> for &[T] {
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
        // Positive indexes keep the count-free ASCII fast path; only an
        // end-relative (or zero) index pays the char count.
        let idx = if index >= 1 {
            (index - 1) as usize
        } else {
            resolve_logos_index(index, self.chars().count())
        };
        match self.as_bytes().get(idx) {
            Some(&b) if b.is_ascii() => {
                // Fast path: ASCII byte
                String::from(b as char)
            }
            _ => {
                // Slow path: Unicode or out of bounds
                self.chars()
                    .nth(idx)
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| panic!("{}", crate::failure::index_out_of_bounds(index)))
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
        let idx = if index >= 1 {
            (index - 1) as usize
        } else {
            resolve_logos_index(index, self.chars().count())
        };
        match self.as_bytes().get(idx) {
            Some(&b) if b.is_ascii() => b as char,
            _ => self
                .chars()
                .nth(idx)
                .unwrap_or_else(|| panic!("{}", crate::failure::index_out_of_bounds(index))),
        }
    }
}

// === LogosSeq<T> with i64 (1-based indexing, reference semantics) ===

impl<T: Clone> LogosIndex<i64> for crate::types::LogosSeq<T> {
    type Output = T;

    #[inline(always)]
    fn logos_get(&self, index: i64) -> T {
        let inner = self.borrow();
        <Vec<T> as LogosIndex<i64>>::logos_get(&*inner, index)
    }
}

impl<T: Clone> LogosIndexMut<i64> for crate::types::LogosSeq<T> {
    #[inline(always)]
    fn logos_set(&mut self, index: i64, value: T) {
        let mut inner = self.borrow_mut();
        <Vec<T> as LogosIndexMut<i64>>::logos_set(&mut *inner, index, value)
    }
}

// === LogosMap<K, V> with K (key-based indexing, reference semantics) ===

impl<K: Eq + Hash + crate::failure::FailureText, V: Clone> LogosIndex<K>
    for crate::types::LogosMap<K, V>
{
    type Output = V;

    #[inline(always)]
    fn logos_get(&self, key: K) -> V {
        let inner = self.borrow();
        inner
            .get(&key)
            .cloned()
            .unwrap_or_else(|| panic!("{}", crate::failure::key_not_found(&key)))
    }
}

impl<K: Eq + Hash + crate::failure::FailureText, V: Clone> LogosIndexMut<K>
    for crate::types::LogosMap<K, V>
{
    #[inline(always)]
    fn logos_set(&mut self, key: K, value: V) {
        self.insert(key, value);
    }
}

// === &str convenience for LogosMap<String, V> ===

impl<V: Clone> LogosIndex<&str> for crate::types::LogosMap<String, V> {
    type Output = V;

    #[inline(always)]
    fn logos_get(&self, key: &str) -> V {
        let inner = self.borrow();
        inner
            .get(key)
            .cloned()
            .unwrap_or_else(|| panic!("{}", crate::failure::key_not_found(key)))
    }
}

impl<V: Clone> LogosIndexMut<&str> for crate::types::LogosMap<String, V> {
    #[inline(always)]
    fn logos_set(&mut self, key: &str, value: V) {
        self.insert(key.to_string(), value);
    }
}

// === HashMap<K, V> with K (key-based indexing) ===

impl<K: Eq + Hash + crate::failure::FailureText, V: Clone> LogosIndex<K> for FxHashMap<K, V> {
    type Output = V;

    #[inline(always)]
    fn logos_get(&self, key: K) -> V {
        self.get(&key)
            .cloned()
            .unwrap_or_else(|| panic!("{}", crate::failure::key_not_found(&key)))
    }
}

impl<K: Eq + Hash + crate::failure::FailureText, V: Clone> LogosIndexMut<K> for FxHashMap<K, V> {
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
        self.get(key)
            .cloned()
            .unwrap_or_else(|| panic!("{}", crate::failure::key_not_found(key)))
    }
}

impl<V: Clone> LogosIndexMut<&str> for FxHashMap<String, V> {
    #[inline(always)]
    fn logos_set(&mut self, key: &str, value: V) {
        self.insert(key.to_string(), value);
    }
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_data/indexing/tests.rs"]
mod tests;
