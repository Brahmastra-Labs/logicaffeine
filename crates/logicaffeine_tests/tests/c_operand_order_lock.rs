//! ★ LOCK — the C tier evaluates operands in the order LOGOS states. ★
//!
//! LOGOS evaluates an operator's operands and a call's arguments left to right; C does not fix that order. When one
//! operand writes a program binding a sibling reads (`counter + next()`, where `next` counts in `counter`), the C tier
//! binds the operands in order to fresh names before the statement (`codegen_c::sequence`), so C reads what the
//! interpreter reads. Each row's program prints values that depend on that order; it runs on the tree-walker for the
//! reference and must print the same bytes through C.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test c_operand_order_lock`

#![cfg(not(target_arch = "wasm32"))]

mod common;

use common::assert_c_output;
use logicaffeine_compile::compile::tw_outcome;

fn as_the_interpreter_does(src: &str) {
    let tw = tw_outcome(src);
    assert_eq!(tw.error, None, "the tree-walker refused:\n{src}");
    assert_c_output(src, tw.output.trim_end());
}

const NEXT: &str = "## To next -> Int:
    Set counter to counter + 1.
    Return counter * 10.

";

#[test]
fn a_read_before_the_call_that_writes_it_reads_the_old_value() {
    as_the_interpreter_does(&format!(
        "{NEXT}## Main\nLet mutable counter be 0.\nShow counter + next().\nShow next() + counter.\nShow counter.\n"
    ));
}

#[test]
fn nested_operands_keep_their_order() {
    as_the_interpreter_does(&format!(
        "{NEXT}## Main\nLet mutable counter be 5.\nLet a be counter.\nShow a + (counter + next()) * (next() - counter).\nShow counter.\n"
    ));
}

#[test]
fn a_calls_arguments_keep_their_order() {
    as_the_interpreter_does(&format!(
        "{NEXT}## To pair (x: Int, y: Int, z: Int) -> Int:
    Return x * 10000 + y * 100 + z.

## Main
Let mutable counter be 1.
Show pair(counter, next(), counter).
Show counter.
"
    ));
}

#[test]
fn a_set_and_a_return_keep_their_order() {
    as_the_interpreter_does(&format!(
        "{NEXT}## To both -> Int:
    Return counter - next().

## Main
Let mutable counter be 3.
Let mutable acc be 0.
Set acc to counter + next().
Show acc.
Show both().
Show counter.
"
    ));
}
