//! W14 — `rank_lock`: the earned rank is computed from evidence.
//!
//! The discipline layer. Every run ends in a fitness report with a RANK the suite
//! EARNS — never self-declares: UNTRIED (nothing tested) → FAILING (a test
//! failed) → GREEN (all pass) → HARDENED (a `For all` property held). The
//! higher rungs (COVERED/PROVEN/BATTLE-READY) light up as coverage, mutation,
//! and proofs land in later waves.

use logicaffeine_compile::testrun::{render_fitness, run_tests, Rank, TestConfig};

fn rank_of(suite: &str) -> Rank {
    run_tests(suite, &TestConfig::default())
        .expect("parses")
        .rank()
}

#[test]
fn a_program_with_no_tests_is_untried() {
    assert_eq!(rank_of("## Main\n\x20   Show 1.\n"), Rank::Untried);
}

#[test]
fn a_failing_test_earns_no_rank() {
    let suite = "## Test: it fails\n\x20   Expect that 1 is 2.\n";
    assert_eq!(rank_of(suite), Rank::Failing);
}

#[test]
fn all_passing_earns_green() {
    let suite = "## Test: it holds\n\x20   Expect that 1 is 1.\n";
    assert_eq!(rank_of(suite), Rank::Green);
}

#[test]
fn a_passing_property_earns_hardened() {
    let suite = "## Test: doubling is even\n\
        \x20   For all integers n:\n\
        \x20       Expect that n plus n is divisible by 2.\n";
    assert_eq!(
        rank_of(suite),
        Rank::Hardened,
        "a held property is adversarial evidence"
    );
}

/// The fitness report names the rank and the evidence, and points at the next
/// rung — the ladder is always visible.
#[test]
fn fitness_report_names_rank_and_next_rung() {
    let report = run_tests(
        "## Test: it holds\n\x20   Expect that 1 is 1.\n",
        &TestConfig::default(),
    )
    .unwrap();
    let fitness = render_fitness(&report);
    assert!(
        fitness.contains("RANK: GREEN"),
        "the rank is stated: {fitness}"
    );
    assert!(
        fitness.contains("Next rank:"),
        "the next rung is named: {fitness}"
    );
    assert!(
        fitness.contains("HARDENED"),
        "and how to reach it: {fitness}"
    );
}

/// A rank is EARNED, not declared: a suite cannot claim HARDENED without a
/// property, nor GREEN with a failure.
#[test]
fn rank_cannot_be_faked() {
    // No property ⇒ caps at GREEN, never HARDENED.
    assert_eq!(
        rank_of("## Test: plain\n\x20   Expect that 2 is 2.\n"),
        Rank::Green
    );
    // A failure ⇒ FAILING regardless of other passing tests.
    let mixed = "## Test: a\n\x20   Expect that 1 is 1.\n\n\
                 ## Test: b\n\x20   Expect that 1 is 2.\n";
    assert_eq!(rank_of(mixed), Rank::Failing);
}
