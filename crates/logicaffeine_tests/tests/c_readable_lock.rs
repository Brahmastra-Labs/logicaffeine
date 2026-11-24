//! ★ LOCK — the C a program compiles to reads as a C programmer writes it. ★
//!
//! What a C programmer flags on sight in the AST C emitter's output, one row each.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test c_readable_lock`

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::compile::compile_to_c;

/// An `if` and a `while` write their condition in the statement's own parentheses, not three pairs deep.
#[test]
fn a_condition_is_written_in_one_pair_of_parentheses() {
    let src = "## To countdown (n: Int) -> Int:\n    Let mutable k be n.\n    Let mutable steps be 0.\n\
               \x20   While k is greater than 0:\n        Set k to k - 1.\n        Set steps to steps + 1.\n\
               \x20   If steps is less than 2:\n        Return 0.\n    Return steps.\n\n\
               ## Main\nShow countdown(5).\n";
    let c = compile_to_c(src).expect("C for a program with an if and a while");
    let body: String = c.lines().skip_while(|l| !l.contains("countdown(int64_t n)")).collect::<Vec<_>>().join("\n");
    assert!(body.contains("if (steps < 2LL) {"), "the if's condition in one pair:\n{body}");
    assert!(
        body.lines().any(|l| l.trim_start().starts_with("while (") && !l.contains("while ((")),
        "the while's condition in one pair:\n{body}"
    );
    assert!(!body.contains("if ((") && !body.contains("while (("), "no condition two pairs deep:\n{body}");
}

/// A conjunction of comparisons reads as C writes one: `n >= 1LL && n <= 10LL`, the whole in the statement's pair.
#[test]
fn a_conjunction_of_comparisons_reads_as_one() {
    let src = "## To inRange (n: Int) -> Int:\n    If n is at least 1 and n is at most 10:\n        Return 1.\n\
               \x20   Return 0.\n\n## Main\nShow inRange(4).\n";
    let c = compile_to_c(src).expect("C for a program with a conjunction");
    assert!(c.contains("if (n >= 1LL && n <= 10LL) {"), "a conjunction of comparisons as C writes one:\n{c}");
}

/// An expression carries the parentheses C needs and no others: `zr * zr + zi * zi > 4.0`.
#[test]
fn an_expression_is_written_with_the_parentheses_c_needs() {
    let src = "## To escapes (zr: Real) and (zi: Real) -> Bool:\n    Return zr * zr + zi * zi is greater than 4.0.\n\n\
               ## Main\nShow escapes(1.5, 1.5).\n";
    let c = compile_to_c(src).expect("C for a program with arithmetic");
    assert!(c.contains("zr * zr + zi * zi > 4.0"), "the parentheses C needs and no others:\n{c}");
    assert!(!c.contains("(zr * zr)"), "no product in parentheses of its own under a sum:\n{c}");
}

/// An `Option` asks for its variant by its own name: `missing.tag == Option__Int_None`.
#[test]
fn an_option_is_asked_for_its_variant_by_name() {
    let src = "## To find (xs: Seq of Int, target: Int) -> Option of Int:\n    Repeat for i from 1 to length of xs:\n\
               \x20       If item i of xs equals target:\n            Return some i.\n    Return none.\n\n\
               ## Main\nLet missing be find([4, 7, 9], 5).\nIf missing is none:\n    Show \"absent\".\n";
    let c = compile_to_c(src).expect("C for a program that asks an Option");
    assert!(c.contains("if (missing.tag == Option__Int_None) {"), "the variant asked by name:\n{c}");
}

/// A call passes each argument as itself: `pairs(n - 1LL, …)`, not `pairs((n - 1LL), …)`.
#[test]
fn a_call_passes_each_argument_in_no_parentheses_of_its_own() {
    let src = "## To pairs (n: Int) and (k: Int) -> Int:\n    If n is less than 1:\n        Return k.\n\
               \x20   Return pairs(n - 1, k + n).\n\n## Main\nShow pairs(10, 0).\n";
    let c = compile_to_c(src).expect("C for a recursive program");
    assert!(c.contains("pairs(n - 1LL, "), "each argument as itself:\n{c}");
    assert!(!c.contains("pairs(("), "no argument in parentheses of its own:\n{c}");
}
