//! Wave 7: floor division `//` (binds like `* / %`, left-associative). Unlike `/`
//! — which truncates TOWARD ZERO (`-7 / 2 → -3`) — `//` rounds TOWARD NEGATIVE
//! INFINITY (`-7 // 2 → -4`), the universal meaning of the operator. Integer floor
//! is EXACT (promotes to BigInt); a zero divisor is a loud error on every tier.

mod common;
use common::{assert_compiled_equals_interpreted_eq, run_interpreter, run_logos};
use logicaffeine_compile::compile::{tw_outcome, vm_outcome};

/// The bytecode VM must agree byte-for-byte with the tree-walker on `//` — output
/// AND error text. Error cells are as load-bearing as success cells.
fn assert_vm_matches_tw(src: &str) {
    let tw = tw_outcome(src);
    let vm = vm_outcome(src);
    assert_eq!(vm.output.trim_end(), tw.output.trim_end(), "VM/tw output diverged for:\n{src}\nvm: {vm:?}\ntw: {tw:?}");
    assert_eq!(vm.error, tw.error, "VM/tw error diverged for:\n{src}");
}

#[test]
fn vm_matches_tw_on_floordiv() {
    for src in [
        "## Main\nShow 7 // 2.\n",
        "## Main\nShow 8 // 3.\n",
        "## Main\nShow -7 // 2.\n",       // floor toward -inf → -4
        "## Main\nShow 7 // -2.\n",       // floor toward -inf → -4
        "## Main\nShow -7 // -2.\n",      // floor toward -inf → 3
        "## Main\nShow 0 // 5.\n",
        "## Main\nShow (10 ** 30) // 7.\n",  // exact BigInt floor
        "## Main\nShow 10 // 3 // 2.\n",  // left-associative → (10//3)//2 = 1
        "## Main\nShow 10 - 8 // 2.\n",   // binds tighter than `-` → 10 - 4 = 6
        "## Main\nShow 5 // 0.\n",        // zero divisor — loud error, both tiers
    ] {
        assert_vm_matches_tw(src);
    }
}

#[test]
fn floordiv_basic() {
    assert_compiled_equals_interpreted_eq("## Main\nShow 7 // 2.\n", "3");
}

#[test]
fn floordiv_of_eight_by_three() {
    assert_compiled_equals_interpreted_eq("## Main\nShow 8 // 3.\n", "2");
}

#[test]
fn floordiv_rounds_toward_negative_infinity_for_a_negative_numerator() {
    // -7/2 = -3.5; floor is -4 (NOT -3, which is truncation).
    assert_compiled_equals_interpreted_eq("## Main\nShow -7 // 2.\n", "-4");
}

#[test]
fn floordiv_rounds_toward_negative_infinity_for_a_negative_denominator() {
    // 7/-2 = -3.5; floor is -4.
    assert_compiled_equals_interpreted_eq("## Main\nShow 7 // -2.\n", "-4");
}

#[test]
fn floordiv_of_two_negatives_is_positive_floor() {
    // -7/-2 = 3.5; floor is 3.
    assert_compiled_equals_interpreted_eq("## Main\nShow -7 // -2.\n", "3");
}

#[test]
fn floordiv_is_distinct_from_truncating_divide() {
    // THE point of the operator: `/` truncates toward zero, `//` floors toward -inf.
    assert_compiled_equals_interpreted_eq("## Main\nShow -7 / 2.\n", "-3");
    assert_compiled_equals_interpreted_eq("## Main\nShow -7 // 2.\n", "-4");
}

#[test]
fn floordiv_of_zero_is_zero() {
    assert_compiled_equals_interpreted_eq("## Main\nShow 0 // 5.\n", "0");
}

#[test]
fn floordiv_is_exact_over_bigint() {
    // 10^30 // 7, floored, stays exact well past i64.
    assert_compiled_equals_interpreted_eq(
        "## Main\nShow (10 ** 30) // 7.\n",
        "142857142857142857142857142857",
    );
}

#[test]
fn floordiv_is_left_associative() {
    // 10 // 3 // 2 = (10//3)//2 = 3//2 = 1, not 10//(3//2) = 10//1 = 10.
    assert_compiled_equals_interpreted_eq("## Main\nShow 10 // 3 // 2.\n", "1");
}

#[test]
fn floordiv_binds_tighter_than_subtraction() {
    // 10 - 8 // 2 = 10 - 4 = 6, not (10-8)//2 = 1.
    assert_compiled_equals_interpreted_eq("## Main\nShow 10 - 8 // 2.\n", "6");
}

#[test]
fn floordiv_on_variables() {
    assert_compiled_equals_interpreted_eq(
        "## Main\nLet a be 17.\nLet b be 5.\nShow a // b.\n",
        "3",
    );
}

#[test]
fn floordiv_by_zero_is_loud() {
    let src = "## Main\nShow 5 // 0.\n";
    let interp = run_interpreter(src);
    assert!(!interp.success, "interp must reject a zero divisor");
    let compiled = run_logos(src);
    assert!(!compiled.success, "compiled must reject a zero divisor: {}", compiled.stdout);
}
