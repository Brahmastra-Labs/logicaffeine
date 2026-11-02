//! ★ LOCK — functions that build and walk the program's own enum values run as native code. ★
//!
//! The Benchmarks Game binary-trees program builds a tree of `Leaf`/`Node` values and walks it back. On the VM with
//! the native tier installed, both functions stayed on the bytecode interpreter: an enum value had no native
//! representation, so `a new Node with …`, `Inspect` and a parameter of an enum type each refused the function.
//!
//! Natively an enum value is a pointer to the VM's own shared node. A parameter or a field read out of a node is
//! BORROWED (the node it came from keeps it alive); a node built by the function or returned by a call is OWNED and
//! is consumed exactly once — stored into a new node's field or returned. A function where that cannot be proven
//! stays on the VM. When native code side-exits mid-function (an integer overflow), every node it holds is handed back
//! to the VM: owned ones transfer, borrowed ones are shared. The allocation count proves nothing leaks either way.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test jit_enum_lock`

#![cfg(not(target_arch = "wasm32"))]

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use logicaffeine_compile::compile::{tw_outcome, vm_outcome_tiered};
use logicaffeine_compile::optimization::Tier;
use logicaffeine_compile::vm::{NativeTier, NATIVE_TIER_THRESHOLD};
use logicaffeine_jit::ForgeTier;

/// Calls from Main that make a non-recursive function hot, with room to spare.
const CALLS: u32 = 3 * NATIVE_TIER_THRESHOLD;

struct Counting;

thread_local! {
    static LIVE: Cell<i64> = const { Cell::new(0) };
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let _ = LIVE.try_with(|l| l.set(l.get() + 1));
        System.alloc(layout)
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        let _ = LIVE.try_with(|l| l.set(l.get() - 1));
        System.dealloc(ptr, layout)
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

/// Run `src` on the optimized run path with `tier` installed; answer its output and how many functions compiled.
fn native(src: &str, tier: &ForgeTier) -> (String, u32) {
    let run = vm_outcome_tiered(src, &[], Tier::T3, Some(tier as &dyn NativeTier));
    assert_eq!(run.error, None, "the native tier refused:\n{src}");
    (run.output.trim_end().to_string(), tier.function_counts().1)
}

fn reference(src: &str) -> String {
    let tw = tw_outcome(src);
    assert_eq!(tw.error, None, "the tree-walker refused:\n{src}");
    tw.output.trim_end().to_string()
}

const TREE: &str = "## A Tree is either:
    A Leaf.
    A Node with left Tree and right Tree.

## To bottomUpTree (depth: Int) -> Tree:
    If depth equals 0:
        Return a new Leaf.
    Return a new Node with left bottomUpTree(depth - 1) and right bottomUpTree(depth - 1).

## To check (tree: Tree) -> Int:
    Inspect tree:
        When Leaf: Return 1.
        When Node (left, right): Return 1 + check(left) + check(right).

";

#[test]
fn a_tree_build_and_walk_compile_to_native_code() {
    let src = format!(
        "{TREE}## Main
Let mutable total be 0.
Repeat for i from 1 to 40:
    Set total to total + check(bottomUpTree(8)).
Show total.
Show check(bottomUpTree(12)).
"
    );
    let tier = ForgeTier::new();
    let (out, functions) = native(&src, &tier);
    assert_eq!(out, reference(&src));
    #[cfg(target_arch = "x86_64")]
    assert!(
        functions >= 2,
        "bottomUpTree and check must both compile to native code; {functions} did"
    );
}

#[test]
fn variant_fields_that_hold_integers_are_read_natively() {
    let src = "## A Tree is either:
    A Leaf.
    A Node with value Int and left Tree and right Tree.

## To grow (depth: Int, v: Int) -> Tree:
    If depth equals 0:
        Return a new Leaf.
    Return a new Node with value v and left grow(depth - 1, v * 2) and right grow(depth - 1, v * 2 + 1).

## To total (tree: Tree) -> Int:
    Inspect tree:
        When Leaf: Return 0.
        When Node (value, left, right): Return value + total(left) + total(right).

## Main
Let mutable sum be 0.
Repeat for i from 1 to 40:
    Set sum to sum + total(grow(7, i)).
Show sum.
";
    let tier = ForgeTier::new();
    let (out, functions) = native(src, &tier);
    assert_eq!(out, reference(src));
    #[cfg(target_arch = "x86_64")]
    assert!(
        functions >= 2,
        "grow and total must both compile to native code; {functions} did"
    );
}

/// The values grow past a machine word deep in the build, after nodes have been built and are still held: the native
/// code side-exits there and the VM finishes the build with every node it was handed.
#[test]
fn a_build_that_overflows_midway_hands_its_nodes_back() {
    let src = "## A Tree is either:
    A Leaf.
    A Node with value Int and left Tree and right Tree.

## To grow (depth: Int, v: Int) -> Tree:
    If depth equals 0:
        Return a new Leaf.
    Let left be grow(depth - 1, v).
    Let w be v * v * v.
    Return a new Node with value w and left left and right grow(depth - 1, v + 1).

## To total (tree: Tree) -> Int:
    Inspect tree:
        When Leaf: Return 0.
        When Node (value, left, right): Return value % 1000003 + total(left) + total(right).

## Main
Let mutable sum be 0.
Repeat for i from 1 to 30:
    Set sum to sum + total(grow(6, i)).
Show sum.
Show total(grow(6, 3000000)).
";
    let tier = ForgeTier::new();
    let (out, _) = native(src, &tier);
    assert_eq!(out, reference(src));
}

/// `src` run twice on one warm tier: how far the second run moved the live allocation count.
fn allocation_drift(src: &str) -> i64 {
    let tier = ForgeTier::new();
    native(src, &tier);
    let before = LIVE.with(Cell::get);
    native(src, &tier);
    LIVE.with(Cell::get) - before
}

/// `program(n)` does `n` units of node work and leaves the allocator where it found it: every node it builds is freed,
/// none twice. A run's own fixed allocations (the compiled program, the pipeline's caches) are the same at any `n`, so
/// a run four times as long ends level with a plain one unless something leaks or double-frees per unit of work.
fn assert_no_leak(program: impl Fn(u32) -> String) {
    let leak = allocation_drift(&program(4 * CALLS)) - allocation_drift(&program(CALLS));
    assert!(
        leak.abs() < (CALLS / 2) as i64,
        "{leak} more live allocations at four times the work: native node code leaks or double-frees per call\n{}",
        program(CALLS)
    );
}

/// Run `src` natively: it prints what the tree-walker prints, and at least `floor` functions compiled.
fn natively_with_floor(src: &str, floor: u32, which: &str) {
    let tier = ForgeTier::new();
    let (out, functions) = native(src, &tier);
    assert_eq!(out, reference(src));
    #[cfg(target_arch = "x86_64")]
    assert!(
        functions >= floor,
        "{which} must compile to native code; {functions} of them did"
    );
    #[cfg(not(target_arch = "x86_64"))]
    let _ = (functions, floor, which);
}

#[test]
fn native_tree_code_leaks_nothing() {
    assert_no_leak(|n| {
        format!(
            "{TREE}## Main
Let mutable total be 0.
Repeat for i from 1 to {n}:
    Set total to total + check(bottomUpTree(7)).
Show total.
"
        )
    });
}

const OVERFLOWING_GROW: &str = "## A Tree is either:
    A Leaf.
    A Node with value Int and left Tree and right Tree.

## To grow (depth: Int, v: Int) -> Tree:
    If depth equals 0:
        Return a new Leaf.
    Let left be grow(depth - 1, v).
    Let w be v * v * v.
    Return a new Node with value w and left left and right grow(depth - 1, v + 1).

## To total (tree: Tree) -> Int:
    Inspect tree:
        When Leaf: Return 0.
        When Node (value, left, right): Return value % 1000003 + total(left) + total(right).

";

/// A side exit hands every node the native frames hold to the VM: the owned ones without a copy and the borrowed ones
/// shared. Every call here side-exits, so a node handed back twice or not at all shows up in proportion to the work.
#[test]
fn a_side_exit_holding_nodes_leaks_nothing() {
    let program = |n: u32| {
        format!(
            "{OVERFLOWING_GROW}## Main
Let mutable sum be 0.
Repeat for i from 1 to {CALLS}:
    Set sum to sum + total(grow(4, i)).
Repeat for i from 1 to {n}:
    Set sum to sum + total(grow(4, 3000000 + i)).
Show sum.
"
        )
    };
    natively_with_floor(&program(CALLS), 2, "grow and total");
    assert_no_leak(program);
}

/// A parameter returned as it came, through a chain of self-calls, and a field of one, are borrowed nodes the caller
/// keeps: each takes a count of its own on the way out.
#[test]
fn a_borrowed_node_returned_to_the_caller_takes_a_count_of_its_own() {
    let program = |n: u32| {
        format!(
            "{TREE}## To same (t: Tree, hops: Int) -> Tree:
    If hops equals 0:
        Return t.
    Return same(t, hops - 1).

## To leftOf (t: Tree) -> Tree:
    Inspect t:
        When Leaf: Return t.
        When Node (l, r): Return l.

## Main
Let mutable total be 0.
Repeat for i from 1 to {n}:
    Let t be bottomUpTree(5).
    Set total to total + check(same(t, 3)) + check(leftOf(t)) + check(t).
Show total.
"
        )
    };
    natively_with_floor(&program(CALLS), 4, "bottomUpTree, check, same and leftOf");
    assert_no_leak(program);
}

/// A field read out of a node the function built outlives that node: the field takes a count before the node is
/// released.
#[test]
fn a_field_of_a_node_built_here_outlives_the_node() {
    let program = |n: u32| {
        format!(
            "{TREE}## To rightOfFresh (k: Int) -> Tree:
    Let t be a new Node with left (a new Leaf) and right (a new Node with left (a new Leaf) and right (a new Leaf)).
    Inspect t:
        When Leaf: Return t.
        When Node (l, r): Return r.

## Main
Let mutable total be 0.
Repeat for i from 1 to {n}:
    Set total to total + check(rightOfFresh(i)).
Show total.
"
        )
    };
    natively_with_floor(&program(CALLS), 2, "rightOfFresh and check");
    assert_no_leak(program);
}

/// One node stored in both fields of a new node: the first store shares it, the last use hands it over.
#[test]
fn a_node_stored_twice_is_shared_then_handed_over() {
    let program = |n: u32| {
        format!(
            "{TREE}## To twin (depth: Int) -> Tree:
    If depth equals 0:
        Return a new Leaf.
    Let sub be twin(depth - 1).
    Return a new Node with left sub and right sub.

## Main
Let mutable total be 0.
Repeat for i from 1 to {n}:
    Set total to total + check(twin(6)).
Show total.
"
        )
    };
    natively_with_floor(&program(CALLS), 2, "twin and check");
    assert_no_leak(program);
}

const MY_LIST: &str = "## A MyList is either:
    An Empty.
    A Cons with head Int and tail MyList.

";

/// A list built by reassignment in a loop: each new cell takes the old list without a copy, and the walk that sums it
/// carries a borrowed cursor around its loop.
#[test]
fn a_list_built_and_walked_in_native_loops() {
    let program = |n: u32| {
        format!(
            "{MY_LIST}## To build (n: Int) -> MyList:
    Let mutable l be a new Empty.
    Let mutable i be 1.
    While i is at most n:
        Set l to a new Cons with head i and tail l.
        Set i to i + 1.
    Return l.

## To total (l: MyList) -> Int:
    Let mutable sum be 0.
    Let mutable cur be l.
    Let mutable going be true.
    While going:
        Inspect cur:
            When Empty: Set going to false.
            When Cons (h, t):
                Set sum to sum + h.
                Set cur to t.
    Return sum.

## Main
Let mutable s be 0.
Repeat for i from 1 to {n}:
    Set s to s + total(build(50)).
Show s.
"
        )
    };
    natively_with_floor(&program(CALLS), 2, "build and total");
    assert_no_leak(program);
}

/// A register holding a borrowed parameter on one path and a node built here on the other: the borrowed copy takes a
/// count of its own, so both paths own what they hold where they meet.
#[test]
fn a_borrowed_node_and_a_built_node_meeting_in_one_variable() {
    let program = |n: u32| {
        format!(
            "{TREE}## To orLeaf (t: Tree, fresh: Bool) -> Tree:
    Let mutable x be t.
    If fresh:
        Set x to a new Leaf.
    Return x.

## Main
Let mutable total be 0.
Repeat for i from 1 to {n}:
    Let t be bottomUpTree(5).
    Set total to total + check(orLeaf(t, i % 3 equals 0)).
Show total.
"
        )
    };
    natively_with_floor(&program(CALLS), 3, "bottomUpTree, check and orLeaf");
    assert_no_leak(program);
}

/// Sibling branches that bind a node and an Int to the same register, and an `Inspect` over a node that dies on the
/// arm that never reads it: each count is released on the edge where its register dies, so the branches agree where
/// they meet.
#[test]
fn sibling_branches_binding_a_node_and_an_int() {
    let program = |n: u32| {
        format!(
            "{TREE}## To weigh (depth: Int) -> Int:
    Let mutable w be 0.
    If depth is greater than 2:
        Let t be a new Node with left (a new Leaf) and right (a new Leaf).
        Inspect t:
            When Leaf: Set w to 1.
            When Node (l, r): Set w to 3.
    Otherwise:
        Let k be depth * 7.
        Set w to k.
    Return w + depth.

## To weighLast (depth: Int) -> Int:
    Let mutable w be 0.
    If depth is greater than 2:
        Let k be depth * 7.
        Set w to k.
    Otherwise:
        Let t be a new Node with left (a new Leaf) and right (a new Leaf).
        Inspect t:
            When Leaf: Set w to 1.
            When Node (l, r): Set w to 3.
    Return w + depth.

## Main
Let mutable total be 0.
Repeat for i from 1 to {n}:
    Set total to total + weigh(i % 5) + weighLast(i % 5).
Show total.
"
        )
    };
    natively_with_floor(&program(CALLS), 2, "weigh and weighLast");
    assert_no_leak(program);
}

/// An owned node read only inside an `If`: when the condition is false, the branch's taken edge is where the node dies,
/// and the count is released on that edge.
#[test]
fn a_node_read_only_on_one_branch_is_released_on_the_other() {
    let program = |n: u32| {
        format!(
            "{TREE}## To maybeWeigh (depth: Int, look: Bool) -> Int:
    Let t be a new Node with left (a new Leaf) and right (a new Leaf).
    Let mutable w be depth.
    If look:
        Inspect t:
            When Leaf: Set w to w + 1.
            When Node (l, r): Set w to w + 3.
    Return w.

## Main
Let mutable total be 0.
Repeat for i from 1 to {n}:
    Set total to total + maybeWeigh(i, i % 2 equals 0).
Show total.
"
        )
    };
    natively_with_floor(&program(CALLS), 1, "maybeWeigh");
    assert_no_leak(program);
}

/// `Bool` and `Float` fields travel unboxed into and out of a node.
#[test]
fn bool_and_float_fields_are_built_and_read_natively() {
    let src = "## A Shape is either:
    A Dot.
    A Box with on Bool and weight Float and inner Shape.

## To make (n: Int) -> Shape:
    If n equals 0:
        Return a new Dot.
    Return a new Box with on (n % 2 equals 0) and weight 0.5 and inner make(n - 1).

## To mass (s: Shape) -> Float:
    Inspect s:
        When Dot: Return 0.0.
        When Box (on, weight, inner):
            If on:
                Return weight + mass(inner).
            Return mass(inner).

## Main
Let mutable total be 0.0.
Repeat for i from 1 to 40:
    Set total to total + mass(make(10)).
Show total.
";
    natively_with_floor(src, 2, "make and mass");
}

/// `LOGOS_JIT_NODES=0` keeps every function that handles enum values on the VM, with the same output.
#[test]
fn the_enum_lane_kill_switch_keeps_node_functions_on_the_vm() {
    let src = format!(
        "{TREE}## Main
Let mutable total be 0.
Repeat for i from 1 to 40:
    Set total to total + check(bottomUpTree(6)).
Show total.
"
    );
    // nextest runs each test in its own process, so the variable reaches only this test.
    unsafe { std::env::set_var("LOGOS_JIT_NODES", "0") };
    let tier = ForgeTier::new();
    let (out, functions) = native(&src, &tier);
    unsafe { std::env::remove_var("LOGOS_JIT_NODES") };
    assert_eq!(out, reference(&src));
    assert_eq!(
        functions, 0,
        "the kill switch must keep bottomUpTree and check on the VM"
    );
}

/// An in-out enum parameter hands its final node back through the caller's variable. Natively a node parameter is
/// borrowed for the whole call, so a function that reassigns one stays on the VM; the functions around it still
/// compile, and the caller sees every graft.
#[test]
fn a_function_reassigning_an_in_out_node_parameter_keeps_the_caller_s_view() {
    let program = |n: u32| {
        format!(
            "{TREE}## To graft (t: mutable Tree, d: Int) -> Int:
    Set t to a new Node with left t and right t.
    Return d.

## Main
Let mutable total be 0.
Repeat for i from 1 to {n}:
    Let mutable t be a new Leaf.
    Set total to total + graft(t, 2).
    Set total to total + check(t).
Show total.
"
        )
    };
    natively_with_floor(&program(CALLS), 1, "check");
    assert_no_leak(program);
}
