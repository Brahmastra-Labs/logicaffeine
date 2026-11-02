//! Projection-only packages must bind the source parameter/index split.
//! New controls; existing assertions and authentic fixture bytes remain unchanged.
use logicaffeine_kernel::certificate::{EnvironmentCertificate, EnvironmentEntry};
use logicaffeine_kernel::interface::lean::package::{self, BuildOptions, Manifest, PackageFiles};
use logicaffeine_kernel::interface::lean::receipt::{Receipt, StageStatus};
use serde_json::{json, Value};

const SOURCE: &str = include_str!("fixtures/lean_semantics/LF_W03_IndexedProjection.ndjson");
const TARGET: &str = "LF_W03P.paramFirst";
const FAMILY: &str = "/s:\"LF_W03P\"/s:\"PBox\"";

fn passed(receipt: &Receipt) {
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

fn redigest(files: &mut PackageFiles) {
    let mut manifest: Manifest = serde_json::from_slice(&files[package::MANIFEST]).unwrap();
    manifest.files = files
        .iter()
        .filter(|(path, _)| *path != package::MANIFEST)
        .map(|(path, bytes)| (path.clone(), package::sha256_hex(bytes)))
        .collect();
    manifest.source.export_sha256 = manifest.files[package::EXPORT].clone();
    manifest.result_id = package::result_id(&manifest.target.lean, &manifest.files);
    files.insert(package::MANIFEST.into(), package::canonical_json(&manifest));
}

fn name_id(rows: &[Value], name: &str) -> u64 {
    name.split('.').fold(0, |prefix, part| {
        rows.iter()
            .find(|row| row["str"]["pre"] == prefix && row["str"]["str"] == part)
            .unwrap()["in"]
            .as_u64()
            .unwrap()
    })
}

fn original() -> PackageFiles {
    let built = package::build(SOURCE, TARGET, &BuildOptions::default()).unwrap();
    passed(&built.receipt);
    passed(&package::replay(&built.files));
    let certificate: EnvironmentCertificate =
        serde_json::from_slice(&built.files[package::CERTIFICATE]).unwrap();
    assert!(certificate.entries.iter().any(|entry| matches!(entry,
        EnvironmentEntry::Inductive { types, .. } if types.iter().any(|member|
            member.name == FAMILY && member.num_params == 1))));
    assert!(
        !certificate
            .entries
            .iter()
            .flat_map(|entry| entry.names())
            .any(|name| name.starts_with(&format!("{FAMILY}/s:\"rec"))),
        "this fixture must exercise projection metadata without a carried family recursor"
    );
    built.files
}

#[test]
fn authentic_projection_only_package_preserves_its_source_metadata() {
    original();
}

#[test]
fn rehashed_source_parameter_reclassification_cannot_reuse_certificate_projection_semantics() {
    let mut files = original();
    let mut rows: Vec<Value> = SOURCE
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let family = name_id(&rows, "LF_W03P.PBox");
    let original_expressions: Vec<_> = rows
        .iter()
        .filter(|row| row.get("ie").is_some())
        .cloned()
        .collect();
    let block = rows
        .iter_mut()
        .find_map(|row| {
            row.get_mut("inductive").filter(|block| {
                block["types"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|member| member["name"] == family)
            })
        })
        .unwrap();
    let member = block["types"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|member| member["name"] == family)
        .unwrap();
    assert_eq!(member["numParams"], 1);
    assert_eq!(member["numIndices"], 1);
    member["numParams"] = json!(0);
    member["numIndices"] = json!(2);
    // Same constructor type, but the formerly uniform type parameter becomes a field.
    for constructor in block["ctors"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .filter(|constructor| constructor["induct"] == family)
    {
        assert_eq!(constructor["numParams"], 1);
        let fields = constructor["numFields"].as_u64().unwrap();
        constructor["numParams"] = json!(0);
        constructor["numFields"] = json!(fields + 1);
    }
    assert_eq!(
        original_expressions,
        rows.iter()
            .filter(|row| row.get("ie").is_some())
            .cloned()
            .collect::<Vec<_>>()
    );
    // This is an intentionally corrupted source export, not an authentic Lean fixture.
    // No carried recursor permits canonical block derivation to catch the mismatch for us.
    let mutated: String = rows.iter().map(|row| row.to_string() + "\n").collect();
    files.insert(package::EXPORT.into(), mutated.into_bytes());
    redigest(&mut files);
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
        "the source cannot borrow the certificate's conflicting parameter split: {replay:#?}"
    );
    assert!(!replay.verified());
}
