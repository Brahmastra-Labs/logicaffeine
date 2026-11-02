//! W5.1 — `failure_message_goldens`: failure messages are PRODUCT, byte-locked.
//!
//! A failed
//! `Expect` renders a POWER-ASSERT: the condition in English, then every named
//! subexpression annotated with its actual value, then a Socratic hint keyed on
//! the failure's shape. Hiding values is impossible — there is no value-less
//! failure path. These goldens pin the exact text.

use logicaffeine_compile::testrun::{run_tests, TestConfig};

/// The single failing test's failure lines, joined — the unit these goldens lock.
fn failure_text(suite: &str) -> String {
    let report = run_tests(suite, &TestConfig::default()).expect("the suite parses");
    let failed: Vec<&_> = report
        .outcomes
        .iter()
        .filter(|o| o.verdict == logicaffeine_compile::testrun::Verdict::Failed)
        .collect();
    assert_eq!(
        failed.len(),
        1,
        "these goldens expect exactly one failing test"
    );
    failed[0].failures.join("\n---\n")
}

/// A comparison against a literal: the named side shows its value.
#[test]
fn comparison_with_literal() {
    let text =
        failure_text("## Test: t\n\x20   Let x be 3.\n\x20   Expect that x is at least 10.\n");
    assert_eq!(text, "Expected that x is at least 10.\n    x → 3");
}

/// A COMPOUND expression: every operand's value is shown (power-assert) —
/// the whole computation and each of its named parts.
#[test]
fn power_assert_shows_every_operand() {
    let text = failure_text(
        "## Test: t\n\x20   Let a be 1.\n\x20   Let b be 2.\n\
         \x20   Expect that a plus b is 100.\n",
    );
    assert_eq!(text, "Expected that a plus b is 100.\n    a → 1\n    b → 2");
}

/// Float equality earns the Socratic hint (the drift trap).
#[test]
fn float_equality_earns_the_socratic_hint() {
    let text =
        failure_text("## Test: t\n\x20   Let r be 0.1 plus 0.2.\n\x20   Expect that r is 0.3.\n");
    assert_eq!(
        text,
        "Expected that r is 0.3.\n    r → 0.30000000000000004\n    \
         Hint: floats drift — did you mean `is approximately X within <eps>`? (bare `is` is IEEE bit-exact)"
    );
}

/// A boolean sub-condition still names its parts.
#[test]
fn membership_condition_names_its_parts() {
    let text = failure_text(
        "## Test: t\n\x20   Let xs be [1, 2, 3].\n\x20   Let n be 9.\n\
         \x20   Expect that xs contains n.\n",
    );
    assert_eq!(
        text,
        "Expected that xs contains n.\n    xs → [1, 2, 3]\n    n → 9"
    );
}

/// Every failure carries values — there is no value-less failure path.
#[test]
fn no_failure_is_ever_value_less() {
    let text = failure_text(
        "## Test: t\n\x20   Let count be 0.\n\x20   Expect that count is greater than 5.\n",
    );
    assert!(
        text.contains("count → 0"),
        "the failing value is always shown: {text:?}"
    );
}
