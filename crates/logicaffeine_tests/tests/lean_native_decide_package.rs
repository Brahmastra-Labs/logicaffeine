//! New package gates for a checked replacement proof with preserved original Lean evidence.
//! Authentic exports stay unchanged; manifest mutations are canonicalized and re-digested.
#[path = "harvest_support/checker.rs"]
mod checker;
use logicaffeine_kernel::interface::lean::native_decide;
use logicaffeine_kernel::interface::lean::package::{
    self, BuildOptions, InformationLoss, Manifest, PackageFiles, EXPORT, MANIFEST,
};
use logicaffeine_kernel::interface::lean::receipt::{Receipt, StageStatus};
use std::path::PathBuf;
use std::sync::OnceLock;

const TARGET: &str = "T08_014.viaNative";
const SHA: &str = "bb2f24bc2e131b2c40301a8130ec551662d2deae0bd19fdcf98fb988a2810273";
fn original() -> String {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/lean_conformance/cases_lean414/native_decide");
    let (export, provenance) = checker::fixture_in(&dir, "T08-014.viaNative");
    assert_eq!(package::sha256_hex(export.as_bytes()), SHA);
    assert_eq!(
        provenance["lean_print_axioms"],
        serde_json::json!(["'T08_014.viaNative' depends on axioms: [Lean.ofReduceBool]"])
    );
    export
}
fn original_axioms() -> Vec<String> {
    vec![native_decide::ELIMINATED_AXIOM.into()]
}
fn files() -> &'static PackageFiles {
    static FILES: OnceLock<PackageFiles> = OnceLock::new();
    FILES.get_or_init(|| {
        package::assemble_observed(
            &original(),
            TARGET,
            &BuildOptions::default(),
            Some(&original_axioms()),
        )
        .unwrap_or_else(|v| panic!("authentic native-rederived package: {v:?}"))
    })
}
fn manifest(files: &PackageFiles) -> Manifest {
    serde_json::from_slice(&files[MANIFEST]).unwrap()
}
fn edited(edit: impl FnOnce(&mut Manifest)) -> PackageFiles {
    let mut files = files().clone();
    edit_manifest(&mut files, edit);
    files
}
fn edit_manifest(files: &mut PackageFiles, edit: impl FnOnce(&mut Manifest)) {
    let mut m = manifest(files);
    edit(&mut m);
    m.files = files
        .iter()
        .filter(|(p, _)| p.as_str() != MANIFEST)
        .map(|(p, b)| (p.clone(), package::sha256_hex(b)))
        .collect();
    m.source.export_sha256 = m.files[EXPORT].clone();
    // The proposed /2 identity format is intentionally reproduced here so this test module
    // compiles against the unmodified public APIs for a genuine pre-fix RED run.
    let mut preimage = format!("{}\0{}\0", m.schema, m.target.lean);
    for (path, digest) in &m.files {
        preimage.push_str(path);
        preimage.push('\0');
        preimage.push_str(digest);
        preimage.push('\n');
    }
    m.result_id = format!("sha256:{}", package::sha256_hex(preimage.as_bytes()));
    files.insert(MANIFEST.into(), package::canonical_json(&m));
}
fn native_loss(m: &mut Manifest) -> &mut InformationLoss {
    m.information_losses
        .iter_mut()
        .find(|loss| loss.category == native_decide::INFORMATION_CATEGORY)
        .expect("authentic reconstruction disclosure")
}
fn semantic_refusal(receipt: &Receipt, stage: &str, code: &str) {
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
        .expect("semantic tampering must refuse");
    assert_eq!(
        (failure.name, failure.code.as_deref()),
        (stage, Some(code)),
        "{receipt:#?}"
    );
    assert!(!receipt.replayed());
    assert_eq!(receipt.status("fresh_replay"), StageStatus::Failed);
}

#[test]
fn native_package_preserves_original_evidence_and_replays_the_axiom_free_replacement() {
    let files = files();
    let m = manifest(files);
    assert_eq!(m.schema, "logos-lean-package/2");
    assert_eq!(m.assumptions.lean_print_axioms, Some(original_axioms()));
    assert!(m.assumptions.transitive.is_empty());
    assert_eq!(package::sha256_hex(&files[EXPORT]), SHA);
    let export = std::str::from_utf8(&files[EXPORT]).unwrap();
    let closure = native_decide::NativeDecisions::read(export).closure_of(&m.target.kernel);
    assert!(closure.eliminates());
    closure
        .disclosed_by(
            m.information_losses
                .iter()
                .map(|l| (l.category.as_str(), l.detail.as_str())),
        )
        .unwrap();
    assert_ne!(
        m.result_id,
        package::result_id(&m.target.lean, &m.files),
        "schema domains must differ"
    );
    let receipt = package::replay(files);
    assert!(receipt.replayed(), "{receipt:#?}");
    assert_eq!(receipt.status("translated"), StageStatus::Passed);
    assert_eq!(receipt.status("assumption_policy"), StageStatus::Passed);
    assert_eq!(receipt.status("lean_accepted"), StageStatus::NotRun);
    assert!(
        !receipt.verified(),
        "replay does not independently rerun Lean"
    );
}

#[test]
fn native_package_replay_rejects_missing_disclosure_after_both_kernels_pass() {
    let files = edited(|m| {
        m.information_losses
            .retain(|l| l.category != native_decide::INFORMATION_CATEGORY)
    });
    semantic_refusal(
        &package::replay(&files),
        "translated",
        "native_rederivation_disclosure",
    );
}

#[test]
fn native_package_replay_rejects_forged_or_duplicate_disclosure() {
    let forged = edited(|m| {
        native_loss(m)
            .detail
            .push_str(" The original proof was replayed unchanged.")
    });
    semantic_refusal(
        &package::replay(&forged),
        "translated",
        "native_rederivation_disclosure",
    );
    let hidden = edited(|m| native_loss(m).category = "UNRECOGNIZED_NATIVE_CHANGE".into());
    semantic_refusal(
        &package::replay(&hidden),
        "translated",
        "native_rederivation_disclosure",
    );
    let duplicate = edited(|m| {
        let extra = native_loss(m).clone();
        m.information_losses.push(extra);
    });
    semantic_refusal(
        &package::replay(&duplicate),
        "translated",
        "native_rederivation_disclosure",
    );
}

#[test]
fn native_package_replay_requires_original_evidence_separately_from_actual_assumptions() {
    for original in [
        None,
        Some(Vec::new()),
        Some(vec![native_decide::ELIMINATED_AXIOM.into(); 2]),
    ] {
        let files = edited(|m| m.assumptions.lean_print_axioms = original);
        semantic_refusal(
            &package::replay(&files),
            "assumption_policy",
            "assumption_disagreement",
        );
    }
}

#[test]
fn native_package_replay_recomputes_replacement_assumptions_instead_of_trusting_manifest() {
    let files =
        edited(|m| m.assumptions.transitive = vec!["/s:\"Lean\"/s:\"ofReduceBool\"".into()]);
    semantic_refusal(
        &package::replay(&files),
        "assumption_policy",
        "manifest_mismatch",
    );
}

#[test]
fn native_package_cannot_downgrade_to_the_original_proof_contract_even_with_recomputed_identity() {
    let files = edited(|m| {
        m.schema = package::PACKAGE_SCHEMA.into();
        m.information_losses
            .retain(|l| l.category != native_decide::INFORMATION_CATEGORY);
        m.assumptions.lean_print_axioms = None;
    });
    semantic_refusal(
        &package::replay(&files),
        "translated",
        "native_rederivation_schema",
    );
}

#[test]
fn native_package_build_does_not_publish_without_valid_original_assumption_evidence() {
    let export = original();
    for original in [
        None,
        Some(Vec::new()),
        Some(vec![native_decide::ELIMINATED_AXIOM.into(); 2]),
    ] {
        let build = package::build_observed(
            &export,
            TARGET,
            &BuildOptions::default(),
            original.as_deref(),
        )
        .unwrap_or_else(|v| {
            panic!("observed build should retain a precise refusal receipt: {v:?}")
        });
        assert_eq!(build.receipt.status("primary_check"), StageStatus::Passed);
        assert_eq!(
            build.receipt.status("independent_recheck"),
            StageStatus::Passed
        );
        let stage = build.receipt.stage("assumption_policy").unwrap();
        assert_eq!(stage.status, StageStatus::Failed);
        assert_eq!(stage.code.as_deref(), Some("assumption_disagreement"));
        assert!(package::publishable(build).is_err());
    }
}

#[test]
fn ordinary_package_keeps_its_schema_and_rejects_a_fabricated_native_disclosure() {
    let export = include_str!("fixtures/lean_package/W05.clean.ndjson");
    let mut files =
        package::assemble_observed(export, "W05.clean", &BuildOptions::default(), Some(&[]))
            .unwrap_or_else(|v| panic!("ordinary positive control: {v:?}"));
    let m = manifest(&files);
    assert_eq!(m.schema, package::PACKAGE_SCHEMA);
    assert_eq!(m.result_id, package::result_id(&m.target.lean, &m.files));
    assert!(package::replay(&files).replayed());
    edit_manifest(&mut files, |m| {
        m.information_losses.push(InformationLoss {
            category: native_decide::INFORMATION_CATEGORY.into(),
            detail: "a nonexistent decision was re-derived".into(),
        })
    });
    semantic_refusal(
        &package::replay(&files),
        "translated",
        "native_rederivation_disclosure",
    );
}

#[test]
fn an_authentic_general_compiler_bridge_keeps_its_escape_and_is_not_published() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/lean_conformance/cases_lean414/native_decide");
    let (export, _) = checker::fixture_in(&dir, "T06-014.bridgeShape");
    let original = vec!["Lean.trustCompiler".into(), "Lean.ofReduceBool".into()];
    let build = package::build_observed(
        &export,
        "T06_014.bridgeShape",
        &BuildOptions::default(),
        Some(&original),
    )
    .unwrap_or_else(|v| panic!("checked general compiler bridge: {v:?}"));
    assert_eq!(build.receipt.status("primary_check"), StageStatus::Passed);
    assert_eq!(
        build.receipt.status("independent_recheck"),
        StageStatus::Passed
    );
    assert_eq!(
        build.receipt.status("assumption_policy"),
        StageStatus::Failed
    );
    let m = manifest(&build.files);
    assert_eq!(m.schema, package::PACKAGE_SCHEMA);
    assert_eq!(m.assumptions.lean_print_axioms, Some(original));
    assert!(m
        .assumptions
        .transitive
        .contains(&"/s:\"Lean\"/s:\"ofReduceBool\"".to_string()));
    assert!(!m
        .information_losses
        .iter()
        .any(|l| l.category == native_decide::INFORMATION_CATEGORY));
    assert!(package::publishable(build).is_err());
}

#[test]
fn pipeline_result_discloses_exactly_the_native_changes_its_package_carries() {
    use logicaffeine_kernel::interface::lean::pipeline::{self, LeanEvidence, Limits};
    let evidence = LeanEvidence::Accepted {
        axioms: original_axioms(),
        sources: Default::default(),
        toolchain: "leanprover/lean4:v4.14.0".into(),
        exporter: "lean4export 20c4a64aa1b76f7c79a6db15d089084359f8c3dc".into(),
    };
    // Accepted evidence is the fixture's recorded oracle, not a claim this test reran Lean.
    let checked = pipeline::check_export(&original(), TARGET, &evidence, &Limits::default());
    let files = checked
        .package
        .as_ref()
        .unwrap_or_else(|| panic!("pipeline refusal: {:#?}", checked.result));
    let m = manifest(files);
    let result_changes: Vec<_> = checked
        .result
        .information_losses
        .iter()
        .filter(|l| l.category == native_decide::INFORMATION_CATEGORY)
        .map(|l| (&l.category, &l.detail))
        .collect();
    let package_changes: Vec<_> = m
        .information_losses
        .iter()
        .filter(|l| l.category == native_decide::INFORMATION_CATEGORY)
        .map(|l| (&l.category, &l.detail))
        .collect();
    assert_eq!(result_changes, package_changes);
    assert_eq!(result_changes.len(), 1);
    assert!(checked.result.assumptions.is_empty());
    assert_eq!(m.assumptions.lean_print_axioms, Some(original_axioms()));
    assert_eq!(
        checked.result.receipt.status("fresh_replay"),
        StageStatus::NotRun
    );
}
