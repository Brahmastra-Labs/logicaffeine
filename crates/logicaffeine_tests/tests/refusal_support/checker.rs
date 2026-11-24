//! The refusal checker: one original Lean declaration whose own `#print axioms` names a trust escape,
//! from its pinned export to a complete refusal (Tristen's ruling of 2026-09-24).
//!
//! For each row:
//! * the fixture is the pinned original per-root export (sha256, record count, header, toolchain,
//!   exporter, structural keys and the unchanged original source all match its provenance), and the
//!   provenance names this root;
//! * the axioms the test states are exactly Lean's `#print axioms` for the root, read from the Lean
//!   oracle (`crates/logicaffeine_tests/tests/fixtures/lean_conformance/lean_ground_truth`), and include `sorryAx`, `lcProof` or `Lean.trustCompiler`;
//! * the production pipeline (`pipeline::check_export`), given Lean's acceptance and Lean's axioms,
//!   translates the declaration and both kernels check it (`translated`, `primary_check`,
//!   `independent_recheck` passed) and the assumption policy refuses it (`assumption_policy` failed):
//!   outcome `DisallowedAssumptions` (exit 3), every diagnostic typed `DisallowedAssumption` at that
//!   stage and about this declaration, nothing emitted, nothing published;
//! * the result's assumptions are exactly Lean's axioms;
//! * for each escape the report gives the taint path from the target, and that path is the one Lean's
//!   own export yields (computed at pin time from the export alone, by a walk that shares no code with
//!   the kernel);
//! * control: the same export with a Lean report that leaves the escapes out is still refused — the
//!   policy's own trace disagrees with the report — and still publishes nothing.

#![allow(dead_code)]

use adapter::pipeline::{self, LeanEvidence, Limits};
use adapter::receipt::StageStatus;
use adapter::{DiagnosticCode, Outcome};
use logicaffeine_kernel::interface::lean as adapter;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

/// One refused row, with the facts Lean recorded for it and the taint path to each escape.
pub struct Refusal {
    pub fixture: &'static str,
    pub lean: &'static str,
    pub kind: &'static str,
    pub axioms: &'static [&'static str],
    pub taint: &'static [&'static [&'static str]],
}

/// The escapes whose rows close as `refused`.
pub const REFUSED_ESCAPES: [&str; 3] = ["sorryAx", "lcProof", "Lean.trustCompiler"];
/// Lean's three standard axioms: an assumption outside them is refused and gets a taint path.
pub const ALLOWED_AXIOMS: [&str; 3] = ["propext", "Quot.sound", "Classical.choice"];

fn repository() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/lean_conformance/cases_lean414/refusal")
}

/// The pinned export and its provenance, verified byte for byte against each other and against the
/// unchanged original source.
pub fn pinned(stem: &str) -> (String, Value, Vec<u8>) {
    let dir = fixtures();
    let export =
        std::fs::read_to_string(dir.join(format!("{stem}.ndjson"))).expect("pinned export");
    let provenance: Value = serde_json::from_str(
        &std::fs::read_to_string(dir.join(format!("{stem}.provenance.json"))).expect("provenance"),
    )
    .expect("provenance parses");
    assert_eq!(
        adapter::package::sha256_hex(export.as_bytes()),
        provenance["export_sha256"],
        "{stem}: export bytes"
    );
    assert_eq!(
        export.lines().count() as u64,
        provenance["records"].as_u64().unwrap(),
        "{stem}: record count"
    );
    let header: Value = serde_json::from_str(export.lines().next().unwrap()).unwrap();
    assert_eq!(header, provenance["export_header"], "{stem}: export header");
    assert_eq!(provenance["toolchain"], "leanprover/lean4:v4.14.0");
    assert_eq!(
        provenance["exporter_commit"],
        "20c4a64aa1b76f7c79a6db15d089084359f8c3dc"
    );
    assert_eq!(provenance["expression_equality"], "structural");
    assert_eq!(
        provenance["declaration_selection"],
        "explicit_authored_roots"
    );
    assert_eq!(provenance["compiler_exit"], 0);
    assert_eq!(provenance["exporter_exit"], 0);
    let source = std::fs::read(repository().join(provenance["source"].as_str().unwrap()))
        .expect("original source");
    assert_eq!(
        adapter::package::sha256_hex(&source),
        provenance["source_sha256"],
        "{stem}: the original source changed"
    );
    (export, provenance, source)
}

/// Lean's `#print axioms` for `lean`, as the Lean 4.14.0 oracle recorded it.
pub fn lean_print_axioms(lean: &str) -> Vec<String> {
    let case = lean.split('.').next().unwrap().replace('_', "-");
    let path = repository().join(format!(
        "crates/logicaffeine_tests/tests/fixtures/lean_conformance/lean_ground_truth/{case}.json"
    ));
    let truth: Value =
        serde_json::from_str(&std::fs::read_to_string(&path).expect("the Lean oracle's record"))
            .unwrap();
    assert_eq!(
        truth["lean"], "4.14.0",
        "{case}: the oracle ran the pinned Lean"
    );
    assert_eq!(
        truth["compiles"], true,
        "{case}: Lean compiled the original"
    );
    let declaration = truth["declarations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["name"] == lean)
        .unwrap_or_else(|| panic!("the Lean oracle has no {lean}"));
    declaration["axioms"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a.as_str().unwrap().to_string())
        .collect()
}

fn set(items: impl IntoIterator<Item = impl Into<String>>) -> BTreeSet<String> {
    items.into_iter().map(Into::into).collect()
}

/// What Lean's acceptance of the original tells the pipeline in project mode: the pinned toolchain
/// compiled the unchanged source (its provenance records `compiler_exit` 0), and `#print axioms` named
/// `axioms`.
fn accepted(provenance: &Value, source: &[u8], axioms: Vec<String>) -> LeanEvidence {
    let module = format!(
        "{}.lean",
        provenance["case"].as_str().unwrap().replace('-', "_")
    );
    LeanEvidence::Accepted {
        axioms,
        sources: BTreeMap::from([(module, source.to_vec())]),
        toolchain: provenance["toolchain"].as_str().unwrap().to_string(),
        exporter: format!(
            "lean4export {}",
            provenance["exporter_commit"].as_str().unwrap()
        ),
        dependencies: Vec::new(),
    }
}

/// The report's taint line for one path.
pub fn taint_line(path: &[&str]) -> String {
    format!(
        "taint path to {}: {}",
        path.last().unwrap(),
        path.join(" → ")
    )
}

pub fn check_refusal(r: &Refusal) {
    let (export, provenance, source) = pinned(r.fixture);
    let pinned_for = &provenance["refusal"];
    assert_eq!(
        pinned_for["root"], r.lean,
        "{}: the fixture was pinned for another root",
        r.fixture
    );
    assert_eq!(pinned_for["kind"], r.kind, "{}: kind", r.lean);

    // Lean's own #print axioms, and the escape that makes the refusal correct.
    let lean_axioms = lean_print_axioms(r.lean);
    assert_eq!(
        set(r.axioms.iter().copied()),
        set(lean_axioms.clone()),
        "{}: the stated axioms are Lean's",
        r.lean
    );
    assert_eq!(
        r.axioms.len(),
        lean_axioms.len(),
        "{}: no axiom is stated twice",
        r.lean
    );
    assert_eq!(
        pinned_for["lean_print_axioms"],
        serde_json::json!(lean_axioms),
        "{}: the pinned axioms are Lean's",
        r.lean
    );
    let escapes: BTreeSet<String> = set(lean_axioms
        .iter()
        .filter(|a| !ALLOWED_AXIOMS.contains(&a.as_str()))
        .cloned());
    assert!(
        escapes.iter().any(|a| REFUSED_ESCAPES.contains(&a.as_str())),
        "{}: Lean names no sorryAx, lcProof or Lean.trustCompiler — refusing it would refuse valid mathematics",
        r.lean
    );

    // The production pipeline, as project mode runs it.
    let checked = pipeline::check_export(
        &export,
        r.lean,
        &accepted(&provenance, &source, lean_axioms.clone()),
        &Limits::default(),
    );
    let result = &checked.result;
    let describe = || serde_json::to_string_pretty(&result.to_json()).unwrap();
    assert!(
        checked.package.is_none(),
        "{}: a refusal publishes nothing\n{}",
        r.lean,
        describe()
    );
    assert!(
        result.package.is_none(),
        "{}: the result names no package\n{}",
        r.lean,
        describe()
    );
    assert_eq!(
        result.outcome(),
        Outcome::DisallowedAssumptions,
        "{}: outcome\n{}",
        r.lean,
        describe()
    );
    assert_eq!(result.exit_code(), 3, "{}: exit code", r.lean);
    for (stage, status) in [
        ("lean_accepted", StageStatus::Passed),
        ("translated", StageStatus::Passed),
        ("primary_check", StageStatus::Passed),
        ("independent_recheck", StageStatus::Passed),
        ("assumption_policy", StageStatus::Failed),
        ("emission_reparsed", StageStatus::NotRun),
        ("fresh_replay", StageStatus::NotRun),
    ] {
        assert_eq!(
            result.status(stage),
            status,
            "{}: stage {stage}\n{}",
            r.lean,
            describe()
        );
    }
    assert!(
        result.target_logos.is_none(),
        "{}: nothing was emitted",
        r.lean
    );
    assert!(
        !result.diagnostics.is_empty(),
        "{}: a refusal says why",
        r.lean
    );
    for d in &result.diagnostics {
        assert_eq!(
            d.code,
            DiagnosticCode::DisallowedAssumption,
            "{}: {d:?}",
            r.lean
        );
        assert_eq!(d.stage, "assumption_policy", "{}: {d:?}", r.lean);
        assert_eq!(
            d.outcome(),
            Outcome::DisallowedAssumptions,
            "{}: {d:?}",
            r.lean
        );
        assert_eq!(d.declaration.as_deref(), Some(r.lean), "{}: {d:?}", r.lean);
    }
    assert_eq!(
        set(result.assumptions.clone()),
        set(lean_axioms.clone()),
        "{}: the reported axioms are exactly Lean's",
        r.lean
    );
    assert_eq!(
        result.assumptions.len(),
        lean_axioms.len(),
        "{}: no axiom reported twice",
        r.lean
    );

    // The taint path to each escape, as the report gives it and as Lean's own export yields it.
    let paths: BTreeMap<&str, &[&str]> = r
        .taint
        .iter()
        .map(|p| (*p.last().expect("a path ends at its escape"), *p))
        .collect();
    assert_eq!(
        paths.len(),
        r.taint.len(),
        "{}: one path per escape",
        r.lean
    );
    assert_eq!(
        set(paths.keys().copied()),
        escapes,
        "{}: a taint path for every escape and nothing else",
        r.lean
    );
    for (escape, path) in &paths {
        assert_eq!(
            path.first(),
            Some(&r.lean),
            "{}: a taint path starts at the target",
            r.lean
        );
        assert_eq!(
            pinned_for["lean_taint"][*escape],
            serde_json::json!(path),
            "{}: the path to {escape} is the one Lean's export yields",
            r.lean
        );
        let line = taint_line(path);
        let reported: Vec<&str> = result
            .diagnostics
            .iter()
            .filter(|d| d.message.starts_with(&format!("taint path to {escape}: ")))
            .map(|d| d.message.as_str())
            .collect();
        assert_eq!(
            reported,
            vec![line.as_str()],
            "{}: the report's taint path to {escape}\n{}",
            r.lean,
            describe()
        );
    }
    let taint_lines = result
        .diagnostics
        .iter()
        .filter(|d| d.message.starts_with("taint path to "))
        .count();
    assert_eq!(
        taint_lines,
        escapes.len(),
        "{}: one taint line per escape\n{}",
        r.lean,
        describe()
    );

    // The machine-readable document says the same.
    let json = result.to_json();
    assert_eq!(json["outcome"], "disallowed_assumptions");
    assert!(json["package"].is_null());
    assert_eq!(json["stages"]["assumption_policy"], "failed");
    assert_eq!(
        set(json["assumptions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| a.as_str().unwrap().to_string())),
        set(lean_axioms.clone())
    );

    // Control: a Lean report that leaves the escapes out is not a way in.
    let omitted: Vec<String> = lean_axioms
        .iter()
        .filter(|a| !escapes.contains(*a))
        .cloned()
        .collect();
    let lied = pipeline::check_export(
        &export,
        r.lean,
        &accepted(&provenance, &source, omitted),
        &Limits::default(),
    );
    assert!(
        lied.package.is_none() && lied.result.package.is_none(),
        "{}: a misreported Lean run publishes nothing",
        r.lean
    );
    assert_eq!(
        lied.result.outcome(),
        Outcome::DisallowedAssumptions,
        "{}: a misreported Lean run is still refused",
        r.lean
    );
    assert_eq!(
        lied.result.status("assumption_policy"),
        StageStatus::Failed,
        "{}",
        r.lean
    );
    assert_eq!(
        lied.result.diagnostics.first().map(|d| d.code),
        Some(DiagnosticCode::AssumptionDisagreement),
        "{}: the policy's own trace contradicts the report",
        r.lean
    );
}
