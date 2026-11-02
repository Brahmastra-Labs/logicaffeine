//! A sealed value's downloadable implementation remains bound to its original source.
mod lean_package_support;
use lean_package_support::Export;
use logicaffeine_kernel::interface::lean::package::{self, BuildOptions, Manifest};
use logicaffeine_kernel::interface::lean::receipt::{Receipt, StageStatus};
use serde_json::json;

fn source(second: bool) -> String {
    // opaque choose : (A : Type) -> A -> A -> A := fun A x y => x (or y)
    // The two implementations are well typed and axiom free, but compute differently.
    let mut e = Export::new();
    e.lines[0] = include_str!("fixtures/lean_package/W05.clean.ndjson")
        .lines()
        .next()
        .unwrap()
        .into();
    let root = e.name("opaqueChoose", 0);
    let a = e.name("A", 0);
    let x = e.name("x", 0);
    let y = e.name("y", 0);
    let level = e.succ(0);
    let type0 = e.sort(level);
    let zero = e.bvar(0);
    let one = e.bvar(1);
    let two = e.bvar(2);
    let y_ty = e.forall(y, one, two);
    let x_ty = e.forall(x, zero, y_ty);
    let ty = e.forall(a, type0, x_ty);
    let y_body = e.lam(y, one, if second { zero } else { one });
    let x_body = e.lam(x, zero, y_body);
    let body = e.lam(a, type0, x_body);
    e.raw(json!({"opaque":{"name":root,"levelParams":[],"type":ty,"value":body,"isUnsafe":false,"all":[root]}}).to_string());
    e.text()
}

fn successful(receipt: &Receipt) {
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
fn replay_compares_opaque_value_bodies_without_unsealing_their_declarations() {
    let first_source = source(false);
    let second_source = source(true);
    let first = package::build(&first_source, "opaqueChoose", &BuildOptions::default()).unwrap();
    let second = package::build(&second_source, "opaqueChoose", &BuildOptions::default()).unwrap();
    successful(&first.receipt);
    successful(&second.receipt);
    assert!(package::replay(&first.files).replayed());
    assert!(package::replay(&second.files).replayed());
    let mut files = second.files;
    files.insert(package::EXPORT.into(), first_source.into_bytes());
    let mut manifest: Manifest = serde_json::from_slice(&files[package::MANIFEST]).unwrap();
    manifest.source.export_sha256 = package::sha256_hex(&files[package::EXPORT]);
    manifest.files.insert(
        package::EXPORT.into(),
        manifest.source.export_sha256.clone(),
    );
    manifest.result_id = package::result_id(&manifest.target.lean, &manifest.files);
    files.insert(package::MANIFEST.into(), package::canonical_json(&manifest));
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
        "a different well-typed implementation is not the submitted opaque value: {replay:#?}"
    );
    assert!(!replay.replayed());
}
