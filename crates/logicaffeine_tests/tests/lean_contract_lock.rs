//! LEAN IMPORT CONTRACT — the recorded MaCSad fixtures obey `LEAN_IMPORT_CONTRACT.md`.
//!
//! `crates/logicaffeine_tests/tests/fixtures/lean_conformance/contract/fixtures/` holds one recorded `largo lean import` result per outcome, the diagnostic
//! stream beside it, a published package and its fresh-process replay receipt, all produced by
//! `scripts/lean/lean-contract-fixtures.sh` from real runs. MaCSad builds its worker and frontend against them. This
//! lock keeps them honest: every outcome is present, every document has the contract's shape, the exit code is
//! the outcome's, the stderr stream is the result's diagnostics, every diagnostic code is a wire name the reader
//! defines, `verified` means every stage passed and a package was published, a refusal publishes nothing, and the
//! fixtures are marked as recordings with intact digests.

#![cfg(not(target_arch = "wasm32"))]

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

const OUTCOMES: [(&str, i64); 6] = [
    ("verified", 0),
    ("invalid_source", 1),
    ("unsupported_feature", 2),
    ("disallowed_assumptions", 3),
    ("resource_limit", 4),
    ("infrastructure_failure", 5),
];

const STAGES: [&str; 7] = [
    "lean_accepted",
    "translated",
    "primary_check",
    "independent_recheck",
    "assumption_policy",
    "emission_reparsed",
    "fresh_replay",
];

const STAGE_STATUSES: [&str; 4] = ["passed", "failed", "not_run", "unsupported"];

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn fixtures() -> PathBuf {
    root().join("crates/logicaffeine_tests/tests/fixtures/lean_conformance/contract/fixtures")
}

fn json(path: &Path) -> Value {
    let text = fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{} is not JSON: {e}", path.display()))
}

/// Every `DiagnosticCode` wire name, read from the reader's own source so this lock cannot drift from it.
fn wire_names() -> BTreeSet<String> {
    let source = fs::read_to_string(
        root().join("crates/logicaffeine_kernel/src/interface/lean/diagnostic.rs"),
    )
    .expect("diagnostic.rs");
    let body = &source[source
        .find("pub fn as_str(self)")
        .expect("DiagnosticCode::as_str")..];
    let body = &body[..body.find("\n    }\n").expect("end of as_str")];
    body.lines()
        .filter_map(|l| {
            l.split("=> \"")
                .nth(1)
                .and_then(|r| r.split('"').next())
                .map(str::to_string)
        })
        .collect()
}

#[test]
fn every_outcome_has_a_recorded_fixture_with_its_exit_code() {
    for (outcome, exit) in OUTCOMES {
        let result = json(&fixtures().join(format!("{outcome}.result.json")));
        assert_eq!(
            result["outcome"], outcome,
            "{outcome}.result.json records another outcome"
        );
        let recorded: i64 = fs::read_to_string(fixtures().join(format!("{outcome}.exit")))
            .unwrap_or_else(|e| panic!("{outcome}.exit: {e}"))
            .trim()
            .parse()
            .expect("an exit code");
        assert_eq!(recorded, exit, "{outcome} must exit {exit}");
    }
}

#[test]
fn every_result_has_the_contract_shape() {
    let codes = wire_names();
    assert!(
        codes.len() >= 8,
        "the reader defines its diagnostic codes: {codes:?}"
    );
    for (outcome, _) in OUTCOMES {
        let path = fixtures().join(format!("{outcome}.result.json"));
        let r = json(&path);
        assert_eq!(r["schema"], "logos-lean-result/1", "{outcome}");
        let stages = r["stages"]
            .as_object()
            .unwrap_or_else(|| panic!("{outcome}: stages"));
        let names: BTreeSet<&str> = stages.keys().map(String::as_str).collect();
        assert_eq!(
            names,
            STAGES.into_iter().collect(),
            "{outcome}: exactly the seven named stages (R1)"
        );
        for (stage, status) in stages {
            assert!(
                STAGE_STATUSES.contains(&status.as_str().unwrap_or("")),
                "{outcome}: stage {stage} has status {status}"
            );
        }
        assert!(r["target"]["lean"].is_string(), "{outcome}: target.lean");
        assert!(r["assumptions"].is_array(), "{outcome}: assumptions");
        assert!(
            r["information_losses"].is_array(),
            "{outcome}: information_losses"
        );
        for d in r["diagnostics"]
            .as_array()
            .unwrap_or_else(|| panic!("{outcome}: diagnostics"))
        {
            let code = d["code"]
                .as_str()
                .unwrap_or_else(|| panic!("{outcome}: diagnostic without a code: {d}"));
            assert!(
                codes.contains(code),
                "{outcome}: diagnostic code {code:?} is not a wire name the reader defines"
            );
            assert_ne!(
                code, "unclassified",
                "{outcome}: an unclassified diagnostic reached a result"
            );
            assert!(
                d["message"].is_string() && d["stage"].is_string(),
                "{outcome}: diagnostic {d}"
            );
        }
        let stream =
            fs::read_to_string(fixtures().join(format!("{outcome}.diagnostics.ndjson"))).unwrap();
        let lines: Vec<Value> = stream
            .lines()
            .map(|l| serde_json::from_str(l).expect("an NDJSON line"))
            .collect();
        assert_eq!(
            &lines,
            r["diagnostics"].as_array().unwrap(),
            "{outcome}: stderr carries the result's diagnostics"
        );
    }
}

#[test]
fn verified_means_every_stage_passed_and_a_package_was_published() {
    let r = json(&fixtures().join("verified.result.json"));
    for stage in STAGES {
        assert_eq!(
            r["stages"][stage], "passed",
            "verified requires {stage} to have passed"
        );
    }
    assert!(
        r["diagnostics"].as_array().unwrap().is_empty(),
        "a verified result carries no diagnostics"
    );
    assert!(
        r["package"]["manifest_sha256"].is_string(),
        "a verified result names its published package"
    );
    let package = fixtures().join("verified.package");
    let manifest =
        fs::read(package.join("manifest.json")).expect("the recorded package's manifest");
    let manifest_json: Value = serde_json::from_slice(&manifest).unwrap();
    assert_eq!(manifest_json["schema"], "logos-lean-package/1");
    for (file, digest) in manifest_json["files"].as_object().expect("manifest files") {
        let bytes =
            fs::read(package.join(file)).unwrap_or_else(|e| panic!("package file {file}: {e}"));
        assert_eq!(
            digest.as_str().unwrap(),
            sha256_hex(&bytes),
            "the recorded package's {file} matches its manifest digest"
        );
    }
    assert!(
        manifest_json["files"]
            .as_object()
            .unwrap()
            .keys()
            .any(|f| f.ends_with(".lg")),
        "a package carries its LOGOS source"
    );
    let receipt = json(&fixtures().join("replay.receipt.json"));
    assert_eq!(receipt["replayed"], true, "the recorded package replays");
    assert_eq!(receipt["stages"]["fresh_replay"]["status"], "passed");
}

#[test]
fn a_refusal_publishes_nothing_and_names_why() {
    for (outcome, _) in OUTCOMES.into_iter().skip(1) {
        let r = json(&fixtures().join(format!("{outcome}.result.json")));
        assert!(
            r["package"].is_null(),
            "{outcome}: a refusal publishes no package"
        );
        assert!(
            !r["diagnostics"].as_array().unwrap().is_empty(),
            "{outcome}: a refusal carries at least one diagnostic saying why"
        );
        assert!(
            STAGES.iter().any(|s| r["stages"][*s] != "passed"),
            "{outcome}: a refusal cannot have every stage passed"
        );
    }
}

#[test]
fn the_fixtures_are_marked_as_recordings_with_intact_digests() {
    let index = json(&fixtures().join("FIXTURES.json"));
    assert_eq!(
        index["kind"], "recorded",
        "fixtures are recordings, never presented as live results"
    );
    let files = index["files"].as_object().expect("FIXTURES.json files");
    for (outcome, _) in OUTCOMES {
        for suffix in ["result.json", "diagnostics.ndjson", "exit"] {
            assert!(
                files.contains_key(&format!("{outcome}.{suffix}")),
                "{outcome}.{suffix} is indexed"
            );
        }
    }
    for (name, digest) in files {
        let bytes = fs::read(fixtures().join(name)).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(
            digest.as_str().unwrap(),
            sha256_hex(&bytes),
            "{name} changed after it was recorded"
        );
    }
}

/// SHA-256 from the compile crate's implementation, not the package's own, so the digests are checked by code
/// the package did not write.
fn sha256_hex(bytes: &[u8]) -> String {
    logicaffeine_compile::crypto_accel::sha256(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
