use super::*;

fn make_interner() -> Interner {
    Interner::new()
}

#[test]
fn test_verification_pass_creation() {
    let interner = make_interner();
    let pass = VerificationPass::new(&interner);
    // Just verify it constructs without panic
    drop(pass);
}

// ─────────── `largo verify` passes a program only when it checked every claim it makes ───────────
//
// The pass said "passed" over three kinds of program it never checked: an `Assert` that is false, a
// refinement type broken inside a function body, and a refinement type on a function call's value,
// which crashed the verifier instead. Each is pinned here next to its control, because a verifier
// that rejected everything would pass the failing half of every pair while being just as useless.
// The rule: a claim the pass can check is checked, a claim it cannot model is refused by name, and
// nothing it does not understand is counted as verified.

use crate::compile::compile_to_rust_verified;

/// The verified compile's verdict, with a crash inside the verifier read as its own outcome so a
/// panic fails the test that met it instead of aborting the binary.
fn verify(source: &str) -> Result<(), String> {
    match std::panic::catch_unwind(|| compile_to_rust_verified(source)) {
        Ok(Ok(_)) => Ok(()),
        Ok(Err(e)) => Err(format!("{e:?}")),
        Err(panic) => {
            let message = panic
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| panic.downcast_ref::<&str>().map(|s| s.to_string()))
                .unwrap_or_default();
            panic!("the verifier crashed instead of answering: {message}\n--- program ---\n{source}")
        }
    }
}

fn assert_rejected(source: &str, why: &str) -> String {
    match verify(source) {
        Ok(()) => panic!("{why}, yet `largo verify` passed it:\n{source}"),
        Err(e) => e,
    }
}

fn assert_verified(source: &str, why: &str) {
    if let Err(e) = verify(source) {
        panic!("{why}, yet `largo verify` refused it: {e}\n--- program ---\n{source}");
    }
}

// ─────────────────────────── 1. A false Assert ───────────────────────────

#[test]
fn a_false_assert_in_words_fails_verification() {
    assert_rejected(
        "## Main\nLet x be 5.\nAssert that x is greater than 10.\n",
        "x is 5, so `x is greater than 10` is false",
    );
}

#[test]
fn a_false_assert_in_symbols_fails_verification() {
    assert_rejected(
        "## Main\nLet watts be 900.\nAssert that watts <= 450.\n",
        "watts is 900, so `watts <= 450` is false",
    );
}

#[test]
fn a_true_assert_verifies() {
    assert_verified(
        "## Main\nLet x be 5.\nAssert that x is less than 10.\nAssert that x >= 5.\n",
        "x is 5, so both asserts hold",
    );
}

#[test]
fn a_false_assert_inside_a_function_fails_verification() {
    assert_rejected(
        "## To policy (watts: Int) -> Int:\n    Let doubled be watts * 2.\n    Assert that doubled is at most watts.\n    Return doubled.\n\n## Main\nShow policy(100).\n",
        "doubling a positive number makes it larger, and `watts` is any integer here",
    );
}

#[test]
fn a_reassignment_is_a_new_value_not_a_second_equation() {
    // Read as two equations, `x = 5` and `x = 6` contradict each other, and from a contradiction
    // every assertion follows. The assert below is false for the program as it runs.
    assert_rejected(
        "## Main\nLet mutable x be 5.\nSet x to 6.\nAssert that x equals 7.\n",
        "x ends as 6, so `x equals 7` is false",
    );
    assert_verified(
        "## Main\nLet mutable x be 5.\nSet x to 6.\nAssert that x equals 6.\n",
        "x ends as 6",
    );
}

#[test]
fn a_branch_that_did_not_run_does_not_decide_an_assert() {
    assert_rejected(
        "## Main\nLet mutable x be 5.\nIf x is greater than 100:\n    Set x to 0.\nAssert that x equals 0.\n",
        "the branch never runs, so x stays 5",
    );
    assert_verified(
        "## Main\nLet mutable x be 5.\nIf x is greater than 100:\n    Set x to 0.\nAssert that x equals 5.\n",
        "the branch never runs, so x stays 5",
    );
    assert_verified(
        "## Main\nLet mutable x be 5.\nIf x is greater than 1:\n    Set x to 0.\nOtherwise:\n    Set x to 1.\nAssert that x equals 0.\n",
        "the first branch runs, so x is 0",
    );
}

#[test]
fn a_loop_body_does_not_pin_the_value_after_the_loop() {
    assert_rejected(
        "## Main\nLet mutable x be 0.\nLet mutable i be 0.\nWhile i is less than 3:\n    Set x to 1.\n    Set i to i + 1.\nAssert that x equals 0.\n",
        "the loop runs, so x is 1 afterwards",
    );
}

#[test]
fn two_calls_of_one_function_are_not_assumed_to_agree() {
    // A function may read the console, a clock or a global it changes, so two calls with the same
    // arguments need not return the same value.
    assert_rejected(
        "## To next () -> Int:\n    Return 1.\n\n## Main\nLet a be next().\nLet b be next().\nAssert that a equals b.\n",
        "the verifier knows nothing about what `next` returns",
    );
}

#[test]
fn an_assert_the_verifier_cannot_model_is_refused_not_passed() {
    let err = assert_rejected(
        "## Main\nLet name be \"grid\".\nAssert that name equals \"grid\".\n",
        "Z3 has no model of text here, so it cannot have checked this assert",
    );
    assert!(
        err.contains("Assert") && err.contains("cannot"),
        "the refusal must say that an Assert could not be checked: {err}"
    );
}

#[test]
fn an_assert_inside_a_construct_the_verifier_skips_is_refused() {
    let err = assert_rejected(
        "## Main\nLet items be a new Seq of Int.\nPush 3 to items.\nRepeat for n in items:\n    Assert that n is greater than 5.\n",
        "the loop variable takes the value 3, which is not greater than 5",
    );
    let _ = err;
}

// ─────────────────────── 2. A refinement broken in a function ───────────────────────

#[test]
fn a_refinement_broken_by_a_let_inside_a_function_fails_verification() {
    assert_rejected(
        "## To policy (watts: Int) -> Int:\n    Let capped: Int where it <= 450 be watts * 2.\n    Return capped.\n\n## Main\nShow policy(400).\n",
        "watts is any integer, so `watts * 2` can exceed the cap of 450",
    );
}

#[test]
fn a_refinement_broken_by_a_later_set_fails_verification() {
    assert_rejected(
        "## Main\nLet mutable power: Int where it <= 450 be 400.\nSet power to power * 2.\n",
        "power becomes 800, past its cap of 450",
    );
    assert_rejected(
        "## To policy (watts: Int) -> Int:\n    Let mutable power: Int where it <= 450 be 400.\n    Set power to power * 2.\n    Return power.\n\n## Main\nShow policy(1).\n",
        "inside the policy, power becomes 800, past its cap of 450",
    );
    assert_verified(
        "## Main\nLet mutable power: Int where it <= 450 be 400.\nSet power to power - 100.\n",
        "power becomes 300, inside its cap",
    );
}

#[test]
fn a_return_refinement_broken_by_the_body_fails_verification() {
    assert_rejected(
        "## To policy (watts: Int) -> Int where it <= 450:\n    Return watts * 2.\n\n## Main\nShow policy(100).\n",
        "the policy returns twice any integer, which can exceed 450",
    );
    assert_verified(
        "## To policy (watts: Int) -> Int where it <= 450:\n    If watts is greater than 450:\n        Return 450.\n    Return watts.\n\n## Main\nShow policy(100).\n",
        "every path returns at most 450",
    );
    assert_rejected(
        "## To policy (watts: Int) -> Int where it <= 450:\n    If watts is greater than 450:\n        Return 450.\n    Return watts + 1.\n\n## Main\nShow policy(100).\n",
        "for watts of exactly 450 the second path returns 451",
    );
    assert_rejected(
        "## To policy (watts: Int) -> Int where it <= 450:\n    If watts is greater than 450:\n        Show watts.\n    Return watts.\n\n## Main\nShow policy(100).\n",
        "the first branch does not return, so a watts of 900 reaches `Return watts`",
    );
}

#[test]
fn a_function_body_does_not_leak_its_facts_into_main() {
    assert_rejected(
        "## To setup () -> Int:\n    Let x be 1.\n    Return x.\n\n## Main\nLet y be setup().\nLet x be 2.\nAssert that x equals 1.\n",
        "Main's x is 2, whatever the function called its own x",
    );
}

// ─────────────────── 3. A refinement on a call's value ───────────────────

#[test]
fn a_refinement_on_a_call_value_is_answered_not_crashed() {
    let err = assert_rejected(
        "## To policy (watts: Int) -> Int:\n    Return watts * 2.\n\n## Main\nLet capped: Int where it <= 450 be policy(900).\n",
        "the verifier does not know what `policy` returns, so the cap cannot have been checked",
    );
    assert!(
        err.contains("capped"),
        "the refusal must name the binding whose refinement went unchecked: {err}"
    );
}

#[test]
fn a_refinement_on_a_call_inside_arithmetic_is_answered_not_crashed() {
    assert_rejected(
        "## To policy (watts: Int) -> Int:\n    Return watts.\n\n## Main\nLet capped: Int where it <= 450 be policy(900) + 1.\n",
        "the verifier does not know what `policy` returns",
    );
}

#[test]
fn a_call_in_a_condition_is_answered_not_crashed() {
    assert_rejected(
        "## To over (watts: Int) -> Bool:\n    Return watts is greater than 450.\n\n## Main\nLet mutable w be 900.\nIf over(w):\n    Set w to 450.\nAssert that w equals 450.\n",
        "the verifier does not know what `over` returns, so it cannot know the branch ran",
    );
}

// ─────────────────── The verdicts the pass always gave right stay right ───────────────────

#[test]
fn the_programs_verify_always_accepted_still_verify() {
    for (source, why) in [
        ("## Main\nLet x: Int where it > 0 be 10.", "10 is positive"),
        ("## Main\nLet a be 5.\nLet b: Int where it > 0 be a.", "a is 5"),
        ("## Main\nLet x: Int where it > 0 and it < 100 be 50.", "50 is inside 0 and 100"),
        ("## Main\nLet x: Int where it >= 0 be 0.", "0 is at least 0"),
        ("## Main\nLet x: Int where it == 42 be 42.", "42 is 42"),
        (
            "## Main\nLet x be 10.\nWhile x > 0 (decreasing x):\n    Set x to x - 1.\n",
            "x falls by 1 and stays at least 0 under the guard",
        ),
        ("## Main\nLet x be 100.\nWhile x > 0 (decreasing x):\n    Set x to x - 10.\n", "x falls by 10"),
        (
            "## Main\nLet x be 10.\nWhile x > 0:\n    Set x to x + 1.\n",
            "a loop with no `decreasing` makes no termination claim",
        ),
        (
            "## Main\nLet x be 10.\nWhile x > 0 (decreasing x):\n    Let temp be x - 1.\n    Set x to temp.\n",
            "x falls by 1 through a temporary",
        ),
    ] {
        assert_verified(source, why);
    }
}

#[test]
fn the_programs_verify_always_rejected_are_still_rejected() {
    for (source, why) in [
        ("## Main\nLet x: Int where it > 0 be -5.", "-5 is not positive"),
        ("## Main\nLet a be -5.\nLet b: Int where it > 0 be a.", "a is -5"),
        ("## Main\nLet x: Int where it > 0 and it < 100 be 150.", "150 is past 100"),
        ("## Main\nLet x: Int where it < 10 be 15.", "15 is not below 10"),
        ("## Main\nLet x: Int where it == 42 be 43.", "43 is not 42"),
        (
            "## Main\nLet x be 10.\nWhile x > 0 (decreasing x):\n    Set x to x + 1.\n",
            "x grows, so the loop need not end",
        ),
        ("## Main\nLet x be 10.\nWhile x > 0 (decreasing x):\n    Let y be 5.\n", "x never changes"),
    ] {
        assert_rejected(source, why);
    }
}
