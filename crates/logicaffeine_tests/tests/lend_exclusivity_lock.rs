//! LOCK — a binding lent to a `mutable` parameter is the callee's alone for the call.
//!
//! The law of exclusivity: while a call runs, the binding it lends to a `mutable` parameter is the
//! callee's to read and write, and nothing else in that call may touch it — not another argument
//! reading it, not the same call lending it again, not a call inside another argument lending it.
//! Such a call is refused before it runs, on every tier, in one sentence that names the binding,
//! the parameter it is lent to and the other use, at the line and column of the call.
//!
//! A read of the binding in a call that lends nothing stays legal, even when one of its arguments
//! is a call that lends the binding: that argument's call is over before the outer one begins, and
//! every tier gives the same answer.

mod common;

use logicaffeine_compile::compile::{compile_to_rust, tw_outcome, vm_outcome};
use logicaffeine_language::error::ParseError;

const POP_LAST: &str = "## To popLast (xs: mutable Seq of Int) -> Int:\n    Let v be item (length of xs) of xs.\n    Pop from xs.\n    Return v.\n\n";

fn message_of(e: &ParseError) -> String {
    logicaffeine_language::socratic_explanation(e, &logicaffeine_language::Interner::new())
}

/// `src` is refused on the tree-walker, the VM and the compiled tier, each in the same sentence
/// carrying every one of `phrases`, and the compiled tier's refusal sits inside `call`.
fn refused_everywhere(src: &str, phrases: &[&str], call: &str) {
    let tw = tw_outcome(src)
        .error
        .unwrap_or_else(|| panic!("the tree-walker refuses it:\n{src}"));
    let vm = vm_outcome(src)
        .error
        .unwrap_or_else(|| panic!("the VM refuses it:\n{src}"));
    let err = compile_to_rust(src).expect_err("the compiled tier refuses it");
    let aot = message_of(&err);
    for phrase in phrases {
        for (tier, said) in [("tree-walker", &tw), ("VM", &vm), ("compiled tier", &aot)] {
            assert!(
                said.contains(phrase),
                "the {tier} says {phrase:?}: {said}\n{src}"
            );
        }
    }
    let at = src
        .find(call)
        .unwrap_or_else(|| panic!("the fixture holds {call:?}"));
    assert!(
        err.span.start >= at && err.span.start < at + call.len(),
        "the refusal sits at the call `{call}` (line {:?}), not at line {:?}:\n{src}",
        logicaffeine_language::diagnostic::line_col(src, at),
        logicaffeine_language::diagnostic::line_col(src, err.span.start),
    );
}

/// `src` shows `expected` on the tree-walker, the VM and as a compiled binary.
fn agrees_everywhere(src: &str, expected: &str) {
    let tw = tw_outcome(src);
    assert_eq!(
        (tw.output.trim(), tw.error.as_deref()),
        (expected, None),
        "the tree-walker:\n{src}"
    );
    let vm = vm_outcome(src);
    assert_eq!(
        (vm.output.trim(), vm.error.as_deref()),
        (expected, None),
        "the VM:\n{src}"
    );
    let aot = common::run_logos(src);
    assert!(
        aot.success,
        "the program builds:\n{}\n{}",
        aot.stderr, aot.rust_code
    );
    assert_eq!(aot.stdout.trim(), expected, "the compiled binary:\n{src}");
}

#[test]
fn a_list_lent_and_read_by_the_same_call_is_refused() {
    refused_everywhere(
        "## To h (xs: mutable Seq of Int, ys: Seq of Int) -> Int:\n    Push 9 to xs.\n    Return length of ys.\n\n\
         ## Main\nLet a be [1, 2, 3].\nShow h(a, a).\n",
        &["`a` is lent to `h`'s `mutable` parameter `xs`", "argument 2 of the same call reads it"],
        "h(a, a)",
    );
}

#[test]
fn a_list_lent_twice_by_the_same_call_is_refused() {
    refused_everywhere(
        "## To both (xs: mutable Seq of Int, ys: mutable Seq of Int):\n    Push 1 to xs.\n    Push 2 to ys.\n\n\
         ## Main\nLet a be [0].\nboth(a, a).\nShow a.\n",
        &["`a` is lent to `both`'s `mutable` parameter `xs`", "argument 2 of the same call lends it again"],
        "both(a, a)",
    );
}

#[test]
fn a_list_lent_and_lent_again_inside_another_argument_is_refused() {
    refused_everywhere(
        &format!(
            "{POP_LAST}## To h3 (ys: mutable Seq of Int, n: Int) -> Int:\n    Return n + length of ys.\n\n\
             ## Main\nLet a be [5, 1, 4].\nShow h3(a, popLast(a)).\n"
        ),
        &["`a` is lent to `h3`'s `mutable` parameter `ys`", "argument 2 of the same call lends it again, to `popLast`"],
        "h3(a, popLast(a))",
    );
}

#[test]
fn a_record_lent_and_its_field_read_by_the_same_call_is_refused() {
    refused_everywhere(
        "## A Box has:\n    a n, which is Int.\n\n\
         ## To bump (b: mutable Box) and (k: Int):\n    Set b's n to b's n + k.\n\n\
         ## Main\nLet mutable b be a new Box with n 1.\nbump(b, b's n).\nShow b's n.\n",
        &[
            "`b` is lent to `bump`'s `mutable` parameter `b`",
            "argument 2 of the same call reads it",
        ],
        "bump(b, b's n)",
    );
}

/// Inside a function the refusal still sits at the call, not at the function's header.
#[test]
fn a_refusal_inside_a_function_sits_at_the_call() {
    refused_everywhere(
        "## To h (xs: mutable Seq of Int, ys: Seq of Int) -> Int:\n    Push 9 to xs.\n    Return length of ys.\n\n\
         ## To outer -> Int:\n    Let a be [1].\n    Let n be 0.\n    Return n + h(a, a).\n\n\
         ## Main\nShow outer().\n",
        &["`a` is lent to `h`'s `mutable` parameter `xs`"],
        "h(a, a)",
    );
}

/// A call that lends nothing may read a binding an inner call lends: the inner call is over
/// before the outer one begins.
#[test]
fn a_read_beside_an_inner_lend_in_a_call_that_lends_nothing_is_legal() {
    agrees_everywhere(
        &format!(
            "{POP_LAST}## To joined (xs: Seq of Int, last: Int) -> Seq of Int:\n    Let mutable out be copy of xs.\n    Push last to out.\n    Return out.\n\n\
             ## Main\nLet mutable a be [5, 1, 4].\nShow joined(a, popLast(a)).\nShow a.\n"
        ),
        "[5, 1, 4, 4]\n[5, 1]",
    );
}

/// Two bindings lent to two `mutable` parameters of one call, and a lent binding beside a read of
/// a different one, are each the callee's alone — legal.
#[test]
fn distinct_bindings_lent_by_one_call_are_legal() {
    agrees_everywhere(
        "## To both (xs: mutable Seq of Int, ys: mutable Seq of Int):\n    Push 1 to xs.\n    Push 2 to ys.\n\n\
         ## To grow (xs: mutable Seq of Int, by: Seq of Int):\n    Push length of by to xs.\n\n\
         ## Main\nLet a be [0].\nLet b be [0].\nboth(a, b).\ngrow(a, b).\nShow a.\nShow b.\n",
        "[0, 1, 2]\n[0, 2]",
    );
}
