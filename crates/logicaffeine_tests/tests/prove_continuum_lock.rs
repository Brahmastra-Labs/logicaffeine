//! W15 — `prove_continuum_lock`: the test→proof continuum's foundation.
//!
//! A `For all`
//! property is the same English sentence whether tested or proved. Under
//! `--prove`, a FINITE-domain property (booleans) is checked over EVERY case —
//! that is a proof, not a sample — and the rank upgrades to PROVEN. Infinite
//! domains (integers) stay HARDENED until the FOL-kernel bridge lands. A false
//! property still fails, with or without `--prove`.

use logicaffeine_compile::testrun::{run_tests, Rank, TestConfig};

fn prove_config() -> TestConfig {
    TestConfig {
        prove: true,
        ..TestConfig::default()
    }
}

/// A boolean tautology is PROVED by exhaustion under `--prove`.
#[test]
fn a_boolean_tautology_is_proven() {
    let suite = "## Test: A bool equals itself\n\
        \x20   For all booleans b:\n\
        \x20       Expect that b is b.\n";
    let report = run_tests(suite, &prove_config()).expect("parses");
    assert_eq!(
        report.failed, 0,
        "the tautology holds: {:?}",
        report.outcomes
    );
    assert_eq!(report.proven, 1, "it is PROVED by exhausting both cases");
    assert_eq!(report.rank(), Rank::Proven, "the rank upgrades to PROVEN");
}

/// Without `--prove`, the same property is only HARDENED (sampled), not proven.
#[test]
fn without_prove_it_is_only_hardened() {
    let suite = "## Test: A bool equals itself\n\
        \x20   For all booleans b:\n\
        \x20       Expect that b is b.\n";
    let report = run_tests(suite, &TestConfig::default()).expect("parses");
    assert_eq!(report.proven, 0, "no proof without --prove");
    assert_eq!(report.rank(), Rank::Hardened);
}

/// An INFINITE domain stays HARDENED even with `--prove` — exhaustion is only
/// honest for finite domains (integers await the kernel bridge).
#[test]
fn an_infinite_domain_is_not_falsely_proven() {
    let suite = "## Test: Identity over integers\n\
        \x20   For all integers n:\n\
        \x20       Expect that n is n.\n";
    let report = run_tests(suite, &prove_config()).expect("parses");
    assert_eq!(
        report.proven, 0,
        "integers are infinite — sampled, not proved"
    );
    assert_eq!(report.rank(), Rank::Hardened);
}

/// A FALSE boolean property still fails under `--prove` (a counterexample is a
/// disproof — the continuum runs both directions). `a is b` over two booleans
/// is false on the two of four cases where they disagree; exhaustion finds one.
#[test]
fn a_false_property_still_fails_under_prove() {
    let suite = "## Test: All bool pairs are equal (false)\n\
        \x20   For all booleans a and b:\n\
        \x20       Expect that a is b.\n";
    let report = run_tests(suite, &prove_config()).expect("parses");
    assert_eq!(report.failed, 1, "a != b disproves it");
    assert_eq!(report.proven, 0);
    assert_eq!(report.rank(), Rank::Failing);
}
