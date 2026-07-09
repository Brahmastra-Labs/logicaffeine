//! Wave 7: English arithmetic words — the spoken spellings of `+ - * / % **`.
//! LOGOS is an English programming language, so `the sum of a and b` and
//! `a plus b` must mean exactly what `a + b` means, on every engine.
//!
//! Two families:
//!   • Prefix, unambiguous: `the <op> of A and B` (`sum`/`product`/`difference`/
//!     `quotient`/`remainder`). Atomic — it reads as one parenthesised value.
//!   • Infix: `A plus/minus/times B`, `A divided by B`, `A to the power of B`.
//!     `plus`/`minus` are additive precedence; `times`/`divided by` multiplicative;
//!     `to the power of` binds tightest (mirrors `**`).

mod common;
use common::assert_compiled_equals_interpreted_eq as eq;
use common::run_interpreter;
use logicaffeine_compile::compile::{tw_outcome, vm_outcome};

/// The bytecode VM must agree byte-for-byte with the tree-walker (output + error).
fn tw_vm_agree(src: &str) {
    let tw = tw_outcome(src);
    let vm = vm_outcome(src);
    assert_eq!(vm.output.trim_end(), tw.output.trim_end(), "VM/tw output diverged for:\n{src}\nvm: {vm:?}\ntw: {tw:?}");
    assert_eq!(vm.error, tw.error, "VM/tw error diverged for:\n{src}");
}

// ---- Prefix: `the <op> of A and B` ----

#[test]
fn sum_of() {
    eq("## Main\nShow the sum of 3 and 4.\n", "7");
}

#[test]
fn product_of() {
    eq("## Main\nShow the product of 3 and 4.\n", "12");
}

#[test]
fn difference_of() {
    eq("## Main\nShow the difference of 10 and 4.\n", "6");
}

#[test]
fn quotient_of() {
    eq("## Main\nShow the quotient of 20 and 4.\n", "5");
}

#[test]
fn remainder_of() {
    eq("## Main\nShow the remainder of 20 and 7.\n", "6");
}

#[test]
fn prefix_is_atomic_in_a_larger_expression() {
    // `the product of 2 and 3` is one value: 2 * (2*3) = 12, never (2*2)*3 via a leak.
    eq("## Main\nShow 2 * the product of 2 and 3.\n", "12");
}

#[test]
fn prefix_over_variables() {
    eq("## Main\nLet a be 8.\nLet b be 5.\nShow the sum of a and b.\n", "13");
}

// ---- Infix: `A word B` ----

#[test]
fn plus_infix() {
    eq("## Main\nShow 3 plus 4.\n", "7");
}

#[test]
fn minus_infix() {
    eq("## Main\nShow 10 minus 4.\n", "6");
}

#[test]
fn times_infix() {
    eq("## Main\nShow 3 times 4.\n", "12");
}

#[test]
fn divided_by_infix() {
    eq("## Main\nShow 20 divided by 4.\n", "5");
}

#[test]
fn to_the_power_of_infix() {
    // The spoken spelling of `**`.
    eq("## Main\nShow 2 to the power of 10.\n", "1024");
}

#[test]
fn infix_precedence_times_binds_tighter_than_plus() {
    // 3 plus 4 times 2 = 3 + (4*2) = 11, not (3+4)*2 = 14.
    eq("## Main\nShow 3 plus 4 times 2.\n", "11");
}

#[test]
fn infix_over_variables() {
    eq("## Main\nLet a be 3.\nLet b be 4.\nShow a plus b times b.\n", "19");
}

#[test]
fn power_word_binds_tighter_than_times() {
    // 2 times 3 to the power of 2 = 2 * 9 = 18.
    eq("## Main\nShow 2 times 3 to the power of 2.\n", "18");
}

// ---- Regression guard: iteration is untouched by the infix arith-word parsing ----

#[test]
fn range_loop_still_iterates() {
    let src = "## Main\nLet total be 0.\nRepeat for i from 1 to 3:\n    Set total to total + i.\nShow total.\n";
    let r = run_interpreter(src);
    assert!(r.success, "a range loop must still parse and run: {}", r.error);
    assert_eq!(r.output.trim(), "6");
}

#[test]
fn arith_words_agree_across_tw_and_vm() {
    for src in [
        "## Main\nShow the sum of 3 and 4.\n",
        "## Main\nShow 3 plus 4 times 2.\n",
        "## Main\nShow 2 to the power of 10.\n",
        "## Main\nShow the quotient of 20 and 4.\n",
    ] {
        tw_vm_agree(src);
    }
}
