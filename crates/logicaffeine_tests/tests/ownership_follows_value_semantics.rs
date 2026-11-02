//! What a call, a binding and a `Give` do to a collection — asked of every tier, and of the
//! ownership checker, which must tell the author the story the program actually lives.
//!
//! Value semantics is the default in every engine (`semantics::collections::value_semantics_enabled`):
//! a plain parameter receives a copy, `Let b be a` copies, and only `Give` hands a value over.
//! Each probe below runs on the tree-walker and the VM (`assert_all_tiers`) and as compiled
//! Rust (`assert_compiled_equals_interpreted`), and the checker must then accept it —
//! the Studio's bubble-sort, binary-search and merge-sort ran on every tier while the checker
//! called them use-after-move, and put a red squiggle under each.

mod common;

use common::{assert_all_tiers, assert_compiled_equals_interpreted};
use logicaffeine_compile::compile::compile_to_rust_checked;

const CALL_THEN_REUSE: &str = "\
## To total (xs: Seq of Int) -> Int:
    Let mutable s be 0.
    Repeat for x in xs:
        Set s to s + x.
    Return s.

## Main
Let items be [1, 2, 3].
Let a be total(items).
Let b be total(items).
Show a + b.
Show items.
";

const BUILTIN_CALL_THEN_REUSE: &str = "\
## Main
Let items be [4, 5].
Let n be length(items).
Show n.
Show items.
";

const CALLEE_CHANGES_ITS_OWN_COPY: &str = "\
## To grow (xs: Seq of Int) -> Int:
    Push 99 to xs.
    Return length of xs.

## Main
Let items be [1, 2, 3].
Show grow(items).
Show items.
";

const BINDING_THEN_REUSE: &str = "\
## Main
Let a be [1, 2].
Let b be a.
Show a.
Show b.
";

fn checker_accepts(source: &str) {
    if let Err(e) = compile_to_rust_checked(source) {
        panic!("every tier runs this program, but the checker rejects it:\n{source}\n{:?}", e.kind);
    }
}

#[test]
fn a_value_passed_to_a_function_is_still_the_callers() {
    assert_all_tiers(CALL_THEN_REUSE, "12\n[1, 2, 3]");
}

#[test]
fn compiled_code_agrees_a_function_argument_is_still_the_callers() {
    assert_compiled_equals_interpreted(CALL_THEN_REUSE);
}

#[test]
fn the_checker_lets_a_function_argument_be_used_again() {
    checker_accepts(CALL_THEN_REUSE);
}

#[test]
fn a_value_measured_by_a_builtin_is_still_the_callers() {
    assert_all_tiers(BUILTIN_CALL_THEN_REUSE, "2\n[4, 5]");
}

#[test]
fn compiled_code_agrees_a_builtin_argument_is_still_the_callers() {
    assert_compiled_equals_interpreted(BUILTIN_CALL_THEN_REUSE);
}

#[test]
fn the_checker_lets_a_builtin_argument_be_used_again() {
    checker_accepts(BUILTIN_CALL_THEN_REUSE);
}

#[test]
fn a_callee_changes_only_its_own_copy() {
    assert_all_tiers(CALLEE_CHANGES_ITS_OWN_COPY, "4\n[1, 2, 3]");
}

#[test]
fn compiled_code_agrees_a_callee_changes_only_its_own_copy() {
    assert_compiled_equals_interpreted(CALLEE_CHANGES_ITS_OWN_COPY);
}

#[test]
fn the_checker_lets_a_value_be_used_after_a_callee_changed_its_copy() {
    checker_accepts(CALLEE_CHANGES_ITS_OWN_COPY);
}

#[test]
fn a_bound_value_is_still_the_originals() {
    assert_all_tiers(BINDING_THEN_REUSE, "[1, 2]\n[1, 2]");
}

#[test]
fn compiled_code_agrees_a_bound_value_is_still_the_originals() {
    assert_compiled_equals_interpreted(BINDING_THEN_REUSE);
}

#[test]
fn the_checker_lets_a_bound_value_be_used_again() {
    checker_accepts(BINDING_THEN_REUSE);
}

/// `Give` hands a value over: after it, the name holds nothing.
#[test]
fn a_value_given_away_cannot_be_used() {
    let source = "## Main\nLet items be [1, 2].\nGive items to keeper.\nShow items.\n";
    let e = compile_to_rust_checked(source).expect_err("a use after Give is an ownership error");
    assert!(format!("{:?}", e.kind).contains("giving it away"), "{:?}", e.kind);
}

/// Giving into a call is the call-site move (`Call keep with give items.`): the backend passes
/// the value itself, not a copy, so the caller's name holds nothing afterwards.
#[test]
fn a_value_given_to_a_function_cannot_be_used() {
    let source = "## To keep (xs: Seq of Int):\n    Show length of xs.\n\n## Main\nLet items be [1, 2].\nCall keep with give items.\nShow items.\n";
    let e = compile_to_rust_checked(source).expect_err("a use after giving into a call is an ownership error");
    assert!(format!("{:?}", e.kind).contains("giving it away"), "{:?}", e.kind);
}
