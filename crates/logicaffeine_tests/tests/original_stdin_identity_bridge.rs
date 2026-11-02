//! The two original instance identities produced when Lean's source module is `_stdin`.
//! Fixtures are byte copies of the September 25 original exports, not renamed declarations.
#[path = "harvest_support/checker.rs"]
mod checker;

use logicaffeine_kernel::interface::lean::{
    self as lean,
    package::{self, BuildOptions, Manifest, PackageFiles, PackageLimits, EXPORT, MANIFEST},
    receipt::StageStatus,
    reference::Reference,
};
use logicaffeine_kernel::{infer_type, recheck, Term, Universe};
use serde_json::json;
use std::path::{Path, PathBuf};

// Same re-executed test-binary door as lean_package_replay's R11 control. The child
// receives only a package directory; no importer context or producer receipt is passed.
const CHILD_PACKAGE: &str = "LOGOS_STDIN_IDENTITY_REPLAY_PACKAGE";
const BEGIN: &str = "<<<logos-stdin-identity-replay>>>";
const END: &str = "<<<end>>>";

#[test]
fn replay_child_entry_point() {
    let Ok(dir) = std::env::var(CHILD_PACKAGE) else {
        return;
    };
    let files = package::read_directory(Path::new(&dir), &PackageLimits::default())
        .expect("the child reads the downloaded package");
    let receipt = package::replay(&files);
    let mut document =
        package::receipt_document(&receipt, Some(&package::sha256_hex(&files[MANIFEST])));
    document["pid"] = json!(std::process::id());
    println!("{BEGIN}{}{END}", serde_json::to_string(&document).unwrap());
}

fn replay_in_fresh_process(files: &PackageFiles, tag: &str) -> serde_json::Value {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "logos-stdin-replay-{tag}-{}-{nonce}",
        std::process::id(),
    ));
    package::publish(&dir, files).expect("exclusive test package publication");
    assert_eq!(
        package::read_directory(&dir, &PackageLimits::default()).unwrap(),
        *files
    );
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "replay_child_entry_point",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(CHILD_PACKAGE, &dir)
        .output();
    // Remove only the unique directory successfully published by this invocation.
    std::fs::remove_dir_all(&dir).expect("owned test package cleanup");
    let output = output.expect("the test binary re-executes");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "child failed:\n{stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let start = stdout.find(BEGIN).expect("child receipt marker") + BEGIN.len();
    let end = stdout[start..].find(END).expect("child receipt end") + start;
    let receipt: serde_json::Value = serde_json::from_str(&stdout[start..end]).unwrap();
    assert_ne!(
        receipt["pid"].as_u64().unwrap(),
        u64::from(std::process::id())
    );
    assert_eq!(receipt["schema"], package::REPLAY_SCHEMA);
    assert_eq!(
        receipt["manifest_sha256"],
        package::sha256_hex(&files[MANIFEST])
    );
    assert_eq!(receipt["stages"]["lean_accepted"]["status"], "not_run");
    assert_eq!(receipt["verified"], false);
    receipt
}

fn original_checks_and_replays(
    case: &str,
    target: &'static str,
    export_sha: &str,
    source_sha: &str,
) {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let fixtures = root.join("tests/fixtures/lean_conformance/cases_lean414/stdin_identity");
    let stem = format!("{case}.instCoeBoolNat__stdin");
    let (export, provenance) = checker::fixture_in(&fixtures, &stem);
    assert_eq!(package::sha256_hex(export.as_bytes()), export_sha);
    assert_eq!(export.len(), 40_374);
    assert_eq!(export.lines().count(), 701);
    assert_eq!(provenance["module"], "_stdin");
    assert_eq!(provenance["declarations"], json!([target]));
    assert_eq!(provenance["source_sha256"], source_sha);
    assert_eq!(
        provenance["lean_print_axioms"],
        json!([format!("'{target}' does not depend on any axioms")]),
    );

    // The shared checker reads the original type independently and checks the actual imported
    // definition body and constant against it in BOTH kernels, including the exact assumptions.
    let reference = Reference::read(&export).expect("independent original source translation");
    let obligation = checker::Root {
        lean: target,
        kind: "def",
        axioms: &[],
    };
    let (context, report) = lean::import_audited_retaining(&export, &[])
        .unwrap_or_else(|error| panic!("{target}: original import: {error:?}"));
    let checked = checker::check_in(&export, &context, &report, &reference, &obligation)
        .unwrap_or_else(|error| panic!("{target}: {error}"));
    let wrong_type = Term::Pi {
        param: "__corrupted".into(),
        param_type: Box::new(Term::Sort(Universe::Prop)),
        body_type: Box::new(checked.statement),
    };
    let wrong_claim = Term::App(
        Box::new(Term::Lambda {
            param: "__wrong_statement".into(),
            param_type: Box::new(wrong_type),
            body: Box::new(Term::Var("__wrong_statement".into())),
        }),
        Box::new(checked.constant),
    );
    assert!(
        infer_type(&context, &wrong_claim).is_err(),
        "wrong original type: primary"
    );
    assert!(
        recheck(&context, &wrong_claim).is_err(),
        "wrong original type: independent"
    );

    let source_path = provenance["source"].as_str().expect("original source path");
    let source = std::fs::read(root.join("../..").join(source_path)).expect("original Lean source");
    assert_eq!(package::sha256_hex(&source), source_sha);
    let filename = std::path::Path::new(source_path)
        .file_name()
        .unwrap()
        .to_str()
        .unwrap();
    let mut options = BuildOptions::default();
    options
        .lean_sources
        .insert(filename.to_string(), source.clone());
    // This test did not invoke Lean: provenance is checked, but no producing Lean-run claim is made.
    assert!(options.lean_run.is_none());
    let files = package::assemble_observed(&export, target, &options, Some(&[]))
        .unwrap_or_else(|error| panic!("{target}: original package refused: {error:?}"));
    assert_eq!(files[EXPORT], export.as_bytes());
    assert_eq!(files[&format!("source/{filename}")], source);
    let manifest: Manifest = serde_json::from_slice(&files[MANIFEST]).unwrap();
    assert_eq!(manifest.schema, "logos-lean-package/1");
    assert_eq!(manifest.target.lean, target);
    assert_eq!(manifest.target.kind, "def");
    assert!(manifest.target.level_params.is_empty());
    assert!(manifest.assumptions.transitive.is_empty());
    assert_eq!(manifest.assumptions.lean_print_axioms, Some(vec![]));
    assert!(!files[manifest.document.as_ref().expect("actual LOGOS document")].is_empty());

    // Replay receives only file bytes; it rebuilds the certificate, binds original source bodies
    // and independently reparses emitted LOGOS. This is an in-process replay, not a subprocess claim.
    let replay = package::replay(&files);
    for stage in [
        "primary_check",
        "independent_recheck",
        "translated",
        "assumption_policy",
        "emission_reparsed",
        "fresh_replay",
    ] {
        assert_eq!(
            replay.status(stage),
            StageStatus::Passed,
            "{target}: {replay:#?}"
        );
    }
    assert!(replay.replayed());
    assert_eq!(replay.status("lean_accepted"), StageStatus::NotRun);
    assert!(
        !replay.verified(),
        "this test does not run the Lean compiler"
    );

    let child = replay_in_fresh_process(&files, case);
    for stage in [
        "primary_check",
        "independent_recheck",
        "translated",
        "assumption_policy",
        "emission_reparsed",
        "fresh_replay",
    ] {
        assert_eq!(
            child["stages"][stage]["status"], "passed",
            "{target}: {child}"
        );
    }
    assert_eq!(child["replayed"], true);

    // Replace emitted LOGOS and recompute every affected digest and result identity. The
    // unchanged certificate remains well typed; authentic emission/reparse must still refuse.
    let mut tampered = files.clone();
    let mut changed = manifest;
    let document = changed.document.clone().unwrap();
    tampered.insert(document.clone(), b"not a LOGOS proof document\n".to_vec());
    changed
        .files
        .insert(document.clone(), package::sha256_hex(&tampered[&document]));
    changed.result_id = package::result_id(target, &changed.files);
    tampered.insert(MANIFEST.into(), package::canonical_json(&changed));
    let refused = package::replay(&tampered);
    assert_eq!(
        refused.status("primary_check"),
        StageStatus::Passed,
        "{refused:#?}"
    );
    assert_eq!(
        refused.status("independent_recheck"),
        StageStatus::Passed,
        "{refused:#?}"
    );
    assert_eq!(
        refused.status("translated"),
        StageStatus::Passed,
        "{refused:#?}"
    );
    assert_eq!(
        refused.status("emission_reparsed"),
        StageStatus::Failed,
        "{refused:#?}"
    );
    assert_eq!(refused.status("fresh_replay"), StageStatus::Failed);
    assert!(!refused.replayed());
    assert!(!refused.verified());

    // The same re-digested corruption is refused by a NEW process too.
    let child = replay_in_fresh_process(&tampered, &format!("{case}-tampered"));
    for stage in ["primary_check", "independent_recheck", "translated"] {
        assert_eq!(child["stages"][stage]["status"], "passed", "{child}");
    }
    assert_eq!(child["stages"]["emission_reparsed"]["status"], "failed");
    assert_eq!(child["stages"]["fresh_replay"]["status"], "failed");
    assert_eq!(child["replayed"], false);
}

#[test]
fn t05_011_original_stdin_instance_checks_and_replays() {
    original_checks_and_replays(
        "T05-011",
        "T05_011.instCoeBoolNat__stdin",
        "a7f354af06a0ba3202985ddc91a7c674134fb560283c38e96b5ded58c953761c",
        "a0db1727d0fb112d2649e260c6203fcf328c735ac1621173b11ad07dae1e2886",
    );
}

#[test]
fn t05_018_original_stdin_instance_checks_and_replays() {
    original_checks_and_replays(
        "T05-018",
        "T05_018.instCoeBoolNat__stdin",
        "077006c441ce78293e4ae79d60563ff10add2ce0006d0bcf1a1a26658b25b15b",
        "a0bb1cb3c7a3827894635a64b7b0b1caf70687c25c5690b1a03b52ad2f6902f2",
    );
}
