//! The export decides a package's contract. A target whose checked proof re-derives native
//! decisions is `logos-lean-package/2`, every other target `/1`, whatever the manifest says; the
//! schema is the result identity's domain; and Lean's report is held against the checked proof as
//! the contract says — under `/2` it is required and must name exactly the proof's assumptions and
//! the eliminated escape.
use super::*;
use crate::interface::lean::native_decide::{Rederivation, ELIMINATED_AXIOM};
use std::sync::OnceLock;

const CLEAN: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../logicaffeine_tests/tests/fixtures/lean_package/W05.clean.ndjson"
));
const NATIVE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../logicaffeine_tests/tests/fixtures/lean_conformance/cases_lean414/native_decide/T08-014.viaNative.ndjson"
));
const NATIVE_TARGET: &str = "T08_014.viaNative";
const TRUST_COMPILER: &str = "Lean.trustCompiler";

fn native_key() -> String {
    crate::interface::lean::intake::kernel_name(NATIVE_TARGET).unwrap()
}

fn native_elimination() -> Elimination {
    let elimination = NativeDecisions::read(NATIVE).closure_of(&native_key());
    assert!(
        elimination.eliminates(),
        "the fixture's target re-derives its native decision"
    );
    elimination
}

fn clean() -> PackageFiles {
    static FILES: OnceLock<PackageFiles> = OnceLock::new();
    FILES
        .get_or_init(|| {
            assemble_observed(CLEAN, "W05.clean", &BuildOptions::default(), Some(&[]))
                .expect("the ordinary package builds")
        })
        .clone()
}

fn native() -> PackageFiles {
    static FILES: OnceLock<PackageFiles> = OnceLock::new();
    FILES
        .get_or_init(|| {
            assemble_observed(
                NATIVE,
                NATIVE_TARGET,
                &BuildOptions::default(),
                Some(&[ELIMINATED_AXIOM.to_string()]),
            )
            .expect("the native-rederived package builds")
        })
        .clone()
}

fn manifest(files: &PackageFiles) -> Manifest {
    serde_json::from_slice(&files[MANIFEST]).unwrap()
}

/// `change` applied to the manifest, which is then re-encoded canonically with every digest current.
/// `identity` computes the result identity of the changed manifest.
fn reissued(
    files: &PackageFiles,
    change: impl FnOnce(&mut Manifest),
    identity: impl FnOnce(&Manifest) -> String,
) -> PackageFiles {
    let mut files = files.clone();
    let mut m = manifest(&files);
    change(&mut m);
    m.files = files
        .iter()
        .filter(|(p, _)| p.as_str() != MANIFEST)
        .map(|(p, b)| (p.clone(), sha256_hex(b)))
        .collect();
    m.source.export_sha256 = m.files[EXPORT].clone();
    m.result_id = identity(&m);
    files.insert(MANIFEST.into(), canonical_json(&m));
    files
}

fn own_identity(m: &Manifest) -> String {
    result_id_in(&m.schema, &m.target.lean, &m.files)
}

fn refused_after_both_kernels(files: &PackageFiles, stage: &str, code: &str) {
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
        .expect("the package must be refused");
    assert_eq!(
        (failure.name, failure.code.as_deref()),
        (stage, Some(code)),
        "{receipt:#?}"
    );
    assert!(!receipt.replayed());
}

fn refused_at_admission(files: &PackageFiles, code: &str) {
    let receipt = replay(files);
    let failure = receipt
        .first_failure()
        .expect("the package must be refused");
    assert_eq!(
        (failure.name, failure.code.as_deref()),
        ("fresh_replay", Some(code)),
        "{receipt:#?}"
    );
    assert_eq!(
        receipt.status("primary_check"),
        StageStatus::NotRun,
        "admission refuses before any checking work"
    );
}

#[test]
fn the_schema_follows_the_elimination_and_nothing_else() {
    let native = native_elimination();
    assert_eq!(contract_schema(&native), NATIVE_REDERIVED_SCHEMA);
    assert_eq!(contract_schema(&Elimination::default()), PACKAGE_SCHEMA);
    let escape_kept = Elimination {
        rederivations: native.rederivations.clone(),
        escape_remains: true,
    };
    assert_eq!(
        contract_schema(&escape_kept),
        PACKAGE_SCHEMA,
        "a proof that still rests on the escape eliminated nothing"
    );
    assert_eq!(PACKAGE_SCHEMAS, [PACKAGE_SCHEMA, NATIVE_REDERIVED_SCHEMA]);
}

#[test]
fn the_schema_is_the_identity_domain() {
    let files = BTreeMap::from([("a".to_string(), "00".to_string())]);
    assert_eq!(
        result_id("T", &files),
        result_id_in(PACKAGE_SCHEMA, "T", &files)
    );
    assert_ne!(
        result_id_in(PACKAGE_SCHEMA, "T", &files),
        result_id_in(NATIVE_REDERIVED_SCHEMA, "T", &files)
    );
    let native = manifest(&native());
    assert_eq!(native.schema, NATIVE_REDERIVED_SCHEMA);
    assert_eq!(
        native.result_id,
        result_id_in(NATIVE_REDERIVED_SCHEMA, &native.target.lean, &native.files)
    );
    let ordinary = manifest(&clean());
    assert_eq!(ordinary.schema, PACKAGE_SCHEMA);
    assert_eq!(
        ordinary.result_id,
        result_id(&ordinary.target.lean, &ordinary.files)
    );
}

#[test]
fn a_package_carrying_the_other_contracts_identity_is_refused_before_any_work() {
    let native_under_ordinary_identity =
        reissued(&native(), |_| {}, |m| result_id(&m.target.lean, &m.files));
    refused_at_admission(&native_under_ordinary_identity, "result_id_mismatch");
    let relabeled_keeping_its_identity = reissued(
        &clean(),
        |m| m.schema = NATIVE_REDERIVED_SCHEMA.into(),
        |m| m.result_id.clone(),
    );
    refused_at_admission(&relabeled_keeping_its_identity, "result_id_mismatch");
}

#[test]
fn an_ordinary_package_relabeled_native_is_refused_even_with_its_identity_recomputed() {
    let relabeled = reissued(
        &clean(),
        |m| m.schema = NATIVE_REDERIVED_SCHEMA.into(),
        own_identity,
    );
    refused_after_both_kernels(&relabeled, "translated", "native_rederivation_schema");
    let with_a_borrowed_disclosure = reissued(
        &clean(),
        |m| {
            m.schema = NATIVE_REDERIVED_SCHEMA.into();
            m.information_losses.extend(
                native_elimination()
                    .information_changes()
                    .into_iter()
                    .map(|(category, detail)| InformationLoss { category, detail }),
            );
        },
        own_identity,
    );
    refused_after_both_kernels(
        &with_a_borrowed_disclosure,
        "translated",
        "native_rederivation_schema",
    );
}

#[test]
fn the_contract_is_stated_by_schema_and_disclosure_together() {
    let elimination = native_elimination();
    let stated = manifest(&native());
    assert!(states_contract(&stated, &elimination).is_ok());
    let mut downgraded = stated.clone();
    downgraded.schema = PACKAGE_SCHEMA.into();
    assert_eq!(
        states_contract(&downgraded, &elimination).unwrap_err().0,
        "native_rederivation_schema"
    );
    let mut undisclosed = stated.clone();
    undisclosed
        .information_losses
        .retain(|l| l.category != crate::interface::lean::native_decide::INFORMATION_CATEGORY);
    assert_eq!(
        states_contract(&undisclosed, &elimination).unwrap_err().0,
        "native_rederivation_disclosure"
    );
    let ordinary = manifest(&clean());
    assert!(states_contract(&ordinary, &Elimination::default()).is_ok());
    assert_eq!(
        states_contract(&ordinary, &elimination).unwrap_err().0,
        "native_rederivation_schema"
    );
}

#[test]
fn leans_report_is_held_against_the_checked_proof_as_the_contract_says() {
    let reference = Reference::read(NATIVE).unwrap();
    let kernel = |lean: &str| reference.source_lean_name(lean).unwrap();
    let (escape, trust) = (kernel(ELIMINATED_AXIOM), kernel(TRUST_COMPILER));
    let native = native_elimination();
    let ordinary = Elimination::default();
    let report = |names: &[&str]| names.iter().map(|n| n.to_string()).collect::<Vec<String>>();
    let agrees = |contract: &Elimination, lean: Option<Vec<String>>, rests_on: Vec<String>| {
        agree_with_lean(&reference, contract, lean.as_deref(), &rests_on)
    };
    // The native-rederived contract: Lean's report, required, is the proof's assumptions and the escape.
    assert_eq!(
        agrees(&native, Some(report(&[ELIMINATED_AXIOM])), vec![]),
        Ok(())
    );
    assert_eq!(
        agrees(
            &native,
            Some(report(&[TRUST_COMPILER, ELIMINATED_AXIOM])),
            vec![trust.clone()]
        ),
        Ok(())
    );
    for (lean, rests_on, why) in [
        (
            None,
            vec![],
            "without Lean's report there is no evidence of what the replaced proof rested on",
        ),
        (
            Some(report(&[])),
            vec![],
            "Lean's report must name the escape it says was eliminated",
        ),
        (
            Some(report(&[TRUST_COMPILER])),
            vec![trust.clone()],
            "only the escape Lean names is removed",
        ),
        (
            Some(report(&[ELIMINATED_AXIOM, ELIMINATED_AXIOM])),
            vec![],
            "a name reported twice is not a report",
        ),
        (
            Some(report(&[ELIMINATED_AXIOM, TRUST_COMPILER])),
            vec![],
            "Lean names an assumption the proof does not rest on",
        ),
        (
            Some(report(&[ELIMINATED_AXIOM])),
            vec![trust.clone()],
            "the proof rests on an assumption Lean does not name",
        ),
        (
            Some(report(&[ELIMINATED_AXIOM, "Lean.unheardOf"])),
            vec![],
            "Lean names an axiom the export does not declare",
        ),
    ] {
        assert!(
            agrees(&native, lean.clone(), rests_on.clone()).is_err(),
            "{why}: {lean:?} against {rests_on:?}"
        );
    }
    // The ordinary contract: nothing is removed, and an unobserved Lean is held to nothing.
    assert_eq!(agrees(&ordinary, None, vec![]), Ok(()));
    assert_eq!(agrees(&ordinary, None, vec![trust.clone()]), Ok(()));
    assert_eq!(
        agrees(
            &ordinary,
            Some(report(&[ELIMINATED_AXIOM])),
            vec![escape.clone()]
        ),
        Ok(())
    );
    assert!(
        agrees(&ordinary, Some(report(&[ELIMINATED_AXIOM])), vec![]).is_err(),
        "the ordinary contract never removes the escape"
    );
    // A disagreement reads in Lean's spelling throughout.
    let detail = agrees(
        &native,
        Some(report(&[ELIMINATED_AXIOM, TRUST_COMPILER])),
        vec![],
    )
    .unwrap_err();
    assert!(
        detail.contains(TRUST_COMPILER) && !detail.contains("/s:"),
        "{detail}"
    );
    let detail = agrees(&native, Some(report(&[ELIMINATED_AXIOM])), vec![trust]).unwrap_err();
    assert!(
        detail.contains(TRUST_COMPILER) && !detail.contains("/s:"),
        "{detail}"
    );
}

#[test]
fn a_native_build_without_leans_report_keeps_the_audit_apart_from_the_contract() {
    let build = build_observed(NATIVE, NATIVE_TARGET, &BuildOptions::default(), None).unwrap();
    let policy = build.receipt.stage("assumption_policy").unwrap();
    assert_eq!(
        (policy.status, policy.code.as_deref()),
        (StageStatus::Failed, Some("assumption_disagreement")),
        "{policy:?}"
    );
    assert_eq!(build.audit.name, "assumption_policy");
    assert_eq!(
        (build.audit.status, build.audit.code.as_deref()),
        (StageStatus::Passed, None),
        "{:?}",
        build.audit
    );
    assert_eq!(
        build.audit.detail,
        "the statement and proof rest on no assumption"
    );
    let m = manifest(&build.files);
    assert_eq!(
        (m.schema.as_str(), m.assumptions.lean_print_axioms.as_ref()),
        (NATIVE_REDERIVED_SCHEMA, None)
    );
    assert!(publishable(build).is_err());
    let ordinary = build_observed(CLEAN, "W05.clean", &BuildOptions::default(), None).unwrap();
    assert_eq!(
        ordinary.receipt.stage("assumption_policy").unwrap(),
        &ordinary.audit,
        "no contract adds to an unobserved ordinary proof"
    );
}

#[test]
fn the_disclosure_names_each_replaced_declaration_once() {
    let elimination = native_elimination();
    let stated: Vec<InformationLoss> = manifest(&native())
        .information_losses
        .into_iter()
        .filter(|l| l.category == crate::interface::lean::native_decide::INFORMATION_CATEGORY)
        .collect();
    let replaced: Vec<&Rederivation> = elimination.rederivations.iter().collect();
    assert_eq!(stated.len(), replaced.len());
    for (loss, rederivation) in stated.iter().zip(replaced) {
        assert_eq!(loss.detail, rederivation.detail());
        assert!(
            loss.detail.starts_with(&format!("{NATIVE_TARGET}:")),
            "{}",
            loss.detail
        );
    }
}
