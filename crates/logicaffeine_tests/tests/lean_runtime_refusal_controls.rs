//! Controls for a non-publishable conditional audit of quarantined runtime code.
//! The original two refusal tests specify the complete positive diagnostic path.
mod lean_package_support;

use adapter::pipeline::{self, Checked, LeanEvidence, Limits};
use adapter::receipt::StageStatus;
use lean_package_support::Export;
use logicaffeine_kernel::interface::lean::{self as adapter, DiagnosticCode};
use serde_json::Value;

const COERCE: &str =
    include_str!("fixtures/lean_conformance/cases_lean414/refusal/T02-008.coerceAnything.ndjson");
const TARGET: &str = "T02_008.coerceAnything";

fn checked(export: &str, target: &str) -> Checked {
    pipeline::check_export(
        export,
        target,
        &LeanEvidence::NotObserved,
        &Limits::default(),
    )
}

fn never_published(checked: &Checked) {
    assert!(checked.package.is_none());
    assert!(checked.result.package.is_none());
    assert!(checked.result.target_logos.is_none());
    assert_eq!(
        checked.result.status("emission_reparsed"),
        StageStatus::NotRun
    );
    assert_eq!(checked.result.status("fresh_replay"), StageStatus::NotRun);
}

fn rows() -> Vec<Value> {
    COERCE
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

fn text(rows: &[Value]) -> String {
    rows.iter().map(|row| row.to_string() + "\n").collect()
}

#[test]
fn a_complete_refusal_report_never_makes_the_runtime_root_importable() {
    let key = adapter::intake::kernel_name(TARGET).unwrap();
    let intake = adapter::intake::read(COERCE, &adapter::intake::STAGE_A).unwrap();
    let closure = intake.closure(&key).unwrap();
    let report = checked(COERCE, TARGET);
    never_published(&report);
    assert_eq!(
        report.result.status("assumption_policy"),
        StageStatus::Failed,
        "{:#?}",
        report.result.to_json()
    );
    assert_eq!(report.result.assumptions, vec!["lcProof".to_string()]);
    assert_conditional_scope(&report, TARGET);
    assert!(
        adapter::import_retaining_closure(COERCE, &key, &closure).is_err(),
        "conditional checking is not an ordinary import capability"
    );
    assert!(adapter::import_audited(COERCE).is_err());
    assert!(adapter::check_str(COERCE).is_err());
}

#[test]
fn a_safe_declaration_using_quarantined_code_cannot_enter_the_conditional_refusal_path() {
    let mut rows = rows();
    let target = rows
        .iter_mut()
        .rev()
        .find(|row| row.get("def").is_some())
        .unwrap();
    assert_eq!(target["def"]["safety"], "unsafe");
    target["def"]["safety"] = serde_json::json!("safe");
    let report = checked(&text(&rows), TARGET);
    never_published(&report);
    assert_eq!(
        report.result.diagnostics.first().map(|d| d.code),
        Some(DiagnosticCode::InvalidProof)
    );
    assert_eq!(
        report.result.status("assumption_policy"),
        StageStatus::NotRun
    );
    assert_ne!(report.result.status("primary_check"), StageStatus::Passed);
}

#[test]
fn an_ill_typed_runtime_body_is_not_credited_with_two_kernel_checks_because_it_reaches_lc_proof() {
    let mut rows = rows();
    let target_index = rows
        .iter()
        .rposition(|row| row.get("def").is_some())
        .unwrap();
    assert_eq!(rows[target_index]["def"]["safety"], "unsafe");
    let body = rows[target_index]["def"]["value"].as_u64().unwrap();
    let next = rows
        .iter()
        .filter_map(|row| row["ie"].as_u64())
        .max()
        .unwrap()
        + 1;
    rows[target_index]["def"]["value"] = serde_json::json!(next);
    // The original body expects a type as its first argument; its function value is not a type.
    // Both references retain the original dependency on the actual unsafe lcProof leaf.
    rows.insert(
        target_index,
        serde_json::json!({"ie": next, "app": {"fn": body, "arg": body}}),
    );
    let report = checked(&text(&rows), TARGET);
    never_published(&report);
    assert_eq!(
        report.result.diagnostics.first().map(|d| d.code),
        Some(DiagnosticCode::InvalidProof)
    );
    assert_eq!(
        report.result.status("assumption_policy"),
        StageStatus::NotRun
    );
    assert_ne!(report.result.status("primary_check"), StageStatus::Passed);
    assert_ne!(
        report.result.status("independent_recheck"),
        StageStatus::Passed
    );
}

#[test]
fn an_unsafe_axiom_header_without_a_body_is_not_a_completed_policy_refusal() {
    let mut export = Export::new();
    export.lines[0] = include_str!("fixtures/lean_package/W05.clean.ndjson")
        .lines()
        .next()
        .unwrap()
        .to_string();
    let name = export.name("lcProof", 0);
    let p = export.name("P", 0);
    let prop = export.sort(0);
    let variable = export.bvar(0);
    let ty = export.forall(p, prop, variable);
    export.raw(format!(
        r#"{{"axiom":{{"name":{name},"levelParams":[],"type":{ty},"isUnsafe":true}}}}"#
    ));
    let report = checked(&export.text(), "lcProof");
    never_published(&report);
    assert_eq!(
        report.result.diagnostics.first().map(|d| d.code),
        Some(DiagnosticCode::UnsupportedFeature)
    );
    assert_eq!(
        report.result.status("assumption_policy"),
        StageStatus::NotRun
    );
    assert_ne!(report.result.status("primary_check"), StageStatus::Passed);
    assert_ne!(
        report.result.status("independent_recheck"),
        StageStatus::Passed
    );
}

fn assert_conditional_scope(report: &Checked, target: &str) {
    let audit = report
        .result
        .audit
        .as_ref()
        .expect("conditional audit metadata");
    assert_eq!(audit["schema"], "logos-lean-runtime-refusal-audit/1");
    assert_eq!(audit["scope"], "quarantined_conditional_judgment");
    assert_eq!(audit["original_target"], target);
    assert_eq!(audit["original_target_proved"], false);
    assert_eq!(audit["original_target_importable"], false);
    assert_eq!(audit["unsafe_globals_registered"], false);
    assert!(!audit["conditional_type"].as_str().unwrap().is_empty());
    for stage in ["primary_check", "independent_recheck"] {
        assert_eq!(
            report.result.status(stage),
            StageStatus::Passed,
            "{:#?}",
            report.result.to_json()
        );
    }
    let instances = audit["instances"].as_array().unwrap();
    let assumptions: Vec<_> = instances
        .iter()
        .filter(|instance| instance["assumption"] == true)
        .map(|instance| instance["declaration"].as_str().unwrap())
        .collect();
    assert_eq!(assumptions, vec!["lcProof"]);
    assert!(instances
        .iter()
        .any(|instance| instance["declaration"] == target && instance["assumption"] == false));
    for dependency in ["unsafeCast", "unsafeCast.proof_1"] {
        let instance = instances
            .iter()
            .find(|instance| instance["declaration"] == dependency)
            .unwrap();
        assert_eq!(
            instance["assumption"], false,
            "unsafe runtime definitions must have checked bodies"
        );
        assert!(
            !instance["universe_arguments"]
                .as_array()
                .unwrap()
                .is_empty(),
            "the real polymorphic dependency must be instantiated explicitly"
        );
        assert!(!instance["declared_type"].as_str().unwrap().is_empty());
    }
}

#[test]
fn runtime_caller_checks_actual_polymorphic_instances_and_discloses_the_conditional_scope() {
    const EXPORT: &str =
        include_str!("fixtures/lean_conformance/cases_lean414/refusal/T02-008.useCoerce.ndjson");
    let report = checked(EXPORT, "T02_008.useCoerce");
    never_published(&report);
    assert_conditional_scope(&report, "T02_008.useCoerce");
    assert_eq!(
        report.result.status("assumption_policy"),
        StageStatus::Failed
    );
    assert_eq!(report.result.assumptions, vec!["lcProof".to_string()]);
    let taint = report.result.audit.as_ref().unwrap()["taint"]
        .as_array()
        .unwrap();
    assert_eq!(
        taint,
        &vec![serde_json::json!({
            "assumption":"lcProof",
            "path":["T02_008.useCoerce", "T02_008.coerceAnything", "unsafeCast", "unsafeCast.proof_1", "lcProof"]
        })]
    );
}

#[test]
fn wrong_universe_arity_cannot_be_hidden_by_a_runtime_local_assumption() {
    let mut rows = rows();
    let runtime_names: Vec<Value> = rows
        .iter()
        .filter(|row| row["def"]["safety"] == "unsafe")
        .map(|row| row["def"]["name"].clone())
        .collect();
    let mut modified = 0;
    for row in &mut rows {
        if row.get("const").is_some()
            && runtime_names.contains(&row["const"]["name"])
            && row["const"]["us"]
                .as_array()
                .is_some_and(|levels| !levels.is_empty())
        {
            row["const"]["us"]
                .as_array_mut()
                .unwrap()
                .push(serde_json::json!(0));
            modified += 1;
        }
    }
    assert!(modified > 0);
    let report = checked(&text(&rows), TARGET);
    never_published(&report);
    assert_ne!(report.result.status("primary_check"), StageStatus::Passed);
    assert_eq!(
        report.result.status("assumption_policy"),
        StageStatus::NotRun
    );
}
