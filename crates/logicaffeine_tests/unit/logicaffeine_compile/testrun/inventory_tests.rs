//! Pure unit locks over the INVENTORY reporting layer — no interpreter, so
//! the classification and its honesty invariants are pinned independently of
//! what any test program computes.
use super::*;

fn outcome(name: &str, verdict: Verdict) -> TestOutcome {
    TestOutcome {
        name: name.to_string(),
        verdict,
        failures: Vec::new(),
        skip_reason: None,
        ops: 0,
        vacuous: false,
        mounted: false,
        property: None,
        proved_by_exhaustion: false,
        tier: None,
    }
}

/// The same outcome, but for a test that MOUNTED a page — the `Component` classification.
fn mounted_outcome(name: &str, verdict: Verdict) -> TestOutcome {
    TestOutcome {
        mounted: true,
        ..outcome(name, verdict)
    }
}

#[test]
fn a_mounted_test_classifies_as_a_component_claim() {
    // A component test drives a DOCUMENT; an example checks a VALUE. Reporting both as
    // `example` told a reader a run of twenty examples had checked twenty values when some had
    // checked a page — and made "does this project test its UI?" unanswerable.
    let inv = Inventory::from_run_report(&RunReport {
        seed: 1,
        outcomes: vec![
            mounted_outcome("clicks", Verdict::Passed),
            outcome("arithmetic", Verdict::Passed),
        ],
        passed: 2,
        failed: 0,
        skipped: 0,
        properties: 0,
        proven: 0,
        page_tiers: Vec::new(),
    });
    assert_eq!(inv.items[0].kind, ClaimKind::Component);
    assert_eq!(
        inv.items[1].kind,
        ClaimKind::Example,
        "the new kind does not swallow the old"
    );
    // Finite testing, never a proof: it checked the cases it drove and nothing else.
    assert!(!inv.items[0].proof);
    assert!(inv.items[0].finite_enumeration);
}

#[test]
fn a_passing_expect_only_test_is_a_passing_example() {
    let report = RunReport {
        seed: 0,
        outcomes: vec![outcome("adds", Verdict::Passed)],
        passed: 1,
        failed: 0,
        skipped: 0,
        properties: 0,
        proven: 0,
        page_tiers: Vec::new(),
    };
    let inv = Inventory::from_run_report(&report);
    assert_eq!(inv.items.len(), 1);
    assert_eq!(inv.items[0].kind, ClaimKind::Example);
    assert_eq!(inv.items[0].status, ClaimStatus::Passed);
    assert!(!inv.items[0].proof, "an example is never a proof");
    assert!(
        inv.items[0].finite_enumeration,
        "an example is finite testing"
    );
}

#[test]
fn a_vacuous_test_is_flagged_and_statused_vacuous() {
    let mut o = outcome("checks nothing", Verdict::Failed);
    o.vacuous = true;
    o.failures.push("vacuous test: …".to_string());
    let report = RunReport {
        seed: 0,
        outcomes: vec![o],
        passed: 0,
        failed: 1,
        skipped: 0,
        properties: 0,
        proven: 0,
        page_tiers: Vec::new(),
    };
    let inv = Inventory::from_run_report(&report);
    assert!(inv.items[0].vacuous);
    assert_eq!(inv.items[0].status, ClaimStatus::Vacuous);
}

#[test]
fn a_falsified_property_carries_its_counterexample_and_bound() {
    let mut o = outcome("all positive", Verdict::Failed);
    o.property = Some(PropertyInfo {
        domain: PropertyDomain::Integers,
        vars: 1,
    });
    o.failures
        .push("Property failed. Counterexample: n = 0".to_string());
    let report = RunReport {
        seed: 0,
        outcomes: vec![o],
        passed: 0,
        failed: 1,
        skipped: 0,
        properties: 1,
        proven: 0,
        page_tiers: Vec::new(),
    };
    let item = &Inventory::from_run_report(&report).items[0];
    assert_eq!(item.kind, ClaimKind::Property);
    assert_eq!(item.status, ClaimStatus::Falsified);
    assert_eq!(item.counterexample.as_deref(), Some("n = 0"));
    assert!(item.domain_bound.as_ref().unwrap().contains("integers"));
    assert!(!item.proof, "a property is never an unrestricted proof");
}

#[test]
fn an_exhausted_boolean_property_is_proved_by_exhaustion_but_never_a_proof() {
    let mut o = outcome("commutes", Verdict::Passed);
    o.property = Some(PropertyInfo {
        domain: PropertyDomain::Booleans,
        vars: 2,
    });
    o.proved_by_exhaustion = true;
    let report = RunReport {
        seed: 0,
        outcomes: vec![o],
        passed: 1,
        failed: 0,
        skipped: 0,
        properties: 1,
        proven: 1,
        page_tiers: Vec::new(),
    };
    let item = &Inventory::from_run_report(&report).items[0];
    assert_eq!(item.status, ClaimStatus::ProvedByExhaustion);
    assert!(item.finite_enumeration, "still finite — not unrestricted");
    assert!(
        !item.proof,
        "exhaustion over booleans is NOT an unrestricted proof"
    );
    assert!(item.domain_bound.as_ref().unwrap().contains("booleans"));
    // proven (actual proofs) counts only kernel theorems — never this.
    assert_eq!(
        Inventory::from_run_report(&report).summary.proven,
        0,
        "a finite-enumeration property is never counted as a proof"
    );
}

#[test]
fn a_proved_kernel_theorem_is_a_proof_an_axiom_is_an_assumption() {
    let theorems = vec![ProvenClaim {
        name: "reflexivity".to_string(),
        verified: true,
        finite_bound: None,
        certificate: None,
    }];
    let axioms = vec!["flip".to_string()];
    let inv = Inventory::from_prove(&theorems, &axioms, &[]).expect("no collision");
    let thm = inv.items.iter().find(|i| i.name == "reflexivity").unwrap();
    assert_eq!(thm.kind, ClaimKind::Theorem);
    assert_eq!(thm.status, ClaimStatus::Proved);
    assert!(thm.proof, "a kernel-certified unbounded theorem IS a proof");
    let ax = inv.items.iter().find(|i| i.name == "flip").unwrap();
    assert_eq!(ax.kind, ClaimKind::Axiom);
    assert_eq!(ax.status, ClaimStatus::Assumption);
    assert!(!ax.proof, "an axiom is an assumption, never a proof");
    assert_eq!(inv.summary.proven, 1, "only the theorem is a proof");
    assert_eq!(inv.summary.axioms, 1);
}

#[test]
fn a_bounded_theorem_is_finite_enumeration_never_an_unrestricted_proof() {
    let theorems = vec![ProvenClaim {
        name: "small case".to_string(),
        verified: true,
        finite_bound: Some("checked over n in [0, 8]".to_string()),
        certificate: None,
    }];
    let inv = Inventory::from_prove(&theorems, &[], &[]).expect("no collision");
    let thm = &inv.items[0];
    assert!(
        thm.finite_enumeration,
        "a bounded theorem is finite enumeration"
    );
    assert!(!thm.proof, "a bounded check is NOT an unrestricted proof");
    assert_eq!(
        inv.summary.proven, 0,
        "a bounded theorem is not counted as a proof"
    );
}

#[test]
fn a_theorem_axiom_name_collision_is_rejected() {
    let theorems = vec![ProvenClaim {
        name: "flip".to_string(),
        verified: true,
        finite_bound: None,
        certificate: None,
    }];
    let axioms = vec!["flip".to_string()];
    let err = Inventory::from_prove(&theorems, &axioms, &[]).unwrap_err();
    assert!(
        err.contains("collision"),
        "the collision is explained: {err}"
    );
    assert!(err.contains("flip"), "it names the colliding claim: {err}");
}

#[test]
fn an_admitted_assumption_collision_is_also_rejected() {
    let theorems = vec![ProvenClaim {
        name: "lemma".to_string(),
        verified: true,
        finite_bound: None,
        certificate: None,
    }];
    let admitted = vec!["lemma".to_string()];
    assert!(
        Inventory::from_prove(&theorems, &[], &admitted).is_err(),
        "a theorem can't also be an admitted assumption"
    );
}

#[test]
fn counts_are_internally_consistent() {
    let mut prop = outcome("prop", Verdict::Passed);
    prop.property = Some(PropertyInfo {
        domain: PropertyDomain::Naturals,
        vars: 1,
    });
    let report = RunReport {
        seed: 0,
        outcomes: vec![outcome("ex", Verdict::Passed), prop],
        passed: 2,
        failed: 0,
        skipped: 0,
        properties: 1,
        proven: 0,
        page_tiers: Vec::new(),
    };
    let inv = Inventory::from_run_report(&report);
    let s = inv.summary;
    assert_eq!(
        s.examples + s.properties + s.theorems + s.axioms + s.admitted_assumptions,
        s.claims,
        "every item is classified into exactly one kind"
    );
    assert_eq!(s.claims, inv.items.len());
    assert_eq!(s.examples, 1);
    assert_eq!(s.properties, 1);
}

#[test]
fn skipped_tests_are_not_inventoried_as_evidence() {
    let mut skip = outcome("live only", Verdict::Skipped);
    skip.skip_reason = Some("uses a real resource".to_string());
    let report = RunReport {
        seed: 0,
        outcomes: vec![skip, outcome("real", Verdict::Passed)],
        passed: 1,
        failed: 0,
        skipped: 1,
        properties: 0,
        proven: 0,
        page_tiers: Vec::new(),
    };
    let inv = Inventory::from_run_report(&report);
    assert_eq!(
        inv.items.len(),
        1,
        "the skipped test carries no verified claim"
    );
    assert_eq!(inv.items[0].name, "real");
}
