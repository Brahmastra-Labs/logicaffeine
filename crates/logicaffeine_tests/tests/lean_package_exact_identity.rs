//! Package-level routing of a raw target identity through manifest Lean spelling.
mod lean_package_support;

use lean_package_support::Export;
use logicaffeine_kernel::interface::lean::package::{
    self, BuildOptions, CorrespondenceFile, Manifest,
};
use logicaffeine_kernel::interface::lean::receipt::{Receipt, StageStatus};
use logicaffeine_kernel::interface::lean::DiagnosticCode;
use serde_json::{json, Value};

const AUTHENTIC: &str = include_str!("fixtures/lean_semantics/LF_W03_IndexedProjection.ndjson");
const USER_ZERO: &str = "/s:\"Zero\"";

fn root_name(rows: &[Value], spelling: &str) -> u64 {
    rows.iter()
        .find(|row| row["str"]["pre"] == 0 && row["str"]["str"] == spelling)
        .unwrap()["in"]
        .as_u64()
        .unwrap()
}

fn source() -> String {
    let rows: Vec<Value> = AUTHENTIC
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let nat = root_name(&rows, "Nat");
    let alias = root_name(&rows, "Zero");
    let zero = rows
        .iter()
        .find(|row| row["str"]["pre"] == nat && row["str"]["str"] == "zero")
        .unwrap()["in"]
        .as_u64()
        .unwrap();
    let next = rows
        .iter()
        .filter_map(|row| row["ie"].as_u64())
        .max()
        .unwrap()
        + 1;
    let mut out = Export::new();
    // Keep the authentic header/tables and canonical Nat block exactly as recorded.
    // Other declarations, including the authentic Zero class, are omitted.
    out.lines = rows
        .iter()
        .filter(|row| {
            row.get("meta").is_some()
                || row.get("in").is_some()
                || row.get("il").is_some()
                || row.get("ie").is_some()
                || row["inductive"]["types"]
                    .as_array()
                    .is_some_and(|types| types.iter().any(|member| member["name"] == nat))
        })
        .map(Value::to_string)
        .collect();
    // This appended declaration is a synthetic adversarial fixture, not a claimed Lean export:
    // def Zero : Nat := Nat.zero
    out.raw(json!({"ie":next,"const":{"name":nat,"us":[]}}).to_string());
    out.raw(json!({"ie":next+1,"const":{"name":zero,"us":[]}}).to_string());
    out.raw(
        json!({"def":{"name":alias,"levelParams":[],"type":next,"value":next+1,
        "safety":"safe","hints":"abbrev","all":[alias]}})
        .to_string(),
    );
    out.text()
}

fn checked(receipt: &Receipt) {
    for stage in [
        "primary_check",
        "independent_recheck",
        "translated",
        "assumption_policy",
    ] {
        assert_eq!(
            receipt.status(stage),
            StageStatus::Passed,
            "{stage}: {receipt:#?}"
        );
    }
}

#[test]
fn raw_zero_target_roundtrips_through_lean_manifest_without_becoming_nat_zero() {
    let source = source();
    // Public mixed-name requests retain their explicit ambiguity refusal.
    let error = package::build(&source, "Zero", &BuildOptions::default())
        .err()
        .expect("ambiguous public target");
    assert_eq!(error.reason().code(), DiagnosticCode::MalformedExport);

    let built = package::build(&source, USER_ZERO, &BuildOptions::default()).unwrap();
    checked(&built.receipt);
    let manifest: Manifest = serde_json::from_slice(&built.files[package::MANIFEST]).unwrap();
    assert_eq!(manifest.target.lean, "Zero");
    assert_eq!(manifest.target.kernel, USER_ZERO);
    let correspondence: CorrespondenceFile =
        serde_json::from_slice(&built.files[package::CORRESPONDENCE]).unwrap();
    for (lean, kernel) in [("Zero", USER_ZERO), ("Nat.zero", "Zero")] {
        assert!(
            correspondence
                .rows
                .iter()
                .any(|row| row.lean == lean && row.kernel == kernel),
            "missing distinct identity {lean} -> {kernel}"
        );
    }
    let replay = package::replay(&built.files);
    checked(&replay);
    assert_eq!(
        replay.status("fresh_replay"),
        StageStatus::Passed,
        "{replay:#?}"
    );

    // Rehash the manifest after replacing only its explicit Lean target. The certificate still
    // proves the checked definition /s:"Zero"; the constructor Zero must not become that target.
    let mut files = built.files;
    let mut forged = manifest;
    forged.target.lean = "Nat.zero".into();
    forged.result_id = package::result_id(&forged.target.lean, &forged.files);
    files.insert(package::MANIFEST.into(), package::canonical_json(&forged));
    let replay = package::replay(&files);
    assert_eq!(
        replay.status("primary_check"),
        StageStatus::Passed,
        "{replay:#?}"
    );
    assert_eq!(
        replay.status("independent_recheck"),
        StageStatus::Passed,
        "{replay:#?}"
    );
    assert_eq!(
        replay.status("translated"),
        StageStatus::Failed,
        "{replay:#?}"
    );
    assert_eq!(
        replay.stage("translated").unwrap().code.as_deref(),
        Some("target_mismatch"),
        "{replay:#?}"
    );
    assert!(!replay.verified());
}
