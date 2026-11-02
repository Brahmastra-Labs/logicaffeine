//! Arena allocation for stable AST references.
//!
//! This module provides owning allocation for AST nodes, ensuring references
//! remain valid throughout parsing and semantic analysis. The arena pattern
//! eliminates reference counting overhead and enables zero-copy parsing.
//!
//! ## Example
//!
//! ```
//! use logicaffeine_base::Arena;
//!
//! let arena: Arena<String> = Arena::new();
//! let s1 = arena.alloc("hello".to_string());
//! let s2 = arena.alloc("world".to_string());
//!
//! // Both references remain valid as long as arena is alive
//! assert_eq!(s1, "hello");
//! assert_eq!(s2, "world");
//! ```
//!
//! ## REPL Reuse
//!
//! For interactive use, call [`Arena::reset`] between evaluations to
//! reclaim memory while keeping allocated capacity.

use std::cell::RefCell;

/// An owning arena for stable references.
///
/// Values stay at their allocated addresses until the arena is dropped or reset.
/// Their destructors run at that boundary, including destructors of owned AST
/// vectors and strings.
pub struct Arena<T> {
    chunks: RefCell<Chunks<T>>,
}

// Owning Vec<T>s give both ordinary drop glue and Rust's drop checking access to T.
// Moving a Vec header never moves its elements; a published chunk never reallocates.
struct Chunks<T> {
    values: Vec<Vec<T>>,
    active: usize,
}

impl<T> Arena<T> {
    /// Creates an empty arena.
    pub fn new() -> Self {
        Self {
            chunks: RefCell::new(Chunks {
                values: Vec::new(),
                active: 0,
            }),
        }
    }

    /// Allocates a value and returns a reference valid for the arena's lifetime.
    pub fn alloc(&self, value: T) -> &T {
        let mut chunks = self.chunks.borrow_mut();
        let has_room = chunks.active > 0 && {
            let current = &chunks.values[chunks.active - 1];
            current.len() < current.capacity()
        };
        if !has_room {
            let next = chunks.active;
            if next == chunks.values.len() {
                let initial = (4096 / std::mem::size_of::<T>().max(1)).max(1);
                let previous = chunks.values.last().map_or(0, Vec::capacity);
                let capacity = previous.saturating_mul(2).max(initial);
                chunks.values.push(Vec::with_capacity(capacity));
            }
            debug_assert!(chunks.values[next].is_empty());
            chunks.active += 1;
        }
        let current = chunks.active - 1;
        let chunk = &mut chunks.values[current];
        let index = chunk.len();
        debug_assert!(index < chunk.capacity());
        chunk.push(value);
        // SAFETY: push stayed within the existing capacity. This and every later
        // allocation leave previously published elements at their addresses. The
        // buffer is owned by self; reset needs exclusive access and drop ends the
        // borrow. Do not index through a mutable slice spanning earlier values.
        unsafe { &*chunk.as_ptr().add(index) }
    }

    /// Allocates a contiguous slice from an iterator.
    ///
    /// The iterator must implement ExactSizeIterator so its allocation can be
    /// reserved up front. It may itself allocate from this arena.
    pub fn alloc_slice<I>(&self, items: I) -> &[T]
    where
        I: IntoIterator<Item = T>,
        I::IntoIter: ExactSizeIterator,
    {
        let iter = items.into_iter();
        let length = iter.len();
        // Take an unused buffer without holding the RefCell borrow while user
        // iterator code runs. Its Vec owns partial initialization if that code
        // panics; no initialized element can be forgotten.
        let mut buffer = {
            let mut chunks = self.chunks.borrow_mut();
            let spare = (chunks.active..chunks.values.len())
                .find(|&i| chunks.values[i].capacity() >= length);
            match spare {
                Some(i) => chunks.values.swap_remove(i),
                None => Vec::with_capacity(length),
            }
        };
        debug_assert!(buffer.is_empty());
        buffer.extend(iter);
        if buffer.is_empty() {
            return &[];
        }
        let pointer = buffer.as_ptr();
        let length = buffer.len();
        let mut chunks = self.chunks.borrow_mut();
        let next = chunks.active;
        chunks.values.insert(next, buffer);
        chunks.active += 1;
        // SAFETY: moving the Vec into the arena preserves its buffer address.
        // Future scalar allocations check its capacity before pushing, and slice
        // allocations use separate unused buffers. The shared arena borrow keeps
        // reset and drop from running while the returned slice is live.
        unsafe { std::slice::from_raw_parts(pointer, length) }
    }

    /// Drops every value, invalidating references while retaining reusable storage.
    ///
    /// Repeated evaluations can reuse the arena without retaining AST-owned data.
    pub fn reset(&mut self) {
        let chunks = self.chunks.get_mut();
        for chunk in &mut chunks.values {
            chunk.clear();
        }
        chunks.active = 0;
    }
}

impl<T> Default for Arena<T> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_base/arena/tests.rs"]
mod tests;
