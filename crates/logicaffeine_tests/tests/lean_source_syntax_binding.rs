//! Replay must check original source syntax, including expressions erased by reduction.
mod lean_package_support;
use lean_package_support::Export;
use logicaffeine_kernel::interface::lean::{
    package,
    receipt::{Receipt, StageStatus},
};

#[derive(Clone, Copy)]
enum HiddenValue {
    None,
    Valid,
    WrongType,
    UnknownConstant,
    UndeclaredUniverse,
}

fn source(hidden: HiddenValue) -> String {
    let mut e = Export::new();
    e.lines[0] = include_str!("fixtures/lean_package/W05.clean.ndjson")
        .lines()
        .next()
        .unwrap()
        .to_string();
    let ns = e.name("SyntaxBinding", 0);
    let meaning = e.name("Meaning", ns);
    let root = e.name("root", ns);
    let p = e.name("P", 0);
    let h = e.name("h", 0);
    let dead = e.name("unused", 0);
    let prop = e.sort(0);
    let p0 = e.bvar(0);
    let p1 = e.bvar(1);
    let identity_statement = e.forall(h, p0, p1);
    let closed_statement = e.forall(p, prop, identity_statement);
    let value = match hidden {
        HiddenValue::None => closed_statement,
        HiddenValue::Valid => e.let_in(dead, prop, closed_statement, closed_statement),
        HiddenValue::WrongType => {
            let function = e.lam(p, prop, p0);
            e.let_in(dead, prop, function, closed_statement)
        }
        HiddenValue::UnknownConstant => {
            let missing_name = e.name("neverDeclared", ns);
            let missing = e.constant(missing_name, &[]);
            e.let_in(dead, prop, missing, closed_statement)
        }
        HiddenValue::UndeclaredUniverse => {
            let u_name = e.name("unlistedUniverse", ns);
            let u = e.param(u_name);
            let successor = e.succ(u);
            let ty = e.sort(successor);
            let value = e.sort(u);
            e.let_in(dead, ty, value, closed_statement)
        }
    };
    e.def(meaning, prop, value);
    let meaning_ref = e.constant(meaning, &[]);
    let goal = e.forall(h, meaning_ref, meaning_ref);
    let proof = e.lam(h, meaning_ref, p0);
    e.thm(root, goal, proof);
    e.text()
}

fn require_success(receipt: &Receipt) {
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

fn replay_with_source(hidden: HiddenValue) -> Receipt {
    let original = package::build(
        &source(HiddenValue::None),
        "SyntaxBinding.root",
        &package::BuildOptions::default(),
    )
    .unwrap();
    require_success(&original.receipt);
    require_success(&package::replay(&original.files));
    let mut files = original.files;
    files.insert(package::EXPORT.into(), source(hidden).into_bytes());
    let mut manifest: package::Manifest =
        serde_json::from_slice(&files[package::MANIFEST]).unwrap();
    manifest.files = files
        .iter()
        .filter(|(name, _)| *name != package::MANIFEST)
        .map(|(name, bytes)| (name.clone(), package::sha256_hex(bytes)))
        .collect();
    manifest.source.export_sha256 = manifest.files[package::EXPORT].clone();
    manifest.result_id = package::result_id(&manifest.target.lean, &manifest.files);
    files.insert(package::MANIFEST.into(), package::canonical_json(&manifest));
    package::replay(&files)
}

fn require_source_refusal(hidden: HiddenValue) {
    let receipt = replay_with_source(hidden);
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
    assert_eq!(
        receipt.status("translated"),
        StageStatus::Failed,
        "source syntax cannot be trusted merely because its normal form matches: {receipt:#?}"
    );
    assert!(!receipt.verified());
}

#[test]
fn a_well_typed_erased_source_let_remains_accepted() {
    require_success(&replay_with_source(HiddenValue::Valid));
}

#[test]
fn an_ill_typed_erased_source_value_is_refused() {
    require_source_refusal(HiddenValue::WrongType);
}

#[test]
fn an_undeclared_constant_in_erased_source_syntax_is_refused() {
    require_source_refusal(HiddenValue::UnknownConstant);
}

#[test]
fn an_undeclared_universe_in_erased_source_syntax_is_refused() {
    require_source_refusal(HiddenValue::UndeclaredUniverse);
}
