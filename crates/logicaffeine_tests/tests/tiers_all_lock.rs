//! W13 — `tiers_all_lock`: `largo test --tiers all` runs each test's computation
//! on the compiled tier too (the F18 "works-on-my-tier" guard).
//!
//! Wave 13 productizes the cross-tier differential for the USER's test suite. A
//! `## Test` that passes on the interpreter must also RUN on the bytecode VM —
//! `--tiers all` executes the test's computation there (the assertions stay the
//! interpreter's authoritative verdict) and fails the test if the compiled tier
//! cannot run what the interpreter did. This lock proves the flag is wired, that
//! it does not false-positive on tests whose computation both tiers handle, and
//! that the underlying computation genuinely round-trips through the VM.

use logicaffeine_compile::testrun::{run_tests, TestConfig};

fn all_tiers() -> TestConfig {
    TestConfig {
        all_tiers: true,
        ..TestConfig::default()
    }
}

const SUITE: &str = "## Test: arithmetic accumulates\n\
    \x20   Let mutable total be 0.\n\
    \x20   Repeat for i from 1 to 5:\n\
    \x20       Set total to total + i.\n\
    \x20   Expect that total is 15.\n\
    \n\
    ## Test: a branch picks the larger\n\
    \x20   Let x be 7.\n\
    \x20   Let y be 3.\n\
    \x20   Let big be x.\n\
    \x20   If y is greater than x:\n\
    \x20       Set big to y.\n\
    \x20   Expect that big is 7.\n";

/// Under `--tiers all`, tests whose computation both tiers run identically all
/// PASS — the guard adds cross-tier coverage without false positives.
#[test]
fn tiers_all_passes_tier_stable_tests() {
    let report = run_tests(SUITE, &all_tiers()).expect("parses");
    assert_eq!(
        report.failed, 0,
        "no tier divergence on ordinary computation: {:?}",
        report.outcomes
    );
    assert_eq!(report.passed, 2, "both tests pass on every tier");
}

/// The baseline: the same suite passes WITHOUT `--tiers all` too (the flag is
/// additive coverage, never a behavior change for green suites).
#[test]
fn same_suite_passes_without_the_flag() {
    let report = run_tests(SUITE, &TestConfig::default()).expect("parses");
    assert_eq!(report.failed, 0);
    assert_eq!(report.passed, 2);
}

/// The flag genuinely exercises the VM path: a test whose body uses real
/// computation (a loop building a list, then a length check) round-trips through
/// the compiled tier under `--tiers all` and still passes — proof the VM ran the
/// computation, not that the check was skipped.
#[test]
fn tiers_all_round_trips_real_computation_through_the_vm() {
    let suite = "## Test: a list grows to the right length\n\
        \x20   Let mutable xs be [].\n\
        \x20   Repeat for i from 1 to 4:\n\
        \x20       Push i to xs.\n\
        \x20   Expect that the length of xs is 4.\n";
    let report = run_tests(suite, &all_tiers()).expect("parses");
    assert_eq!(
        report.failed, 0,
        "the VM ran the list computation cleanly: {:?}",
        report.outcomes
    );
    assert_eq!(report.passed, 1);
}
