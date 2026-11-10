//! Compiling a program to Rust does each function's work once: a program of four times the functions
//! allocates about four times as much, never sixteen. What the codegen knows about every function at
//! once (its calling roles, its return type) is built once per program and read by each function's
//! codegen, not copied into it.

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::compile::compile_to_rust;
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

struct Counting;

thread_local! {
    static ALLOCATIONS: Cell<u64> = const { Cell::new(0) };
}

fn record() {
    let _ = ALLOCATIONS.try_with(|n| n.set(n.get() + 1));
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        record();
        System.alloc(layout)
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        record();
        System.alloc_zeroed(layout)
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        System.dealloc(ptr, layout);
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        record();
        System.realloc(ptr, layout, size)
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

/// The smaller program's function count; the larger holds [`GROWTH`] times as many.
const FUNCTIONS: usize = 150;

const GROWTH: usize = 4;

/// The most the allocations may grow by when the functions grow [`GROWTH`]-fold: linear work grows by
/// [`GROWTH`] and the program's fixed work by less, and work quadratic in the functions grows by
/// `GROWTH²`.
const MOST_GROWTH: f64 = 5.0;

/// A program of `functions` functions, each borrowing a sequence (a calling role) and declaring its
/// return type, all called from a Main whose sequence the compiler cannot know.
fn program(functions: usize) -> String {
    let mut source = String::new();
    for k in 1..=functions {
        source.push_str(&format!("## To f{k} (xs: Seq of Int) -> Int:\n    Return length of xs + {k}.\n\n"));
    }
    source.push_str("## Main\nLet mutable xs be a new Seq of Int.\nLet n be length of args().\nLet mutable i be 0.\n");
    source.push_str("While i is less than n:\n    Push i to xs.\n    Set i to i + 1.\nLet mutable total be 0.\n");
    for k in 1..=functions {
        source.push_str(&format!("Set total to total + f{k}(xs).\n"));
    }
    source.push_str("Show total.\n");
    source
}

/// The allocations compiling `source` to Rust makes, and the Rust.
fn compiled(source: &str) -> (u64, String) {
    let before = ALLOCATIONS.with(Cell::get);
    let rust = compile_to_rust(source).expect("the program compiles to Rust");
    (ALLOCATIONS.with(Cell::get) - before, rust)
}

#[test]
fn compiling_to_rust_is_linear_in_the_function_count() {
    compiled(&program(8));
    let (small, small_rust) = compiled(&program(FUNCTIONS));
    let (large, large_rust) = compiled(&program(FUNCTIONS * GROWTH));
    for (rust, functions) in [(&small_rust, FUNCTIONS), (&large_rust, FUNCTIONS * GROWTH)] {
        assert!(
            rust.contains(&format!("fn f{functions}(")),
            "the Rust defines every function, f{functions} among them"
        );
    }
    let growth = large as f64 / small as f64;
    assert!(
        growth <= MOST_GROWTH,
        "{GROWTH} times the functions took {growth:.2} times the allocations ({small} for {FUNCTIONS}, {large} for {}): \
         compiling to Rust does work quadratic in the function count",
        FUNCTIONS * GROWTH
    );
}
