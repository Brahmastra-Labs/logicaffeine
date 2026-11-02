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

use std::cell::Cell;
use std::rc::Rc;

struct DropProbe {
    id: usize,
    dropped: Rc<Cell<usize>>,
}

impl Drop for DropProbe {
    fn drop(&mut self) {
        self.dropped.set(self.dropped.get() + 1);
    }
}

#[test]
fn dropping_the_arena_releases_single_and_slice_values_once() {
    let dropped = Rc::new(Cell::new(0));
    {
        let arena = Arena::new();
        let first = arena.alloc(DropProbe {
            id: 0,
            dropped: dropped.clone(),
        });
        let slice = arena.alloc_slice((1..101).map(|id| DropProbe {
            id,
            dropped: dropped.clone(),
        }));
        assert_eq!(first.id, 0);
        assert_eq!(slice[99].id, 100);
        assert_eq!(dropped.get(), 0);
    }
    assert_eq!(dropped.get(), 101);
    assert_eq!(Rc::strong_count(&dropped), 1);
}

#[test]
fn reset_releases_values_and_the_next_epoch_drops_only_its_own_values() {
    let dropped = Rc::new(Cell::new(0));
    let mut arena = Arena::new();
    for epoch in 0..4 {
        for id in 0..37 {
            arena.alloc(DropProbe {
                id,
                dropped: dropped.clone(),
            });
        }
        assert_eq!(dropped.get(), epoch * 37);
        arena.reset();
        assert_eq!(dropped.get(), (epoch + 1) * 37);
    }
    arena.alloc(DropProbe {
        id: 999,
        dropped: dropped.clone(),
    });
    drop(arena);
    assert_eq!(dropped.get(), 149);
    assert_eq!(Rc::strong_count(&dropped), 1);
}

#[test]
fn growth_keeps_references_stable_and_drops_every_owned_value() {
    let dropped = Rc::new(Cell::new(0));
    {
        let arena = Arena::new();
        let refs: Vec<_> = (0..10_000)
            .map(|id| {
                arena.alloc(DropProbe {
                    id,
                    dropped: dropped.clone(),
                })
            })
            .collect();
        for (id, value) in refs.iter().enumerate() {
            assert_eq!(value.id, id);
        }
        assert_eq!(dropped.get(), 0);
    }
    assert_eq!(dropped.get(), 10_000);
    assert_eq!(Rc::strong_count(&dropped), 1);
}

thread_local! {
    static ZERO_SIZED_DROPS: Cell<usize> = const { Cell::new(0) };
}

struct ZeroSizedDrop;

impl Drop for ZeroSizedDrop {
    fn drop(&mut self) {
        ZERO_SIZED_DROPS.with(|n| n.set(n.get() + 1));
    }
}

#[test]
fn zero_sized_values_are_dropped_on_reset_and_arena_drop() {
    ZERO_SIZED_DROPS.with(|n| n.set(0));
    let mut arena = Arena::new();
    arena.alloc(ZeroSizedDrop);
    arena.alloc_slice((0..100).map(|_| ZeroSizedDrop));
    arena.reset();
    ZERO_SIZED_DROPS.with(|n| assert_eq!(n.get(), 101));
    arena.alloc_slice((0..23).map(|_| ZeroSizedDrop));
    drop(arena);
    ZERO_SIZED_DROPS.with(|n| assert_eq!(n.get(), 124));
}

#[test]
fn panicking_slice_iterator_does_not_leak_initialized_values() {
    let dropped = Rc::new(Cell::new(0));
    let mut arena = Arena::new();
    arena.alloc(DropProbe {
        id: 999,
        dropped: dropped.clone(),
    });
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        arena.alloc_slice((0..100).map(|id| {
            assert!(id != 7, "intentional iterator panic");
            DropProbe {
                id,
                dropped: dropped.clone(),
            }
        }));
    }));
    assert!(result.is_err());
    arena.reset();
    assert_eq!(dropped.get(), 8);
    arena.alloc(DropProbe {
        id: 1000,
        dropped: dropped.clone(),
    });
    drop(arena);
    assert_eq!(dropped.get(), 9);
    assert_eq!(Rc::strong_count(&dropped), 1);
}

#[test]
fn panicking_destructor_does_not_leak_the_other_initialized_values() {
    struct PanicsOnce {
        id: usize,
        dropped: Rc<Cell<usize>>,
    }
    impl Drop for PanicsOnce {
        fn drop(&mut self) {
            self.dropped.set(self.dropped.get() + 1);
            assert!(self.id != 2, "intentional destructor panic");
        }
    }

    let dropped = Rc::new(Cell::new(0));
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let arena = Arena::new();
        arena.alloc_slice((0..10).map(|id| PanicsOnce {
            id,
            dropped: dropped.clone(),
        }));
        drop(arena);
    }));
    assert!(result.is_err());
    assert_eq!(dropped.get(), 10);
    assert_eq!(Rc::strong_count(&dropped), 1);
}

#[test]
fn a_slice_iterator_can_allocate_from_the_same_arena() {
    let arena = Arena::new();
    let first = arena.alloc(7);
    let slice = arena.alloc_slice((0..100).map(|n| *arena.alloc(n * 2) + 1));
    assert_eq!(*first, 7);
    assert_eq!(slice, &(0..100).map(|n| n * 2 + 1).collect::<Vec<_>>()[..]);
}

#[test]
fn arena_values_can_refer_to_other_values_in_the_same_arena() {
    struct Node<'a> {
        value: usize,
        links: Vec<&'a Node<'a>>,
    }
    let arena = Arena::new();
    let leaf = arena.alloc(Node {
        value: 3,
        links: vec![],
    });
    let root = arena.alloc(Node {
        value: 5,
        links: vec![leaf, leaf],
    });
    for value in 0..10_000 {
        arena.alloc(Node {
            value,
            links: vec![root],
        });
    }
    assert_eq!(
        root.value + root.links.iter().map(|n| n.value).sum::<usize>(),
        11
    );
}
