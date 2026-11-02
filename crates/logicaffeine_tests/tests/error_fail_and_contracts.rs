//! Error handling, phase 1: a program can STOP WITH ITS REASON.
//!
//! * `Fail with <text>.` raises its reason; with no `Result` to return it into and no `Try:` around
//!   it, the program aborts with exactly that reason.
//! * `Assert that` / `Require that` / `Assume` accept `, otherwise <text>` — the reason the contract
//!   exists, reported when it breaks. The reason is evaluated ONLY on failure.
//! * The reason is identical on every tier: tree-walker ≡ VM (exact error text), AOT-Rust and C
//!   (on stderr, non-zero exit).

mod common;
use common::{assert_all_tiers, assert_c_traps, assert_panics, run_all_tiers};

fn aborts_with(src: &str, reason: &str) {
    let r = run_all_tiers(src);
    assert_eq!(
        r.error.as_deref(),
        Some(reason),
        "tree-walker/VM must abort with exactly the reason\n{src}"
    );
}

fn aborts_everywhere_with(src: &str, reason: &str) {
    aborts_with(src, reason);
    assert_panics(src, reason);
    assert_c_traps(src, reason);
}

// ─── Fail with ───────────────────────────────────────────────────────────────

#[test]
fn fail_with_a_literal_reason() {
    aborts_with("## Main\nFail with \"boom\".\n", "boom");
}

#[test]
fn fail_with_a_computed_reason() {
    aborts_with(
        "## Main\nLet n be 7.\nFail with \"bad value: \" + format(n).\n",
        "bad value: 7",
    );
}

#[test]
fn output_before_the_failure_is_kept() {
    let r = run_all_tiers("## Main\nShow \"before\".\nFail with \"stop\".\nShow \"after\".\n");
    assert_eq!(r.output.trim(), "before");
    assert_eq!(r.error.as_deref(), Some("stop"));
}

#[test]
fn fail_with_inside_a_function_that_returns_a_plain_value_aborts() {
    aborts_with(
        "## To checked (n: Int) -> Int:\n    If n is less than 0:\n        Fail with \"negative: \" + format(n).\n    Return n.\n\n## Main\nShow checked(3).\nShow checked(0 - 2).\n",
        "negative: -2",
    );
}

#[test]
fn fail_with_on_a_path_not_taken_does_nothing() {
    assert_all_tiers(
        "## Main\nLet n be 5.\nIf n is greater than 10:\n    Fail with \"too big\".\nShow n.\n",
        "5",
    );
}

#[test]
fn fail_with_inside_a_loop() {
    aborts_with(
        "## Main\nLet mutable i be 0.\nWhile i is less than 10:\n    If i equals 3:\n        Fail with \"stopped at \" + format(i).\n    Set i to i + 1.\n",
        "stopped at 3",
    );
}

#[test]
fn fail_with_aborts_identically_on_every_tier() {
    aborts_everywhere_with(
        "## Main\nShow 1.\nFail with \"reason \" + format(2).\n",
        "reason 2",
    );
}

// ─── Contracts with reasons ──────────────────────────────────────────────────

#[test]
fn assert_with_a_reason() {
    aborts_with(
        "## Main\nLet x be 0 - 1.\nAssert that x is greater than 0, otherwise \"x must be positive\".\n",
        "x must be positive",
    );
}

#[test]
fn require_with_a_reason() {
    aborts_with(
        "## Main\nLet size be 0.\nRequire that size is greater than 0, otherwise \"size must be positive\".\n",
        "size must be positive",
    );
}

#[test]
fn assume_with_a_reason() {
    aborts_with(
        "## Main\nLet t be 200.\nAssume t is at most 100, otherwise \"t out of range: \" + format(t).\n",
        "t out of range: 200",
    );
}

#[test]
fn a_contract_reason_can_be_computed_from_the_failing_values() {
    aborts_with(
        "## To withdraw (balance: Int, amount: Int) -> Int:\n    Require that amount is at most balance, otherwise \"cannot withdraw \" + format(amount) + \" from \" + format(balance).\n    Return balance - amount.\n\n## Main\nShow withdraw(10, 3).\nShow withdraw(10, 30).\n",
        "cannot withdraw 30 from 10",
    );
}

#[test]
fn a_holding_contract_never_evaluates_its_reason() {
    assert_all_tiers(
        "## To noisy -> Text:\n    Show \"reason evaluated\".\n    Return \"r\".\n\n## Main\nRequire that 1 is less than 2, otherwise noisy().\nAssert that true, otherwise noisy().\nShow \"ok\".\n",
        "ok",
    );
}

#[test]
fn a_contract_without_a_reason_still_says_assertion_failed() {
    aborts_with("## Main\nAssert that 1 equals 2.\n", "Assertion failed");
}

#[test]
fn require_with_a_reason_aborts_identically_on_every_tier() {
    aborts_everywhere_with(
        "## Main\nLet n be 3.\nRequire that n is greater than 5, otherwise \"n is \" + format(n).\n",
        "n is 3",
    );
}

#[test]
fn a_contract_reason_must_be_text() {
    let r = run_all_tiers("## Main\nRequire that 1 equals 2, otherwise 42.\n");
    let err = r.error.expect("a non-Text reason is refused");
    assert!(
        err.contains("reason") && err.contains("Text"),
        "the refusal names the rule: {err}"
    );
}
