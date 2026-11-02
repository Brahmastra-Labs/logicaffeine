//! ★ LOCK — a variable stored into a record or a variant is MOVED only at its last use, and copied otherwise. ★
//!
//! A record owns its fields, so storing a variable in one either hands the value over (a move, free) or copies it.
//! Handing it over is right only when the program never reads that variable again: `Let bag be a new Bag with items
//! b. Push 3 to b.` must leave `bag` holding the list as it was and `b` usable. The Rust backend moved every list it
//! stored and every variant field at its last mention, so a later read of the variable did not build (E0382), and it
//! copied every record it stored even where nothing read it again.
//!
//! The decision now rests on liveness, per statement: a variable is moved into a construction when the statement
//! names it only there and either reassigns it (`Set l to a new Cons with head i and tail l.`) or is the last
//! statement that uses it. The correctness rows run on every tier; the move rows also read the generated Rust, so a
//! fix that copied everything would fail them.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test construction_move_lock`

#![cfg(not(target_arch = "wasm32"))]

mod common;

use common::{assert_c_output, assert_exact_output};
use logicaffeine_compile::compile::{compile_to_rust, tw_outcome, vm_outcome, vm_outcome_tiered};
use logicaffeine_compile::optimization::Tier;

/// `src` prints exactly `expected` on the tree-walker, the VM, the optimized run path, the Rust backend and the C
/// backend.
fn every_tier(src: &str, expected: &str) {
    let tw = tw_outcome(src);
    assert_eq!(tw.error, None, "the tree-walker refused:\n{src}");
    assert_eq!(
        tw.output.trim_end(),
        expected,
        "the tree-walker's answer:\n{src}"
    );
    let vm = vm_outcome(src);
    assert_eq!(vm.error, None, "the VM refused:\n{src}");
    assert_eq!(vm.output.trim_end(), expected, "the VM's answer:\n{src}");
    let run = vm_outcome_tiered(src, &[], Tier::T3, None);
    assert_eq!(run.error, None, "the optimized run path refused:\n{src}");
    assert_eq!(
        run.output.trim_end(),
        expected,
        "the optimized run path's answer:\n{src}"
    );
    assert_exact_output(src, expected);
    assert_c_output(src, expected);
}

/// The Rust the backend generates for `src`.
fn rust_of(src: &str) -> String {
    compile_to_rust(src).unwrap_or_else(|e| panic!("compiles to Rust: {e:?}\n{src}"))
}

const BAG: &str = "## A Bag has:
    An items: Seq of Int.

";

const TREE: &str = "## A Tree is either:
    A Leaf.
    A Node with left Tree and right Tree.

## To size (t: Tree) -> Int:
    Inspect t:
        When Leaf: Return 1.
        When Node (l, r): Return 1 + size(l) + size(r).

";

#[test]
fn a_list_stored_in_a_record_and_pushed_to_afterwards_keeps_both_values() {
    every_tier(
        &format!(
            "{BAG}## Main
Let mutable b be a new Seq of Int.
Push 1 to b.
Push 2 to b.
Let bag be a new Bag with items b.
Push 3 to b.
Show length of bag's items.
Show length of b.
"
        ),
        "2\n3",
    );
}

#[test]
fn a_list_stored_in_a_record_every_iteration_is_copied_each_time() {
    every_tier(
        &format!(
            "{BAG}## Main
Let mutable b be a new Seq of Int.
Let mutable bags be a new Seq of Bag.
Repeat for i from 1 to 3:
    Push i to b.
    Push (a new Bag with items b) to bags.
Repeat for bg in bags:
    Show length of bg's items.
"
        ),
        "1\n2\n3",
    );
}

#[test]
fn a_list_stored_in_a_record_and_changed_later_in_a_function_keeps_both_values() {
    every_tier(
        &format!(
            "{BAG}## To pack (n: Int) -> Int:
    Let mutable b be a new Seq of Int.
    Push n to b.
    Let bag be a new Bag with items b.
    Push n + 1 to b.
    Return length of bag's items * 10 + length of b.

## Main
Show pack(5).
"
        ),
        "12",
    );
}

#[test]
fn a_list_stored_in_a_record_in_one_branch_is_still_read_after_the_branch() {
    every_tier(
        &format!(
            "{BAG}## To pick (n: Int) -> Int:
    Let mutable b be a new Seq of Int.
    Push n to b.
    Let mutable total be 0.
    If n is greater than 1:
        Let bag be a new Bag with items b.
        Set total to length of bag's items.
    Return total + length of b.

## Main
Show pick(4).
"
        ),
        "2",
    );
}

#[test]
fn a_variant_field_named_and_read_after_the_construction_keeps_its_value() {
    every_tier(
        &format!(
            "{TREE}## Main
Let leaf be a new Leaf.
Let t be a new Node with left leaf and right leaf.
Show size(t).
Show size(leaf).
"
        ),
        "3\n1",
    );
}

#[test]
fn a_variant_field_read_after_the_construction_inside_a_function_keeps_its_value() {
    every_tier(
        &format!(
            "{TREE}## To grow (depth: Int) -> Int:
    Let sub be a new Leaf.
    Let t be a new Node with left sub and right (a new Leaf).
    Return size(t) * 10 + size(sub).

## Main
Show grow(2).
"
        ),
        "31",
    );
}

#[test]
fn a_list_read_by_a_later_show_after_a_call_keeps_its_value() {
    every_tier(
        "## To grow (xs: Seq of Int) -> Int:
    Push 0 to xs.
    Return length of xs.

## To go (n: Int) -> Int:
    Let mutable b be a new Seq of Int.
    Push n to b.
    Let t be grow(b).
    Show b.
    Return t.

## Main
Show go(4).
",
        "[4]\n2",
    );
}

#[test]
fn a_list_stored_before_a_swap_that_names_it_keeps_its_value() {
    every_tier(
        &format!(
            "{BAG}## To trade (n: Int) -> Int:
    Let mutable b be a new Seq of Int.
    Push n to b.
    Let mutable c be a new Seq of Int.
    Push n + 1 to c.
    Push n + 2 to c.
    Let bag be a new Bag with items b.
    Swap b and c.
    Return length of bag's items * 100 + length of b * 10 + length of c.

## Main
Show trade(1).
"
        ),
        "121",
    );
}

#[test]
fn a_list_stored_every_iteration_with_no_other_use_in_the_body_is_copied_each_time() {
    every_tier(
        &format!(
            "{BAG}## To collect (n: Int) -> Int:
    Let mutable b be a new Seq of Int.
    Push n to b.
    Let mutable bags be a new Seq of Bag.
    Repeat for i from 1 to 3:
        Let bag be a new Bag with items b.
        Push bag to bags.
    Return length of bags * 10 + length of b.

## Main
Show collect(7).
"
        ),
        "31",
    );
}

/// ★ A LAST USE IS STILL FREE. The list is stored and never read again, so the record takes it without a copy.
#[test]
fn a_list_stored_at_its_last_use_in_a_function_is_moved() {
    let src = format!(
        "{BAG}## To wrap (n: Int) -> Bag:
    Let mutable b be a new Seq of Int.
    Push n to b.
    Push n + 1 to b.
    Let bag be a new Bag with items b.
    Return bag.

## Main
Show length of wrap(3)'s items.
"
    );
    every_tier(&src, "2");
    let rust = rust_of(&src);
    assert!(
        rust.contains("items: b }"),
        "the record takes `b` itself:\n{rust}"
    );
    assert!(
        !rust.contains("items: b.clone()"),
        "no copy of `b` at its last use:\n{rust}"
    );
}

#[test]
fn a_list_stored_at_its_last_use_in_main_is_moved() {
    let src = format!(
        "{BAG}## Main
Let mutable b be a new Seq of Int.
Push 7 to b.
Let bag be a new Bag with items b.
Show length of bag's items.
"
    );
    every_tier(&src, "1");
    let rust = rust_of(&src);
    assert!(
        rust.contains("items: b }"),
        "the record takes `b` itself:\n{rust}"
    );
    assert!(
        !rust.contains("items: b.clone()"),
        "no copy of `b` at its last use:\n{rust}"
    );
}

#[test]
fn a_record_stored_at_its_last_use_is_moved_and_one_read_again_is_copied() {
    let src = "## A Point has:
    An x: Int.
    A y: Int.

## A Line has:
    A start: Point.
    An end: Point.

## To span (n: Int) -> Int:
    Let p be a new Point with x n and y 0.
    Let q be a new Point with x 0 and y n.
    Let line be a new Line with start p and end q.
    Return line's start's x + line's end's y + q's y.

## Main
Show span(4).
";
    every_tier(src, "12");
    let rust = rust_of(src);
    assert!(
        rust.contains("start: p,") || rust.contains("start: p }"),
        "`p` is not read again: moved\n{rust}"
    );
    assert!(
        rust.contains("end: q.clone()"),
        "`q` is read again: copied\n{rust}"
    );
}

/// ★ A LIST BUILT BY REASSIGNMENT NEVER COPIES ITS TAIL. `Set l to a new Cons with … tail l.` reassigns `l`, so its
/// old value is dead the moment the statement ends and the new cell takes it without a copy — the build stays
/// linear, inside a loop as at the top.
#[test]
fn a_list_built_by_reassignment_in_a_loop_moves_its_tail() {
    let src = "## A MyList is either:
    An Empty.
    A Cons with head Int and tail MyList.

## To total (l: MyList) -> Int:
    Inspect l:
        When Empty: Return 0.
        When Cons (h, t): Return h + total(t).

## Main
Let mutable l be a new Empty.
Let mutable i be 1.
While i is at most 25:
    Set l to a new Cons with head i and tail l.
    Set i to i + 1.
Show total(l).
";
    every_tier(src, "325");
    let rust = rust_of(src);
    assert!(
        !rust.contains("l.clone()"),
        "the tail is moved, never copied:\n{rust}"
    );
}

#[test]
fn a_variable_named_twice_in_one_construction_is_copied_where_needed() {
    every_tier(
        &format!(
            "{TREE}## Main
Let leaf be a new Leaf.
Let t be a new Node with left leaf and right leaf.
Let u be a new Node with left t and right t.
Show size(u).
"
        ),
        "7",
    );
}

#[test]
fn a_list_stored_before_a_while_break_remains_available_at_the_exit() {
    every_tier(
        &format!(
            "{BAG}## To pack (n: Int) -> Int:
    Let mutable b be a new Seq of Int.
    Push n to b.
    While n is greater than 0:
        Let bag be a new Bag with items b.
        Show length of bag's items.
        If n is greater than 1:
            Break.
        Set b to a new Seq of Int.
        Set n to n - 1.
    Return length of b.

## Main
Show pack(2).
Show pack(1).
"
        ),
        "1\n1\n1\n0",
    );
}

#[test]
fn a_list_stored_before_a_repeat_break_remains_available_at_the_exit() {
    every_tier(
        &format!(
            "{BAG}## To pack (n: Int) -> Int:
    Let mutable b be a new Seq of Int.
    Push n to b.
    Repeat for i from 1 to 3:
        Let bag be a new Bag with items b.
        Show length of bag's items.
        If n is greater than 1:
            Break.
        Set b to a new Seq of Int.
    Return length of b.

## Main
Show pack(2).
Show pack(1).
"
        ),
        "1\n1\n1\n0\n0\n0",
    );
}
