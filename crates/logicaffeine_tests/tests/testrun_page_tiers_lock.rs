//! ★ LOCK — `largo test` catches a page divergence for free, on every run, not only under
//! `--tiers all`. ★
//!
//! "What ships is not what is tested": a project's `## Test`
//! runs on the tree-walker alone unless a person remembers `--tiers all`, and even then the
//! cross-tier guard only re-ran a bare COMPUTATION, never a page. A MOUNTED test now also runs,
//! statement by statement, on every candidate of [`TestConfig::page_tiers`] beyond the reference
//! (`PageTierKind::LoweredHere`, the page's lowered LOGOS on the tree-walker) — unconditionally — and a real disagreement is `tier:
//! Some(TierEvidence::Diverged { .. })`, never a silent pass.

use logicaffeine_compile::page_host::{Drive, PageTierKind};
use logicaffeine_compile::testrun::{
    run_tests, run_tests_with_launcher, TestConfig, TierEvidence, Verdict,
};

#[path = "common/divergent_page.rs"]
mod divergent_page;

const COUNTER: &str = "## Component App\n\
    State:\n    Let n be 0.\n\
    Actions:\n    To bump:\n        Set n to n + 1.\n\
    View:\n    <main><p id=\"n\">{n}</p><button id=\"b\" on-click={bump}>x</button></main>\n\n\
    ## Test: clicking counts\n\
    \x20   Mount App.\n\
    \x20   Click \"#b\".\n\
    \x20   Expect that the text of \"#n\" is equal to \"1\".\n";

#[test]
fn the_default_page_tiers_are_the_lowered_page_on_the_tree_walker_and_on_the_vm() {
    let config = TestConfig::default();
    assert_eq!(
        config.page_tiers,
        vec![PageTierKind::LoweredHere, PageTierKind::LoweredVm]
    );
}

#[test]
fn a_mounted_test_that_agrees_reports_agreed_evidence() {
    let report = run_tests(COUNTER, &TestConfig::default()).expect("must parse");
    assert_eq!(report.outcomes.len(), 1);
    let o = &report.outcomes[0];
    assert_eq!(o.verdict, Verdict::Passed, "{:?}", o.failures);
    assert!(o.mounted);
    assert_eq!(
        o.tier,
        Some(TierEvidence::Agreed),
        "the counter page agrees on every tier"
    );
}

#[test]
fn a_mounted_test_is_not_checked_when_page_tiers_names_only_the_reference() {
    let config = TestConfig {
        page_tiers: vec![PageTierKind::LoweredHere],
        ..TestConfig::default()
    };
    let report = run_tests(COUNTER, &config).expect("must parse");
    assert_eq!(
        report.outcomes[0].tier, None,
        "the reference alone — nothing to compare"
    );
}

#[test]
fn a_non_mounted_test_never_gets_page_tier_evidence() {
    let src = "## Test: arithmetic\n    Expect that 1 + 1 is equal to 2.\n";
    let report = run_tests(src, &TestConfig::default()).expect("must parse");
    assert_eq!(
        report.outcomes[0].tier, None,
        "no page is mounted — nothing for page_tiers to run"
    );
}

#[test]
fn a_real_divergence_is_reported_with_the_first_differing_statement() {
    // A tier that loses the effect of the `Click` — statement 2, after the program (0) and the
    // `Mount` (1) — and agrees on every other statement (`common/divergent_page.rs`), run as the
    // `Browser` candidate after the lowered VM. The divergence exists by construction, so no fix
    // anywhere can take it away from this test; the check must find it, and find it THERE.
    let tier = divergent_page::LosesOneStatement::at(COUNTER, 2);
    let config = TestConfig {
        page_tiers: vec![
            PageTierKind::LoweredHere,
            PageTierKind::LoweredVm,
            PageTierKind::Browser,
        ],
        ..TestConfig::default()
    };
    let report = run_tests_with_launcher(COUNTER, &config, Some(&tier)).expect("must parse");
    let o = &report.outcomes[0];
    assert_eq!(
        o.verdict,
        Verdict::Failed,
        "a tier divergence fails the test — never a silent pass"
    );
    match &o.tier {
        Some(TierEvidence::Diverged {
            detail,
            statement,
            line,
        }) => {
            assert_eq!(
                *statement,
                Some(2),
                "the first differing statement is the lost `Click`: {detail}"
            );
            assert!(
                detail.starts_with("document differs after statement 2:"),
                "the report names the axis and where it first differs: {detail}"
            );
            assert_eq!(
                *line, None,
                "no statement carries a span yet, and a line is never guessed"
            );
        }
        other => panic!("expected a divergence, got {other:?}: {:?}", o.failures),
    }
    assert_eq!(
        o.failures.len(),
        1,
        "the divergence is the only failure: {:?}",
        o.failures
    );
    assert!(
        o.failures[0].starts_with("TIER DIVERGENCE on browser:"),
        "the divergence surfaces as a failure naming the tier, after the lowered VM agreed: {:?}",
        o.failures
    );
    let seen = tier.seen();
    let seen = seen.borrow();
    assert_eq!(seen.launches, ["/"], "the candidate page was launched once");
    assert_eq!(
        seen.drives,
        [
            Drive::Mount {
                component: "App".into(),
                at: None
            },
            Drive::Click {
                selector: "#b".into()
            }
        ],
        "every driving statement was handed over, in order"
    );

    // A candidate that cannot run the test at all fails it too: a `Browser` tier with no browser
    // attached is refused by name — a refusal by construction, not a gap that closes.
    let config = TestConfig {
        page_tiers: vec![PageTierKind::LoweredHere, PageTierKind::Browser],
        ..TestConfig::default()
    };
    let report = run_tests(COUNTER, &config).expect("must parse");
    let o = &report.outcomes[0];
    assert_eq!(
        o.verdict,
        Verdict::Failed,
        "a tier refusal fails the test — never a silent pass"
    );
    match &o.tier {
        Some(TierEvidence::Refused { reason }) => {
            assert!(
                reason.contains("no browser"),
                "names what it is missing: {reason}"
            );
        }
        other => panic!("expected a refusal, got {other:?}"),
    }
    assert!(
        o.failures.iter().any(|f| f.contains("tier: refused")),
        "the refusal surfaces as a failure: {:?}",
        o.failures
    );
}

#[test]
fn the_journal_axis_alone_is_not_a_divergence_the_default_config_reports() {
    // `noteTurn` is not yet emitted on the lowered tier (R3) — every click diverges on `journal`
    // alone. Punishing every project's own tests for a known, ledgered gap would make this
    // feature useless the day it ships; it must stay silent about `journal` until R3 lands.
    let report = run_tests(COUNTER, &TestConfig::default()).expect("must parse");
    assert_eq!(
        report.outcomes[0].tier,
        Some(TierEvidence::Agreed),
        "journal alone must not surface as a tier divergence"
    );
}
