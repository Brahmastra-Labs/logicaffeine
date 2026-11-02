//! G13 — proof/test NON-VACUITY. A `## Test` that executes ZERO assertions
//! (no `Expect`, no `For all` property) must FAIL as vacuous, never silently
//! pass; a filter matching ZERO tests must fail, not report a green empty run.
//! A test that skips (declared intent) stays Skipped, not vacuous-failed; a
//! real assertion still passes.
//!
//!  ⚠️  YOU DO NOT GET TO WEAKEN THIS LOCK TO MAKE IT PASS.  ⚠️
//!  A vacuous green is exactly the false-confidence this gate exists to kill.

use logicaffeine_compile::testrun::{run_tests, TestConfig, Verdict};

fn config() -> TestConfig {
    TestConfig::default()
}

/// A test that asserts NOTHING is vacuous — it must be Failed, and its failure
/// must say so (not a silent Passed).
#[test]
fn a_test_with_no_assertion_fails_as_vacuous() {
    let suite = "## Test: Does some work but checks nothing\n\
                 \x20   Let x be 2 plus 2.\n\
                 \x20   Let y be x plus 1.\n";
    let report = run_tests(suite, &config()).expect("parses");
    let outcome = &report.outcomes[0];
    assert_eq!(
        outcome.verdict,
        Verdict::Failed,
        "a test with no Expect/property must FAIL as vacuous, got {:?}",
        outcome.verdict
    );
    assert!(
        outcome
            .failures
            .iter()
            .any(|f| f.to_lowercase().contains("no")
                && (f.to_lowercase().contains("assert") || f.to_lowercase().contains("expect"))),
        "the vacuous failure must name the missing assertion; got {:?}",
        outcome.failures
    );
    assert_eq!(report.passed, 0, "a vacuous test must not count as passed");
    assert_eq!(report.failed, 1);
}

/// A real `Expect` still passes — the gate only catches the empty case.
#[test]
fn a_test_with_a_real_assertion_still_passes() {
    let suite = "## Test: Checks something real\n\
                 \x20   Let x be 2 plus 2.\n\
                 \x20   Expect that x is 4.\n";
    let report = run_tests(suite, &config()).expect("parses");
    assert_eq!(report.outcomes[0].verdict, Verdict::Passed);
    assert_eq!(report.passed, 1);
    assert_eq!(report.failed, 0);
}

/// An `Expect output:` block is an assertion too — a test whose only check is
/// output comparison is NOT vacuous.
#[test]
fn an_expect_output_test_is_not_vacuous() {
    let suite = "## Test: Output check counts\n\
                 \x20   Show \"hi\".\n\
                 \x20   Expect output:\n\
                 \x20       hi\n";
    let report = run_tests(suite, &config()).expect("parses");
    assert_eq!(
        report.outcomes[0].verdict,
        Verdict::Passed,
        "an Expect output: block is a real assertion; failures={:?}",
        report.outcomes[0].failures
    );
}

/// A declared skip is intentional, not vacuous — it stays Skipped.
#[test]
fn a_skipped_test_is_not_vacuous_failed() {
    let suite = "## Test: Deliberately deferred\n\
                 \x20   Skip this test because \"the feature has not shipped\".\n";
    let report = run_tests(suite, &config()).expect("parses");
    assert_eq!(report.outcomes[0].verdict, Verdict::Skipped);
    assert_eq!(report.failed, 0);
}

/// A filter that matches ZERO tests is a failure — a green empty run is exactly
/// the false-confidence (a typo'd `--filter` silently "passing") this kills.
#[test]
fn a_zero_match_filter_fails() {
    let suite = "## Test: Real one\n\
                 \x20   Expect that 1 is 1.\n";
    let mut cfg = config();
    cfg.filter = Some("this-name-matches-nothing".to_string());
    let result = run_tests(suite, &cfg);
    match result {
        Err(_) => {} // a hard error is an acceptable "zero match" signal
        Ok(report) => assert!(
            report.failed > 0 || (report.passed == 0 && !report.outcomes.is_empty()),
            "a zero-match filter must FAIL, not report a green empty run: {report:?}"
        ),
    }
}

/// A suite with ZERO discovered tests fails — never a green "nothing to do".
#[test]
fn zero_discovered_tests_fails() {
    let suite = "## Main\nShow 1.\n"; // no `## Test` at all
    let result = run_tests(suite, &config());
    match result {
        Err(_) => {}
        Ok(report) => assert!(
            report.outcomes.is_empty() && report.passed == 0,
            "zero discovered tests must not be a green pass: {report:?}"
        ),
    }
}
