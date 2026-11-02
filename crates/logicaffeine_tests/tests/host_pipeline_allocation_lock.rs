//! Repeated host evaluations must release allocations once results and private tiers are dropped.
//! This counts both live allocations and bytes; comparing large and small programs alone would
//! miss a fixed leak on every evaluation.

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::codegen_c::refused_program;
use logicaffeine_compile::compile::{
    compile_to_c, compile_to_program, compile_to_program_optimized, compile_to_rust, tw_outcome,
    vm_outcome, vm_outcome_tiered,
};
use logicaffeine_compile::optimization::Tier;
use logicaffeine_jit::ForgeTier;
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

struct Counting;
thread_local! {
    static LIVE: Cell<(isize, isize, usize)> = const { Cell::new((0, 0, 0)) };
}

fn record(allocations: isize, bytes: isize) {
    let _ = LIVE.try_with(|value| {
        let (n, b, total) = value.get();
        value.set((
            n + allocations,
            b + bytes,
            total + usize::from(allocations > 0),
        ));
    });
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = System.alloc(layout);
        if !ptr.is_null() {
            record(1, layout.size() as isize);
        }
        ptr
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let ptr = System.alloc_zeroed(layout);
        if !ptr.is_null() {
            record(1, layout.size() as isize);
        }
        ptr
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        record(-1, -(layout.size() as isize));
        System.dealloc(ptr, layout);
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        let result = System.realloc(ptr, layout, size);
        if !result.is_null() {
            record(0, size as isize - layout.size() as isize);
        }
        result
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

// A recursive enum exercises nested AST vectors, function metadata, optimizer clones,
// bytecode ownership, runtime values, and the synchronous native tier.
const SOURCE: &str = "## A Tree is either:
    A Leaf.
    A Node with left Tree and right Tree.

## To grow (depth: Int) -> Tree:
    If depth equals 0:
        Return a new Leaf.
    Return a new Node with left grow(depth - 1) and right grow(depth - 1).

## To count (tree: Tree) -> Int:
    Inspect tree:
        When Leaf: Return 1.
        When Node (left, right): Return 1 + count(left) + count(right).

## Main
Let mutable total be 0.
Repeat for i from 1 to 40:
    Set total to total + count(grow(3)).
Show total.
";

fn balanced(mut run: impl FnMut()) {
    // Initialize bounded per-thread tables before measuring steady-state work.
    for _ in 0..3 {
        run();
    }
    let before = LIVE.with(Cell::get);
    for _ in 0..16 {
        run();
    }
    let after = LIVE.with(Cell::get);
    assert_eq!(
        (after.0 - before.0, after.1 - before.1),
        (0, 0),
        "sixteen completed evaluations retained allocations or bytes"
    );
}

fn correct(run: logicaffeine_compile::compile::RunOutcome) {
    assert_eq!(run.error, None);
    assert!(
        run.output == "600" || run.output == "600\n",
        "{:?}",
        run.output
    );
}

#[test]
fn repeated_tree_walker_runs_release_host_allocations() {
    balanced(|| correct(tw_outcome(SOURCE)));
}

#[test]
fn repeated_vm_runs_release_host_allocations() {
    balanced(|| correct(vm_outcome(SOURCE)));
}

#[test]
fn repeated_optimized_vm_runs_release_host_allocations() {
    balanced(|| correct(vm_outcome_tiered(SOURCE, &[], Tier::T3, None)));
}

#[test]
fn repeated_private_native_tiers_release_host_allocations() {
    balanced(|| {
        let tier = ForgeTier::new();
        correct(vm_outcome_tiered(SOURCE, &[], Tier::T3, Some(&tier)));
        #[cfg(target_arch = "x86_64")]
        assert!(
            tier.function_counts().1 >= 2,
            "both enum functions must reach native code"
        );
    });
}

#[test]
fn repeated_bytecode_compiles_release_host_allocations() {
    balanced(|| {
        drop(compile_to_program(SOURCE).expect("valid program"));
    });
}

#[test]
fn repeated_optimized_bytecode_compiles_release_host_allocations() {
    balanced(|| {
        drop(compile_to_program_optimized(SOURCE).expect("valid program"));
    });
}

#[test]
fn repeated_rust_compiles_release_host_allocations() {
    balanced(|| {
        drop(compile_to_rust(SOURCE).expect("valid program"));
    });
}

#[test]
fn repeated_c_compiles_release_host_allocations() {
    balanced(|| {
        drop(compile_to_c(SOURCE).expect("valid program"));
    });
}

#[test]
fn repeated_parse_failures_release_host_allocations() {
    const INVALID: &str = "## Main\nLet values be [1, 2, 3].\nLet broken be .\n";
    balanced(|| {
        assert!(tw_outcome(INVALID).error.is_some());
        let refusal = vm_outcome(INVALID)
            .error
            .expect("the VM refuses the program");
        assert!(compile_to_rust(INVALID).is_err());
        assert_eq!(
            compile_to_c(INVALID)
                .expect("a program the VM refuses compiles to the C that reports the refusal"),
            refused_program(&refusal)
        );
    });
}

#[test]
fn resetting_a_warmed_arena_reuses_its_storage_without_allocating() {
    let mut arena = logicaffeine_base::Arena::new();
    for epoch in 0..19 {
        let before = LIVE.with(Cell::get);
        for value in 0..10_000 {
            arena.alloc(value);
        }
        arena.alloc_slice(0..1_000);
        arena.reset();
        let after = LIVE.with(Cell::get);
        if epoch >= 3 {
            assert_eq!(after, before, "reset must preserve reusable capacity");
        }
    }
}
