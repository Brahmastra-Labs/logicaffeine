//! AOT-Rust: a recursive function over a sequence, specialized on its constant counter.
//!
//! The partial evaluator unrolls `lastOf(xs, i + 1)` into `lastOf_s1_2`, `lastOf_s1_3`, … — each a
//! clone that only READS `xs`, so the borrow pass hands it `xs: &[T]`. The last clone calls the
//! original `lastOf`, whose `xs` stays an owned `LogosSeq<T>`; a borrowed slice passed where an owned
//! sequence is declared failed to build (E0308 "expected `LogosSeq<i64>`, found `&[i64]`"). The call
//! now rebuilds the owned sequence the parameter declares. Each program must build and print what
//! the interpreter prints.

mod common;
use common::assert_compiled_equals_interpreted;

#[test]
fn a_specialized_recursion_over_ints_calls_back_into_its_original() {
    assert_compiled_equals_interpreted(
        "## To lastOf (xs: Seq of Int, i: Int) -> Int:\n    If i equals length of xs:\n        Return item i of xs.\n    Return lastOf(xs, i + 1).\n\n## Main\nShow lastOf([4, 5, 6], 1).\n",
    );
}

#[test]
fn a_specialized_recursion_over_texts_calls_back_into_its_original() {
    assert_compiled_equals_interpreted(
        "## To lastOf (xs: Seq of Text, i: Int) -> Text:\n    If i equals length of xs:\n        Return item i of xs.\n    Return lastOf(xs, i + 1).\n\n## Main\nShow lastOf([\"a\", \"b\"], 1).\n",
    );
}

#[test]
fn a_recursion_deeper_than_the_specialization_budget_still_builds() {
    assert_compiled_equals_interpreted(
        "## To lastOf (xs: Seq of Int, i: Int) -> Int:\n    If i equals length of xs:\n        Return item i of xs.\n    Return lastOf(xs, i + 1).\n\n## Main\nShow lastOf([4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14], 1).\n",
    );
}

#[test]
fn a_generic_recursion_is_never_specialized_on_its_type_parameter() {
    assert_compiled_equals_interpreted(
        "## To applyTwice of [T] (f: fn(T) -> T, x: T) -> T:\n    Return f(f(x)).\n\n## Main\nShow applyTwice((n: Int) -> n + 3, 1).\nShow applyTwice((s: Text) -> s + s, \"ab\").\n",
    );
}

/// Two closures in one function that reuse parameter NAMES at different types: each closure's
/// parameters shadow the other's, so `a + x` adds in the first and concatenates in the second.
#[test]
fn closure_parameters_shadow_a_sibling_closures_same_names() {
    assert_compiled_equals_interpreted(
        "## Main\nLet f be (a: Int, x: Int) -> a + x.\nLet g be (a: Text, x: Text) -> a + x.\nShow f(1, 2).\nShow g(\"a\", \"b\").\n",
    );
}
