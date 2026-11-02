//! Source trees obey replay limits even when the carried certificate is shallow.
mod lean_package_support;

use lean_package_support::Export;
use logicaffeine_kernel::certificate::EnvironmentCertificate;
use logicaffeine_kernel::interface::lean::package::{
    self, BuildOptions, PackageFiles, PackageLimits,
};
use logicaffeine_kernel::interface::lean::receipt::{Receipt, StageStatus};
use serde_json::{json, Value};

fn shallow_package(opaque: bool) -> PackageFiles {
    let mut e = Export::new();
    e.lines[0] = include_str!("fixtures/lean_package/W05.clean.ndjson")
        .lines()
        .next()
        .unwrap()
        .to_string();
    let p = e.name("P", 0);
    let x = e.name("x", 0);
    let root = e.name("sourceDepthIdentity", 0);
    let prop = e.sort(0);
    let zero = e.bvar(0);
    let one = e.bvar(1);
    let inner_type = e.forall(x, zero, one);
    let ty = e.forall(p, prop, inner_type);
    let inner_body = e.lam(x, zero, zero);
    let body = e.lam(p, prop, inner_body);
    if opaque {
        e.raw(
            json!({"opaque":{"name":root,"levelParams":[],"type":ty,"value":body,
            "isUnsafe":false,"all":[root]}})
            .to_string(),
        );
    } else {
        e.def(root, ty, body);
    }
    let build = package::build(&e.text(), "sourceDepthIdentity", &BuildOptions::default()).unwrap();
    assert_eq!(
        build.receipt.status("translated"),
        StageStatus::Passed,
        "{:#?}",
        build.receipt
    );
    assert_eq!(
        build.receipt.status("assumption_policy"),
        StageStatus::Passed,
        "{:#?}",
        build.receipt
    );
    build.files
}

/// Deepen the source value and change its declaration kind. Raw value admission precedes
/// the later kind mismatch; an observer must never allocate the unadmitted deep source body.
fn deepen_and_change_source_kind(files: &mut PackageFiles, wrappers: usize, opaque: bool) {
    let export = std::str::from_utf8(&files[package::EXPORT]).unwrap();
    let mut rows: Vec<Value> = export
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let kind = if opaque { "opaque" } else { "def" };
    let at = rows
        .iter()
        .rposition(|row| row.get(kind).is_some())
        .unwrap();
    let original = rows[at][kind]["value"].as_u64().unwrap();
    let ty = rows[at][kind]["type"].as_u64().unwrap();
    let name = rows[at][kind]["name"].as_u64().unwrap();
    let mut next = rows
        .iter()
        .filter_map(|row| row["ie"].as_u64())
        .max()
        .unwrap()
        + 1;
    let mut body = original;
    let mut added = Vec::new();
    for _ in 0..wrappers {
        added.push(json!({"ie":next,"letE":{
            "name":name,"type":ty,"value":original,"body":body,"nonDep":false
        }}));
        body = next;
        next += 1;
    }
    let mut declaration = rows[at][kind].clone();
    declaration["value"] = json!(body);
    if opaque {
        // opaque certificate / transparent source. Definition-specific metadata is explicit.
        declaration["hints"] = json!("abbrev");
        declaration["safety"] = json!("safe");
        declaration.as_object_mut().unwrap().remove("isUnsafe");
        rows[at] = json!({"def":declaration});
    } else {
        // transparent certificate / opaque source. Opaque records have no reducibility hints.
        declaration.as_object_mut().unwrap().remove("hints");
        declaration.as_object_mut().unwrap().remove("safety");
        declaration["isUnsafe"] = json!(false);
        rows[at] = json!({"opaque":declaration});
    }
    rows.splice(at..at, added);
    let export: String = rows.iter().map(|row| row.to_string() + "\n").collect();
    files.insert(package::EXPORT.into(), export.into_bytes());
    let mut manifest: package::Manifest =
        serde_json::from_slice(&files[package::MANIFEST]).unwrap();
    let hash = package::sha256_hex(&files[package::EXPORT]);
    manifest.files.insert(package::EXPORT.into(), hash.clone());
    manifest.source.export_sha256 = hash;
    manifest.result_id = package::result_id(&manifest.target.lean, &manifest.files);
    files.insert(package::MANIFEST.into(), package::canonical_json(&manifest));
}

fn assert_source_resource_refusal(receipt: &Receipt) {
    assert!(!receipt.verified());
    assert_eq!(
        receipt.status("fresh_replay"),
        StageStatus::Failed,
        "{receipt:#?}"
    );
    assert_eq!(
        receipt.stage("fresh_replay").unwrap().code.as_deref(),
        Some("resource_limit"),
        "{receipt:#?}"
    );
    assert_eq!(
        receipt.stage("translated").unwrap().code.as_deref(),
        Some("resource_limit"),
        "{receipt:#?}"
    );
    assert_eq!(
        receipt.status("primary_check"),
        StageStatus::NotRun,
        "source bounds precede all recursive kernel work"
    );
}

#[test]
fn wrong_value_kind_still_admits_source_body_before_observation() {
    for opaque_certificate in [false, true] {
        let mut files = shallow_package(opaque_certificate);
        let certificate: EnvironmentCertificate =
            serde_json::from_slice(&files[package::CERTIFICATE]).unwrap();
        let mut limits = PackageLimits::default();
        let certificate_depth = certificate.admitted_depth(&limits.environment).unwrap();
        limits.environment.max_depth = certificate_depth + 16;
        assert!(certificate.admitted_depth(&limits.environment).is_ok());
        deepen_and_change_source_kind(&mut files, certificate_depth + 32, opaque_certificate);
        assert_source_resource_refusal(&package::replay_with(&files, &limits));
    }
}
