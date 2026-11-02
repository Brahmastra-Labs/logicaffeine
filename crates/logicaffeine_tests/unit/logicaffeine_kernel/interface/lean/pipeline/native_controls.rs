//! A result whose checked proof re-derives native decisions discloses every re-derived step as its
//! package does. A run that did not observe Lean publishes no package, so its result is held to the
//! audit of the checked proof alone: the one thing it lacks is Lean's acceptance, never an agreement
//! with a report nobody recorded.
use super::*;
use crate::interface::lean::native_decide::{NativeDecisions, INFORMATION_CATEGORY};

const NATIVE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../logicaffeine_tests/tests/fixtures/lean_conformance/cases_lean414/native_decide/T08-014.viaNative.ndjson"
));
const TARGET: &str = "T08_014.viaNative";

#[test]
fn an_unobserved_native_result_discloses_the_rederivation_and_lacks_only_leans_acceptance() {
    let checked = check_export(
        NATIVE,
        TARGET,
        &LeanEvidence::NotObserved,
        &Limits::default(),
    );
    assert!(
        checked.package.is_none(),
        "a run that did not observe Lean publishes nothing"
    );
    let mut result = checked.result;
    for stage in [
        "translated",
        "primary_check",
        "independent_recheck",
        "assumption_policy",
        "emission_reparsed",
    ] {
        assert_eq!(
            result.status(stage),
            StageStatus::Passed,
            "{stage}: {result:#?}"
        );
    }
    assert!(result.assumptions.is_empty(), "{result:#?}");
    let closure = NativeDecisions::read(NATIVE).closure_of(&intake::kernel_name(TARGET).unwrap());
    assert!(closure.eliminates());
    let disclosed: Vec<(String, String)> = result
        .information_losses
        .iter()
        .filter(|l| l.category == INFORMATION_CATEGORY)
        .map(|l| (l.category.clone(), l.detail.clone()))
        .collect();
    assert_eq!(disclosed, closure.information_changes());
    assert_eq!(disclosed.len(), 1);
    result.finish();
    let codes: Vec<DiagnosticCode> = result.diagnostics.iter().map(|d| d.code).collect();
    assert_eq!(
        codes,
        vec![DiagnosticCode::LeanAcceptanceNotObserved],
        "{result:#?}"
    );
    assert_eq!(result.outcome(), Outcome::UnsupportedFeature);
}
