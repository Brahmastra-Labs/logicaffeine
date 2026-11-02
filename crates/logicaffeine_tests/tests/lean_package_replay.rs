//! W05 — the portable proof package and its replay (competition rules R1, R11, R12).
//!
//! A package is downloaded, then replayed from its bytes alone in a process that never imported
//! anything, and it must prove the SAME statement the Lean source states. Every tampering control
//! the plan lists is a test here, and each asserts WHICH stage refused the package and with which
//! code: a digest failure where the kernel was supposed to answer is a weaker check passing for a
//! stronger one, so every semantic tamper below is re-digested first, exactly as an attacker who
//! can recompute a hash would do.
mod lean_package_support;

use lean_package_support::{declare_sorry, Export};
use logicaffeine_kernel::certificate::{
    recheck_document, EnvironmentCertificate, EnvironmentEntry, EnvironmentLimits, RecheckOutcome,
    TermNode,
};
use logicaffeine_kernel::interface::lean::package::{
    self, assemble, build, canonical_certificate, canonical_json, read_directory, replay,
    replay_with, sha256_hex, BuildOptions, CorrespondenceFile, Manifest, PackageFiles,
    PackageLimits, CERTIFICATE, CORRESPONDENCE, EXPORT, MANIFEST, PACKAGE_SCHEMA, REPLAY_SCHEMA,
};
use logicaffeine_kernel::interface::lean::receipt::{Receipt, StageStatus, REPLAY_STAGES, STAGES};
use logicaffeine_kernel::interface::lean::DiagnosticCode;
use logicaffeine_kernel::Universe;
use std::path::{Path, PathBuf};

const T06_002: &str = include_str!("fixtures/lean_conformance/cases_lean414/T06-002.ndjson");
const T00_014: &str = include_str!("fixtures/lean_conformance/cases_lean414/T00-014.ndjson");
const W05_SOURCE: &str = include_str!("fixtures/lean_package/W05_package.lean");
const W05_PROVENANCE: &str = include_str!("fixtures/lean_package/W05.provenance.json");
const W05_CLEAN: &str = include_str!("fixtures/lean_package/W05.clean.ndjson");
const W05_TAINTED: &str = include_str!("fixtures/lean_package/W05.tainted.ndjson");
const W05_GOAL_AXIOM: &str = include_str!("fixtures/lean_package/W05.byGoalAxiom.ndjson");
const W05_PROPEXT: &str = include_str!("fixtures/lean_package/W05.usesPropext.ndjson");
const W05_FUNEXT: &str = include_str!("fixtures/lean_package/W05.usesFunext.ndjson");
const W05_CHOICE: &str = include_str!("fixtures/lean_package/W05.usesChoice.ndjson");
const W05_RESTATES: &str = include_str!("fixtures/lean_package/W05.restatesPropext.ndjson");

/// Each genuine W05 export, the declaration it roots, and what Lean's own `#print axioms` reported
/// for it (recorded in the provenance), spelled as the kernel spells those names.
const W05: [(&str, &str, &[&str]); 7] = [
    ("clean", W05_CLEAN, &[]),
    ("tainted", W05_TAINTED, &["/s:\"sorryAx\""]),
    ("byGoalAxiom", W05_GOAL_AXIOM, &["/s:\"W05\"/s:\"goalAx\""]),
    ("usesPropext", W05_PROPEXT, &["/s:\"propext\""]),
    ("usesFunext", W05_FUNEXT, &["/s:\"Quot\"/s:\"sound\""]),
    ("usesChoice", W05_CHOICE, &["/s:\"Classical\"/s:\"choice\""]),
    ("restatesPropext", W05_RESTATES, &["/s:\"propext\""]),
];
const T06_ROOTS: [&str; 6] = [
    "bigMotive",
    "jDependent",
    "symmViaRec",
    "transport",
    "transportRaw",
    "viaMpr",
];
const CHILD_PACKAGE: &str = "LOGOS_LEAN_REPLAY_PACKAGE";
const BEGIN: &str = "<<<logos-lean-replay>>>";
const END: &str = "<<<end>>>";

fn t06(short: &str) -> String {
    format!("T06_002.{short}")
}

// -------------------------------------------------------------------------------------------------
// Helpers: packages, and tampering the way an attacker who can recompute a digest would.
// -------------------------------------------------------------------------------------------------

/// A package whose producing run may have failed stages — for tampering the certificate side
/// without depending on anything else.
fn built(export: &str, target: &str) -> PackageFiles {
    build(export, target, &BuildOptions::default())
        .unwrap_or_else(|v| panic!("build {target}: {v:?}"))
        .files
}

fn manifest(files: &PackageFiles) -> Manifest {
    serde_json::from_slice(&files[MANIFEST]).unwrap()
}

fn certificate(files: &PackageFiles) -> EnvironmentCertificate {
    serde_json::from_slice(&files[CERTIFICATE]).unwrap()
}

fn correspondence(files: &PackageFiles) -> CorrespondenceFile {
    serde_json::from_slice(&files[CORRESPONDENCE]).unwrap()
}

/// Recompute every digest and the result identity, re-encoding the manifest canonically.
fn redigest(files: &mut PackageFiles) {
    let mut m = manifest(files);
    m.files = files
        .iter()
        .filter(|(p, _)| *p != MANIFEST)
        .map(|(p, b)| (p.clone(), sha256_hex(b)))
        .collect();
    m.source.export_sha256 = m.files[EXPORT].clone();
    m.result_id = package::result_id(&m.target.lean, &m.files);
    files.insert(MANIFEST.into(), canonical_json(&m));
}

fn with_certificate(
    files: &PackageFiles,
    edit: impl FnOnce(&mut EnvironmentCertificate),
) -> PackageFiles {
    let mut files = files.clone();
    let mut c = certificate(&files);
    edit(&mut c);
    files.insert(CERTIFICATE.into(), canonical_certificate(&c));
    redigest(&mut files);
    files
}

fn with_manifest(files: &PackageFiles, edit: impl FnOnce(&mut Manifest)) -> PackageFiles {
    let mut files = files.clone();
    let mut m = manifest(&files);
    edit(&mut m);
    files.insert(MANIFEST.into(), canonical_json(&m));
    redigest(&mut files);
    files
}

fn push_node(c: &mut EnvironmentCertificate, node: TermNode) -> u32 {
    c.nodes.push(node);
    (c.nodes.len() - 1) as u32
}

fn entry_at(c: &EnvironmentCertificate, name: &str) -> usize {
    c.entries
        .iter()
        .position(|e| e.names().contains(&name))
        .unwrap_or_else(|| panic!("no entry {name}"))
}

/// The stage that refused `receipt`, and its code.
fn refused(receipt: &Receipt) -> (&'static str, String) {
    let stage = receipt
        .first_failure()
        .unwrap_or_else(|| panic!("the package was not refused: {receipt:#?}"));
    (stage.name, stage.code.clone().unwrap_or_default())
}

fn assert_refused(receipt: &Receipt, stage: &str, code: &str) {
    let (at, why) = refused(receipt);
    assert_eq!((at, why.as_str()), (stage, code), "{receipt:#?}");
    assert!(!receipt.replayed() && !receipt.verified());
    assert_eq!(
        receipt.status("fresh_replay"),
        StageStatus::Failed,
        "a refused replay never reports fresh_replay passed"
    );
}

fn scratch_dir(tag: &str) -> PathBuf {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "logos-lean-package-{tag}-{}-{nonce}",
        std::process::id()
    ))
}

// -------------------------------------------------------------------------------------------------
// R1 — the receipt names every stage.
// -------------------------------------------------------------------------------------------------

#[test]
fn receipt_names_every_stage() {
    assert_eq!(
        STAGES,
        [
            "lean_accepted",
            "translated",
            "primary_check",
            "independent_recheck",
            "assumption_policy",
            "emission_reparsed",
            "fresh_replay"
        ]
    );
    assert_eq!(REPLAY_STAGES.len(), STAGES.len() - 1);
    assert!(
        !REPLAY_STAGES.contains(&"lean_accepted"),
        "a replay never claims Lean ran"
    );
    let files = assemble(T06_002, &t06("symmViaRec")).unwrap_or_else(|v| panic!("{v:?}"));
    for receipt in [
        replay(&files),
        replay(&with_certificate(&files, |c| {
            c.prelude_version = "logos-coc-1".into()
        })),
    ] {
        let names: Vec<&str> = receipt.stages.iter().map(|s| s.name).collect();
        assert_eq!(names, STAGES, "one entry per stage, in order");
        for stage in &receipt.stages {
            assert!(
                !stage.detail.is_empty(),
                "{} says what happened",
                stage.name
            );
        }
        assert_eq!(
            receipt.status("lean_accepted"),
            StageStatus::NotRun,
            "a replay does not run Lean"
        );
        assert!(
            !receipt.verified(),
            "no replay is ever the whole verified claim"
        );
        let document = package::receipt_document(&receipt, None);
        assert_eq!(document["schema"], REPLAY_SCHEMA);
        for name in STAGES {
            let status = document["stages"][name]["status"]
                .as_str()
                .unwrap_or_else(|| panic!("{name} missing"));
            assert_eq!(StageStatus::parse(status), Some(receipt.status(name)));
        }
        assert_eq!(document["replayed"], receipt.replayed());
        assert_eq!(document["verified"], false);
    }
    // The producing run's receipt names every stage too, and never claims a fresh replay.
    let m = manifest(&files);
    assert_eq!(
        m.receipt
            .keys()
            .map(String::as_str)
            .collect::<std::collections::BTreeSet<_>>(),
        STAGES.into_iter().collect()
    );
    assert_eq!(m.receipt["fresh_replay"].status, "not_run");
    assert_eq!(
        m.receipt["lean_accepted"].status, "not_run",
        "this run read an export; it did not run Lean"
    );
    // Unknown, not-run and unsupported are never a pass.
    let mut r = Receipt::unstarted();
    for name in STAGES {
        r.set(name, StageStatus::Passed, "ok");
    }
    assert!(r.verified());
    for spoiler in [
        StageStatus::NotRun,
        StageStatus::Unsupported,
        StageStatus::Failed,
    ] {
        let mut s = r.clone();
        s.set("independent_recheck", spoiler, "x");
        assert!(!s.verified() && !s.replayed());
    }
}

// -------------------------------------------------------------------------------------------------
// R11 — a clean package replays, in a fresh process.
// -------------------------------------------------------------------------------------------------

/// The child half of the fresh-process test: when the parent sets the package path, this process —
/// started from nothing — reads the package from disk and replays it. Run normally, it is inert.
#[test]
fn replay_child_entry_point() {
    let Ok(dir) = std::env::var(CHILD_PACKAGE) else {
        return;
    };
    let files =
        read_directory(Path::new(&dir), &PackageLimits::default()).expect("the package reads");
    let receipt = replay(&files);
    let mut document = package::receipt_document(&receipt, Some(&sha256_hex(&files[MANIFEST])));
    document["pid"] = serde_json::json!(std::process::id());
    println!("{BEGIN}{}{END}", serde_json::to_string(&document).unwrap());
}

fn replay_in_fresh_process(dir: &Path) -> serde_json::Value {
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "replay_child_entry_point",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(CHILD_PACKAGE, dir)
        .output()
        .expect("the test binary re-executes");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "child failed:\n{stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let start = stdout
        .find(BEGIN)
        .unwrap_or_else(|| panic!("no receipt in child output:\n{stdout}"))
        + BEGIN.len();
    let end = stdout[start..].find(END).unwrap() + start;
    serde_json::from_str(&stdout[start..end]).unwrap()
}

#[test]
fn a_clean_package_replays_in_a_fresh_process() {
    let files = assemble(T06_002, &t06("viaMpr")).unwrap_or_else(|v| panic!("{v:?}"));
    let dir = scratch_dir("fresh");
    package::publish(&dir, &files).unwrap();
    // The package on disk is exactly the assembled bytes, and nothing else.
    assert_eq!(
        read_directory(&dir, &PackageLimits::default()).unwrap(),
        files
    );
    let receipt = replay_in_fresh_process(&dir);
    std::fs::remove_dir_all(&dir).unwrap();
    assert_ne!(
        receipt["pid"].as_u64().unwrap(),
        u64::from(std::process::id()),
        "a different process"
    );
    assert_eq!(receipt["schema"], REPLAY_SCHEMA);
    assert_eq!(receipt["manifest_sha256"], sha256_hex(&files[MANIFEST]));
    for stage in REPLAY_STAGES {
        assert_eq!(
            receipt["stages"][stage]["status"], "passed",
            "{stage}: {}",
            receipt["stages"][stage]
        );
    }
    assert_eq!(receipt["stages"]["lean_accepted"]["status"], "not_run");
    assert_eq!(receipt["replayed"], true);
    // And a tampered copy is refused by the same fresh process.
    let tampered = with_certificate(&files, |c| {
        let zero = push_node(c, TermNode::Global("Zero".into()));
        let at = entry_at(c, "/s:\"T06_002\"/s:\"viaMpr\"");
        if let EnvironmentEntry::Theorem { proof, .. } = &mut c.entries[at] {
            *proof = zero;
        }
    });
    let dir = scratch_dir("fresh-tampered");
    package::publish(&dir, &tampered).unwrap();
    let receipt = replay_in_fresh_process(&dir);
    std::fs::remove_dir_all(&dir).unwrap();
    assert_eq!(receipt["stages"]["primary_check"]["status"], "failed");
    assert_eq!(receipt["replayed"], false);
}

#[test]
fn every_t06_002_root_packages_and_replays_clean() {
    for short in T06_ROOTS {
        let files = assemble(T06_002, &t06(short)).unwrap_or_else(|v| panic!("{short}: {v:?}"));
        let receipt = replay(&files);
        assert!(receipt.replayed(), "{short}: {receipt:#?}");
        let m = manifest(&files);
        assert_eq!(m.target.lean, t06(short));
        assert_eq!(m.target.kernel, format!("/s:\"T06_002\"/s:{short:?}"));
        assert_eq!(
            m.target.kind,
            if short == "bigMotive" { "def" } else { "thm" }
        );
        assert!(
            m.assumptions.transitive.is_empty(),
            "{short}: {:?}",
            m.assumptions.transitive
        );
        assert!(m.target.logos.is_some(), "{short}: a LOGOS name");
        assert_eq!(m.emission.status, "emitted");
    }
}

#[test]
fn a_package_resting_on_an_allowed_axiom_replays_and_names_it() {
    let files = assemble(T00_014, "T00_014.needsFunext").unwrap_or_else(|v| panic!("{v:?}"));
    let receipt = replay(&files);
    assert!(receipt.replayed(), "{receipt:#?}");
    let m = manifest(&files);
    assert_eq!(
        m.assumptions.transitive,
        vec!["/s:\"Quot\"/s:\"sound\"".to_string()]
    );
    assert!(
        receipt
            .stage("assumption_policy")
            .unwrap()
            .detail
            .contains("Quot"),
        "{receipt:#?}"
    );
    assert_eq!(
        m.assumptions.allowed,
        vec!["propext", "Quot.sound", "Classical.choice"]
    );
}

#[test]
fn the_package_holds_exactly_its_contracted_files() {
    let files = assemble(T06_002, &t06("symmViaRec")).unwrap();
    let m = manifest(&files);
    assert_eq!(m.schema, PACKAGE_SCHEMA);
    let document = m.document.clone().expect("a LOGOS document");
    assert_eq!(document, "T06_002.symmViaRec.lg");
    let expected: std::collections::BTreeSet<String> = [
        MANIFEST,
        CERTIFICATE,
        CORRESPONDENCE,
        EXPORT,
        document.as_str(),
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    assert_eq!(
        files
            .keys()
            .cloned()
            .collect::<std::collections::BTreeSet<_>>(),
        expected
    );
    assert_eq!(files[EXPORT], T06_002.as_bytes(), "the exact export");
    assert_eq!(
        m.source.export_sha256, "caae696b50c275e0f9c17b0f07b27d10798a684d2e35f3e7550475f1c79d21d3",
        "the export's digest is the one its provenance recorded"
    );
    for (path, digest) in &m.files {
        assert_eq!(&sha256_hex(&files[path]), digest, "{path}");
    }
    assert_eq!(
        m.toolchain.prelude,
        logicaffeine_kernel::certificate::PRELUDE_VERSION
    );
    assert_eq!(
        m.toolchain.certificate_schema,
        "logos-environment-certificate/1"
    );
    assert_eq!(m.source.lean_version, "4.14.0");
    assert_eq!(m.source.format, "3.1.0");
    assert!(
        m.trusted_base
            .iter()
            .any(|s| s.contains("not an independent theorem prover")),
        "the shared TCB is stated"
    );
    assert!(m
        .information_losses
        .iter()
        .any(|l| l.category == "SOURCE_POSITIONS"));
    let c = correspondence(&files);
    let target = c
        .rows
        .iter()
        .find(|r| r.kernel == m.target.kernel)
        .expect("a target row");
    assert_eq!(target.lean, m.target.lean);
    assert_eq!(target.logos, m.target.logos);
    let source = String::from_utf8(files[&document].clone()).unwrap();
    assert!(
        source.contains(
            m.target
                .logos
                .as_deref()
                .unwrap()
                .rsplit('.')
                .next()
                .unwrap()
        ),
        "the document declares the target"
    );
}

#[test]
fn assembly_is_deterministic() {
    assert_eq!(
        assemble(T06_002, &t06("jDependent")).unwrap(),
        assemble(T06_002, &t06("jDependent")).unwrap()
    );
}

#[test]
fn digests_are_sha256() {
    assert_eq!(
        sha256_hex(b""),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
    assert_eq!(
        sha256_hex(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    assert_eq!(
        sha256_hex(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
        "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
    );
    assert_eq!(
        sha256_hex(&vec![b'a'; 1_000_000]),
        "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"
    );
    for n in [55usize, 56, 57, 63, 64, 65, 119, 120] {
        // Padding boundaries: every length near a block edge digests, and distinct lengths differ.
        assert_ne!(sha256_hex(&vec![0u8; n]), sha256_hex(&vec![0u8; n + 1]));
    }
    assert_eq!(
        sha256_hex(T06_002.as_bytes()),
        "caae696b50c275e0f9c17b0f07b27d10798a684d2e35f3e7550475f1c79d21d3"
    );
    assert_eq!(
        sha256_hex(T00_014.as_bytes()),
        "5c9f30cc8f14e1756fd8981f66468ddecb57b2a07a4f371a049a78d9e04b0425"
    );
}

#[test]
fn the_certificate_in_a_package_rechecks_on_its_own() {
    let files = assemble(T06_002, &t06("viaMpr")).unwrap();
    let report = recheck_document(
        std::str::from_utf8(&files[CERTIFICATE]).unwrap(),
        &EnvironmentLimits::default(),
    );
    assert_eq!(
        report.outcome,
        RecheckOutcome::Verified,
        "{:?}",
        report.lines
    );
    assert!(report
        .lines
        .iter()
        .any(|l| l.contains("statement binding: not established by a certificate alone")));
}

#[test]
fn the_recheck_tool_is_a_shell_over_the_library_doors() {
    let source = include_str!("../../logicaffeine_kernel/examples/recheck.rs");
    for door in [
        "recheck_document",
        "package::replay",
        "package::read_directory",
        "package::receipt_document",
        "\"--package\"",
    ] {
        assert!(
            source.contains(door),
            "examples/recheck.rs goes through {door}"
        );
    }
    assert!(
        !source.contains("serde_json::from_str::<Certificate>"),
        "no bare-certificate shortcut"
    );
}

// -------------------------------------------------------------------------------------------------
// The tamper matrix (W05): every control fails, at the stage that should catch it.
// -------------------------------------------------------------------------------------------------

#[test]
fn a_changed_proof_body_is_refused() {
    let files = built(T06_002, &t06("symmViaRec"));
    let tampered = with_certificate(&files, |c| {
        // A declared term of the wrong type: `Eq.refl` itself, where a proof of symmetry belongs.
        let wrong = push_node(
            c,
            TermNode::Const("/s:\"Eq\"/s:\"refl\"".into(), vec![Universe::Type(0)]),
        );
        let at = entry_at(c, "/s:\"T06_002\"/s:\"symmViaRec\"");
        if let EnvironmentEntry::Theorem { proof, .. } = &mut c.entries[at] {
            *proof = wrong;
        }
    });
    let receipt = replay(&tampered);
    assert_refused(&receipt, "primary_check", "ill_typed");
}

#[test]
fn a_changed_goal_is_refused() {
    let files = built(T06_002, &t06("symmViaRec"));
    // The goal changed and the proof kept: the kernel refuses the proof.
    let changed = with_certificate(&files, |c| {
        let prop = push_node(c, TermNode::Sort(Universe::Prop));
        let at = entry_at(c, "/s:\"T06_002\"/s:\"symmViaRec\"");
        if let EnvironmentEntry::Theorem { ty, .. } = &mut c.entries[at] {
            *ty = prop;
        }
    });
    assert_refused(&replay(&changed), "primary_check", "ill_typed");
    // The goal AND proof replaced by a genuine proof of a different theorem — reflexivity,
    // `Π (A : Type) (a : A), a = a` by `Eq.refl` — so the kernel is content, and the statement
    // binding refuses: a true proof of something else is not this theorem.
    let swapped = with_certificate(&files, |c| {
        let level = vec![Universe::Type(0)];
        let type0 = push_node(c, TermNode::Sort(Universe::Type(0)));
        let a_ty = push_node(c, TermNode::Var("A".into()));
        let a = push_node(c, TermNode::Var("a".into()));
        let eq = push_node(c, TermNode::Const("/s:\"Eq\"".into(), level.clone()));
        let eq_a = push_node(c, TermNode::App(eq, a_ty));
        let eq_a_a = push_node(c, TermNode::App(eq_a, a));
        let eq_a_a_a = push_node(c, TermNode::App(eq_a_a, a));
        let inner_ty = push_node(c, TermNode::Pi("a".into(), a_ty, eq_a_a_a));
        let goal = push_node(c, TermNode::Pi("A".into(), type0, inner_ty));
        let refl = push_node(c, TermNode::Const("/s:\"Eq\"/s:\"refl\"".into(), level));
        let refl_a = push_node(c, TermNode::App(refl, a_ty));
        let refl_a_a = push_node(c, TermNode::App(refl_a, a));
        let inner = push_node(c, TermNode::Lambda("a".into(), a_ty, refl_a_a));
        let proof_of_refl = push_node(c, TermNode::Lambda("A".into(), type0, inner));
        let at = entry_at(c, "/s:\"T06_002\"/s:\"symmViaRec\"");
        if let EnvironmentEntry::Theorem { ty, proof, .. } = &mut c.entries[at] {
            *ty = goal;
            *proof = proof_of_refl;
        }
        // A consistent forgery: the binder annotations of the old statement go with it.
        c.binder_kinds.remove("/s:\"T06_002\"/s:\"symmViaRec\"");
    });
    let receipt = replay(&swapped);
    assert_eq!(
        receipt.status("primary_check"),
        StageStatus::Passed,
        "{receipt:#?}"
    );
    assert_refused(&receipt, "translated", "statement_mismatch");
}

#[test]
fn a_proof_of_true_does_not_satisfy_an_unrelated_theorem() {
    // The manifest retargeted at a different Lean declaration while the certificate still proves
    // the original: the binding reads the export's statement for the claimed target.
    let files = built(T06_002, &t06("symmViaRec"));
    let retargeted = with_manifest(&files, |m| m.target.lean = t06("transport"));
    assert_refused(&replay(&retargeted), "translated", "target_mismatch");
}

#[test]
fn a_changed_source_mapping_is_refused() {
    let files = built(T06_002, &t06("viaMpr"));
    // A Lean declaration mapped to the wrong kernel declaration.
    let mut remapped = files.clone();
    let mut c = correspondence(&remapped);
    let target_row = c.rows.iter().position(|r| r.lean == t06("viaMpr")).unwrap();
    let other = c
        .rows
        .iter()
        .position(|r| r.kernel != c.rows[target_row].kernel && r.origin == "authored")
        .unwrap();
    let (a, b) = (c.rows[target_row].lean.clone(), c.rows[other].lean.clone());
    c.rows[target_row].lean = b;
    c.rows[other].lean = a;
    remapped.insert(CORRESPONDENCE.into(), canonical_json(&c));
    redigest(&mut remapped);
    assert_refused(&replay(&remapped), "translated", "correspondence_mismatch");
    // A row dropped.
    let mut dropped = files.clone();
    let mut c = correspondence(&dropped);
    c.rows.retain(|r| r.lean != t06("viaMpr"));
    dropped.insert(CORRESPONDENCE.into(), canonical_json(&c));
    redigest(&mut dropped);
    assert_refused(&replay(&dropped), "translated", "correspondence_mismatch");
    // A row pointing at a declaration the certificate does not have.
    let mut invented = files.clone();
    let mut c = correspondence(&invented);
    c.rows[0].kernel = "/s:\"Invented\"".into();
    invented.insert(CORRESPONDENCE.into(), canonical_json(&c));
    redigest(&mut invented);
    assert_refused(&replay(&invented), "translated", "correspondence_mismatch");
}

#[test]
fn a_removed_definition_is_refused() {
    let files = built(T06_002, &t06("viaMpr"));
    let tampered = with_certificate(&files, |c| {
        let target = entry_at(c, "/s:\"T06_002\"/s:\"viaMpr\"");
        let at = c.entries[..target]
            .iter()
            .rposition(|e| {
                matches!(
                    e,
                    EnvironmentEntry::Definition { .. } | EnvironmentEntry::Theorem { .. }
                )
            })
            .unwrap();
        c.entries.remove(at);
    });
    assert_refused(&replay(&tampered), "primary_check", "unresolved_obligation");
}

#[test]
fn an_ill_typed_definition_with_a_false_declared_type_is_refused() {
    let files = built(T06_002, &t06("bigMotive"));
    let tampered = with_certificate(&files, |c| {
        // An extra definition `lie : False-ish := Zero`, declared at the type of the target goal.
        let at = entry_at(c, "/s:\"T06_002\"/s:\"bigMotive\"");
        let goal = match &c.entries[at] {
            EnvironmentEntry::Definition { ty, .. } => *ty,
            _ => unreachable!(),
        };
        let zero = push_node(c, TermNode::Global("Zero".into()));
        c.entries.insert(
            at,
            EnvironmentEntry::Definition {
                name: "/s:\"lie\"".into(),
                level_params: vec![],
                ty: goal,
                body: zero,
            },
        );
    });
    assert_refused(&replay(&tampered), "primary_check", "ill_typed");
}

#[test]
fn an_unresolved_forward_obligation_is_refused() {
    let files = built(T06_002, &t06("viaMpr"));
    let tampered = with_certificate(&files, |c| {
        let at = entry_at(c, "/s:\"T06_002\"/s:\"viaMpr\"");
        let moved = c.entries.remove(at);
        c.entries.insert(0, moved);
    });
    assert_refused(&replay(&tampered), "primary_check", "unresolved_obligation");
    // A reference to a name nothing declares.
    let dangling = with_certificate(&files, |c| {
        let ghost = push_node(c, TermNode::Global("/s:\"ghost\"".into()));
        let at = entry_at(c, "/s:\"T06_002\"/s:\"viaMpr\"");
        let ty = match &c.entries[at] {
            EnvironmentEntry::Theorem { ty, .. } => *ty,
            _ => unreachable!(),
        };
        c.entries.insert(
            at,
            EnvironmentEntry::Definition {
                name: "/s:\"uses_ghost\"".into(),
                level_params: vec![],
                ty,
                body: ghost,
            },
        );
    });
    assert_refused(&replay(&dangling), "primary_check", "unresolved_obligation");
}

#[test]
fn a_replaced_prelude_is_refused() {
    let files = built(T06_002, &t06("viaMpr"));
    let retired = with_certificate(&files, |c| c.prelude_version = "logos-coc-1".into());
    assert_refused(&replay(&retired), "fresh_replay", "incompatible_prelude");
    let unread = "logos-coc-999";
    assert_ne!(
        unread,
        logicaffeine_kernel::certificate::PRELUDE_VERSION,
        "the premise: a prelude this replay does not read"
    );
    let manifest_claims = with_manifest(&files, |m| m.toolchain.prelude = unread.into());
    assert_refused(
        &replay(&manifest_claims),
        "fresh_replay",
        "unsupported_format",
    );
    let rebased = with_certificate(&files, |c| {
        c.base = logicaffeine_kernel::certificate::EnvironmentBase::StandardLibrary
    });
    assert_refused(
        &replay(&rebased),
        "primary_check",
        "trusted_name_replacement",
    );
    for trusted in ["Quot_lift", "dne", "Eq", "kernel.quot.type"] {
        let smuggled = with_certificate(&files, |c| {
            let prop = push_node(c, TermNode::Sort(Universe::Prop));
            c.entries.insert(
                0,
                EnvironmentEntry::Axiom {
                    name: trusted.into(),
                    level_params: vec![],
                    ty: prop,
                },
            );
        });
        assert_refused(
            &replay(&smuggled),
            "primary_check",
            "trusted_name_replacement",
        );
    }
}

#[test]
fn an_altered_universe_or_constructor_is_refused() {
    let files = built(T06_002, &t06("symmViaRec"));
    let eq = |c: &EnvironmentCertificate| {
        c.entries.iter().position(|e| matches!(e, EnvironmentEntry::Inductive { types, .. } if types[0].name == "/s:\"Eq\"")).unwrap()
    };
    // A universe telescope renamed: the block no longer binds its own level.
    let renamed = with_certificate(&files, |c| {
        let at = eq(c);
        if let EnvironmentEntry::Inductive { level_params, .. } = &mut c.entries[at] {
            level_params[0] = "/s:\"v\"".into();
        }
    });
    assert_refused(&replay(&renamed), "primary_check", "ill_typed");
    // Eq placed in `Type` instead of `Prop`: a well-formed datatype, and not Lean's.
    let lifted = with_certificate(&files, |c| {
        let at = eq(c);
        let arity = match &c.entries[at] {
            EnvironmentEntry::Inductive { types, .. } => types[0].arity,
            _ => unreachable!(),
        };
        let mut row = arity as usize;
        // Walk the arity's telescope to its sort and rebuild it at Type 0.
        let mut path = Vec::new();
        while let TermNode::Pi(name, domain, body) = c.nodes[row].clone() {
            path.push((name, domain));
            row = body as usize;
        }
        let mut top = push_node(c, TermNode::Sort(Universe::Type(0)));
        for (name, domain) in path.into_iter().rev() {
            top = push_node(c, TermNode::Pi(name, domain, top));
        }
        if let EnvironmentEntry::Inductive { types, .. } = &mut c.entries[at] {
            types[0].arity = top;
        }
    });
    let receipt = replay(&lifted);
    let (stage, _) = refused(&receipt);
    assert!(
        matches!(stage, "primary_check" | "translated"),
        "{receipt:#?}"
    );
    // A constructor with a different type.
    let bent = with_certificate(&files, |c| {
        let at = eq(c);
        let family = push_node(c, TermNode::Global("/s:\"Eq\"".into()));
        if let EnvironmentEntry::Inductive { types, .. } = &mut c.entries[at] {
            types[0].constructors[0].ty = family;
        }
    });
    assert_refused(&replay(&bent), "primary_check", "ill_typed");
}

#[test]
fn an_injected_axiom_is_refused() {
    let files = built(T06_002, &t06("symmViaRec"));
    // The proof replaced by an axiom stating the goal, re-digested, the manifest updated to confess it.
    let injected = with_certificate(&files, |c| {
        let at = entry_at(c, "/s:\"T06_002\"/s:\"symmViaRec\"");
        let goal = match &c.entries[at] {
            EnvironmentEntry::Theorem { ty, .. } => *ty,
            _ => unreachable!(),
        };
        let axiom = push_node(c, TermNode::Global("/s:\"goalAx\"".into()));
        if let EnvironmentEntry::Theorem { proof, .. } = &mut c.entries[at] {
            *proof = axiom;
        }
        c.entries.insert(
            at,
            EnvironmentEntry::Axiom {
                name: "/s:\"goalAx\"".into(),
                level_params: vec![],
                ty: goal,
            },
        );
    });
    let injected = with_manifest(&injected, |m| {
        m.assumptions.transitive = vec!["/s:\"goalAx\"".into()]
    });
    let receipt = replay(&injected);
    assert_eq!(
        receipt.status("primary_check"),
        StageStatus::Passed,
        "the kernel accepts an assumed goal: {receipt:#?}"
    );
    assert_eq!(receipt.status("assumption_policy"), StageStatus::Failed);
    assert_eq!(
        receipt.stage("assumption_policy").unwrap().code.as_deref(),
        Some("goal_axiom")
    );
    // It is also an invention: the export declares no such axiom.
    assert_eq!(
        receipt.stage("translated").unwrap().code.as_deref(),
        Some("unsourced_declaration")
    );
    assert_eq!(receipt.status("fresh_replay"), StageStatus::Failed);
    // The same injection with the manifest left silent is a lie about the assumptions as well.
    let silent = with_certificate(&files, |c| {
        let at = entry_at(c, "/s:\"T06_002\"/s:\"symmViaRec\"");
        let goal = match &c.entries[at] {
            EnvironmentEntry::Theorem { ty, .. } => *ty,
            _ => unreachable!(),
        };
        let axiom = push_node(c, TermNode::Global("/s:\"goalAx\"".into()));
        if let EnvironmentEntry::Theorem { proof, .. } = &mut c.entries[at] {
            *proof = axiom;
        }
        c.entries.insert(
            at,
            EnvironmentEntry::Axiom {
                name: "/s:\"goalAx\"".into(),
                level_params: vec![],
                ty: goal,
            },
        );
    });
    assert_eq!(
        replay(&silent).status("assumption_policy"),
        StageStatus::Failed
    );
}

#[test]
fn an_indirect_sorry_is_refused() {
    // The genuine Lean artifact: `tainted := lemmaAlias α a b`, `lemmaAlias := lemmaAdmitted α a b`,
    // `lemmaAdmitted := sorry`. Lean itself warns only on `lemmaAdmitted`; `tainted` compiles clean.
    let provenance: serde_json::Value = serde_json::from_str(W05_PROVENANCE).unwrap();
    assert_eq!(
        provenance["exports"]["tainted"]["lean_print_axioms"],
        serde_json::json!(["sorryAx"])
    );
    assert!(W05_SOURCE.contains("theorem tainted (α : Type) (a b : α) : a = b := lemmaAlias α a b"));
    // Assembly refuses: no package is handed out for an admitted proof.
    let verdict =
        assemble(W05_TAINTED, "W05.tainted").expect_err("an admitted proof is never packaged");
    assert_eq!(
        verdict.reason().code(),
        DiagnosticCode::DisallowedAssumption,
        "{verdict:?}"
    );
    assert!(
        verdict.reason().message().contains("sorryAx"),
        "{verdict:?}"
    );
    // The package a producing run can still build records the refusal, and a replay reaches the
    // same verdict from the bytes alone — by structure, through the opaque lemma and the alias.
    let files = built(W05_TAINTED, "W05.tainted");
    let c = certificate(&files);
    assert!(c.entries.iter().any(|e| matches!(e, EnvironmentEntry::Theorem { name, .. } if name == "/s:\"W05\"/s:\"lemmaAdmitted\"")),
        "the opaque lemma is carried with its proof");
    assert!(c.entries.iter().any(|e| matches!(e, EnvironmentEntry::Definition { name, .. } if name == "/s:\"W05\"/s:\"lemmaAlias\"")),
        "the alias is carried");
    let m = manifest(&files);
    assert_eq!(m.receipt["assumption_policy"].status, "failed");
    assert_eq!(
        m.assumptions.transitive,
        vec!["/s:\"sorryAx\"".to_string()],
        "exactly Lean's #print axioms"
    );
    let receipt = replay(&files);
    assert_eq!(
        receipt.status("primary_check"),
        StageStatus::Passed,
        "an admission type-checks: {receipt:#?}"
    );
    assert_eq!(
        receipt.status("translated"),
        StageStatus::Passed,
        "and states the right theorem: {receipt:#?}"
    );
    assert_eq!(receipt.status("assumption_policy"), StageStatus::Failed);
    assert_eq!(
        receipt.stage("assumption_policy").unwrap().code.as_deref(),
        Some("admitted_proof")
    );
    assert!(receipt
        .stage("assumption_policy")
        .unwrap()
        .detail
        .contains("sorryAx"));
    assert!(!receipt.replayed());
    // Renamed, the admission is still an admission: its type inhabits every sort.
    let renamed = W05_TAINTED.replace("\"sorryAx\"", "\"harmlessLemma\"");
    assert_ne!(renamed, W05_TAINTED);
    let receipt = replay(&built(&renamed, "W05.tainted"));
    assert_eq!(
        receipt.stage("assumption_policy").unwrap().code.as_deref(),
        Some("admitted_proof"),
        "{receipt:#?}"
    );
    assert!(receipt
        .stage("assumption_policy")
        .unwrap()
        .detail
        .contains("harmlessLemma"));
    // Renamed to an allowed axiom's name, it does not borrow that axiom's standing.
    let impostor = W05_TAINTED.replace("\"sorryAx\"", "\"propext\"");
    let receipt = replay(&built(&impostor, "W05.tainted"));
    assert_eq!(
        receipt.stage("assumption_policy").unwrap().code.as_deref(),
        Some("admitted_proof"),
        "{receipt:#?}"
    );
    // The hand-built shape (sorryAx through an opaque lemma and an alias over an axiom `P`) is
    // refused the same way.
    let receipt = replay(&built(&indirect_sorry_export(), "T"));
    assert_eq!(
        receipt.stage("assumption_policy").unwrap().code.as_deref(),
        Some("admitted_proof"),
        "{receipt:#?}"
    );
}

/// `sorryAx`, a theorem `lemma : P` proved by it, a definition `alias := lemma`, a theorem
/// `T : P := alias`, over an assumed `P`.
fn indirect_sorry_export() -> String {
    let mut e = Export::new();
    let (sorry, bool_false) = declare_sorry(&mut e);
    let p = e.name("P", 0);
    let lemma = e.name("lemma", 0);
    let alias = e.name("alias", 0);
    let t = e.name("T", 0);
    let prop = e.sort(0);
    e.axiom(p, prop, &[]);
    let pc = e.constant(p, &[]);
    let sorry_at_prop = e.constant(sorry, &[0]);
    let sorry_p = e.app(sorry_at_prop, pc);
    let false_c = e.constant(bool_false, &[]);
    let admitted = e.app(sorry_p, false_c);
    e.thm(lemma, pc, admitted);
    let lemma_c = e.constant(lemma, &[]);
    e.def(alias, pc, lemma_c);
    let alias_c = e.constant(alias, &[]);
    e.thm(t, pc, alias_c);
    e.text()
}

#[test]
fn the_w05_fixtures_are_the_recorded_lean_artifacts() {
    let provenance: serde_json::Value = serde_json::from_str(W05_PROVENANCE).unwrap();
    assert_eq!(provenance["toolchain"], "leanprover/lean4:v4.14.0");
    assert_eq!(
        provenance["source_sha256"],
        sha256_hex(W05_SOURCE.as_bytes())
    );
    assert_eq!(
        provenance["exporter_source_sha256"]["Export.lean"],
        "5fecc2ebc45c62b618ec47705d47479117533676ae2f8e1e7e6740d941153224",
        "the same structural exporter that produced the corpus exports"
    );
    assert_eq!(provenance["compiler_exit"], 0);
    for (root, export, _) in W05 {
        let row = &provenance["exports"][root];
        assert_eq!(row["sha256"], sha256_hex(export.as_bytes()), "{root}");
        assert_eq!(row["records"], export.lines().count(), "{root}");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(export.lines().next().unwrap()).unwrap(),
            provenance["export_header"]
        );
    }
}

/// Every genuine W05 root: the transitive assumptions a package records for statement and proof
/// are exactly what Lean's own `#print axioms` reported for it, and the policy verdict follows.
#[test]
fn package_assumptions_are_exactly_leans_print_axioms() {
    for (root, export, lean_axioms) in W05 {
        let files = built(export, &format!("W05.{root}"));
        let m = manifest(&files);
        let expected: Vec<String> = lean_axioms.iter().map(|s| s.to_string()).collect();
        assert_eq!(m.assumptions.transitive, expected, "{root}");
        let receipt = replay(&files);
        let policy = receipt.stage("assumption_policy").unwrap();
        match root {
            "tainted" => assert_eq!(
                policy.code.as_deref(),
                Some("admitted_proof"),
                "{root}: {receipt:#?}"
            ),
            "byGoalAxiom" => assert_eq!(
                policy.code.as_deref(),
                Some("goal_axiom"),
                "{root}: {receipt:#?}"
            ),
            _ => assert_eq!(policy.status, StageStatus::Passed, "{root}: {receipt:#?}"),
        }
        for stage in ["translated", "primary_check", "independent_recheck"] {
            assert_eq!(
                receipt.status(stage),
                StageStatus::Passed,
                "{root}: {stage}: {receipt:#?}"
            );
        }
    }
}

#[test]
fn genuine_lean_theorems_resting_on_allowed_axioms_package_and_replay_clean() {
    for (root, export, _) in W05.iter().filter(|(r, _, _)| {
        matches!(
            *r,
            "clean" | "usesPropext" | "usesFunext" | "usesChoice" | "restatesPropext"
        )
    }) {
        let files =
            assemble(export, &format!("W05.{root}")).unwrap_or_else(|v| panic!("{root}: {v:?}"));
        let receipt = replay(&files);
        assert!(receipt.replayed(), "{root}: {receipt:#?}");
    }
}

#[test]
fn a_genuine_goal_axiom_is_never_a_solution() {
    let verdict =
        assemble(W05_GOAL_AXIOM, "W05.byGoalAxiom").expect_err("an assumed goal is not a result");
    assert_eq!(
        verdict.reason().code(),
        DiagnosticCode::DisallowedAssumption,
        "{verdict:?}"
    );
    assert!(verdict.reason().message().contains("goalAx"), "{verdict:?}");
    let receipt = replay(&built(W05_GOAL_AXIOM, "W05.byGoalAxiom"));
    assert_eq!(
        receipt.status("translated"),
        StageStatus::Passed,
        "Lean declared the axiom; it is not an invention: {receipt:#?}"
    );
    assert_eq!(
        receipt.stage("assumption_policy").unwrap().code.as_deref(),
        Some("goal_axiom")
    );
}

#[test]
fn corrupted_bytes_are_refused() {
    let files = assemble(T06_002, &t06("symmViaRec")).unwrap();
    for path in [CERTIFICATE, CORRESPONDENCE, EXPORT] {
        let mut corrupted = files.clone();
        let bytes = corrupted.get_mut(path).unwrap();
        let middle = bytes.len() / 2;
        bytes[middle] ^= 0x20;
        assert_refused(&replay(&corrupted), "fresh_replay", "digest_mismatch");
    }
    let document = manifest(&files).document.unwrap();
    let mut corrupted = files.clone();
    corrupted.get_mut(&document).unwrap().push(b' ');
    assert_refused(&replay(&corrupted), "fresh_replay", "digest_mismatch");
    let mut truncated = files.clone();
    truncated.get_mut(MANIFEST).unwrap().truncate(40);
    assert_refused(&replay(&truncated), "fresh_replay", "malformed");
    // A certificate re-encoded with different whitespace is refused: one encoding, never two.
    let mut spaced = files.clone();
    let pretty = serde_json::to_vec_pretty(&certificate(&files)).unwrap();
    spaced.insert(CERTIFICATE.into(), pretty);
    redigest(&mut spaced);
    assert_refused(&replay(&spaced), "fresh_replay", "non_canonical_encoding");
    // A digest the manifest merely asserts, with the result identity stale.
    let mut stale = files.clone();
    let mut m = manifest(&stale);
    m.result_id = format!("sha256:{}", "0".repeat(64));
    stale.insert(MANIFEST.into(), canonical_json(&m));
    assert_refused(&replay(&stale), "fresh_replay", "result_id_mismatch");
}

#[test]
fn an_unsupported_format_is_refused() {
    let files = built(T06_002, &t06("symmViaRec"));
    let mut future = files.clone();
    let mut value: serde_json::Value = serde_json::from_slice(&future[MANIFEST]).unwrap();
    let unread = "logos-lean-package/999";
    assert!(
        !package::PACKAGE_SCHEMAS.contains(&unread),
        "the premise: a schema this replay does not read"
    );
    value["schema"] = serde_json::json!(unread);
    future.insert(MANIFEST.into(), serde_json::to_vec_pretty(&value).unwrap());
    assert_refused(&replay(&future), "fresh_replay", "unsupported_format");
    let mut extended = files.clone();
    let mut value: serde_json::Value = serde_json::from_slice(&extended[MANIFEST]).unwrap();
    value["mandatory_extension"] = serde_json::json!({"critical": true});
    extended.insert(MANIFEST.into(), serde_json::to_vec_pretty(&value).unwrap());
    assert_refused(&replay(&extended), "fresh_replay", "unsupported_format");
    let mut schema = files.clone();
    let mut value: serde_json::Value = serde_json::from_slice(&schema[CERTIFICATE]).unwrap();
    value["schema"] = serde_json::json!("logos-environment-certificate/9");
    let mut bytes = serde_json::to_vec(&value).unwrap();
    bytes.push(b'\n');
    schema.insert(CERTIFICATE.into(), bytes);
    redigest(&mut schema);
    assert_refused(&replay(&schema), "fresh_replay", "unsupported_format");
    let mut export = files.clone();
    let text = T06_002.replacen(
        "\"version\":\"3.1.0\"},\"exporter\"",
        "\"version\":\"4.0.0\"},\"exporter\"",
        1,
    );
    assert_ne!(text, T06_002);
    export.insert(EXPORT.into(), text.into_bytes());
    redigest(&mut export);
    assert_refused(&replay(&export), "fresh_replay", "unsupported_format");
}

#[test]
fn an_exhausted_budget_is_refused_before_any_work() {
    let files = built(T06_002, &t06("viaMpr"));
    let generous = PackageLimits::default();
    for (what, limits, code) in [
        (
            "files",
            PackageLimits {
                max_files: 2,
                ..generous
            },
            "resource_limit",
        ),
        (
            "file bytes",
            PackageLimits {
                max_file_bytes: 1000,
                ..generous
            },
            "resource_limit",
        ),
        (
            "total bytes",
            PackageLimits {
                max_total_bytes: 10_000,
                ..generous
            },
            "resource_limit",
        ),
        (
            "path bytes",
            PackageLimits {
                max_path_bytes: 8,
                ..generous
            },
            "unsafe_path",
        ),
        (
            "term rows",
            PackageLimits {
                environment: EnvironmentLimits {
                    max_nodes: 10,
                    ..generous.environment
                },
                ..generous
            },
            "resource_limit",
        ),
        (
            "term depth",
            PackageLimits {
                environment: EnvironmentLimits {
                    max_depth: 4,
                    ..generous.environment
                },
                ..generous
            },
            "resource_limit",
        ),
        (
            "unfolded size",
            PackageLimits {
                environment: EnvironmentLimits {
                    max_tree_nodes: 50,
                    ..generous.environment
                },
                ..generous
            },
            "resource_limit",
        ),
    ] {
        let receipt = replay_with(&files, &limits);
        assert_refused(&receipt, "fresh_replay", code);
        for stage in [
            "translated",
            "primary_check",
            "independent_recheck",
            "assumption_policy",
            "emission_reparsed",
        ] {
            assert_eq!(
                receipt.status(stage),
                StageStatus::NotRun,
                "{what}: {stage} ran before the limit was enforced"
            );
        }
    }
    // An exponential DAG in the certificate is refused by its unfolded size, never built.
    let bomb = with_certificate(&files, |c| {
        let mut top = push_node(c, TermNode::Global("Nat".into()));
        for _ in 0..64 {
            top = push_node(c, TermNode::App(top, top));
        }
        c.entries.push(EnvironmentEntry::Axiom {
            name: "/s:\"bomb\"".into(),
            level_params: vec![],
            ty: top,
        });
    });
    let limits = PackageLimits {
        environment: EnvironmentLimits {
            max_depth: 1000,
            ..generous.environment
        },
        ..generous
    };
    let receipt = replay_with(&bomb, &limits);
    assert_refused(&receipt, "fresh_replay", "resource_limit");
    assert_eq!(receipt.status("primary_check"), StageStatus::NotRun);
}

#[test]
fn a_malicious_dependency_cycle_is_refused() {
    let files = built(T06_002, &t06("viaMpr"));
    // A term row citing itself.
    let self_row = with_certificate(&files, |c| {
        let next = c.nodes.len() as u32;
        c.nodes.push(TermNode::App(next, 0));
    });
    assert_refused(&replay(&self_row), "fresh_replay", "malformed");
    // Two declarations citing each other.
    let mutual = with_certificate(&files, |c| {
        let nat = push_node(c, TermNode::Global("Nat".into()));
        let a = push_node(c, TermNode::Global("/s:\"cycle\"/s:\"b\"".into()));
        let b = push_node(c, TermNode::Global("/s:\"cycle\"/s:\"a\"".into()));
        c.entries.push(EnvironmentEntry::Definition {
            name: "/s:\"cycle\"/s:\"a\"".into(),
            level_params: vec![],
            ty: nat,
            body: a,
        });
        c.entries.push(EnvironmentEntry::Definition {
            name: "/s:\"cycle\"/s:\"b\"".into(),
            level_params: vec![],
            ty: nat,
            body: b,
        });
    });
    assert_refused(&replay(&mutual), "primary_check", "dependency_cycle");
}

// -------------------------------------------------------------------------------------------------
// The package as a set of files.
// -------------------------------------------------------------------------------------------------

#[test]
fn unknown_missing_and_unsafe_files_are_refused() {
    let files = built(T06_002, &t06("symmViaRec"));
    let mut extra = files.clone();
    extra.insert("notes.txt".into(), b"trust me".to_vec());
    assert_refused(&replay(&extra), "fresh_replay", "unknown_file");
    let mut listed_extra = extra.clone();
    redigest(&mut listed_extra);
    assert_refused(&replay(&listed_extra), "fresh_replay", "unknown_file");
    let mut missing = files.clone();
    missing.remove(CORRESPONDENCE);
    assert_refused(&replay(&missing), "fresh_replay", "missing_file");
    let mut no_manifest = files.clone();
    no_manifest.remove(MANIFEST);
    assert_refused(&replay(&no_manifest), "fresh_replay", "missing_file");
    for path in [
        "../escape.json",
        "/etc/passwd",
        "source/../../x",
        "a\\b",
        ".hidden",
        "source//x",
    ] {
        let mut hostile = files.clone();
        hostile.insert(path.into(), b"x".to_vec());
        assert_refused(&replay(&hostile), "fresh_replay", "unsafe_path");
    }
}

#[test]
fn the_manifest_cannot_misstate_the_assumptions() {
    let files = built(T06_002, &t06("viaMpr"));
    let claims_more = with_manifest(&files, |m| {
        m.assumptions.transitive = vec!["/s:\"propext\"".into()]
    });
    assert_refused(
        &replay(&claims_more),
        "assumption_policy",
        "manifest_mismatch",
    );
}

#[test]
fn publication_is_atomic_and_never_replaces_a_result() {
    let files = built(T06_002, &t06("symmViaRec"));
    let dir = scratch_dir("publish");
    package::publish(&dir, &files).unwrap();
    assert_eq!(
        read_directory(&dir, &PackageLimits::default()).unwrap(),
        files
    );
    let error = package::publish(&dir, &files).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
    let parent = dir.parent().unwrap();
    let stem = dir.file_name().unwrap().to_str().unwrap();
    let leftovers: Vec<_> = std::fs::read_dir(parent)
        .unwrap()
        .flatten()
        .filter(|e| {
            e.file_name()
                .to_str()
                .is_some_and(|n| n.starts_with(&format!(".{stem}.staging")))
        })
        .collect();
    assert!(leftovers.is_empty(), "no staging directory survives");
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink("/etc/hostname", dir.join("link")).unwrap();
        assert!(read_directory(&dir, &PackageLimits::default())
            .unwrap_err()
            .contains("symbolic link"));
    }
    std::fs::remove_dir_all(&dir).unwrap();
    let mut hostile = files.clone();
    hostile.insert("../outside".into(), vec![]);
    assert!(package::publish(&scratch_dir("hostile"), &hostile).is_err());
}

#[test]
fn a_package_is_bound_to_its_own_export() {
    // A clean certificate carried with a different export: the binding reads the carried export.
    let files = built(T06_002, &t06("symmViaRec"));
    let mut foreign = files.clone();
    foreign.insert(EXPORT.into(), T00_014.as_bytes().to_vec());
    redigest(&mut foreign);
    let receipt = replay(&foreign);
    assert_eq!(
        receipt.status("translated"),
        StageStatus::Failed,
        "{receipt:#?}"
    );
}

#[test]
fn a_target_that_is_not_proved_is_never_packaged() {
    let mut e = Export::new();
    let p = e.name("P", 0);
    let prop = e.sort(0);
    e.axiom(p, prop, &[]);
    let pc = e.constant(p, &[]);
    let a = e.name("A", 0);
    e.axiom(a, pc, &[]);
    let verdict = build(&e.text(), "A", &BuildOptions::default())
        .err()
        .expect("an axiom is not a result");
    assert_eq!(verdict.reason().code(), DiagnosticCode::InvalidProof);
}

// -------------------------------------------------------------------------------------------------
// Lean's `#print axioms` and the package's transitive assumptions: one definition, both sides.
// -------------------------------------------------------------------------------------------------

fn lean_print_axioms(root: &str) -> Vec<String> {
    let provenance: serde_json::Value = serde_json::from_str(W05_PROVENANCE).unwrap();
    provenance["exports"][root]["lean_print_axioms"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a.as_str().unwrap().to_string())
        .collect()
}

#[test]
fn observed_print_axioms_agree_with_the_package_on_every_genuine_root() {
    for (root, export, _) in W05 {
        let lean = lean_print_axioms(root);
        let build = package::build_observed(
            export,
            &format!("W05.{root}"),
            &BuildOptions::default(),
            Some(&lean),
        )
        .unwrap();
        let m = manifest(&build.files);
        assert_eq!(
            m.assumptions.lean_print_axioms.as_ref(),
            Some(&lean),
            "{root}: recorded"
        );
        let policy = build.receipt.stage("assumption_policy").unwrap();
        assert_ne!(
            policy.code.as_deref(),
            Some("assumption_disagreement"),
            "{root}: {policy:?}"
        );
        let replayed = replay(&build.files);
        assert_ne!(
            replayed.stage("assumption_policy").unwrap().code.as_deref(),
            Some("assumption_disagreement"),
            "{root}"
        );
    }
}

#[test]
fn a_disagreement_with_leans_print_axioms_is_refused_both_ways() {
    // Lean says nothing, the proof rests on propext.
    let silent = package::build_observed(
        W05_PROPEXT,
        "W05.usesPropext",
        &BuildOptions::default(),
        Some(&[]),
    )
    .unwrap();
    assert_eq!(
        silent
            .receipt
            .stage("assumption_policy")
            .unwrap()
            .code
            .as_deref(),
        Some("assumption_disagreement")
    );
    let verdict = package::assemble_observed(
        W05_PROPEXT,
        "W05.usesPropext",
        &BuildOptions::default(),
        Some(&[]),
    )
    .unwrap_err();
    assert_eq!(
        verdict.reason().code(),
        DiagnosticCode::AssumptionDisagreement,
        "{verdict:?}"
    );
    // Lean names an axiom the proof does not rest on.
    let extra = vec!["propext".to_string(), "Classical.choice".to_string()];
    let receipt = package::build_observed(
        W05_CHOICE,
        "W05.usesChoice",
        &BuildOptions::default(),
        Some(&extra),
    )
    .unwrap()
    .receipt;
    assert_eq!(
        receipt.stage("assumption_policy").unwrap().code.as_deref(),
        Some("assumption_disagreement")
    );
    // A manifest that records a forged Lean report is refused by replay.
    let honest = package::build_observed(
        W05_PROPEXT,
        "W05.usesPropext",
        &BuildOptions::default(),
        Some(&lean_print_axioms("usesPropext")),
    )
    .unwrap()
    .files;
    let forged = with_manifest(&honest, |m| m.assumptions.lean_print_axioms = Some(vec![]));
    let receipt = replay(&forged);
    assert_eq!(
        receipt.stage("assumption_policy").unwrap().code.as_deref(),
        Some("assumption_disagreement"),
        "{receipt:#?}"
    );
}

#[test]
fn the_trusted_compiler_and_reduce_bool_are_never_allowed() {
    for (namespace, name) in [
        ("Lean", "trustCompiler"),
        ("Lean", "ofReduceBool"),
        ("", "sorryAx"),
    ] {
        let mut e = Export::new();
        let p = e.name("P", 0);
        let prop = e.sort(0);
        e.axiom(p, prop, &[]);
        let pc = e.constant(p, &[]);
        let root = if namespace.is_empty() {
            0
        } else {
            e.name(namespace, 0)
        };
        let axiom = e.name(name, root);
        e.axiom(axiom, pc, &[]);
        let alias = e.name("alias", 0);
        let ac = e.constant(axiom, &[]);
        e.def(alias, pc, ac);
        let t = e.name("T", 0);
        let alias_c = e.constant(alias, &[]);
        e.thm(t, pc, alias_c);
        let verdict = assemble(&e.text(), "T").expect_err(name);
        assert_eq!(
            verdict.reason().code(),
            DiagnosticCode::DisallowedAssumption,
            "{name}: {verdict:?}"
        );
        let receipt = replay(&built(&e.text(), "T"));
        assert_eq!(
            receipt.status("assumption_policy"),
            StageStatus::Failed,
            "{name}"
        );
        assert!(
            receipt
                .stage("assumption_policy")
                .unwrap()
                .detail
                .contains(name),
            "{name}: {receipt:#?}"
        );
    }
}

#[test]
fn a_multi_module_projects_sources_keep_their_layout_and_are_bound_by_digest() {
    let mut options = BuildOptions::default();
    options
        .lean_sources
        .insert("W05/Package.lean".into(), W05_SOURCE.as_bytes().to_vec());
    options
        .lean_sources
        .insert("Root.lean".into(), b"import W05.Package\n".to_vec());
    let files = package::assemble_with(W05_CLEAN, "W05.clean", &options)
        .unwrap_or_else(|v| panic!("{v:?}"));
    assert_eq!(files["source/W05/Package.lean"], W05_SOURCE.as_bytes());
    assert!(files.contains_key("source/Root.lean"));
    assert_eq!(
        manifest(&files).source.lean_sources,
        vec![
            "source/Root.lean".to_string(),
            "source/W05/Package.lean".to_string()
        ]
    );
    assert!(replay(&files).replayed());
    let mut edited = files.clone();
    edited
        .get_mut("source/W05/Package.lean")
        .unwrap()
        .push(b'\n');
    assert_refused(&replay(&edited), "fresh_replay", "digest_mismatch");
    for hostile in [
        "../escape.lean",
        "W05/../../x.lean",
        "/abs.lean",
        "W05/.hidden.lean",
        "notes.txt",
    ] {
        let mut options = BuildOptions::default();
        options
            .lean_sources
            .insert(hostile.into(), b"-- x\n".to_vec());
        assert!(
            package::build(W05_CLEAN, "W05.clean", &options).is_err(),
            "{hostile}"
        );
    }
}
