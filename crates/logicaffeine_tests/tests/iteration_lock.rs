//! ★ LOCK — a `Repeat` counts through an Int range, and walks its sequence in place exactly when nothing the loop runs
//! can change it. ★
//!
//! `Repeat for i from a to b` over Int bounds is a counter and a comparison on the VM: no list of the range's Ints is
//! built (`optimize::iteration`, `Fact::IntRange`). `Repeat for x in xs` iterates a snapshot of `xs`. The shared
//! optimizer proves when the sequence is its own snapshot for the whole loop (`Fact::LentIteration`): it holds a sequence on every binding, value semantics hold, and
//! nothing the loop runs (its body, a callee writing a global of that name, a lend to a `mutable` parameter) can write
//! through it. The VM then walks it by position with no iterator, the counted loop its native tier compiles, the Rust
//! backend reads it through a shared borrow, and the C backends read its buffer. Where the proof fails, every tier keeps
//! the snapshot: the rows that
//! change the sequence change an item the loop has not reached yet, so a walk in place would print a different answer.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test iteration_lock`

#![cfg(not(target_arch = "wasm32"))]

mod common;

use common::{assert_c_output, assert_exact_output, build_c_release};
use logicaffeine_compile::compile::{
    compile_to_c_via_bytecode, tw_outcome, vm_outcome, vm_outcome_tiered,
};
use logicaffeine_compile::optimization::Tier;
use logicaffeine_compile::ui_bridge::vm_listing;

/// `src` prints what the tree-walker prints on the VM, the optimized run path, the Rust backend and both C backends.
fn every_tier(src: &str) {
    let tw = tw_outcome(src);
    assert_eq!(tw.error, None, "the tree-walker refused:\n{src}");
    let expected = tw.output.trim_end();
    assert_eq!(
        vm_outcome(src).output.trim_end(),
        expected,
        "the VM's answer:\n{src}"
    );
    let run = vm_outcome_tiered(src, &[], Tier::T3, None);
    assert_eq!(
        run.output.trim_end(),
        expected,
        "the optimized run path's answer:\n{src}"
    );
    assert_exact_output(src, expected);
    assert_c_output(src, expected);
    assert_bytecode_c_output(src, expected);
}

/// `src` built through C from its bytecode, with the release C flags, prints `expected`.
fn assert_bytecode_c_output(src: &str, expected: &str) {
    let c = compile_to_c_via_bytecode(src)
        .unwrap_or_else(|e| panic!("the bytecode C generator refused: {e:?}\n{src}"));
    let dir = tempfile::tempdir().expect("temp dir");
    let (c_path, bin) = (dir.path().join("main.c"), dir.path().join("main"));
    std::fs::write(&c_path, &c).expect("write the C");
    let built = build_c_release(&c_path, &bin).expect("run the C compiler");
    assert!(
        built.status.success(),
        "the bytecode C does not compile:\n{}",
        String::from_utf8_lossy(&built.stderr)
    );
    let run = std::process::Command::new(&bin)
        .output()
        .expect("run the bytecode C");
    assert_eq!(
        String::from_utf8_lossy(&run.stdout).trim_end(),
        expected,
        "the bytecode C's answer:\n{src}"
    );
}

/// Whether the bytecode the VM runs iterates a snapshot anywhere.
fn snapshots(src: &str) -> bool {
    vm_listing(src)
        .expect("the program compiles")
        .contains("IterPrepare")
}

/// Whether the bytecode the VM runs builds a range value anywhere.
fn builds_a_range(src: &str) -> bool {
    vm_listing(src)
        .expect("the program compiles")
        .contains("NewRange")
}

#[test]
fn an_int_range_is_a_counted_loop() {
    let src = "## Main
Let n be 1000.
Let mutable total be 0.
Repeat for i from 1 to n:
    Set total to total + i * i.
Show total.
";
    every_tier(src);
    assert!(
        !builds_a_range(src) && !snapshots(src),
        "an Int range needs no range value:\n{}",
        vm_listing(src).unwrap()
    );
}

#[test]
fn a_range_whose_start_is_past_its_end_runs_no_pass() {
    let src = "## Main
Let mutable passes be 0.
Repeat for i from 10 to 1:
    Set passes to passes + 1.
Show passes.
";
    every_tier(src);
    assert!(!builds_a_range(src));
}

#[test]
fn a_range_keeps_the_bounds_it_started_with() {
    let src = "## Main
Let mutable n be 5.
Let mutable passes be 0.
Repeat for i from 1 to n:
    Set n to n + 10.
    Set passes to passes + 1.
Show passes.
Show n.
";
    every_tier(src);
}

#[test]
fn a_range_with_a_bound_that_is_not_an_int_fails_alike_everywhere() {
    let src = "## Main
Let mutable passes be 0.
Repeat for i from 1 to 2.5:
    Set passes to passes + 1.
Show passes.
";
    let tw = tw_outcome(src);
    let vm = vm_outcome(src);
    assert_eq!(
        vm.error.is_some(),
        tw.error.is_some(),
        "the VM and the tree-walker agree on whether 2.5 is a bound"
    );
    assert_eq!(vm.output.trim_end(), tw.output.trim_end());
}

const UNCHANGED: &str = "## Main
Let mutable xs be a new Seq of Int.
Let mutable k be 1.
While k is at most 50:
    Push k * k to xs.
    Set k to k + 1.
Let mutable total be 0.
Repeat for x in xs:
    Set total to total + x.
Show total.
";

#[test]
fn a_sequence_nothing_changes_is_walked_in_place() {
    every_tier(UNCHANGED);
    assert!(
        !snapshots(UNCHANGED),
        "an unchanged sequence needs no snapshot:\n{}",
        vm_listing(UNCHANGED).unwrap()
    );
}

#[test]
fn a_loop_that_changes_its_own_sequence_keeps_the_snapshot() {
    let src = "## Main
Let mutable xs be a new Seq of Int.
Push 1 to xs.
Push 2 to xs.
Push 3 to xs.
Let mutable total be 0.
Repeat for x in xs:
    Set item 3 of xs to 1000.
    Set total to total + x.
Show total.
Show item 3 of xs.
";
    every_tier(src);
    assert!(
        snapshots(src),
        "the loop writes item 3 before reading it, so it must iterate the snapshot"
    );
}

#[test]
fn a_callee_that_changes_the_sequence_keeps_the_snapshot() {
    let src = "## To bump (k: Int) -> Int:
    Set item 3 of xs to 1000.
    Return k.

## Main
Let mutable xs be a new Seq of Int.
Push 1 to xs.
Push 2 to xs.
Push 3 to xs.
Let mutable total be 0.
Repeat for x in xs:
    Set total to total + bump(x).
Show total.
Show item 3 of xs.
";
    every_tier(src);
    assert!(
        snapshots(src),
        "bump writes the global xs mid-loop, so the loop must iterate the snapshot"
    );
}

#[test]
fn a_lend_to_a_mutable_parameter_keeps_the_snapshot() {
    let src = "## To bump (ys: mutable Seq of Int) -> Int:
    Set item 3 of ys to 1000.
    Return 0.

## Main
Let mutable xs be a new Seq of Int.
Push 1 to xs.
Push 2 to xs.
Push 3 to xs.
Let mutable total be 0.
Repeat for x in xs:
    Set total to total + x + bump(xs).
Show total.
Show item 3 of xs.
";
    every_tier(src);
    assert!(
        snapshots(src),
        "xs is lent to a mutable parameter mid-loop, so the loop must iterate the snapshot"
    );
}

#[test]
fn no_borrow_keeps_the_snapshot() {
    let src = format!("## No borrow\n{UNCHANGED}");
    every_tier(&src);
    assert!(
        snapshots(&src),
        "`## No borrow` turns the in-place walk off"
    );
}

#[test]
fn a_function_walks_its_sequence_parameter_in_place() {
    let src = "## To total (xs: Seq of Int) -> Int:
    Let mutable t be 0.
    Repeat for x in xs:
        Set t to t + x.
    Return t.

## Main
Let mutable xs be a new Seq of Int.
Let mutable k be 1.
While k is at most 30:
    Push k to xs.
    Set k to k + 1.
Let mutable grand be 0.
Let mutable r be 1.
While r is at most 40:
    Set grand to grand + total(xs).
    Set r to r + 1.
Show grand.
";
    every_tier(src);
    assert!(
        !snapshots(src),
        "a read-only sequence parameter needs no snapshot:\n{}",
        vm_listing(src).unwrap()
    );
}

/// Under value semantics a second handle copies before it writes, so writing through it mid-walk leaves the walk alone.
#[test]
fn a_write_through_another_handle_leaves_the_walk_alone() {
    let src = "## Main
Let mutable xs be a new Seq of Int.
Push 1 to xs.
Push 2 to xs.
Push 3 to xs.
Let mutable ys be xs.
Let mutable total be 0.
Repeat for x in xs:
    Push x to ys.
    Set item 1 of ys to 100.
    Set total to total + x.
Show total.
Show length of xs.
Show length of ys.
Show item 1 of xs.
Show item 1 of ys.
";
    every_tier(src);
    assert!(
        !snapshots(src),
        "a write through ys does not write xs:\n{}",
        vm_listing(src).unwrap()
    );
}

/// A sequence of constant size built by pushes is the one a backend may hold as a fixed array.
#[test]
fn a_fixed_size_sequence_is_walked_alike() {
    let src = "## Main
Let xs be a new Seq of Int.
Push 7 to xs.
Push 11 to xs.
Push 13 to xs.
Let mutable total be 0.
Repeat for x in xs:
    Set total to total + x * x.
Show total.
Show item 2 of xs.
Show length of xs.
";
    every_tier(src);
}

/// A function that fills a sequence of constant size and returns it, and a caller that walks the result.
#[test]
fn a_returned_fixed_size_sequence_is_walked_alike() {
    let src = "## To block (k: Int) -> Seq of Int:
    Let out be a new Seq of Int.
    Push k to out.
    Push k + 1 to out.
    Push k * 3 to out.
    Push k - 2 to out.
    Return out.

## Main
Let mutable grand be 0.
Let mutable r be 1.
While r is at most 6:
    Let h be block(r).
    Repeat for x in h:
        Set grand to grand + x * r.
    Set r to r + 1.
Show grand.
";
    every_tier(src);
}

/// A constant table read by a computed position, and walked.
#[test]
fn a_constant_table_read_by_position_is_walked_alike() {
    let src = "## Main
Let shifts be [7, 12, 17, 22].
Let mutable acc be 0.
Let mutable i be 0.
While i is less than 16:
    Set acc to acc + item (i % 4 + 1) of shifts.
    Set i to i + 1.
Repeat for s in shifts:
    Set acc to acc * 3 + s.
Show acc.
";
    every_tier(src);
}

#[test]
fn a_break_out_of_an_in_place_walk_leaves_the_loop() {
    let src = "## Main
Let mutable xs be a new Seq of Int.
Let mutable k be 1.
While k is at most 20:
    Push k to xs.
    Set k to k + 1.
Let mutable seen be 0.
Repeat for x in xs:
    If x is greater than 7:
        Break.
    Set seen to seen + x.
Show seen.
";
    every_tier(src);
    assert!(!snapshots(src), "a break does not need the snapshot");
}
