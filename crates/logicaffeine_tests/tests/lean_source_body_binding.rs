//! A same-typed source definition must retain its meaning through rehashed package replay.
mod lean_package_support;
use lean_package_support::Export;
use logicaffeine_kernel::interface::lean::receipt::StageStatus;
use logicaffeine_kernel::interface::lean::{self as adapter, intake, package};
use logicaffeine_kernel::{
    axiom_dependencies_of, defeq_for_test, infer_type, normalize_checked, Context, Term,
};

// Original: def Meaning : Prop := forall P : Prop, P
// Mutant:   def Meaning : Prop := forall P : Prop, P -> P
// Both:     theorem root : Meaning -> Meaning := fun h => h
fn body_binding_export(mutant: bool) -> String {
    let mut e = Export::new();
    e.lines[0] = include_str!("fixtures/lean_package/W05.clean.ndjson")
        .lines()
        .next()
        .unwrap()
        .to_string();
    let ns = e.name("BodyBinding", 0);
    let meaning = e.name("Meaning", ns);
    let root = e.name("root", ns);
    let p = e.name("P", 0);
    let h = e.name("h", 0);
    let prop = e.sort(0);
    let p0 = e.bvar(0);
    let inner = if mutant {
        let p1 = e.bvar(1);
        e.forall(h, p0, p1)
    } else {
        p0
    };
    let meaning_body = e.forall(p, prop, inner);
    e.def(meaning, prop, meaning_body);
    let meaning_ref = e.constant(meaning, &[]);
    let goal = e.forall(h, meaning_ref, meaning_ref);
    let h0 = e.bvar(0);
    let proof = e.lam(h, meaning_ref, h0);
    e.thm(root, goal, proof);
    e.text()
}

fn require_success(receipt: &logicaffeine_kernel::interface::lean::receipt::Receipt) {
    for stage in [
        "translated",
        "primary_check",
        "independent_recheck",
        "assumption_policy",
        "emission_reparsed",
    ] {
        assert_eq!(
            receipt.status(stage),
            StageStatus::Passed,
            "{stage}: {receipt:#?}"
        );
    }
}

#[test]
fn a_rehashed_package_cannot_change_a_definition_used_by_its_statement() {
    let original_export = body_binding_export(false);
    let mutant_export = body_binding_export(true);
    let root = intake::kernel_name("BodyBinding.root").unwrap();
    let mut statements = Vec::new();
    for export in [&original_export, &mutant_export] {
        let (ctx, _) = adapter::import_audited(export).unwrap();
        let ty = infer_type(&ctx, &Term::Global(root.clone())).unwrap();
        statements.push(normalize_checked(&ctx, &ty).unwrap());
        let axioms = axiom_dependencies_of(&ctx, &root).unwrap();
        assert!(axioms.axioms.is_empty() && axioms.unresolved.is_empty());
    }
    assert!(
        !defeq_for_test(&Context::new(), &statements[0], &statements[1]),
        "the two closed statements differ after expanding Meaning"
    );
    let original = package::build(
        &original_export,
        "BodyBinding.root",
        &package::BuildOptions::default(),
    )
    .unwrap();
    let mutant = package::build(
        &mutant_export,
        "BodyBinding.root",
        &package::BuildOptions::default(),
    )
    .unwrap();
    require_success(&original.receipt);
    require_success(&mutant.receipt);
    require_success(&package::replay(&original.files));
    require_success(&package::replay(&mutant.files));

    // The adversary carries a fully valid certificate and LOGOS document for the mutant,
    // but claims the unchanged original Lean export as their source.
    let mut forged = mutant.files;
    forged.insert(package::EXPORT.into(), original_export.as_bytes().to_vec());
    let mut manifest: package::Manifest =
        serde_json::from_slice(&forged[package::MANIFEST]).unwrap();
    manifest.files = forged
        .iter()
        .filter(|(path, _)| *path != package::MANIFEST)
        .map(|(path, bytes)| (path.clone(), package::sha256_hex(bytes)))
        .collect();
    manifest.source.export_sha256 = manifest.files[package::EXPORT].clone();
    manifest.result_id = package::result_id(&manifest.target.lean, &manifest.files);
    forged.insert(package::MANIFEST.into(), package::canonical_json(&manifest));
    assert_eq!(forged[package::EXPORT], original.files[package::EXPORT]);
    let replay = package::replay(&forged);
    assert_eq!(
        replay.status("primary_check"),
        StageStatus::Passed,
        "the mutation is a valid proof of a different statement: {replay:#?}"
    );
    assert_eq!(
        replay.status("independent_recheck"),
        StageStatus::Passed,
        "{replay:#?}"
    );
    assert_eq!(
        replay.status("translated"),
        StageStatus::Failed,
        "binding must compare the meaning of source definitions, not just their types: {replay:#?}"
    );
    assert!(!replay.verified());
}
