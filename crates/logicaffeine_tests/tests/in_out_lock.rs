//! ★ LOCK — a `mutable` parameter is the caller's variable. ★
//!
//! `## To bump (x: mutable Int)` lends the callee the caller's variable: every write the callee makes is the caller's
//! value when the call returns. The VM keeps the parameter's argument register on return and the caller copies it back
//! into the place it lent. The AST C backend passes the caller's variable by pointer; the bytecode C backend passes
//! the argument register by pointer, and the callee stores each in-out parameter back before it returns; a
//! WebAssembly function returns each in-out parameter's final value after its own result, and the caller stores it
//! into the argument it lent. Each row runs on the tree-walker (the reference), the VM, both C backends and
//! WebAssembly, and prints the same bytes: every kind a parameter holds, every place a caller lends from, a parameter
//! handed on to another in-out call, and an integer that outgrows a machine word inside the callee.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test in_out_lock`

#![cfg(not(target_arch = "wasm32"))]

mod common;

#[cfg(feature = "wasm-jit")]
#[path = "compile_common/mod.rs"]
mod compile_common;

use common::{assert_c_output, build_c_release};
use logicaffeine_compile::compile::{compile_to_c_via_bytecode, tw_outcome, vm_outcome};

/// `src` prints what the tree-walker prints on the VM, both C backends and WebAssembly.
fn every_tier(src: &str) {
    let tw = tw_outcome(src);
    assert_eq!(tw.error, None, "the tree-walker refused:\n{src}");
    let expected = tw.output.trim_end();
    assert_eq!(
        vm_outcome(src).output.trim_end(),
        expected,
        "the VM's answer:\n{src}"
    );
    assert_c_output(src, expected);
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
    #[cfg(feature = "wasm-jit")]
    match compile_common::wasm_host::run_show(src) {
        Ok(got) => assert_eq!(
            got.trim_end(),
            expected,
            "the WebAssembly module's answer:\n{src}"
        ),
        Err(why) => panic!("the WebAssembly module could not run it ({why}):\n{src}"),
    }
}

#[test]
fn an_int_written_by_the_callee_is_the_callers() {
    every_tier(
        "## To bump (x: mutable Int):
    Set x to x + 1.

## Main
Let mutable a be 5.
bump(a).
bump(a).
Show a.
",
    );
}

#[test]
fn a_callee_that_returns_a_value_still_writes_the_callers_variable() {
    every_tier(
        "## To bump (x: mutable Int) -> Int:
    Set x to x + 1.
    Return x * 2.

## Main
Let mutable a be 5.
Show bump(a).
Show a.
Show bump(a) + a.
Show a.
",
    );
}

/// A read after the lend, in the very expression that lends, sees the value the callee wrote; one before it sees
/// the value from before the call.
#[test]
fn a_read_in_the_lending_expression_sees_the_value_of_its_moment() {
    every_tier(
        "## To bump (x: mutable Int) -> Int:
    Set x to x + 1.
    Return x * 2.

## Main
Let mutable a be 5.
Show bump(a) + a.
Show a + bump(a) + a.
Show a.
",
    );
}

#[test]
fn every_return_of_the_callee_writes_back() {
    every_tier(
        "## To settle (x: mutable Int, k: Int) -> Int:
    If k is less than 0:
        Set x to 0 - 1.
        Return 0.
    If k equals 0:
        Return 1.
    Set x to x + k.
    Return 2.

## Main
Let mutable a be 10.
Show settle(a, 3).
Show a.
Show settle(a, 0).
Show a.
Show settle(a, 0 - 5).
Show a.
",
    );
}

#[test]
fn a_recursion_writes_the_callers_variable_at_every_level() {
    every_tier(
        "## To walk (seen: mutable Int, row: Int, n: Int) -> Int:
    If row equals n:
        Return 1.
    Set seen to seen + 1.
    Let mutable count be 0.
    Let mutable k be 0.
    While k is less than 3:
        Set count to count + walk(seen, row + 1, n).
        Set k to k + 1.
    Return count.

## Main
Let mutable seen be 0.
Show walk(seen, 0, 4).
Show seen.
",
    );
}

#[test]
fn a_parameter_handed_on_to_another_in_out_call_reaches_the_first_caller() {
    every_tier(
        "## To inner (y: mutable Int):
    Set y to y * 3.

## To outer (x: mutable Int):
    Set x to x + 1.
    inner(x).
    Set x to x + 2.

## Main
Let mutable a be 4.
outer(a).
Show a.
",
    );
}

#[test]
fn two_in_out_parameters_and_a_value_one_between_them() {
    every_tier(
        "## To exchange (a: mutable Int, k: Int, b: mutable Int):
    Let t be a.
    Set a to b + k.
    Set b to t - k.

## Main
Let mutable x be 7.
Let mutable y be 30.
exchange(x, 2, y).
Show x.
Show y.
",
    );
}

#[test]
fn a_real_a_bool_and_a_text_written_by_the_callee_are_the_callers() {
    every_tier(
        "## To halve (f: mutable Real):
    Set f to f / 2.0.

## To flip (b: mutable Bool):
    Set b to not b.

## To grow (s: mutable Text):
    Set s to s combined with \"!\".

## Main
Let mutable f be 9.0.
halve(f).
Show f.
Let mutable b be true.
flip(b).
Show b.
Let mutable s be \"hi\".
grow(s).
grow(s).
Show s.
",
    );
}

#[test]
fn a_sequence_changed_in_place_and_one_replaced_whole_are_the_callers() {
    every_tier(
        "## To extend (xs: mutable Seq of Int):
    Push 7 to xs.
    Set item 1 of xs to 100.

## To replace (xs: mutable Seq of Int):
    Let fresh be a new Seq of Int.
    Push 1 to fresh.
    Push 2 to fresh.
    Set xs to fresh.

## Main
Let mutable xs be a new Seq of Int.
Push 3 to xs.
Let mutable keep be xs.
extend(xs).
Show xs.
Show keep.
replace(xs).
Show xs.
Show keep.
",
    );
}

#[test]
fn an_element_and_a_field_lent_as_the_parameter_are_written_back_in_place() {
    every_tier(
        "## A Pt has:
    An x: Int.
    A y: Int.

## To bump (v: mutable Int):
    Set v to v + 10.

## Main
Let mutable xs be a new Seq of Int.
Push 1 to xs.
Push 2 to xs.
Push 3 to xs.
bump(item 2 of xs).
Show xs.
Let mutable p be a new Pt with x 4 and y 5.
bump(p's y).
Show p's x.
Show p's y.
",
    );
}

#[test]
fn an_integer_that_outgrows_the_word_inside_the_callee_is_the_callers() {
    every_tier(
        "## To square (x: mutable Int):
    Set x to x * x.

## Main
Let mutable a be 3037000500.
square(a).
Show a.
square(a).
Show a.
",
    );
}
