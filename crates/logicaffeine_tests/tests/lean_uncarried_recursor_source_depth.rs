//! A selected recursor binds the entire independent source block before kernel replay.
use logicaffeine_kernel::certificate::EnvironmentCertificate;
use logicaffeine_kernel::interface::lean::package::{
    self, BuildOptions, Manifest, PackageFiles, PackageLimits,
};
use logicaffeine_kernel::interface::lean::receipt::StageStatus;
use serde_json::{json, Value};
use std::collections::BTreeMap;

fn text(rows: &[Value]) -> String {
    rows.iter().map(|row| row.to_string() + "\n").collect()
}
fn kernel_name(id: u64, names: &BTreeMap<u64, Value>) -> String {
    if id == 0 {
        return String::new();
    }
    let row = &names[&id];
    if let Some(s) = row.get("str") {
        format!(
            "{}/s:{}",
            kernel_name(s["pre"].as_u64().unwrap(), names),
            serde_json::to_string(s["str"].as_str().unwrap()).unwrap()
        )
    } else {
        let n = &row["num"];
        format!(
            "{}/n:{}",
            kernel_name(n["pre"].as_u64().unwrap(), names),
            n["num"].as_u64().unwrap()
        )
    }
}
fn rehash(files: &mut PackageFiles) {
    let mut manifest: Manifest = serde_json::from_slice(&files[package::MANIFEST]).unwrap();
    manifest.source.export_sha256 = package::sha256_hex(&files[package::EXPORT]);
    manifest.files.insert(
        package::EXPORT.into(),
        manifest.source.export_sha256.clone(),
    );
    manifest.result_id = package::result_id(&manifest.target.lean, &manifest.files);
    files.insert(package::MANIFEST.into(), package::canonical_json(&manifest));
}
#[test]
fn deep_rule_of_an_uncarried_sibling_is_refused_before_primary_replay() {
    let source = include_str!("fixtures/lean_semantics/LF_W03_NestedRose.ndjson");
    let mut rows: Vec<Value> = source
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let block_at = rows
        .iter()
        .position(|row| {
            row["inductive"]["recs"]
                .as_array()
                .is_some_and(|rs| rs.len() > 1)
        })
        .unwrap();
    let recs = rows[block_at]["inductive"]["recs"]
        .as_array()
        .unwrap()
        .clone();
    let selected = &recs[0];
    // Export a transparent alias of exactly one authentic recursor; capture should not add
    // the sibling's implementation merely because it shares an original source block.
    let target_name = rows
        .iter()
        .filter_map(|row| row["in"].as_u64())
        .max()
        .unwrap()
        + 1;
    let value = rows
        .iter()
        .filter_map(|row| row["ie"].as_u64())
        .max()
        .unwrap()
        + 1;
    let levels: Vec<Value> = selected["levelParams"]
        .as_array()
        .unwrap()
        .iter()
        .map(|name| {
            json!(rows
                .iter()
                .find(|row| row.get("param") == Some(name))
                .unwrap()["il"]
                .as_u64()
                .unwrap())
        })
        .collect();
    rows.push(json!({"in":target_name,"str":{"pre":0,"str":"oneRecursorOnly"}}));
    rows.push(json!({"ie":value,"const":{"name":selected["name"],"us":levels}}));
    rows.push(json!({"def":{"name":target_name,"levelParams":selected["levelParams"],"type":selected["type"],
        "value":value,"safety":"safe","hints":{"regular":1},"all":[target_name]}}));
    let build = package::build(&text(&rows), "oneRecursorOnly", &BuildOptions::default()).unwrap();
    assert_eq!(
        build.receipt.status("translated"),
        StageStatus::Passed,
        "{:#?}",
        build.receipt
    );
    let mut files = build.files;
    let original = package::replay(&files);
    assert_eq!(
        original.status("primary_check"),
        StageStatus::Passed,
        "{original:#?}"
    );
    assert_eq!(
        original.status("translated"),
        StageStatus::Passed,
        "{original:#?}"
    );
    let certificate: EnvironmentCertificate =
        serde_json::from_slice(&files[package::CERTIFICATE]).unwrap();
    let carried: Vec<&str> = certificate
        .entries
        .iter()
        .flat_map(|entry| entry.names())
        .collect();
    let names: BTreeMap<u64, Value> = rows
        .iter()
        .filter_map(|row| row["in"].as_u64().map(|id| (id, row.clone())))
        .collect();
    let uncarried = recs
        .iter()
        .enumerate()
        .skip(1)
        .find(|(_, rec)| {
            !carried.contains(&kernel_name(rec["name"].as_u64().unwrap(), &names).as_str())
        })
        .map(|(index, _)| index)
        .expect("fixture certificate must omit a sibling to exercise the complete-block preflight");
    let limits = PackageLimits::default();
    assert!(certificate.admitted_depth(&limits.environment).is_ok());
    let original_rhs = recs[uncarried]["rules"][0]["rhs"].as_u64().unwrap();
    let mut rhs = original_rhs;
    let mut next = value + 1;
    let mut added = Vec::new();
    for _ in 0..limits.environment.max_depth + 8 {
        added.push(
            json!({"ie":next,"letE":{"name":target_name,"type":recs[uncarried]["type"],
            "value":original_rhs,"body":rhs,"nonDep":false}}),
        );
        rhs = next;
        next += 1;
    }
    rows[block_at]["inductive"]["recs"][uncarried]["rules"][0]["rhs"] = json!(rhs);
    rows.splice(block_at..block_at, added);
    files.insert(package::EXPORT.into(), text(&rows).into_bytes());
    rehash(&mut files);
    let replay = package::replay_with(&files, &limits);
    assert_eq!(
        replay.stage("translated").unwrap().code.as_deref(),
        Some("resource_limit"),
        "{replay:#?}"
    );
    assert_eq!(
        replay.stage("fresh_replay").unwrap().code.as_deref(),
        Some("resource_limit"),
        "{replay:#?}"
    );
    assert_eq!(
        replay.status("primary_check"),
        StageStatus::NotRun,
        "the uncarried sibling's raw source must be admitted before any kernel replay: {replay:#?}"
    );
}
