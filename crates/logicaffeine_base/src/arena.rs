//! Arena allocation for stable AST references.
//!
//! This module provides bump allocation for AST nodes, ensuring references
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

use bumpalo::Bump;

/// A bump allocator for stable, arena-allocated references.
///
/// Values allocated in an arena live until the arena is dropped or reset.
/// References remain valid across subsequent allocations, making this ideal
/// for AST nodes that reference each other.
pub struct Arena<T> {
    bump: Bump,
    _marker: std::marker::PhantomData<T>,
}

impl<T> Arena<T> {
    /// Creates an empty arena.
    pub fn new() -> Self {
        Arena {
            bump: Bump::new(),
            _marker: std::marker::PhantomData,
        }
    }

    /// Allocates a value and returns a reference valid for the arena's lifetime.
    pub fn alloc(&self, value: T) -> &T {
        self.bump.alloc(value)
    }

    /// Allocates a slice from an iterator.
    ///
    /// The iterator must implement [`ExactSizeIterator`] so the arena can
    /// pre-allocate the correct amount of space.
    pub fn alloc_slice<I>(&self, items: I) -> &[T]
    where
        I: IntoIterator<Item = T>,
        I::IntoIter: ExactSizeIterator,
    {
        self.bump.alloc_slice_fill_iter(items)
    }

    /// Resets the arena, invalidating all references but keeping allocated capacity.
    ///
    /// This enables zero-allocation REPL loops by reusing memory.
    pub fn reset(&mut self) {
        self.bump.reset();
    }
}

impl<T> Default for Arena<T> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alloc_returns_stable_reference() {
        let arena: Arena<i32> = Arena::new();
        let r1 = arena.alloc(42);
        let r2 = arena.alloc(100);
        assert_eq!(*r1, 42);
        assert_eq!(*r2, 100);
    }

    #[test]
    fn references_remain_valid_after_many_allocations() {
        let arena: Arena<i32> = Arena::new();
        let refs: Vec<&i32> = (0..10000).map(|i| arena.alloc(i)).collect();
        for (i, r) in refs.iter().enumerate() {
            assert_eq!(**r, i as i32);
        }
    }

    #[test]
    fn works_with_structs() {
        #[derive(Debug, PartialEq)]
        struct Point {
            x: i32,
            y: i32,
        }

        let arena: Arena<Point> = Arena::new();
        let p1 = arena.alloc(Point { x: 1, y: 2 });
        let p2 = arena.alloc(Point { x: 3, y: 4 });
        assert_eq!(p1, &Point { x: 1, y: 2 });
        assert_eq!(p2, &Point { x: 3, y: 4 });
    }

    #[test]
    fn alloc_slice_works() {
        let arena: Arena<i32> = Arena::new();
        let slice = arena.alloc_slice([1, 2, 3]);
        assert_eq!(slice, &[1, 2, 3]);
    }

    #[test]
    fn alloc_slice_from_vec() {
        let arena: Arena<i32> = Arena::new();
        let vec = vec![10, 20, 30];
        let slice = arena.alloc_slice(vec);
        assert_eq!(slice, &[10, 20, 30]);
    }

    #[test]
    fn alloc_empty_slice() {
        let arena: Arena<i32> = Arena::new();
        let empty: Vec<i32> = vec![];
        let slice = arena.alloc_slice(empty);
        assert!(slice.is_empty());
    }
}
