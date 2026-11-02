//! Opaque data keeps its declaration kind through capture, serialized evidence and replay.
mod lean_package_support;

use lean_package_support::{small_theorem, Export};
use logicaffeine_kernel::certificate::{
    CaptureRequest, ConversionDiscipline, EnvironmentBase, EnvironmentCertificate,
    EnvironmentEntry, EnvironmentLimits, TermNode,
};
use logicaffeine_kernel::interface::lean::receipt::StageStatus;
use logicaffeine_kernel::interface::lean::{self as adapter, intake, package};
use logicaffeine_kernel::Universe;
use std::collections::BTreeMap;

fn opaque_data_export() -> String {
    let mut e = Export::new();
    e.lines[0] = include_str!("fixtures/lean_package/W05.clean.ndjson")
        .lines()
        .next()
        .unwrap()
        .to_string();
    let namespace = e.name("OpaqueData", 0);
    let data = e.name("Data", namespace);
    let sealed = e.name("sealed", namespace);
    let root = e.name("root", namespace);
    let x = e.name("x", 0);
    let level = e.succ(0);
    let type0 = e.sort(level);
    e.axiom(data, type0, &[]);
    let data_type = e.constant(data, &[]);
    let function_type = e.forall(x, data_type, data_type);
    let variable = e.bvar(0);
    let body = e.lam(x, data_type, variable);
    e.raw(format!(
        r#"{{"opaque":{{"name":{sealed},"levelParams":[],"type":{function_type},"value":{body},"isUnsafe":false,"all":[{sealed}]}}}}"#
    ));
    let sealed_value = e.constant(sealed, &[]);
    e.def(root, function_type, sealed_value);
    e.text()
}

fn capture(export: &str, target: &str) -> EnvironmentCertificate {
    let target = intake::kernel_name(target).unwrap();
    let read = intake::read(export, &intake::STAGE_A).unwrap();
    let closure = read.closure(&target).unwrap();
    let (imported, _) = adapter::import_retaining_closure(export, &target, &closure).unwrap();
    let proofs: BTreeMap<_, _> = imported
        .retained_declarations()
        .map(|(name, _, body)| (name.to_string(), body.clone()))
        .collect();
    EnvironmentCertificate::capture(&CaptureRequest {
        context: &imported,
        base: EnvironmentBase::Kernel,
        conversion: ConversionDiscipline::Exact,
        target: &target,
        opaque_proofs: &proofs,
    })
    .unwrap()
}

#[test]
fn opaque_data_survives_capture_serialization_and_both_kernel_checks() {
    let export = opaque_data_export();
    let sealed = intake::kernel_name("OpaqueData.sealed").unwrap();
    let data = intake::kernel_name("OpaqueData.Data").unwrap();
    for target in ["OpaqueData.root", "OpaqueData.sealed"] {
        let certificate = capture(&export, target);
        let json = serde_json::to_value(&certificate).unwrap();
        let entry = json["entries"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["name"] == sealed)
            .unwrap();
        assert_eq!(
            entry["kind"], "opaque",
            "an opaque data function is not a theorem or a transparent definition"
        );
        let decoded: EnvironmentCertificate = serde_json::from_value(json).unwrap();
        assert_eq!(decoded, certificate);
        let rebuilt = decoded.rebuild(&EnvironmentLimits::default()).unwrap();
        assert!(
            rebuilt.context.get_definition_body(&sealed).is_none(),
            "opaque body stays sealed"
        );
        assert!(
            rebuilt.context.get_universe_poly(&sealed).is_none(),
            "opaque data does not become an unfolding theorem"
        );
        assert!(rebuilt.context.poly_declaration(&sealed).is_some());
        assert!(
            rebuilt.proofs.contains_key(&sealed),
            "the checked body remains available as evidence"
        );
        assert_eq!(rebuilt.axioms.axioms, [data.clone()].into_iter().collect());
        assert!(rebuilt.axioms.unresolved.is_empty());
        assert!(rebuilt.independent.iter().any(|(name, _)| name == &sealed));
        assert!(rebuilt.independent.iter().all(|(_, verdict)| matches!(
            verdict,
            logicaffeine_kernel::certificate::IndependentVerdict::Agreed
        )));
    }
}

#[test]
fn an_opaque_body_is_checked_again_when_rebuilding() {
    let export = opaque_data_export();
    let sealed = intake::kernel_name("OpaqueData.sealed").unwrap();
    let certificate = capture(&export, "OpaqueData.root");
    let mut json = serde_json::to_value(&certificate).unwrap();
    let bad_row = json["nodes"].as_array().unwrap().len();
    json["nodes"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::to_value(TermNode::Sort(Universe::Prop)).unwrap());
    let entry = json["entries"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|e| e["name"] == sealed)
        .unwrap();
    assert_eq!(entry["kind"], "opaque");
    entry["body"] = serde_json::json!(bad_row);
    let corrupted: EnvironmentCertificate = serde_json::from_value(json).unwrap();
    assert!(
        corrupted.rebuild(&EnvironmentLimits::default()).is_err(),
        "the carried opaque body is evidence, never an unchecked assertion"
    );
}

#[test]
fn a_failed_rebuild_never_compares_an_unobserved_empty_axiom_set_with_lean() {
    let mut export = small_theorem();
    export.lines[0] = include_str!("fixtures/lean_package/W05.clean.ndjson")
        .lines()
        .next()
        .unwrap()
        .to_string();
    let build = package::build_observed(
        &export.text(),
        "T",
        &package::BuildOptions::default(),
        Some(&["P".to_string(), "p".to_string()]),
    )
    .unwrap();
    assert_eq!(build.receipt.status("primary_check"), StageStatus::Passed);
    let mut files = build.files;
    let mut certificate: EnvironmentCertificate =
        serde_json::from_slice(&files[package::CERTIFICATE]).unwrap();
    let wrong = certificate.nodes.len() as u32;
    certificate.nodes.push(TermNode::Sort(Universe::Prop));
    let target = certificate.target.clone();
    let mut changed = false;
    for entry in &mut certificate.entries {
        if let EnvironmentEntry::Theorem { name, proof, .. } = entry {
            if name == &target {
                *proof = wrong;
                changed = true;
            }
        }
    }
    assert!(changed);
    files.insert(
        package::CERTIFICATE.into(),
        package::canonical_certificate(&certificate),
    );
    let mut manifest: package::Manifest =
        serde_json::from_slice(&files[package::MANIFEST]).unwrap();
    manifest.files = files
        .iter()
        .filter(|(path, _)| *path != package::MANIFEST)
        .map(|(path, bytes)| (path.clone(), package::sha256_hex(bytes)))
        .collect();
    manifest.source.export_sha256 = manifest.files[package::EXPORT].clone();
    manifest.result_id = package::result_id(&manifest.target.lean, &manifest.files);
    files.insert(package::MANIFEST.into(), package::canonical_json(&manifest));
    let replay = package::replay(&files);
    assert_eq!(
        replay.status("primary_check"),
        StageStatus::Failed,
        "{replay:#?}"
    );
    assert_eq!(
        replay.status("assumption_policy"),
        StageStatus::NotRun,
        "{replay:#?}"
    );
    assert_ne!(
        replay.stage("assumption_policy").unwrap().code.as_deref(),
        Some("assumption_disagreement")
    );
    assert!(!replay.verified());
}

#[test]
fn the_original_schema_still_reads_and_rebuilds_its_original_entry_kinds() {
    let mut export = small_theorem();
    export.lines[0] = include_str!("fixtures/lean_package/W05.clean.ndjson")
        .lines()
        .next()
        .unwrap()
        .to_string();
    let certificate = capture(&export.text(), "T");
    assert_eq!(
        certificate.schema, "logos-environment-certificate/1",
        "original entry kinds retain their minimum wire version"
    );
    let json = serde_json::to_string(&certificate).unwrap();
    let read = logicaffeine_kernel::certificate::read_certificate(&json).unwrap();
    let logicaffeine_kernel::certificate::AnyCertificate::Environment(read) = read else {
        panic!("the legacy tag still denotes an environment certificate");
    };
    assert_eq!(
        read.schema, "logos-environment-certificate/1",
        "reading preserves the tag"
    );
    let rebuilt = read.rebuild(&EnvironmentLimits::default()).unwrap();
    assert_eq!(rebuilt.axioms.axioms.len(), 2);
    assert!(rebuilt.axioms.unresolved.is_empty());
}

#[test]
fn an_opaque_entry_cannot_be_smuggled_under_the_original_schema() {
    let mut certificate = capture(&opaque_data_export(), "OpaqueData.root");
    assert_eq!(certificate.schema, "logos-environment-certificate/2");
    certificate.schema = "logos-environment-certificate/1".into();
    let json = serde_json::to_string(&certificate).unwrap();
    assert!(logicaffeine_kernel::certificate::read_certificate(&json).is_err());
    assert!(certificate.rebuild(&EnvironmentLimits::default()).is_err());
}

#[test]
fn the_manifest_records_the_minimum_schema_selected_for_its_certificate() {
    let mut original = small_theorem();
    original.lines[0] = include_str!("fixtures/lean_package/W05.clean.ndjson")
        .lines()
        .next()
        .unwrap()
        .to_string();
    for (export, target, expected) in [
        (original.text(), "T", "logos-environment-certificate/1"),
        (
            opaque_data_export(),
            "OpaqueData.root",
            "logos-environment-certificate/2",
        ),
    ] {
        let build = package::build(&export, target, &package::BuildOptions::default()).unwrap();
        assert_eq!(
            build.receipt.status("primary_check"),
            StageStatus::Passed,
            "{:#?}",
            build.receipt
        );
        let manifest: package::Manifest =
            serde_json::from_slice(&build.files[package::MANIFEST]).unwrap();
        let certificate: EnvironmentCertificate =
            serde_json::from_slice(&build.files[package::CERTIFICATE]).unwrap();
        assert_eq!(certificate.schema, expected);
        assert_eq!(manifest.toolchain.certificate_schema, certificate.schema);
    }
}

#[test]
fn unknown_certificate_versions_are_refused_at_both_reading_doors() {
    let mut certificate = capture(&opaque_data_export(), "OpaqueData.root");
    certificate.schema = "logos-environment-certificate/99".into();
    let json = serde_json::to_string(&certificate).unwrap();
    assert!(logicaffeine_kernel::certificate::read_certificate(&json).is_err());
    assert!(certificate.rebuild(&EnvironmentLimits::default()).is_err());
}

fn resign_kind_mutation(
    files: &mut BTreeMap<String, Vec<u8>>,
    certificate: &EnvironmentCertificate,
) {
    files.insert(
        package::CERTIFICATE.into(),
        package::canonical_certificate(certificate),
    );
    let mut manifest: package::Manifest =
        serde_json::from_slice(&files[package::MANIFEST]).unwrap();
    manifest.toolchain.certificate_schema = certificate.schema.clone();
    manifest.files = files
        .iter()
        .filter(|(path, _)| *path != package::MANIFEST)
        .map(|(path, bytes)| (path.clone(), package::sha256_hex(bytes)))
        .collect();
    manifest.source.export_sha256 = manifest.files[package::EXPORT].clone();
    manifest.result_id = package::result_id(&manifest.target.lean, &manifest.files);
    files.insert(package::MANIFEST.into(), package::canonical_json(&manifest));
}

#[test]
fn a_type_correct_kind_substitution_is_refused_by_the_source_binding() {
    for (target, changed_name, replacement_kind) in [
        ("OpaqueData.root", "OpaqueData.sealed", "definition"),
        ("OpaqueData.sealed", "OpaqueData.sealed", "definition"),
        ("OpaqueData.root", "OpaqueData.root", "opaque"),
    ] {
        let mut files = package::build(
            &opaque_data_export(),
            target,
            &package::BuildOptions::default(),
        )
        .unwrap()
        .files;
        let mut json: serde_json::Value =
            serde_json::from_slice(&files[package::CERTIFICATE]).unwrap();
        let name = intake::kernel_name(changed_name).unwrap();
        let entry = json["entries"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|e| e["name"] == name)
            .unwrap();
        assert_ne!(entry["kind"], replacement_kind);
        entry["kind"] = serde_json::json!(replacement_kind);
        let certificate: EnvironmentCertificate = serde_json::from_value(json).unwrap();
        certificate
            .rebuild(&EnvironmentLimits::default())
            .expect("the adversary kept a valid type and body");
        resign_kind_mutation(&mut files, &certificate);
        let receipt = package::replay(&files);
        assert_eq!(
            receipt.status("primary_check"),
            StageStatus::Passed,
            "{target}: {receipt:#?}"
        );
        assert_eq!(
            receipt.status("translated"),
            StageStatus::Failed,
            "{target}: {receipt:#?}"
        );
        assert_eq!(
            receipt.stage("translated").unwrap().code.as_deref(),
            Some("kind_mismatch"),
            "{receipt:#?}"
        );
        assert!(!receipt.verified());
    }
}

#[test]
fn an_opaque_proof_cannot_impersonate_a_lean_theorem() {
    let mut export = small_theorem();
    export.lines[0] = include_str!("fixtures/lean_package/W05.clean.ndjson")
        .lines()
        .next()
        .unwrap()
        .to_string();
    let mut files = package::build(&export.text(), "T", &package::BuildOptions::default())
        .unwrap()
        .files;
    let mut json: serde_json::Value = serde_json::from_slice(&files[package::CERTIFICATE]).unwrap();
    let target = json["target"].clone();
    let entry = json["entries"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|e| e["name"] == target)
        .unwrap();
    assert_eq!(entry["kind"], "theorem");
    entry["kind"] = serde_json::json!("opaque");
    let proof = entry.as_object_mut().unwrap().remove("proof").unwrap();
    entry["body"] = proof;
    json["schema"] = serde_json::json!("logos-environment-certificate/2");
    let certificate: EnvironmentCertificate = serde_json::from_value(json).unwrap();
    certificate
        .rebuild(&EnvironmentLimits::default())
        .expect("a checked opaque proof remains a proof");
    resign_kind_mutation(&mut files, &certificate);
    let receipt = package::replay(&files);
    assert_eq!(
        receipt.status("primary_check"),
        StageStatus::Passed,
        "{receipt:#?}"
    );
    assert_eq!(
        receipt.stage("translated").unwrap().code.as_deref(),
        Some("kind_mismatch"),
        "{receipt:#?}"
    );
    assert!(!receipt.verified());
}
