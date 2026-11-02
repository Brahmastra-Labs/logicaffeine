//! Source trees obey replay limits even when the carried certificate is shallow.
mod lean_package_support;

use lean_package_support::Export;
use logicaffeine_kernel::certificate::EnvironmentCertificate;
use logicaffeine_kernel::interface::lean::package::{
    self, BuildOptions, PackageFiles, PackageLimits,
};
use logicaffeine_kernel::interface::lean::receipt::{Receipt, StageStatus};
use serde_json::{json, Value};

fn shallow_package() -> PackageFiles {
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
    e.def(root, ty, body);
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

/// Replace only the source body by convertible, well-typed lets. In the DAG variant each
/// new let refers to its predecessor twice; the export grows linearly and its tree exponentially.
fn deepen_source(files: &mut PackageFiles, wrappers: usize, duplicate: bool) {
    let export = std::str::from_utf8(&files[package::EXPORT]).unwrap();
    let mut rows: Vec<Value> = export
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let at = rows
        .iter()
        .rposition(|row| row.get("def").is_some())
        .unwrap();
    let original = rows[at]["def"]["value"].as_u64().unwrap();
    let ty = rows[at]["def"]["type"].as_u64().unwrap();
    let name = rows[at]["def"]["name"].as_u64().unwrap();
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
            "name":name,"type":ty,"value":if duplicate {body} else {original},"body":body,"nonDep":false
        }}));
        body = next;
        next += 1;
    }
    rows[at]["def"]["value"] = json!(body);
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
fn a_shallow_certificate_does_not_exempt_its_deep_source_body_from_the_depth_limit() {
    let mut files = shallow_package();
    let certificate: EnvironmentCertificate =
        serde_json::from_slice(&files[package::CERTIFICATE]).unwrap();
    let mut limits = PackageLimits::default();
    let certificate_depth = certificate.admitted_depth(&limits.environment).unwrap();
    limits.environment.max_depth = certificate_depth + 16;
    assert!(certificate.admitted_depth(&limits.environment).is_ok());
    deepen_source(&mut files, certificate_depth + 32, false);
    assert_source_resource_refusal(&package::replay_with(&files, &limits));
}

#[test]
fn source_dag_tree_expansion_is_charged_even_when_few_expression_records_are_carried() {
    let mut files = shallow_package();
    let certificate: EnvironmentCertificate =
        serde_json::from_slice(&files[package::CERTIFICATE]).unwrap();
    let mut limits = PackageLimits::default();
    limits.environment.max_tree_nodes = 4096;
    assert!(certificate.admitted_depth(&limits.environment).is_ok());
    deepen_source(&mut files, 12, true);
    assert!(
        std::str::from_utf8(&files[package::EXPORT])
            .unwrap()
            .lines()
            .count()
            < 100
    );
    assert_source_resource_refusal(&package::replay_with(&files, &limits));
}

#[test]
fn a_convertible_source_body_deeper_than_its_certificate_remains_admissible_within_limits() {
    let mut files = shallow_package();
    let certificate: EnvironmentCertificate =
        serde_json::from_slice(&files[package::CERTIFICATE]).unwrap();
    let mut limits = PackageLimits::default();
    limits.environment.max_depth = 128;
    let depth = certificate.admitted_depth(&limits.environment).unwrap();
    assert!(depth < 48);
    deepen_source(&mut files, 48, false);
    let replay = package::replay_with(&files, &limits);
    assert!(replay.replayed(), "{replay:#?}");
    assert!(
        !replay.verified(),
        "replay does not rerun the producer Lean check"
    );
    assert_eq!(replay.status("translated"), StageStatus::Passed);
}
