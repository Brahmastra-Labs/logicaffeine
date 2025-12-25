use bumpalo::Bump;

pub struct Arena<T> {
    bump: Bump,
    _marker: std::marker::PhantomData<T>,
}

impl<T> Arena<T> {
    pub fn new() -> Self {
        Arena {
            bump: Bump::new(),
            _marker: std::marker::PhantomData,
        }
    }

    pub fn alloc(&self, value: T) -> &T {
        self.bump.alloc(value)
    }

    pub fn alloc_slice<I>(&self, items: I) -> &[T]
    where
        I: IntoIterator<Item = T>,
        I::IntoIter: ExactSizeIterator,
    {
        self.bump.alloc_slice_fill_iter(items)
    }

    /// Resets the arena, invalidating all references but keeping allocated capacity.
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
