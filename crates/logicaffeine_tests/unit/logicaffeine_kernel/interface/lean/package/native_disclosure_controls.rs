//! Replay validates native disclosure against the carried target's source closure.
use super::*;
use crate::interface::lean::native_decide::{NativeDecisions, INFORMATION_CATEGORY};
use std::sync::OnceLock;

const CLEAN: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../logicaffeine_tests/tests/fixtures/lean_package/W05.clean.ndjson"
));
const NATIVE: &str=include_str!(concat!(env!("CARGO_MANIFEST_DIR"),"/../logicaffeine_tests/tests/fixtures/lean_conformance/cases_lean414/native_decide/T08-014.viaNative.ndjson"));
fn clean() -> PackageFiles {
    static FILES: OnceLock<PackageFiles> = OnceLock::new();
    FILES
        .get_or_init(|| {
            assemble_observed(CLEAN, "W05.clean", &BuildOptions::default(), Some(&[]))
                .expect("unchanged ordinary source builds")
        })
        .clone()
}
fn edit(files: &mut PackageFiles, change: impl FnOnce(&mut Manifest)) {
    let mut manifest: Manifest = serde_json::from_slice(&files[MANIFEST]).unwrap();
    change(&mut manifest);
    manifest.files = files
        .iter()
        .filter(|(name, _)| name.as_str() != MANIFEST)
        .map(|(name, bytes)| (name.clone(), sha256_hex(bytes)))
        .collect();
    manifest.source.export_sha256 = manifest.files[EXPORT].clone();
    manifest.result_id = result_id(&manifest.target.lean, &manifest.files);
    files.insert(MANIFEST.into(), canonical_json(&manifest));
}
fn refused(files: &PackageFiles) {
    let receipt = replay(files);
    assert_eq!(
        receipt.status("primary_check"),
        StageStatus::Passed,
        "{receipt:#?}"
    );
    assert_eq!(
        receipt.status("independent_recheck"),
        StageStatus::Passed,
        "{receipt:#?}"
    );
    let failure = receipt
        .first_failure()
        .expect("semantic disclosure tampering must be refused");
    assert_eq!(
        (failure.name, failure.code.as_deref()),
        ("translated", Some("native_rederivation_disclosure")),
        "{receipt:#?}"
    );
    assert!(!receipt.replayed());
}
fn real_changes() -> Vec<InformationLoss> {
    assert_eq!(
        sha256_hex(NATIVE.as_bytes()),
        "bb2f24bc2e131b2c40301a8130ec551662d2deae0bd19fdcf98fb988a2810273"
    );
    let closure = NativeDecisions::read(NATIVE).closure_of("/s:\"T08_014\"/s:\"viaNative\"");
    assert!(
        closure.eliminates(),
        "fixture must contain the recognized target reconstruction"
    );
    closure
        .information_changes()
        .into_iter()
        .map(|(category, detail)| InformationLoss { category, detail })
        .collect()
}

#[test]
fn ordinary_replay_keeps_its_schema_and_unrelated_metadata() {
    let mut files = clean();
    edit(&mut files, |manifest| {
        manifest.information_losses.push(InformationLoss {
            category: "OTHER_INFORMATION".into(),
            detail: "unrelated metadata is not native authority".into(),
        })
    });
    let manifest: Manifest = serde_json::from_slice(&files[MANIFEST]).unwrap();
    assert_eq!(manifest.schema, PACKAGE_SCHEMA);
    assert_eq!(
        manifest.result_id,
        result_id(&manifest.target.lean, &manifest.files)
    );
    assert!(replay(&files).replayed());
}

#[test]
fn fabricated_and_duplicated_native_disclosures_are_refused_after_both_checks() {
    for count in [1, 2] {
        let mut files = clean();
        edit(&mut files, |manifest| {
            for _ in 0..count {
                manifest.information_losses.push(InformationLoss {
                    category: INFORMATION_CATEGORY.into(),
                    detail: "a nonexistent decision was re-derived".into(),
                });
            }
        });
        refused(&files);
    }
}

#[test]
fn a_genuine_native_disclosure_for_another_source_target_is_not_authority() {
    let changes = real_changes();
    assert!(!changes.is_empty());
    let mut files = clean();
    edit(&mut files, |manifest| {
        manifest.information_losses.extend(changes)
    });
    refused(&files);
}

#[test]
fn exact_source_derived_native_disclosure_is_preserved_by_the_existing_comparator() {
    let native = NativeDecisions::read(NATIVE);
    let closure = native.closure_of("/s:\"T08_014\"/s:\"viaNative\"");
    let changes = real_changes();
    assert!(closure
        .disclosed_by(
            changes
                .iter()
                .map(|l| (l.category.as_str(), l.detail.as_str()))
        )
        .is_ok());
    assert!(closure
        .disclosed_by(std::iter::empty::<(&str, &str)>())
        .is_err());
    let mut duplicated = changes.clone();
    duplicated.extend(changes.clone());
    assert!(closure
        .disclosed_by(
            duplicated
                .iter()
                .map(|l| (l.category.as_str(), l.detail.as_str()))
        )
        .is_err());
    let mut forged = changes.clone();
    forged[0].detail.push_str(" forged");
    assert!(closure
        .disclosed_by(
            forged
                .iter()
                .map(|l| (l.category.as_str(), l.detail.as_str()))
        )
        .is_err());
    assert!(closure
        .disclosed_by(
            changes
                .iter()
                .map(|l| ("OTHER_INFORMATION", l.detail.as_str()))
        )
        .is_err());
}
