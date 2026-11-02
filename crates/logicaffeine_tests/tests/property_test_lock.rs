//! W9 — `property_test_lock`: `For all` property testing with shrinking.
//!
//! A
//! `For all <domain> <vars>:` property inside a `## Test` generates seeded
//! cases, runs the body, and on failure reports a SHRUNK, concrete
//! counterexample. A true property passes; a false one fails minimally; the
//! identity-keyed seed makes the counterexample stable across runs.

use logicaffeine_compile::testrun::{run_tests, TestConfig, Verdict};

fn config() -> TestConfig {
    TestConfig::default()
}

/// A true property passes over all generated cases.
#[test]
fn a_true_property_passes() {
    let suite = "## Test: Doubling is even\n\
        \x20   For all integers n:\n\
        \x20       Expect that n plus n is divisible by 2.\n";
    let report = run_tests(suite, &config()).expect("parses");
    assert_eq!(
        report.outcomes[0].verdict,
        Verdict::Passed,
        "a genuine property holds over every case: {:?}",
        report.outcomes[0].failures
    );
}

/// A false property fails with a concrete counterexample.
#[test]
fn a_false_property_fails_with_a_counterexample() {
    let suite = "## Test: All integers are positive (false)\n\
        \x20   For all integers n:\n\
        \x20       Expect that n is greater than 0.\n";
    let report = run_tests(suite, &config()).expect("parses");
    assert_eq!(report.outcomes[0].verdict, Verdict::Failed);
    let msg = &report.outcomes[0].failures[0];
    assert!(
        msg.contains("Counterexample"),
        "reports a counterexample: {msg}"
    );
    assert!(msg.contains("n ="), "names the failing binding: {msg}");
}

/// Shrinking minimizes the counterexample to the domain BOUNDARY: `n is at
/// most 5` fails for every natural above 5, so the smallest failing case is 6 —
/// shrinking must reach exactly that boundary, not report a large random value.
#[test]
fn counterexample_is_shrunk_to_the_boundary() {
    let suite = "## Test: All naturals are at most 5 (false above 5)\n\
        \x20   For all naturals n:\n\
        \x20       Expect that n is at most 5.\n";
    let report = run_tests(suite, &config()).expect("parses");
    assert_eq!(report.outcomes[0].verdict, Verdict::Failed);
    let msg = &report.outcomes[0].failures[0];
    assert!(
        msg.contains("n = 6"),
        "shrinks to the minimal failing case n=6 (the boundary), not a random value: {msg}"
    );
}

/// The counterexample is STABLE across runs (identity-keyed seed).
#[test]
fn counterexample_is_stable_across_runs() {
    let suite = "## Test: A false property\n\
        \x20   For all integers a and b:\n\
        \x20       Expect that a plus b is greater than a.\n";
    let m1 = run_tests(suite, &config()).unwrap().outcomes[0].failures[0].clone();
    let m2 = run_tests(suite, &config()).unwrap().outcomes[0].failures[0].clone();
    assert_eq!(m1, m2, "same seed ⇒ identical shrunk counterexample");
}

/// A `where` refinement filters the generated cases.
#[test]
fn where_refinement_filters_cases() {
    // Over positive integers, `n times 2` is always ≥ 2 — holds under the filter.
    let suite = "## Test: Positive doubling is at least 2\n\
        \x20   For all integers n where n is greater than 0:\n\
        \x20       Expect that n plus n is at least 2.\n";
    let report = run_tests(suite, &config()).expect("parses");
    assert_eq!(
        report.outcomes[0].verdict,
        Verdict::Passed,
        "the filter restricts to the cases where the property holds: {:?}",
        report.outcomes[0].failures
    );
}
