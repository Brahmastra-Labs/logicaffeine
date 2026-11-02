//! ★ LOCK — walking a recursive value on the VM costs the same per node however deep the tree is. ★
//!
//! `check (tree: Tree) -> Int` inspects its tree and recurses into both subtrees. Binding `left` and `right` in the
//! `Inspect` arm copied each subtree whole on the VM — every node below, with its type and constructor names — so a
//! walk re-copied every node once per level above it: O(N · depth) allocations, and the Benchmarks Game binary-trees
//! program took 80 s at depth 16 through `largo run --interpret` against C's 0.12 s. Under value semantics an enum
//! value is immutable once built, so a binding can share it; the walk then allocates the same few things per node at
//! any depth.
//!
//! ★ THE YARDSTICK IS AN ALLOCATION COUNT, NOT A CLOCK. This binary's allocator counts the allocations the calling
//! thread makes while the program runs. The walk's share is a run that walks the tree four times less the same program
//! walking it twice; divided by the node count, it stays flat from depth 6 to depth 12 when the walk shares, and grows
//! with the depth when it copies. Each program's answer is checked on the VM and the tree-walker, so the sharing
//! is also a value-semantics proof: no walk sees a subtree another binding changed.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test vm_enum_walk_lock`

#![cfg(not(target_arch = "wasm32"))]

mod common;

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use common::assert_all_tiers;
use logicaffeine_compile::compile::vm_run_source;

struct Counting;

thread_local! {
    static COUNTING: Cell<bool> = const { Cell::new(false) };
    static ALLOCATIONS: Cell<u64> = const { Cell::new(0) };
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        count();
        System.alloc(layout)
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        count();
        System.realloc(ptr, layout, new_size)
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        System.dealloc(ptr, layout)
    }
}

fn count() {
    let _ = COUNTING.try_with(|on| {
        if on.get() {
            let _ = ALLOCATIONS.try_with(|a| a.set(a.get() + 1));
        }
    });
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

/// The allocations the VM makes running `src` (parse included — the same for every depth), and its output. The program
/// runs once uncounted first, so the process's one-time work (statics, caches) lands in no measurement.
fn allocations_to_run(src: &str) -> (u64, String) {
    vm_run_source(src).unwrap_or_else(|e| panic!("runs on the VM:\n{src}\n=> {e}"));
    ALLOCATIONS.with(|a| a.set(0));
    COUNTING.with(|on| on.set(true));
    let out = vm_run_source(src);
    COUNTING.with(|on| on.set(false));
    let out = out.unwrap_or_else(|e| panic!("runs on the VM:\n{src}\n=> {e}"));
    (ALLOCATIONS.with(Cell::get), out)
}

const TREE: &str = "## A Tree is either:
    A Leaf.
    A Node with left Tree and right Tree.

## To build (depth: Int) -> Tree:
    If depth equals 0:
        Return a new Leaf.
    Return a new Node with left build(depth - 1) and right build(depth - 1).

## To check (tree: Tree) -> Int:
    Inspect tree:
        When Leaf: Return 1.
        When Node (left, right): Return 1 + check(left) + check(right).
";

/// Build a depth-`d` tree and walk it `walks` times, printing the node count each time.
fn program(d: u32, walks: u32) -> String {
    format!(
        "{TREE}
## Main
Let t be build({d}).
Let mutable total be 0.
Repeat for i from 1 to {walks}:
    Set total to total + check(t).
Show total.
"
    )
}

/// The walk's allocations per node of a depth-`d` tree: a run that walks it four times, less the same program walking
/// it twice — everything but two walks cancels — divided by the nodes those two walks visit.
fn walk_allocations_per_node(d: u32) -> f64 {
    let nodes = (1u64 << (d + 1)) - 1;
    let (twice, twice_out) = allocations_to_run(&program(d, 2));
    let (four, four_out) = allocations_to_run(&program(d, 4));
    assert_eq!(
        twice_out.trim(),
        (2 * nodes).to_string(),
        "two walks count every node twice"
    );
    assert_eq!(
        four_out.trim(),
        (4 * nodes).to_string(),
        "four walks count every node four times"
    );
    assert!(
        four >= twice,
        "two more walks cannot allocate less: {four} < {twice}"
    );
    (four - twice) as f64 / (2 * nodes) as f64
}

#[test]
fn a_walk_allocates_the_same_per_node_at_any_depth() {
    let shallow = walk_allocations_per_node(6);
    let deep = walk_allocations_per_node(12);
    assert!(
        deep <= shallow * 1.3 + 0.5,
        "a depth-12 walk allocates {deep:.2} per node against {shallow:.2} at depth 6: the walk copies subtrees \
         (O(N · depth)) instead of sharing them"
    );
}

#[test]
fn a_shared_subtree_keeps_its_value_when_another_binding_is_rebuilt() {
    let src = format!(
        "{TREE}
## To leftOf (tree: Tree) -> Tree:
    Inspect tree:
        When Leaf: Return tree.
        When Node (left, right): Return left.

## Main
Let mutable t be build(4).
Let l be leftOf(t).
Set t to build(1).
Show check(l).
Show check(t).
"
    );
    assert_all_tiers(&src, "15\n3");
}
